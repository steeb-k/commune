pub(crate) use commune_core::session::UserPresence;
use commune_core::session::{Presence as CorePresence, PresenceList as CorePresenceList};
use futures_util::Stream;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{OwnedUserId, UserId};
use tracing::debug;

use super::Session;
use crate::{Application, spawn, spawn_tokio};

/// The settings key for whether we tell the homeserver that we are here.
const SHARE_PRESENCE_KEY: &str = "share-presence";

/// What a homeserver says about whether a user is around.
///
/// The core's [`Presence`](CorePresence) as a `glib` enum, for the
/// properties that carry it. The Presence module is optional and most
/// homeservers ship it switched off, so [`Self::Unknown`] is both the default
/// and the common case. It is kept apart from [`Self::Offline`] on purpose: a
/// server with presence enabled says "offline" out loud, and a server without
/// it says nothing at all. The two look the same in the interface and must not
/// be confused in the model.
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
        CorePresence::from(self).is_visible()
    }
}

impl From<CorePresence> for Presence {
    fn from(value: CorePresence) -> Self {
        match value {
            CorePresence::Unknown => Self::Unknown,
            CorePresence::Online => Self::Online,
            CorePresence::Unavailable => Self::Unavailable,
            CorePresence::Offline => Self::Offline,
        }
    }
}

impl From<Presence> for CorePresence {
    fn from(value: Presence) -> Self {
        match value {
            Presence::Unknown => Self::Unknown,
            Presence::Online => Self::Online,
            Presence::Unavailable => Self::Unavailable,
            Presence::Offline => Self::Offline,
        }
    }
}

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PresenceList)]
    pub struct PresenceList {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        pub(super) session: glib::WeakRef<Session>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PresenceList {
        const NAME: &'static str = "PresenceList";
        type Type = super::PresenceList;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PresenceList {}

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

        /// Start following the setting for whether we say that we are here.
        ///
        /// The presence arriving in sync is the core's to keep; the setting
        /// is a `GSettings` key, so it is this object's to watch.
        fn init(&self) {
            if self.session.upgrade().is_none() {
                return;
            }

            self.watch_share_setting();
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
            let Some(core) = self.obj().core() else {
                return;
            };

            let share = Application::default()
                .settings()
                .boolean(SHARE_PRESENCE_KEY);

            spawn!(async move {
                let handle = spawn_tokio!(async move { core.share_own_presence(share).await });

                if let Err(error) = handle.await.expect("task was not aborted") {
                    // A homeserver without the Presence module refuses this,
                    // and that is the common case rather than a fault.
                    debug!("Could not set our own presence: {error}");
                }
            });
        }
    }
}

glib::wrapper! {
    /// What the homeserver has said about who is around.
    ///
    /// The list is the core's; this presents it, and keeps the setting for
    /// whether we say that we are here.
    pub struct PresenceList(ObjectSubclass<imp::PresenceList>);
}

impl PresenceList {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The core's list, while the session is there.
    fn core(&self) -> Option<CorePresenceList> {
        self.session()
            .map(|session| session.core().presence_list().clone())
    }

    /// What is known about the presence of the user with the given ID.
    pub(crate) fn get(&self, user_id: &UserId) -> UserPresence {
        self.core()
            .map(|core| core.get(user_id))
            .unwrap_or_default()
    }

    /// Subscribe to what is known about the presence of the user with the
    /// given ID.
    ///
    /// `None` until sync or the store has said something about them.
    pub(crate) fn subscribe(
        &self,
        user_id: &UserId,
    ) -> Option<impl Stream<Item = Option<UserPresence>> + use<>> {
        self.core().map(|core| core.subscribe(user_id))
    }

    /// Load what the store already knows about the given user.
    ///
    /// Sync only sends presence when it changes, so a user we have never seen
    /// change would have no presence at all until they did something. The
    /// store carries what earlier syncs delivered.
    pub(crate) fn load(&self, user_id: OwnedUserId) {
        let Some(core) = self.core() else {
            return;
        };

        spawn_tokio!(async move { core.load(&user_id).await });
    }
}

impl Default for PresenceList {
    fn default() -> Self {
        Self::new()
    }
}
