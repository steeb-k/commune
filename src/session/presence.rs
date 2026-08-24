use std::collections::HashMap;

use gtk::{
    glib,
    glib::{clone, closure_local, subclass::Signal},
    prelude::*,
    subclass::prelude::*,
};
use ruma::{
    OwnedUserId, UserId,
    events::presence::{PresenceEvent, PresenceEventContent},
    presence::PresenceState,
};
use tracing::{debug, error};

use super::Session;
use crate::{Application, spawn, spawn_tokio};

/// The settings key for whether we tell the homeserver that we are here.
const SHARE_PRESENCE_KEY: &str = "share-presence";

/// What a homeserver says about whether a user is around.
///
/// The Presence module is optional and most homeservers ship it switched off,
/// so [`Self::Unknown`] is both the default and the common case. It is kept
/// apart from [`Self::Offline`] on purpose: a server with presence enabled
/// says "offline" out loud, and a server without it says nothing at all. The
/// two look the same in the interface and must not be confused in the model.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "Presence")]
pub enum Presence {
    /// The homeserver has told us nothing about this user.
    #[default]
    Unknown,
    /// The user is around.
    Online,
    /// The user is idle.
    Unavailable,
    /// The user is away.
    Offline,
}

impl Presence {
    /// Whether this is worth drawing a badge for.
    ///
    /// Only being around and being idle are. Offline and unknown both mean
    /// "not known to be here", and drawing them apart would ask people to tell
    /// a server that keeps presence from a server that does not.
    pub(crate) fn is_visible(self) -> bool {
        matches!(self, Self::Online | Self::Unavailable)
    }
}

impl From<&PresenceState> for Presence {
    fn from(state: &PresenceState) -> Self {
        match state {
            PresenceState::Online => Self::Online,
            PresenceState::Unavailable => Self::Unavailable,
            PresenceState::Offline => Self::Offline,
            // The state is non-exhaustive: a state we do not know is not a
            // state we can draw.
            _ => Self::Unknown,
        }
    }
}

/// What is known about one user's presence.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct UserPresence {
    /// Whether the user is around.
    pub(crate) presence: Presence,
    /// The message the user set to go with it, if any.
    pub(crate) status_message: Option<String>,
    /// How long ago the user last did something, in milliseconds.
    ///
    /// Only meaningful together with `currently_active`, and only present when
    /// the homeserver chooses to send it.
    pub(crate) last_active_ago: Option<u64>,
    /// Whether the user is currently active.
    pub(crate) currently_active: bool,
}

impl From<&PresenceEventContent> for UserPresence {
    fn from(content: &PresenceEventContent) -> Self {
        Self {
            presence: (&content.presence).into(),
            status_message: content
                .status_msg
                .as_ref()
                .map(|message| message.trim().to_owned())
                .filter(|message| !message.is_empty()),
            last_active_ago: content.last_active_ago.map(Into::into),
            currently_active: content.currently_active.unwrap_or_default(),
        }
    }
}

mod imp {
    use std::{cell::RefCell, sync::LazyLock};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PresenceList)]
    pub struct PresenceList {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        pub(super) session: glib::WeakRef<Session>,
        /// What is known about each user, by user ID.
        pub(super) presences: RefCell<HashMap<OwnedUserId, UserPresence>>,
        drop_guard: RefCell<Option<matrix_sdk::event_handler::EventHandlerDropGuard>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PresenceList {
        const NAME: &'static str = "PresenceList";
        type Type = super::PresenceList;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PresenceList {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("changed")
                        .param_types([String::static_type()])
                        .build(),
                ]
            });
            SIGNALS.as_ref()
        }
    }

    impl PresenceList {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            self.session.set(session);

            self.init();
            self.obj().notify_session();
        }

        /// Listen for presence arriving in sync.
        ///
        /// Presence is a top-level field of the sync response rather than
        /// anything belonging to a room, and this client uses classic sync, so
        /// an event handler is the whole of the live half.
        fn init(&self) {
            self.drop_guard.take();

            let Some(session) = self.session.upgrade() else {
                return;
            };

            self.watch_share_setting();

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let handle = session
                .client()
                .add_event_handler(move |event: PresenceEvent| {
                    let obj_weak = obj_weak.clone();
                    async move {
                        let user_id = event.sender.clone();
                        let presence = UserPresence::from(&event.content);

                        let ctx = glib::MainContext::default();
                        ctx.spawn(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.imp().update(user_id, presence);
                            }
                        });
                    }
                });
            self.drop_guard
                .replace(Some(session.client().event_handler_drop_guard(handle)));
        }

        /// Follow the setting for whether we say that we are here.
        ///
        /// This is not new behaviour being switched on — it is behaviour being
        /// made switchable. The `set_presence` parameter of `/sync` defaults
        /// to `online` when a client omits it, and the SDK's client-owned
        /// value defaults to `Online` too, so every sync this client has ever
        /// made has told the homeserver we are here. The setting defaults to
        /// on for that reason: turning it off by default would quietly change
        /// what other people see, which is not ours to do. The switch is the
        /// off that did not exist before.
        fn watch_share_setting(&self) {
            let settings = Application::default().settings();

            settings.connect_changed(
                Some(SHARE_PRESENCE_KEY),
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _| {
                        imp.apply_share_setting();
                    }
                ),
            );

            self.apply_share_setting();
        }

        /// Tell the homeserver whether we are here, per the setting.
        fn apply_share_setting(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let share = Application::default()
                .settings()
                .boolean(SHARE_PRESENCE_KEY);
            let presence = if share {
                PresenceState::Online
            } else {
                PresenceState::Offline
            };

            let client = session.client();
            spawn!(async move {
                // `immediate` so the change is visible without waiting for the
                // next sync, and because the setting being flipped is somebody
                // asking for exactly that.
                let handle =
                    spawn_tokio!(async move { client.set_presence(presence, None, true).await });

                if let Err(error) = handle.await.expect("task was not aborted") {
                    // A homeserver without the Presence module refuses this,
                    // and that is the common case rather than a fault.
                    debug!("Could not set our own presence: {error}");
                }
            });
        }

        /// Record what is known about the given user, and say so if it changed.
        pub(super) fn update(&self, user_id: OwnedUserId, presence: UserPresence) {
            if self
                .presences
                .borrow()
                .get(&user_id)
                .is_some_and(|known| *known == presence)
            {
                return;
            }

            let user_id_string = user_id.to_string();
            self.presences.borrow_mut().insert(user_id, presence);

            self.obj().emit_by_name::<()>("changed", &[&user_id_string]);
        }
    }
}

glib::wrapper! {
    /// What the homeserver has said about who is around.
    pub struct PresenceList(ObjectSubclass<imp::PresenceList>);
}

impl PresenceList {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// What is known about the presence of the user with the given ID.
    pub(crate) fn get(&self, user_id: &UserId) -> UserPresence {
        self.imp()
            .presences
            .borrow()
            .get(user_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Load what the store already knows about the given user.
    ///
    /// Sync only sends presence when it changes, so a user we have never seen
    /// change would have no presence at all until they did something. The
    /// store carries what earlier syncs delivered.
    pub(crate) fn load(&self, user_id: OwnedUserId) {
        if self.imp().presences.borrow().contains_key(&user_id) {
            return;
        }
        let Some(session) = self.session() else {
            return;
        };

        let client = session.client();
        let list = self.clone();
        spawn!(async move {
            load_from_store(list, client, user_id).await;
        });
    }

    /// Connect to the signal emitted when what is known about a user changes.
    pub(crate) fn connect_changed<F: Fn(&Self, String) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "changed",
            true,
            closure_local!(move |obj: Self, user_id: String| {
                f(&obj, user_id);
            }),
        )
    }
}

/// Read the presence of the given user out of the store and record it.
async fn load_from_store(list: PresenceList, client: matrix_sdk::Client, user_id: OwnedUserId) {
    let user_id_clone = user_id.clone();
    let handle = spawn_tokio!(async move {
        client
            .state_store()
            .get_presence_event(&user_id_clone)
            .await
    });

    let raw = match handle.await.expect("task was not aborted") {
        Ok(Some(raw)) => raw,
        // A homeserver with presence switched off never sends any, which is
        // the common case and not worth a line in the log.
        Ok(None) => return,
        Err(error) => {
            error!("Could not read the stored presence of {user_id}: {error}");
            return;
        }
    };

    match raw.deserialize() {
        Ok(event) => list
            .imp()
            .update(user_id, UserPresence::from(&event.content)),
        Err(error) => {
            error!("Could not deserialize the stored presence of {user_id}: {error}");
        }
    }
}

impl Default for PresenceList {
    fn default() -> Self {
        Self::new()
    }
}
