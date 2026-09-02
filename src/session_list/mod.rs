use commune_core::{
    VectorDiff,
    session_list::{SessionEntry, SessionList as CoreSessionList, SessionListError},
    settings::SessionListSettings,
};
use futures_util::{StreamExt, select_biased};
use gettextrs::gettext;
use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use indexmap::map::IndexMap;

mod failed_session;
mod new_session;
mod session_info;

pub(crate) use self::{failed_session::*, new_session::*, session_info::*};
use crate::{
    core_bridge::list_model::{ItemsChange, apply_diff},
    prelude::*,
    session::Session,
    spawn, spawn_tokio,
    utils::{LoadingState, OneshotNotifier},
};

/// The stage an entry has reached.
///
/// Part of a row's key: a session restored from a stored one is another
/// row in the same place, not the same row, since the object presenting it
/// is another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum EntryKind {
    /// Being restored.
    Loading,
    /// Could not be restored.
    Failed,
    /// Running.
    Ready,
}

/// The key of a row: the session's ID and the stage its entry has reached.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RowKey {
    /// The local ID of the session.
    session_id: String,
    /// The stage of the entry.
    kind: EntryKind,
}

impl RowKey {
    /// The key of the given entry.
    fn of(entry: &SessionEntry) -> Self {
        let kind = match entry {
            SessionEntry::Loading(_) => EntryKind::Loading,
            SessionEntry::Failed { .. } => EntryKind::Failed,
            SessionEntry::Ready(_) => EntryKind::Ready,
        };

        Self {
            session_id: entry.session_id().to_owned(),
            kind,
        }
    }
}

/// The sentences for the list's error.
fn render_error(error: &SessionListError) -> String {
    let detail = match error {
        SessionListError::Restore(error) => error.to_user_facing(),
        SessionListError::DataDirectory => {
            gettext("An unexpected error happened while accessing the data directory")
        }
    };

    format!(
        "{}\n\n{detail}",
        gettext("Could not restore previous sessions")
    )
}

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::SessionList)]
    pub struct SessionList {
        /// The rows, keyed by session ID and stage, in the core's order.
        pub(super) list: RefCell<IndexMap<RowKey, SessionInfo>>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        state: Cell<LoadingState>,
        /// The error message, if state is set to `LoadingState::Error`.
        #[property(get, nullable)]
        error: RefCell<Option<String>>,
        /// The core's list, which this presents.
        pub(super) core: CoreSessionList,
        /// Whether this list is empty.
        #[property(get = Self::is_empty)]
        is_empty: PhantomData<bool>,
        /// The session the login flow is inserting, so that its row is the
        /// object the flow holds.
        pub(super) pending: RefCell<Option<Session>>,
        /// Notified after every change to the rows.
        pub(super) changed: OneshotNotifier,
    }

    impl Default for SessionList {
        fn default() -> Self {
            Self {
                list: RefCell::default(),
                state: Cell::default(),
                error: RefCell::default(),
                core: CoreSessionList::default(),
                is_empty: PhantomData,
                pending: RefCell::default(),
                changed: OneshotNotifier::new("session_list_changed"),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionList {
        const NAME: &'static str = "SessionList";
        type Type = super::SessionList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for SessionList {
        fn constructed(&self) {
            self.parent_constructed();
            self.watch_core();
        }
    }

    impl ListModelImpl for SessionList {
        fn item_type(&self) -> glib::Type {
            SessionInfo::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get_index(position as usize)
                .map(|(_, v)| v.clone().upcast())
        }
    }

    impl SessionList {
        /// Whether this list is empty.
        fn is_empty(&self) -> bool {
            self.list.borrow().is_empty()
        }

        /// Follow the core's entries and state.
        ///
        /// One task for both, polling the entries first: the core inserts
        /// the sessions it is restoring before it says it is ready, and the
        /// window acts on ready by looking the rows up.
        fn watch_core(&self) {
            let (entries, diffs) = self.core.subscribe_entries();
            let mut diffs = diffs.fuse();
            let mut states = self.core.subscribe_state().fuse();
            let weak = self.obj().downgrade();

            spawn!(async move {
                if let Some(obj) = weak.upgrade() {
                    obj.imp().apply_diff(VectorDiff::Append { values: entries });
                    obj.imp().update_state();
                }

                loop {
                    select_biased! {
                        diff = diffs.next() => {
                            let (Some(diff), Some(obj)) = (diff, weak.upgrade()) else {
                                break;
                            };
                            obj.imp().apply_diff(diff);
                        }
                        state = states.next() => {
                            let (Some(_), Some(obj)) = (state, weak.upgrade()) else {
                                break;
                            };
                            obj.imp().update_state();
                        }
                    }
                }
            });
        }

        /// Mirror the core's state, and its error when there is one.
        fn update_state(&self) {
            let state = self.core.state().into();

            if state == LoadingState::Error {
                let message = self.core.error().map(|error| render_error(&error));
                self.error.replace(message);
                self.obj().notify_error();
            }

            if self.state.get() == state {
                return;
            }

            self.state.set(state);
            self.obj().notify_state();
        }

        /// Apply a change to the core's entries.
        fn apply_diff(&self, diff: VectorDiff<SessionEntry>) {
            let was_empty = self.is_empty();

            // Wrap first, outside the borrow.
            let diff = diff.map(|entry| {
                let key = RowKey::of(&entry);
                let wrapper = self.wrap(&key, entry);
                (key, wrapper)
            });

            let changes = match diff {
                VectorDiff::Set {
                    index,
                    value: (key, wrapper),
                } => self.replace_at(index, key, wrapper),
                diff => apply_diff(&mut self.list.borrow_mut(), diff),
            };

            let obj = self.obj();
            for change in &changes {
                obj.items_changed(change.position, change.removed, change.added);
            }

            if was_empty != self.is_empty() {
                obj.notify_is_empty();
            }

            self.changed.notify();
        }

        /// Replace the row at the given index, as one change.
        ///
        /// A session restored in place of the stored one it came from
        /// keeps the selection where it is, which two changes would not.
        fn replace_at(&self, index: usize, key: RowKey, wrapper: SessionInfo) -> Vec<ItemsChange> {
            let mut list = self.list.borrow_mut();

            match list.get_index(index) {
                Some((old_key, _)) if *old_key == key => Vec::new(),
                Some(_) => {
                    list.shift_remove_index(index);
                    list.shift_insert(index, key, wrapper);

                    vec![ItemsChange {
                        position: index as u32,
                        removed: 1,
                        added: 1,
                    }]
                }
                None => apply_diff(
                    &mut list,
                    VectorDiff::Set {
                        index,
                        value: (key, wrapper),
                    },
                ),
            }
        }

        /// The row for the given entry: the one this list has, the one the
        /// login flow is inserting, or a new one.
        fn wrap(&self, key: &RowKey, entry: SessionEntry) -> SessionInfo {
            if let Some(existing) = self.list.borrow().get(key) {
                return existing.clone();
            }

            match entry {
                SessionEntry::Loading(info) => NewSession::new(&info.into()).upcast(),
                SessionEntry::Failed { info, error } => {
                    FailedSession::new(&info.into(), error).upcast()
                }
                SessionEntry::Ready(core) => {
                    let pending = self
                        .pending
                        .borrow_mut()
                        .take_if(|session| session.session_id() == core.session_id());

                    match pending {
                        Some(session) => session.upcast(),
                        None => Session::from_core(core).upcast(),
                    }
                }
            }
        }

        /// The index of the running session with the given ID, if any.
        fn ready_index(&self, session_id: &str) -> Option<usize> {
            self.list.borrow().get_index_of(&RowKey {
                session_id: session_id.to_owned(),
                kind: EntryKind::Ready,
            })
        }

        /// Insert the given session into the list.
        ///
        /// If a session with the same ID already exists, it is replaced.
        ///
        /// Returns the index of the session, once its row is there.
        pub(super) async fn insert(&self, session: Session) -> usize {
            let session_id = session.session_id().to_owned();
            let core = session.core().clone();

            self.pending.replace(Some(session));
            self.core.insert(SessionEntry::Ready(core));

            loop {
                if let Some(index) = self.ready_index(&session_id) {
                    return index;
                }

                self.changed.listen().await;
            }
        }
    }
}

glib::wrapper! {
    /// List of all logged in sessions.
    pub struct SessionList(ObjectSubclass<imp::SessionList>)
        @implements gio::ListModel;
}

impl SessionList {
    /// Create a new empty `SessionList`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The settings of the sessions.
    pub(crate) fn settings(&self) -> SessionListSettings {
        self.imp().core.settings().clone()
    }

    /// The session with the given ID, if any.
    pub(crate) fn get(&self, session_id: &str) -> Option<SessionInfo> {
        self.imp()
            .list
            .borrow()
            .iter()
            .find(|(key, _)| key.session_id == session_id)
            .map(|(_, v)| v.clone())
    }

    /// The index of the session with the given ID, if any.
    pub(crate) fn index(&self, session_id: &str) -> Option<usize> {
        self.imp()
            .list
            .borrow()
            .iter()
            .position(|(key, _)| key.session_id == session_id)
    }

    /// The first session in the list, if any.
    pub(crate) fn first(&self) -> Option<SessionInfo> {
        self.imp().list.borrow().first().map(|(_, v)| v.clone())
    }

    /// Insert the given session into the list.
    ///
    /// If a session with the same ID already exists, it is replaced.
    ///
    /// Returns the index of the session, once its row is there.
    pub(crate) async fn insert(&self, session: Session) -> usize {
        self.imp().insert(session).await
    }

    /// Restore the logged-in sessions.
    pub(crate) async fn restore_sessions(&self) {
        let core = self.imp().core.clone();
        spawn_tokio!(async move { core.restore_sessions().await })
            .await
            .expect("task was not aborted");
    }
}

impl Default for SessionList {
    fn default() -> Self {
        Self::new()
    }
}
