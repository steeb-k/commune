use commune_core::session::{SpaceChild as CoreSpaceChild, SpaceChildren as CoreSpaceChildren};
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::OwnedRoomId;
use tokio::task::AbortHandle;

use super::RemoteRoom;
use crate::{session::Session, spawn, spawn_tokio, utils::LoadingState};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SpaceChildren)]
    pub struct SpaceChildren {
        /// The rooms directly inside the space.
        #[property(get = Self::list_owned)]
        list: OnceCell<gio::ListStore>,
        /// The session to make the requests with.
        session: glib::WeakRef<Session>,
        /// The ID of the space these rooms are inside.
        room_id: RefCell<Option<OwnedRoomId>>,
        /// The hierarchy, once the core walked it.
        hierarchy: RefCell<Option<CoreSpaceChildren>>,
        /// Whether the listing stopped before the end of the space.
        #[property(get)]
        is_truncated: Cell<bool>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// The abort handle for the current walk.
        abort_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpaceChildren {
        const NAME: &'static str = "SpaceChildren";
        type Type = super::SpaceChildren;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SpaceChildren {
        fn dispose(&self) {
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }
        }
    }

    impl SpaceChildren {
        /// The rooms directly inside the space.
        fn list(&self) -> &gio::ListStore {
            self.list
                .get_or_init(gio::ListStore::new::<super::SpaceChild>)
        }

        /// The owned list of rooms directly inside the space.
        fn list_owned(&self) -> gio::ListStore {
            self.list().clone()
        }

        /// Set the space to list the rooms of.
        pub(super) fn set_space(&self, session: &Session, room_id: OwnedRoomId) {
            if self.room_id.borrow().as_ref() == Some(&room_id) {
                return;
            }

            self.session.set(Some(session));
            self.room_id.replace(Some(room_id));

            self.reload();
        }

        /// List the rooms inside the current space again, from the beginning.
        pub(super) fn reload(&self) {
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }

        /// Set the loading state.
        fn set_loading_state(&self, state: LoadingState) {
            if self.loading_state.get() == state {
                return;
            }

            self.loading_state.set(state);
            self.obj().notify_loading_state();
        }

        /// Set whether the listing stopped before the end of the space.
        fn set_is_truncated(&self, is_truncated: bool) {
            if self.is_truncated.get() == is_truncated {
                return;
            }

            self.is_truncated.set(is_truncated);
            self.obj().notify_is_truncated();
        }

        /// Have the core walk the whole hierarchy of the current space, and
        /// present what it found.
        async fn load(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let Some(room_id) = self.room_id.borrow().clone() else {
                return;
            };

            // Abort any listing of a previous space and forget what it found.
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }
            self.list().remove_all();
            self.hierarchy.take();
            self.set_is_truncated(false);
            self.set_loading_state(LoadingState::Loading);

            let core = session.core().clone();
            let room_id_clone = room_id.clone();
            let handle =
                spawn_tokio!(async move { CoreSpaceChildren::load(&core, room_id_clone).await });
            self.abort_handle.replace(Some(handle.abort_handle()));

            let Ok(result) = handle.await else {
                // The walk was aborted.
                return;
            };

            self.abort_handle.take();

            if self.room_id.borrow().as_deref() != Some(&*room_id) {
                // We are listing a different space now, ignore the response.
                return;
            }

            let Ok(hierarchy) = result else {
                // Already logged by the core.
                self.set_loading_state(LoadingState::Error);
                return;
            };

            self.set_is_truncated(hierarchy.is_truncated());

            // The rows are built once the whole walk is done. A space whose own
            // chunk has not arrived yet looks like a space with nothing in it,
            // and `GtkTreeListModel` remembers the first answer it is given
            // about whether a row can be expanded.
            let children = self.rows(&session, hierarchy.children());
            self.hierarchy.replace(Some(hierarchy));

            self.list().extend_from_slice(&children);
            self.set_loading_state(LoadingState::Ready);
        }

        /// The rows for the given rooms of the hierarchy.
        fn rows(&self, session: &Session, children: Vec<CoreSpaceChild>) -> Vec<super::SpaceChild> {
            children
                .into_iter()
                .map(|child| super::SpaceChild::new(&self.obj(), session, child))
                .collect()
        }

        /// The rows for the rooms inside the given one, if it is a space that
        /// holds any.
        pub(super) fn children_of(
            &self,
            session: &Session,
            child: &CoreSpaceChild,
        ) -> Option<Vec<super::SpaceChild>> {
            let children = child.children(self.hierarchy.borrow().as_ref()?)?;
            Some(self.rows(session, children))
        }
    }
}

glib::wrapper! {
    /// The hierarchy of a space, as far as its homeserver will describe it.
    ///
    /// The whole tree is asked for at once — `/hierarchy` walks it depth-first
    /// and returns every room with the `m.space.child` events of the spaces
    /// among them — so opening a subspace costs no request and never waits.
    /// The listing stops after a fixed number of batches and says so. The walk
    /// is the core's; this presents it.
    pub struct SpaceChildren(ObjectSubclass<imp::SpaceChildren>);
}

impl SpaceChildren {
    /// Construct a new empty `SpaceChildren`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// List the rooms inside the given space.
    pub(crate) fn set_space(&self, session: &Session, room_id: OwnedRoomId) {
        self.imp().set_space(session, room_id);
    }

    /// List the rooms inside the current space again, from the beginning.
    pub(crate) fn reload(&self) {
        self.imp().reload();
    }
}

impl Default for SpaceChildren {
    fn default() -> Self {
        Self::new()
    }
}

mod child_imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SpaceChild)]
    pub struct SpaceChild {
        /// The room this is.
        #[property(get = Self::room_owned)]
        room: OnceCell<RemoteRoom>,
        /// The hierarchy this room was found in.
        ///
        /// A weak reference: the hierarchy owns the rows, directly or through
        /// the rows above them.
        pub(super) hierarchy: glib::WeakRef<super::SpaceChildren>,
        /// The row, as the core describes it: the room and the way down to
        /// it.
        pub(super) core: OnceCell<CoreSpaceChild>,
        /// The rooms inside this one, if it is a space that holds any.
        ///
        /// Asked for once: `GtkTreeListModel` remembers the first answer it
        /// gets about whether a row can be opened.
        pub(super) children: OnceCell<Option<gio::ListStore>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpaceChild {
        const NAME: &'static str = "SpaceChild";
        type Type = super::SpaceChild;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SpaceChild {}

    impl SpaceChild {
        /// Set what this row is.
        pub(super) fn init(
            &self,
            hierarchy: &super::SpaceChildren,
            room: RemoteRoom,
            core: CoreSpaceChild,
        ) {
            self.hierarchy.set(Some(hierarchy));
            let _ = self.room.set(room);
            let _ = self.core.set(core);
        }

        /// The room this row is.
        pub(super) fn room(&self) -> &RemoteRoom {
            self.room.get().expect("room should be initialized")
        }

        /// The owned room this row is.
        fn room_owned(&self) -> RemoteRoom {
            self.room().clone()
        }
    }
}

glib::wrapper! {
    /// One room inside a space, and the way down to it.
    ///
    /// The way down is what stops a hierarchy that points back at itself from
    /// being opened forever: a space that is already above this row is not
    /// offered again.
    pub struct SpaceChild(ObjectSubclass<child_imp::SpaceChild>);
}

impl SpaceChild {
    /// Construct a new `SpaceChild` for the given row of the core.
    fn new(hierarchy: &SpaceChildren, session: &Session, core: CoreSpaceChild) -> Self {
        let room = RemoteRoom::from_core(session, &core.room);

        let obj = glib::Object::new::<Self>();
        obj.imp().init(hierarchy, room, core);
        obj
    }

    /// The rooms inside this one, if it is a space that holds any.
    ///
    /// Returns `None` for a room that is not a space, for a space the walk
    /// found nothing in, and for a space that is already one of the ones
    /// walked through to get here.
    pub(crate) fn children(&self) -> Option<gio::ListStore> {
        let imp = self.imp();

        if let Some(children) = imp.children.get() {
            return children.clone();
        }

        let children = self.build_children();
        let _ = imp.children.set(children.clone());

        children
    }

    /// Build the list of rooms inside this one.
    fn build_children(&self) -> Option<gio::ListStore> {
        let imp = self.imp();
        let core = imp.core.get()?;
        let hierarchy = imp.hierarchy.upgrade()?;
        let session = imp.room().session()?;

        let children = hierarchy.imp().children_of(&session, core)?;

        let list = gio::ListStore::new::<Self>();
        list.extend_from_slice(&children);

        Some(list)
    }
}
