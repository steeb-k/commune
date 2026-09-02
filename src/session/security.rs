use gtk::{glib, prelude::*, subclass::prelude::*};
use tokio::task::AbortHandle;

use super::Session;
use crate::core_bridge::ObjectWatcher;

/// The state of the crypto identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "CryptoIdentityState")]
pub enum CryptoIdentityState {
    /// The state is not known yet.
    #[default]
    Unknown,
    /// The crypto identity does not exist.
    ///
    /// It means that cross-signing is not set up.
    Missing,
    /// There are no other verified sessions.
    LastManStanding,
    /// There are other verified sessions.
    OtherSessions,
}

impl From<commune_core::session::CryptoIdentityState> for CryptoIdentityState {
    fn from(value: commune_core::session::CryptoIdentityState) -> Self {
        use commune_core::session::CryptoIdentityState as Core;

        match value {
            Core::Unknown => Self::Unknown,
            Core::Missing => Self::Missing,
            Core::LastManStanding => Self::LastManStanding,
            Core::OtherSessions => Self::OtherSessions,
        }
    }
}

/// The state of the verification of the session.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "SessionVerificationState")]
pub enum SessionVerificationState {
    /// The state is not known yet.
    #[default]
    Unknown,
    /// The session is verified.
    Verified,
    /// The session is not verified.
    Unverified,
}

impl From<commune_core::session::SessionVerificationState> for SessionVerificationState {
    fn from(value: commune_core::session::SessionVerificationState) -> Self {
        use commune_core::session::SessionVerificationState as Core;

        match value {
            Core::Unknown => Self::Unknown,
            Core::Verified => Self::Verified,
            Core::Unverified => Self::Unverified,
        }
    }
}

/// The state of the recovery.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, glib::Enum)]
#[enum_type(name = "RecoveryState")]
pub enum RecoveryState {
    /// The state is not known yet.
    #[default]
    Unknown,
    /// Recovery is disabled.
    Disabled,
    /// Recovery is enabled and we have all the keys.
    Enabled,
    /// Recovery is enabled and we are missing some keys.
    Incomplete,
}

impl From<commune_core::session::RecoveryState> for RecoveryState {
    fn from(value: commune_core::session::RecoveryState) -> Self {
        use commune_core::session::RecoveryState as Core;

        match value {
            Core::Unknown => Self::Unknown,
            Core::Disabled => Self::Disabled,
            Core::Enabled => Self::Enabled,
            Core::Incomplete => Self::Incomplete,
        }
    }
}

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SessionSecurity)]
    pub struct SessionSecurity {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        session: glib::WeakRef<Session>,
        /// The state of the crypto identity for the current session.
        #[property(get, builder(CryptoIdentityState::default()))]
        crypto_identity_state: Cell<CryptoIdentityState>,
        /// The state of the verification for the current session.
        #[property(get, builder(SessionVerificationState::default()))]
        verification_state: Cell<SessionVerificationState>,
        /// The state of recovery for the current session.
        #[property(get, builder(RecoveryState::default()))]
        recovery_state: Cell<RecoveryState>,
        /// Whether all the cross-signing keys are available.
        #[property(get)]
        cross_signing_keys_available: Cell<bool>,
        /// Whether the room keys backup is enabled.
        #[property(get)]
        backup_enabled: Cell<bool>,
        /// Whether the room keys backup exists on the homeserver.
        #[property(get)]
        backup_exists_on_server: Cell<bool>,
        /// The task following the core's security.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionSecurity {
        const NAME: &'static str = "SessionSecurity";
        type Type = super::SessionSecurity;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SessionSecurity {
        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl SessionSecurity {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            self.session.set(session);
            self.obj().notify_session();

            self.watch_core();
        }

        /// Follow the core's security.
        fn watch_core(&self) {
            type S = super::SessionSecurity;

            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }

            let Some(session) = self.session.upgrade() else {
                return;
            };
            let core = session.core().security();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe_crypto_identity_state(), |obj: &S, state| {
                    obj.imp().set_crypto_identity_state(state.into());
                })
                .follow(core.subscribe_verification_state(), |obj: &S, state| {
                    obj.imp().set_verification_state(state.into());
                })
                .follow(core.subscribe_recovery_state(), |obj: &S, state| {
                    obj.imp().set_recovery_state(state.into());
                })
                .follow(
                    core.subscribe_cross_signing_keys_available(),
                    |obj: &S, available| {
                        obj.imp().set_cross_signing_keys_available(available);
                    },
                )
                .follow(core.subscribe_backup_enabled(), |obj: &S, enabled| {
                    obj.imp().set_backup_enabled(enabled);
                })
                .follow(
                    core.subscribe_backup_exists_on_server(),
                    |obj: &S, exists| {
                        obj.imp().set_backup_exists_on_server(exists);
                    },
                )
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.set_crypto_identity_state(core.crypto_identity_state().into());
            self.set_verification_state(core.verification_state().into());
            self.set_recovery_state(core.recovery_state().into());
            self.set_cross_signing_keys_available(core.cross_signing_keys_available());
            self.set_backup_enabled(core.backup_enabled());
            self.set_backup_exists_on_server(core.backup_exists_on_server());
        }

        /// Set the crypto identity state of the current session.
        fn set_crypto_identity_state(&self, state: CryptoIdentityState) {
            if self.crypto_identity_state.get() == state {
                return;
            }

            self.crypto_identity_state.set(state);
            self.obj().notify_crypto_identity_state();
        }

        /// Set the verification state of the current session.
        fn set_verification_state(&self, state: SessionVerificationState) {
            if self.verification_state.get() == state {
                return;
            }

            self.verification_state.set(state);
            self.obj().notify_verification_state();
        }

        /// Set the recovery state of the current session.
        fn set_recovery_state(&self, state: RecoveryState) {
            if self.recovery_state.get() == state {
                return;
            }

            self.recovery_state.set(state);
            self.obj().notify_recovery_state();
        }

        /// Set whether all the cross-signing keys are available.
        fn set_cross_signing_keys_available(&self, available: bool) {
            if self.cross_signing_keys_available.get() == available {
                return;
            }

            self.cross_signing_keys_available.set(available);
            self.obj().notify_cross_signing_keys_available();
        }

        /// Set whether the room keys backup is enabled.
        fn set_backup_enabled(&self, enabled: bool) {
            if self.backup_enabled.get() == enabled {
                return;
            }

            self.backup_enabled.set(enabled);
            self.obj().notify_backup_enabled();
        }

        /// Set whether the room keys backup exists on the homeserver.
        fn set_backup_exists_on_server(&self, exists: bool) {
            if self.backup_exists_on_server.get() == exists {
                return;
            }

            self.backup_exists_on_server.set(exists);
            self.obj().notify_backup_exists_on_server();
        }
    }
}

glib::wrapper! {
    /// Information about the security of a Matrix session.
    ///
    /// The information is the core's; this presents it.
    pub struct SessionSecurity(ObjectSubclass<imp::SessionSecurity>);
}

impl SessionSecurity {
    /// Construct a new empty `SessionSecurity`.
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for SessionSecurity {
    fn default() -> Self {
        Self::new()
    }
}
