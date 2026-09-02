use commune_core::session::{
    IdentityVerification as CoreIdentityVerification, VerificationList as CoreVerificationList,
};
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::RoomId;
use tokio::task::AbortHandle;
use tracing::error;

use super::{VerificationKey, VerificationState, load_supported_verification_methods};
use crate::{
    core_bridge::ObjectWatcher,
    prelude::*,
    session::{IdentityVerification, Member, Session, User},
    spawn, spawn_tokio,
};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        sync::LazyLock,
    };

    use glib::subclass::Signal;
    use indexmap::IndexMap;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::VerificationList)]
    pub struct VerificationList {
        /// The ongoing verification requests.
        pub(super) list: RefCell<IndexMap<VerificationKey, IdentityVerification>>,
        /// The current session.
        #[property(get, construct_only)]
        session: glib::WeakRef<Session>,
        /// The task following the core's list.
        watch_handle: RefCell<Option<AbortHandle>>,
        /// Whether a sync with the core's list is running, and whether one
        /// was asked for while it ran.
        syncing: Cell<bool>,
        sync_again: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VerificationList {
        const NAME: &'static str = "VerificationList";
        type Type = super::VerificationList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for VerificationList {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("secret-received").build()]);
            SIGNALS.as_ref()
        }

        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl ListModelImpl for VerificationList {
        fn item_type(&self) -> glib::Type {
            IdentityVerification::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get_index(position as usize)
                .map(|(_, item)| item.clone().upcast())
        }
    }

    impl VerificationList {
        /// The core's list, while the session is there.
        pub(super) fn core(&self) -> Option<CoreVerificationList> {
            self.session
                .upgrade()
                .map(|session| session.core().verification_list().clone())
        }

        /// Follow the core's list.
        pub(super) fn watch_core(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }

            let Some(core) = self.core() else {
                return;
            };

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(
                    core.subscribe_changed(),
                    |obj: &super::VerificationList, _| {
                        obj.imp().request_sync();
                    },
                )
                .spawn();
            self.watch_handle.replace(Some(handle));

            self.request_sync();
        }

        /// Sync with the core's list, once the sync that is running is done.
        fn request_sync(&self) {
            if self.syncing.get() {
                self.sync_again.set(true);
                return;
            }

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.syncing.set(true);

                    loop {
                        imp.sync_again.set(false);
                        imp.sync().await;

                        if !imp.sync_again.get() {
                            break;
                        }
                    }

                    imp.syncing.set(false);
                }
            ));
        }

        /// Bring the rows level with the core's list.
        ///
        /// A verification the core dropped goes, with its notification; one
        /// it gained is presented with the user it is shown as, which for an
        /// in-room request is the member, brought up to date first.
        pub(super) async fn sync(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let Some(core) = self.core() else {
                return;
            };

            let snapshot = core.snapshot();
            let keys = snapshot
                .iter()
                .map(|verification| VerificationKey::from(verification.key()))
                .collect::<Vec<_>>();

            let removed = self
                .list
                .borrow()
                .keys()
                .filter(|key| !keys.contains(key))
                .cloned()
                .collect::<Vec<_>>();
            for key in removed {
                self.obj().remove_row(&key);
            }

            for verification in snapshot {
                let key = VerificationKey::from(verification.key());
                if self.list.borrow().contains_key(&key) {
                    continue;
                }

                let Some(presented) = self.present(&session, verification).await else {
                    continue;
                };

                // The core may have dropped it while the member was fetched.
                if core.get(&key.clone().into()).is_none() {
                    continue;
                }

                self.add(presented);
            }
        }

        /// Present the given verification of the core, with the user it is
        /// shown as.
        async fn present(
            &self,
            session: &Session,
            verification: CoreIdentityVerification,
        ) -> Option<IdentityVerification> {
            let Some(room_id) = verification.room_id().cloned() else {
                let presented = IdentityVerification::new(verification, &session.user(), None);

                if presented.state() == VerificationState::Requested {
                    session
                        .notifications()
                        .show_to_device_identity_verification(&presented)
                        .await;
                }

                return Some(presented);
            };

            let Some(room) = session.room_list().get(&room_id) else {
                error!(
                    "Room for verification request `({}, {})` not found",
                    verification.other_user_id(),
                    verification.flow_id()
                );
                return None;
            };

            let other_user_id = verification.other_user_id().to_owned();
            let member = room.members().map_or_else(
                || Member::new(&room, other_user_id.clone()),
                |l| l.get_or_create(other_user_id.clone()),
            );

            // Ensure the member is up-to-date.
            let matrix_room = room.matrix_room().clone();
            let handle =
                spawn_tokio!(async move { matrix_room.get_member_no_sync(&other_user_id).await });
            match handle.await.expect("task was not aborted") {
                Ok(Some(matrix_member)) => member.update_from_room_member(&matrix_member),
                Ok(None) => {
                    error!(
                        "Room member for verification request `({}, {})` not found",
                        verification.other_user_id(),
                        verification.flow_id()
                    );
                    return None;
                }
                Err(error) => {
                    error!(
                        "Could not get room member for verification request `({}, {})`: {error}",
                        verification.other_user_id(),
                        verification.flow_id()
                    );
                    return None;
                }
            }

            let presented =
                IdentityVerification::new(verification, member.upcast_ref(), Some(&room));

            room.set_verification(Some(&presented));

            if presented.state() == VerificationState::Requested {
                session
                    .notifications()
                    .show_in_room_identity_verification(&presented)
                    .await;
            }

            Some(presented)
        }

        /// Add the given verification to the rows.
        fn add(&self, verification: IdentityVerification) {
            let key = verification.key();

            // Don't add request that already exists.
            if self.list.borrow().contains_key(&key) {
                return;
            }

            let (pos, _) = self.list.borrow_mut().insert_full(key, verification);

            self.obj().items_changed(pos as u32, 0, 1);
        }
    }
}

glib::wrapper! {
    /// The list of ongoing verification requests.
    ///
    /// The list is the core's; this presents it.
    pub struct VerificationList(ObjectSubclass<imp::VerificationList>)
        @implements gio::ListModel;
}

impl VerificationList {
    /// Construct a new `VerificationList` with the given session.
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Initialize this list to listen to new verification requests.
    ///
    /// The core listens; this tells it which methods this system supports,
    /// which needs a look at the cameras, and follows its list.
    pub(crate) fn init(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let core = session.core().verification_list().clone();
        spawn!(async move {
            let methods = load_supported_verification_methods().await;
            core.set_supported_methods(methods);
        });

        self.imp().watch_core();
    }

    /// Drop the row of the verification with the given key, and its
    /// notification.
    fn remove_row(&self, key: &VerificationKey) {
        let Some((pos, ..)) = self.imp().list.borrow_mut().shift_remove_full(key) else {
            return;
        };

        self.items_changed(pos as u32, 1, 0);

        if let Some(session) = self.session() {
            session.notifications().withdraw_identity_verification(key);
        }
    }

    /// Get the verification with the given key.
    pub(crate) fn get(&self, key: &VerificationKey) -> Option<IdentityVerification> {
        self.imp().list.borrow().get(key).cloned()
    }

    // Returns the ongoing session verification, if any.
    pub(crate) fn ongoing_session_verification(&self) -> Option<IdentityVerification> {
        let list = self.imp().list.borrow();
        list.values()
            .find(|v| v.is_self_verification() && !v.is_finished())
            .cloned()
    }

    // Returns the ongoing verification in the given room, if any.
    pub(crate) fn ongoing_room_verification(
        &self,
        room_id: &RoomId,
    ) -> Option<IdentityVerification> {
        let list = self.imp().list.borrow();
        list.values()
            .find(|v| v.room().is_some_and(|room| room.room_id() == room_id) && !v.is_finished())
            .cloned()
    }

    /// Create and send a new verification request.
    ///
    /// If `user` is `None`, a new session verification is started for our own
    /// user and sent to other devices.
    pub(crate) async fn create(&self, user: Option<User>) -> Result<IdentityVerification, ()> {
        let Some(core) = self.imp().core() else {
            error!("Could not create identity verification: failed to upgrade session");
            return Err(());
        };

        let user_id = user.map(|user| user.user_id().clone());
        let created = spawn_tokio!(async move { core.create(user_id.as_deref()).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not create identity verification: {error}");
            })?;

        // The core's list changed, and the row follows; present it now
        // rather than waiting for that turn.
        let key = VerificationKey::from(created.key());
        self.imp().sync().await;

        self.get(&key).ok_or(())
    }
}
