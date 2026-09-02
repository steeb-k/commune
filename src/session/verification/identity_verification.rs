use commune_core::session::{
    IdentityVerification as CoreIdentityVerification, VerificationState as CoreVerificationState,
};
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::encryption::verification::{CancelInfo, Emoji, QrVerificationData};
use qrcode::QrCode;
use ruma::{OwnedDeviceId, events::key::verification::VerificationMethod};
use tokio::task::AbortHandle;
use tracing::error;

use super::VerificationKey;
use crate::{
    components::QrCodeScanner,
    core_bridge::ObjectWatcher,
    prelude::*,
    session::{Member, Room, User},
    spawn_tokio,
    utils::BoundConstructOnlyObject,
};

#[glib::flags(name = "VerificationSupportedMethods")]
pub enum VerificationSupportedMethods {
    SAS = 0b0000_0001,
    QR_SHOW = 0b0000_0010,
    QR_SCAN = 0b0000_0100,
}

impl<'a> From<&'a [VerificationMethod]> for VerificationSupportedMethods {
    fn from(methods: &'a [VerificationMethod]) -> Self {
        let mut result = Self::empty();

        for method in methods {
            match method {
                VerificationMethod::SasV1 => result.insert(Self::SAS),
                VerificationMethod::QrCodeScanV1 => result.insert(Self::QR_SCAN),
                VerificationMethod::QrCodeShowV1 => result.insert(Self::QR_SHOW),
                _ => {}
            }
        }

        result
    }
}

impl Default for VerificationSupportedMethods {
    fn default() -> Self {
        Self::empty()
    }
}

/// The state of an identity verification.
///
/// The core's [`VerificationState`](CoreVerificationState) as a `glib`
/// enum, for the properties that carry it.
#[derive(Debug, Default, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "VerificationState")]
pub enum VerificationState {
    /// We created and sent the request.
    ///
    /// We must wait for the other user/device to accept it.
    #[default]
    Created,
    /// The other user/device sent us a request.
    ///
    /// We should ask the user if they want to accept it.
    Requested,
    /// We support none of the other user's verification methods.
    NoSupportedMethods,
    /// The request was accepted.
    ///
    /// We should ask the user to choose a method.
    Ready,
    /// An SAS verification was started.
    ///
    /// We should show the emojis and ask the user to confirm that they match.
    SasConfirm,
    /// The user wants to scan a QR Code.
    QrScan,
    /// The user scanned a QR Code.
    QrScanned,
    /// Our QR Code was scanned.
    ///
    /// We should ask the user to confirm that the QR Code was scanned
    /// successfully.
    QrConfirm,
    /// The verification was successful.
    Done,
    /// The verification was cancelled.
    Cancelled,
    /// The verification was automatically dismissed.
    ///
    /// Happens when a received request is not accepted by us after 2 minutes.
    Dismissed,
    /// The verification was happening in-room but the room was left.
    RoomLeft,
    /// An unexpected error happened.
    Error,
}

impl From<CoreVerificationState> for VerificationState {
    fn from(value: CoreVerificationState) -> Self {
        match value {
            CoreVerificationState::Created => Self::Created,
            CoreVerificationState::Requested => Self::Requested,
            CoreVerificationState::NoSupportedMethods => Self::NoSupportedMethods,
            CoreVerificationState::Ready => Self::Ready,
            CoreVerificationState::SasConfirm => Self::SasConfirm,
            CoreVerificationState::QrScan => Self::QrScan,
            CoreVerificationState::QrScanned => Self::QrScanned,
            CoreVerificationState::QrConfirm => Self::QrConfirm,
            CoreVerificationState::Done => Self::Done,
            CoreVerificationState::Cancelled => Self::Cancelled,
            CoreVerificationState::Dismissed => Self::Dismissed,
            CoreVerificationState::RoomLeft => Self::RoomLeft,
            CoreVerificationState::Error => Self::Error,
        }
    }
}

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::IdentityVerification)]
    pub struct IdentityVerification {
        /// The verification, as the core runs it.
        core: OnceCell<CoreIdentityVerification>,
        /// The user to verify.
        #[property(get, set = Self::set_user, construct_only)]
        user: BoundConstructOnlyObject<User>,
        /// The room of this verification, if any.
        #[property(get, set = Self::set_room, construct_only)]
        room: glib::WeakRef<Room>,
        /// The state of this verification
        #[property(get, builder(VerificationState::default()))]
        state: Cell<VerificationState>,
        /// Whether the verification request was accepted.
        ///
        /// It means that the verification reached at least the `Ready` state.
        #[property(get)]
        was_accepted: Cell<bool>,
        /// Whether this verification is finished.
        #[property(get = Self::is_finished)]
        is_finished: PhantomData<bool>,
        /// The supported methods of the verification request.
        #[property(get = Self::supported_methods, type = VerificationSupportedMethods)]
        supported_methods: RefCell<Vec<VerificationMethod>>,
        /// The flow ID of this verification.
        #[property(get = Self::flow_id)]
        flow_id: PhantomData<String>,
        /// The time and date when the verification request was received.
        #[property(get)]
        received_time: OnceCell<glib::DateTime>,
        /// The display name of this verification.
        #[property(get = Self::display_name)]
        display_name: PhantomData<String>,
        /// The QR Code, if the `QrCodeShowV1` method is supported.
        pub(super) qr_code: RefCell<Option<QrCode>>,
        /// The QR code scanner, if the user wants to scan a QR Code and we
        /// have access to the camera.
        #[property(get)]
        pub(super) qrcode_scanner: RefCell<Option<QrCodeScanner>>,
        /// Whether this verification was viewed by the user.
        #[property(get, set = Self::set_was_viewed, explicit_notify)]
        was_viewed: Cell<bool>,
        /// The task following the core's verification.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IdentityVerification {
        const NAME: &'static str = "IdentityVerification";
        type Type = super::IdentityVerification;
    }

    #[glib::derived_properties]
    impl ObjectImpl for IdentityVerification {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    // The SAS data changed.
                    Signal::builder("sas-data-changed").build(),
                    // The cancel info changed.
                    Signal::builder("cancel-info-changed").build(),
                    // The verification has been replaced by a new one.
                    Signal::builder("replaced")
                        .param_types([super::IdentityVerification::static_type()])
                        .build(),
                    // The verification is done, but has not changed its state yes.
                    //
                    // Return `glib::Propagation::Stop` in a signal handler to prevent the state
                    // from changing to `VerificationState::Done`. Can be used to replace the last
                    // screen of `IdentityVerificationView`.
                    Signal::builder("done").return_type::<bool>().build(),
                    // The verification can be dismissed.
                    Signal::builder("dismiss").build(),
                    // The verification should be removed from the verification list.
                    Signal::builder("remove-from-list").build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl IdentityVerification {
        /// Present the given verification of the core, and follow it.
        pub(super) fn set_core(&self, core: CoreIdentityVerification) {
            type V = super::IdentityVerification;

            let received_time = core
                .received_time()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|elapsed| {
                    glib::DateTime::from_unix_local(i64::try_from(elapsed.as_secs()).ok()?).ok()
                })
                .or_else(|| glib::DateTime::now_local().ok());
            if let Some(received_time) = received_time {
                let _ = self.received_time.set(received_time);
            }

            let core = self.core.get_or_init(|| core).clone();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe_state(), |obj: &V, state| {
                    obj.imp().update_state(state.into());
                })
                .follow(core.subscribe_was_accepted(), |obj: &V, was_accepted| {
                    obj.imp().set_was_accepted(was_accepted);
                })
                .follow(core.subscribe_supported_methods(), |obj: &V, methods| {
                    obj.imp().set_supported_methods(methods);
                })
                .follow(core.subscribe_dismissed(), |obj: &V, dismissed| {
                    if dismissed {
                        obj.remove_from_list();
                        obj.emit_by_name::<()>("dismiss", &[]);
                    }
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.set_supported_methods(core.supported_methods());
            self.set_was_accepted(core.was_accepted());
            self.update_state(core.state().into());
        }

        /// The verification, as the core runs it.
        pub(super) fn core(&self) -> &CoreIdentityVerification {
            self.core.get().expect("core should be initialized")
        }

        /// Set the user to verify.
        fn set_user(&self, user: User) {
            let mut handlers = Vec::new();

            // If the user is a room member, it means it's an in-room verification, we need
            // to keep track of their name since it's used as the display name.
            if user.is::<Member>() {
                let obj = self.obj();
                let display_name_handler = user.connect_display_name_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.notify_display_name();
                    }
                ));
                handlers.push(display_name_handler);
            }

            self.user.set(user, handlers);
        }

        /// Set the room of the verification, if any.
        ///
        /// Leaving the room is the core's to notice.
        fn set_room(&self, room: Option<&Room>) {
            self.room.set(room);
        }

        /// Mirror the state of the core's verification.
        ///
        /// The two signals the application raised on the way to a state are
        /// raised here, before the state changes, as they were.
        fn update_state(&self, state: VerificationState) {
            if self.state.get() == state {
                return;
            }

            match state {
                VerificationState::SasConfirm => {
                    self.obj().emit_by_name::<()>("sas-data-changed", &[]);
                }
                VerificationState::Cancelled => {
                    self.obj().emit_by_name::<()>("cancel-info-changed", &[]);
                }
                _ => {}
            }

            self.set_state(state);
        }

        /// Set the state of this verification.
        pub(super) fn set_state(&self, state: VerificationState) {
            if self.state.get() == state {
                return;
            }
            let obj = self.obj();

            if state == VerificationState::Done {
                let ret = obj.emit_by_name::<bool>("done", &[]);
                if glib::Propagation::from(ret).is_stop() {
                    return;
                }
            } else if state != VerificationState::QrScan && self.qrcode_scanner.take().is_some() {
                obj.notify_qrcode_scanner();
            }

            self.state.set(state);

            obj.notify_state();

            if self.is_finished() {
                obj.notify_is_finished();
            }
        }

        /// Whether this verification is finished.
        fn is_finished(&self) -> bool {
            matches!(
                self.state.get(),
                VerificationState::Cancelled
                    | VerificationState::Dismissed
                    | VerificationState::Done
                    | VerificationState::Error
                    | VerificationState::RoomLeft
            )
        }

        /// Mirror the supported methods of this verification, and render the
        /// QR code to show when it is one of them.
        fn set_supported_methods(&self, supported_methods: Vec<VerificationMethod>) {
            if supported_methods.contains(&VerificationMethod::QrCodeShowV1)
                && self.qr_code.borrow().is_none()
            {
                let qr_code = self.core().qr_to_show().and_then(|qr_verification| {
                    match qr_verification.to_qr_code() {
                        Ok(qr_code) => Some(qr_code),
                        Err(error) => {
                            error!("Could not generate verification QR code: {error}");
                            None
                        }
                    }
                });
                self.qr_code.replace(qr_code);
            }

            if *self.supported_methods.borrow() == supported_methods {
                return;
            }

            self.supported_methods.replace(supported_methods);
            self.obj().notify_supported_methods();
        }

        /// The supported methods of this verifications.
        fn supported_methods(&self) -> VerificationSupportedMethods {
            self.supported_methods.borrow().as_slice().into()
        }

        /// The display name of this verification request.
        fn display_name(&self) -> String {
            let user = self.user.obj();

            if user.is_own_user() {
                // TODO: give this request a name based on the device
                "Login Request".to_string()
            } else {
                user.display_name()
            }
        }

        /// The flow ID of this verification request.
        fn flow_id(&self) -> String {
            self.core().flow_id().to_owned()
        }

        /// Set whether this verification was viewed by the user.
        fn set_was_viewed(&self, was_viewed: bool) {
            if !was_viewed {
                // The user cannot unview the verification.
                return;
            }

            self.core().set_was_viewed();
            self.was_viewed.set(was_viewed);
            self.obj().notify_was_viewed();
        }

        /// Mirror whether this request was accepted.
        fn set_was_accepted(&self, was_accepted: bool) {
            if !was_accepted || self.was_accepted.get() {
                // The state cannot go backwards.
                return;
            }

            self.was_accepted.set(true);
            self.obj().notify_was_accepted();
        }
    }
}

glib::wrapper! {
    /// An identity verification request.
    ///
    /// The verification is the core's; this presents it, with the user it
    /// is shown as, the QR code rendered and the scanner.
    pub struct IdentityVerification(ObjectSubclass<imp::IdentityVerification>);
}

impl IdentityVerification {
    /// Construct a presentation of the given verification of the core.
    pub(super) fn new(core: CoreIdentityVerification, user: &User, room: Option<&Room>) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("user", user)
            .property("room", room)
            .build();
        obj.imp().set_core(core);
        obj
    }

    /// The verification, as the core runs it.
    fn core(&self) -> CoreIdentityVerification {
        self.imp().core().clone()
    }

    /// The unique identifying key of this verification.
    pub(crate) fn key(&self) -> VerificationKey {
        self.imp().core().key().into()
    }

    /// Whether this is a self-verification.
    pub(crate) fn is_self_verification(&self) -> bool {
        self.imp().core().is_self_verification()
    }

    /// Whether we started this verification.
    pub(crate) fn started_by_us(&self) -> bool {
        self.imp().core().started_by_us()
    }

    /// The ID of the other device that is being verified.
    pub(crate) fn other_device_id(&self) -> Option<OwnedDeviceId> {
        self.imp().core().other_device_id()
    }

    /// Information about the verification cancellation, if any.
    pub(crate) fn cancel_info(&self) -> Option<CancelInfo> {
        self.imp().core().cancel_info()
    }

    /// Cancel the verification request.
    ///
    /// This can be used to decline the request or cancel it at any time.
    pub(crate) async fn cancel(&self) -> Result<(), ()> {
        let core = self.core();

        spawn_tokio!(async move { core.cancel().await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not cancel verification request: {error}");
            })
    }

    /// Accept the verification request.
    pub(crate) async fn accept(&self) -> Result<(), ()> {
        let core = self.core();

        spawn_tokio!(async move { core.accept().await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not accept verification request: {error}");
            })
    }

    /// Go back to the state to choose a verification method.
    pub(crate) fn choose_method(&self) {
        self.imp().core().choose_method();
    }

    /// Whether the current SAS verification supports emoji.
    pub(crate) fn sas_supports_emoji(&self) -> bool {
        self.imp().core().sas_supports_emoji()
    }

    /// The list of emojis for the current SAS verification, if any.
    pub(crate) fn sas_emoji(&self) -> Option<[Emoji; 7]> {
        self.imp().core().sas_emoji()
    }

    /// The list of decimals for the current SAS verification, if any.
    pub(crate) fn sas_decimals(&self) -> Option<(u16, u16, u16)> {
        self.imp().core().sas_decimals()
    }

    /// The QR Code, if the `QrCodeShowV1` method is supported.
    pub(crate) fn qr_code(&self) -> Option<QrCode> {
        self.imp().qr_code.borrow().clone()
    }

    /// Whether we have the QR code.
    pub(crate) fn has_qr_code(&self) -> bool {
        self.imp().qr_code.borrow().is_some()
    }

    /// Start a QR Code scan.
    pub(crate) async fn start_qr_code_scan(&self) -> Result<(), ()> {
        let imp = self.imp();

        match QrCodeScanner::new().await {
            Some(qrcode_scanner) => {
                imp.qrcode_scanner.replace(Some(qrcode_scanner));
                self.notify_qrcode_scanner();

                imp.core().start_qr_code_scan();

                Ok(())
            }
            None => Err(()),
        }
    }

    /// The QR Code was scanned.
    pub(crate) async fn qr_code_scanned(&self, data: QrVerificationData) -> Result<(), ()> {
        let core = self.core();

        spawn_tokio!(async move { core.qr_code_scanned(data).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not validate scanned verification QR code: {error}");
            })
    }

    /// Confirm that the QR Code was scanned by the other party.
    pub(crate) async fn confirm_qr_code_scanned(&self) -> Result<(), ()> {
        let core = self.core();

        spawn_tokio!(async move { core.confirm_qr_code_scanned().await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not confirm scanned verification QR code: {error}");
            })
    }

    /// Start a SAS verification.
    pub(crate) async fn start_sas(&self) -> Result<(), ()> {
        let core = self.core();

        spawn_tokio!(async move { core.start_sas().await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not start SAS verification: {error}");
            })
    }

    /// The SAS data does not match.
    pub(crate) async fn sas_mismatch(&self) -> Result<(), ()> {
        let core = self.core();

        spawn_tokio!(async move { core.sas_mismatch().await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not send SAS verification mismatch: {error}");
            })
    }

    /// The SAS data matches.
    pub(crate) async fn sas_match(&self) -> Result<(), ()> {
        let core = self.core();

        spawn_tokio!(async move { core.sas_match().await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not send SAS verification match: {error}");
            })
    }

    /// Restart this verification with a new one to the same user.
    pub(crate) async fn restart(&self) -> Result<Self, ()> {
        let user = self.user();
        let verification_list = user.session().verification_list();

        let new_user = (!self.is_self_verification()).then_some(user);
        let new_verification = verification_list.create(new_user).await?;

        self.emit_by_name::<()>("replaced", &[&new_verification]);

        // If we restart because an unexpected error happened, try to cancel it.
        if self.cancel().await.is_err() {
            self.dismiss();
        }

        Ok(new_verification)
    }

    /// The verification can be dismissed.
    ///
    /// The core removes it from its list, and this object says so with the
    /// signals the interface listens for.
    pub(crate) fn dismiss(&self) {
        self.imp().core().dismiss();
    }

    /// The verification can be removed from the verification list.
    ///
    /// You will usually want to use [`IdentityVerification::dismiss()`] because
    /// the interface listens for the signal it emits, and it calls this method
    /// internally.
    pub(crate) fn remove_from_list(&self) {
        self.emit_by_name::<()>("remove-from-list", &[]);
    }

    /// Connect to the signal emitted when the SAS data changed.
    pub fn connect_sas_data_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "sas-data-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }

    /// Connect to the signal emitted when the cancel info changed.
    pub fn connect_cancel_info_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "cancel-info-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }

    /// Connect to the signal emitted when the verification has been replaced.
    ///
    /// The second parameter to the function is the new verification.
    pub fn connect_replaced<F: Fn(&Self, &Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "replaced",
            true,
            closure_local!(move |obj: Self, new_verification: Self| {
                f(&obj, &new_verification);
            }),
        )
    }

    /// Connect to the signal emitted when the verification is done, but its
    /// state does not reflect that yet.
    ///
    /// Return `glib::Propagation::Stop` in the signal handler to prevent the
    /// state from changing to `VerificationState::Done`. Can be used to replace
    /// the last screen of `IdentityVerificationView`.
    pub fn connect_done<F: Fn(&Self) -> glib::Propagation + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "done",
            true,
            closure_local!(move |obj: Self| {
                let ret = f(&obj);

                if ret.is_stop() {
                    obj.stop_signal_emission_by_name("done");
                }

                bool::from(ret)
            }),
        )
    }

    /// Connect to the signal emitted when the verification can be dismissed.
    pub fn connect_dismiss<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "dismiss",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}
