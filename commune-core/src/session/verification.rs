//! Identity verification: of another of the account's own sessions, or of
//! another user.
//!
//! The headless counterpart of the application's `VerificationList` and
//! `IdentityVerification` (`src/session/verification/`): the list that
//! follows incoming requests — to-device ones for the account's own
//! sessions, in-room ones for other users — and the state machine over the
//! SDK's request and the SAS or QR verification it turns into, with the
//! same states, the same choice of methods, the same two-minute dismissal
//! of a request nobody answered, and the same automatic steps.
//!
//! What stayed in the application: the camera and the QR code scanner, the
//! rendering of the QR code to show, the notification a request raises, the
//! member the request is presented as, and every sentence. The methods this
//! side supports are the embedder's to declare, because whether a camera
//! exists is: [`VerificationList::set_supported_methods`].

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

use eyeball::{SharedObservable, Subscriber};
use futures_util::StreamExt;
use indexmap::IndexMap;
use matrix_sdk::{
    Client, RoomState,
    encryption::{
        identities::UserIdentity,
        verification::{
            CancelInfo, Emoji, QrVerification, QrVerificationData, QrVerificationState, SasState,
            SasVerification, Verification, VerificationRequest, VerificationRequestState,
        },
    },
};
use ruma::{
    OwnedDeviceId, OwnedRoomId, OwnedUserId, UserId,
    events::{
        key::verification::{
            REQUEST_RECEIVED_TIMEOUT, VerificationMethod, cancel::CancelCode,
            request::ToDeviceKeyVerificationRequestEvent,
        },
        room::message::{MessageType, OriginalSyncRoomMessageEvent},
    },
};
use tokio::task::AbortHandle;
use tracing::{debug, error};

use super::{RoomCategory, WeakSession};
use crate::{RUNTIME, UserFacingError, spawn_tokio};

/// The verification methods this side supports before the embedder says
/// otherwise: the ones that need no camera.
pub const DEFAULT_SUPPORTED_METHODS: &[VerificationMethod] = &[
    VerificationMethod::SasV1,
    VerificationMethod::QrCodeShowV1,
    VerificationMethod::ReciprocateV1,
];

/// A unique key to identify an identity verification.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct VerificationKey {
    /// The ID of the user being verified.
    pub user_id: OwnedUserId,
    /// The ID of the verification.
    pub flow_id: String,
}

impl VerificationKey {
    /// Create a new `VerificationKey` with the given user ID and flow ID.
    #[must_use]
    pub fn new(user_id: OwnedUserId, flow_id: String) -> Self {
        Self { user_id, flow_id }
    }

    /// Create a new `VerificationKey` from the given request.
    #[must_use]
    pub fn from_request(request: &VerificationRequest) -> Self {
        Self::new(
            request.other_user_id().to_owned(),
            request.flow_id().to_owned(),
        )
    }
}

/// The state of an identity verification.
#[derive(Debug, Default, Eq, PartialEq, Clone, Copy)]
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

impl VerificationState {
    /// Whether a verification in this state is finished.
    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::Dismissed | Self::Done | Self::Error | Self::RoomLeft
        )
    }
}

/// What can go wrong while creating a verification.
#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    /// The session is gone.
    #[error("the session is no longer available")]
    NoSession,
    /// The user has no cryptographic identity to verify.
    #[error("the cryptographic identity was not found")]
    NoIdentity,
    /// The room of an in-room request is not known.
    #[error("the room of the verification was not found")]
    NoRoom,
    /// The homeserver refused.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(Box<matrix_sdk::Error>),
    /// The verification request could not be sent.
    #[error(transparent)]
    Request(Box<matrix_sdk::encryption::identities::RequestVerificationError>),
    /// The verification is not in a state that allows the step.
    #[error("the verification is not in the state the step needs")]
    WrongState,
}

impl UserFacingError for VerificationError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NoSession => "The session is no longer available.".to_owned(),
            Self::NoIdentity => "Could not find the cryptographic identity".to_owned(),
            Self::NoRoom => "Could not find the room of the verification".to_owned(),
            Self::Server(error) => error.to_string(),
            Self::Request(error) => error.to_string(),
            Self::WrongState => "The verification cannot do that now".to_owned(),
        }
    }
}

/// An identity verification, as the application's state machine over the
/// SDK's request and verification.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct IdentityVerification {
    inner: Arc<VerificationInner>,
}

#[derive(Debug)]
struct VerificationInner {
    /// The SDK's verification request.
    request: VerificationRequest,
    /// The methods this side supports.
    our_methods: Vec<VerificationMethod>,
    /// The room of this verification, if any.
    room_id: Option<OwnedRoomId>,
    /// The state of this verification.
    state: SharedObservable<VerificationState>,
    /// Whether the request was accepted: it reached at least `Ready`.
    was_accepted: SharedObservable<bool>,
    /// The supported methods of the request.
    supported_methods: SharedObservable<Vec<VerificationMethod>>,
    /// When the request was received.
    received_time: SystemTime,
    /// The SDK's verification, if one was started.
    verification: Mutex<Option<Verification>>,
    /// The QR verification to show, if `QrCodeShowV1` is supported.
    qr_to_show: Mutex<Option<QrVerification>>,
    /// Whether this verification was viewed by the user.
    was_viewed: AtomicBool,
    /// Whether the verification asked to be dismissed and removed from its
    /// list.
    dismissed: SharedObservable<bool>,
    /// The tasks following the SDK.
    abort_handles: Mutex<Vec<AbortHandle>>,
    /// The timeout of a received request nobody answered.
    timeout_handle: Mutex<Option<AbortHandle>>,
}

impl Drop for VerificationInner {
    fn drop(&mut self) {
        if let Ok(handles) = self.abort_handles.get_mut() {
            for handle in handles.drain(..) {
                handle.abort();
            }
        }
        if let Ok(Some(handle)) = self.timeout_handle.get_mut().map(Option::take) {
            handle.abort();
        }

        let request = self.request.clone();
        if !request.is_done() && !request.is_passive() && !request.is_cancelled() {
            RUNTIME.spawn(async move {
                if let Err(cancel_error) = request.cancel().await {
                    error!("Could not cancel verification request on drop: {cancel_error}");
                }
            });
        }
    }
}

impl IdentityVerification {
    /// Construct a verification for the given request, following it from
    /// its current state.
    async fn new(
        request: VerificationRequest,
        our_methods: Vec<VerificationMethod>,
        room: Option<&super::Room>,
    ) -> Self {
        let this = Self {
            inner: Arc::new(VerificationInner {
                room_id: room.map(|room| room.room_id().to_owned()),
                request: request.clone(),
                our_methods,
                state: SharedObservable::new(VerificationState::Created),
                was_accepted: SharedObservable::new(false),
                supported_methods: SharedObservable::new(Vec::new()),
                received_time: SystemTime::now(),
                verification: Mutex::new(None),
                qr_to_show: Mutex::new(None),
                was_viewed: AtomicBool::new(false),
                dismissed: SharedObservable::new(false),
                abort_handles: Mutex::new(Vec::new()),
                timeout_handle: Mutex::new(None),
            }),
        };

        // Set up the timeout if we received the request and it is not accepted yet.
        if matches!(request.state(), VerificationRequestState::Requested { .. }) {
            let weak = Arc::downgrade(&this.inner);
            let handle = RUNTIME
                .spawn(async move {
                    tokio::time::sleep(REQUEST_RECEIVED_TIMEOUT).await;
                    if let Some(inner) = weak.upgrade() {
                        let this = IdentityVerification { inner };
                        this.set_state(VerificationState::Dismissed);
                        this.dismiss();
                    }
                })
                .abort_handle();
            *this
                .inner
                .timeout_handle
                .lock()
                .expect("mutex is not poisoned") = Some(handle);
        }

        if let Some(room) = room {
            this.watch_room(room);
        }

        let weak = Arc::downgrade(&this.inner);
        let handle = RUNTIME
            .spawn(request.changes().for_each(move |state| {
                let weak = weak.clone();
                async move {
                    if let Some(inner) = weak.upgrade() {
                        IdentityVerification { inner }
                            .handle_request_state(state)
                            .await;
                    }
                }
            }))
            .abort_handle();
        this.inner
            .abort_handles
            .lock()
            .expect("mutex is not poisoned")
            .push(handle);

        let state = request.state();
        this.handle_request_state(state).await;

        this
    }

    /// Follow the room of an in-room verification: nothing can be done
    /// with it once the user is not in the room anymore.
    fn watch_room(&self, room: &super::Room) {
        let mut category = room.subscribe_category();
        let weak = Arc::downgrade(&self.inner);
        let handle = RUNTIME
            .spawn(async move {
                loop {
                    if matches!(category.get(), RoomCategory::Left) {
                        if let Some(inner) = weak.upgrade() {
                            IdentityVerification { inner }.set_state(VerificationState::RoomLeft);
                        }
                        return;
                    }
                    if category.next().await.is_none() {
                        return;
                    }
                }
            })
            .abort_handle();
        self.inner
            .abort_handles
            .lock()
            .expect("mutex is not poisoned")
            .push(handle);
    }

    /// The unique identifying key of this verification.
    #[must_use]
    pub fn key(&self) -> VerificationKey {
        VerificationKey::from_request(&self.inner.request)
    }

    /// The flow ID of this verification.
    #[must_use]
    pub fn flow_id(&self) -> &str {
        self.inner.request.flow_id()
    }

    /// The ID of the user being verified.
    #[must_use]
    pub fn other_user_id(&self) -> &UserId {
        self.inner.request.other_user_id()
    }

    /// The room of this verification, if it is an in-room one.
    #[must_use]
    pub fn room_id(&self) -> Option<&OwnedRoomId> {
        self.inner.room_id.as_ref()
    }

    /// Whether this is a self-verification.
    #[must_use]
    pub fn is_self_verification(&self) -> bool {
        self.inner.request.is_self_verification()
    }

    /// Whether we started this verification.
    #[must_use]
    pub fn started_by_us(&self) -> bool {
        self.inner.request.we_started()
    }

    /// The state of this verification.
    #[must_use]
    pub fn state(&self) -> VerificationState {
        self.inner.state.get()
    }

    /// Subscribe to the state of this verification.
    pub fn subscribe_state(&self) -> Subscriber<VerificationState> {
        self.inner.state.subscribe()
    }

    /// Whether this verification is finished.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.state().is_finished()
    }

    /// Whether the request was accepted: it reached at least `Ready`.
    #[must_use]
    pub fn was_accepted(&self) -> bool {
        self.inner.was_accepted.get()
    }

    /// Subscribe to whether the request was accepted.
    pub fn subscribe_was_accepted(&self) -> Subscriber<bool> {
        self.inner.was_accepted.subscribe()
    }

    /// The methods supported by both sides.
    #[must_use]
    pub fn supported_methods(&self) -> Vec<VerificationMethod> {
        self.inner.supported_methods.get()
    }

    /// Subscribe to the methods supported by both sides.
    ///
    /// They are known once the request is ready; a `QrCodeShowV1` among
    /// them means [`Self::qr_to_show`] has the code.
    pub fn subscribe_supported_methods(&self) -> Subscriber<Vec<VerificationMethod>> {
        self.inner.supported_methods.subscribe()
    }

    /// When the request was received.
    #[must_use]
    pub fn received_time(&self) -> SystemTime {
        self.inner.received_time
    }

    /// Whether the verification asked to be dismissed.
    pub fn subscribe_dismissed(&self) -> Subscriber<bool> {
        self.inner.dismissed.subscribe()
    }

    /// Say that the user viewed this verification.
    ///
    /// The user cannot unview it.
    pub fn set_was_viewed(&self) {
        self.inner.was_viewed.store(true, Ordering::Relaxed);
    }

    /// The ID of the other device that is being verified.
    #[must_use]
    pub fn other_device_id(&self) -> Option<OwnedDeviceId> {
        let request_state = self.inner.request.state();
        let other_device_data = match &request_state {
            VerificationRequestState::Requested {
                other_device_data, ..
            }
            | VerificationRequestState::Ready {
                other_device_data, ..
            } => other_device_data,
            VerificationRequestState::Transitioned { verification } => match verification {
                Verification::SasV1(sas) => sas.other_device(),
                Verification::QrV1(qr) => qr.other_device(),
                _ => None?,
            },
            VerificationRequestState::Created { .. }
            | VerificationRequestState::Done
            | VerificationRequestState::Cancelled(_) => None?,
        };

        Some(other_device_data.device_id().to_owned())
    }

    /// Information about the verification cancellation, if any.
    #[must_use]
    pub fn cancel_info(&self) -> Option<CancelInfo> {
        self.inner.request.cancel_info()
    }

    /// Set the state of this verification.
    fn set_state(&self, state: VerificationState) {
        self.inner.state.set_if_not_eq(state);
    }

    /// Set whether this request was accepted.
    fn set_was_accepted(&self) {
        self.inner.was_accepted.set_if_not_eq(true);
    }

    /// Handle a change in the request's state.
    async fn handle_request_state(&self, state: VerificationRequestState) {
        let request = &self.inner.request;

        if !matches!(state, VerificationRequestState::Requested { .. })
            && let Ok(Some(handle)) = self
                .inner
                .timeout_handle
                .lock()
                .map(|mut handle| handle.take())
        {
            handle.abort();
        }
        if !matches!(
            state,
            VerificationRequestState::Created { .. } | VerificationRequestState::Requested { .. }
        ) {
            self.set_was_accepted();
        }

        match state {
            VerificationRequestState::Created { .. } => {}
            VerificationRequestState::Requested { their_methods, .. } => {
                let supported_methods =
                    intersect_methods(self.inner.our_methods.clone(), &their_methods);

                if supported_methods.is_empty() {
                    self.set_state(VerificationState::NoSupportedMethods);
                } else {
                    self.set_state(VerificationState::Requested);
                }
            }
            VerificationRequestState::Ready {
                their_methods,
                our_methods,
                ..
            } => {
                let mut supported_methods = intersect_methods(our_methods, &their_methods);

                // Remove the reciprocate method, it's not a flow in itself.
                supported_methods.retain(|method| *method != VerificationMethod::ReciprocateV1);

                // Check that we can get the QR Code, to avoid exposing the method if it doesn't
                // work.
                if supported_methods.contains(&VerificationMethod::QrCodeShowV1)
                    && !self.load_qr_code().await
                {
                    supported_methods.retain(|method| *method != VerificationMethod::QrCodeShowV1);
                }

                if supported_methods.is_empty() {
                    // This should not happen.
                    error!(
                        "Invalid verification: no methods are supported by both sessions, cancelling…"
                    );
                    if self.cancel().await.is_err() {
                        self.set_state(VerificationState::NoSupportedMethods);
                    }
                } else {
                    self.inner
                        .supported_methods
                        .set_if_not_eq(supported_methods.clone());

                    if supported_methods.len() == 1
                        && !request.we_started()
                        && supported_methods[0] == VerificationMethod::SasV1
                    {
                        // We only go forward for SAS, because QrCodeShow is the
                        // same screen as the one to choose a method and we need
                        // to tell the user we are going to need to access the
                        // camera for QrCodeScan.
                        if self.start_sas().await.is_ok() {
                            return;
                        }
                    }

                    self.set_state(VerificationState::Ready);
                }
            }
            VerificationRequestState::Transitioned { verification } => {
                self.set_verification(verification).await;
            }
            VerificationRequestState::Done => {
                self.set_state(VerificationState::Done);
            }
            VerificationRequestState::Cancelled(info) => self.handle_cancelled_state(&info),
        }
    }

    /// Handle when the request was cancelled.
    fn handle_cancelled_state(&self, cancel_info: &CancelInfo) {
        debug!("Verification was cancelled: {cancel_info:?}");
        let cancel_code = cancel_info.cancel_code();

        if cancel_info.cancelled_by_us() && *cancel_code == CancelCode::User {
            // We should handle this already.
            return;
        }

        if *cancel_code == CancelCode::Accepted && !self.inner.was_viewed.load(Ordering::Relaxed) {
            // We can safely remove it.
            self.dismiss();
            return;
        }

        self.set_state(VerificationState::Cancelled);
    }

    /// Set the SDK's verification.
    async fn set_verification(&self, verification: Verification) {
        let weak = Arc::downgrade(&self.inner);
        let handle = match &verification {
            Verification::SasV1(sas_verification) => RUNTIME
                .spawn(sas_verification.changes().for_each(move |state| {
                    let weak = weak.clone();
                    async move {
                        if let Some(inner) = weak.upgrade() {
                            IdentityVerification { inner }
                                .handle_sas_verification_state(state)
                                .await;
                        }
                    }
                }))
                .abort_handle(),
            Verification::QrV1(qr_verification) => RUNTIME
                .spawn(qr_verification.changes().for_each(move |state| {
                    let weak = weak.clone();
                    async move {
                        if let Some(inner) = weak.upgrade() {
                            IdentityVerification { inner }.handle_qr_verification_state(state);
                        }
                    }
                }))
                .abort_handle(),
            _ => {
                error!("We only support SAS and QR verification");
                self.set_state(VerificationState::Error);
                return;
            }
        };

        *self
            .inner
            .verification
            .lock()
            .expect("mutex is not poisoned") = Some(verification.clone());
        self.inner
            .abort_handles
            .lock()
            .expect("mutex is not poisoned")
            .push(handle);

        match verification {
            Verification::SasV1(sas_verification) => {
                self.handle_sas_verification_state(sas_verification.state())
                    .await;
            }
            Verification::QrV1(qr_verification) => {
                self.handle_qr_verification_state(qr_verification.state());
            }
            _ => unreachable!(),
        }
    }

    /// Handle a change in the QR verification's state.
    fn handle_qr_verification_state(&self, state: QrVerificationState) {
        match state {
            QrVerificationState::Started
            | QrVerificationState::Confirmed
            | QrVerificationState::Reciprocated => {}
            QrVerificationState::Scanned => self.set_state(VerificationState::QrConfirm),
            QrVerificationState::Done { .. } => self.set_state(VerificationState::Done),
            QrVerificationState::Cancelled(info) => self.handle_cancelled_state(&info),
        }
    }

    /// The SDK's QR verification, if one was started.
    fn qr_verification(&self) -> Option<QrVerification> {
        match self
            .inner
            .verification
            .lock()
            .expect("mutex is not poisoned")
            .as_ref()?
        {
            Verification::QrV1(qr) => Some(qr.clone()),
            _ => None,
        }
    }

    /// Handle a change in the SAS verification's state.
    async fn handle_sas_verification_state(&self, state: SasState) {
        let Some(sas_verification) = self.sas_verification() else {
            return;
        };

        match state {
            SasState::Created { .. } | SasState::Accepted { .. } | SasState::Confirmed => {}
            SasState::Started { .. } => {
                let handle = spawn_tokio!(async move { sas_verification.accept().await });
                if let Err(accept_error) = handle.await.expect("task was not aborted") {
                    error!("Could not accept SAS verification: {accept_error}");
                    self.set_state(VerificationState::Error);
                }
            }
            SasState::KeysExchanged { .. } => self.set_state(VerificationState::SasConfirm),
            SasState::Done { .. } => self.set_state(VerificationState::Done),
            SasState::Cancelled(info) => self.handle_cancelled_state(&info),
        }
    }

    /// The SDK's SAS verification, if one was started.
    fn sas_verification(&self) -> Option<SasVerification> {
        match self
            .inner
            .verification
            .lock()
            .expect("mutex is not poisoned")
            .as_ref()?
        {
            Verification::SasV1(sas) => Some(sas.clone()),
            _ => None,
        }
    }

    /// Try to load the QR code to show.
    ///
    /// Returns `true` if it was successfully loaded, `false` otherwise.
    async fn load_qr_code(&self) -> bool {
        let request = self.inner.request.clone();
        let handle = spawn_tokio!(async move { request.generate_qr_code().await });

        match handle.await.expect("task was not aborted") {
            Ok(Some(qr_verification)) => {
                *self.inner.qr_to_show.lock().expect("mutex is not poisoned") =
                    Some(qr_verification);
                true
            }
            Ok(None) => {
                error!("Could not start QR verification generation: unknown reason");
                false
            }
            Err(generate_error) => {
                error!("Could not start QR verification generation: {generate_error}");
                false
            }
        }
    }

    /// The QR code to show, as the bytes a QR renderer encodes, if
    /// `QrCodeShowV1` is supported.
    #[must_use]
    pub fn qr_code_bytes(&self) -> Option<Vec<u8>> {
        self.qr_to_show().and_then(|qr| qr.to_bytes().ok())
    }

    /// The QR verification to show, if `QrCodeShowV1` is supported; the
    /// embedder renders it.
    #[must_use]
    pub fn qr_to_show(&self) -> Option<QrVerification> {
        self.inner
            .qr_to_show
            .lock()
            .expect("mutex is not poisoned")
            .clone()
    }

    /// Cancel the verification request.
    ///
    /// This can be used to decline the request or cancel it at any time.
    pub async fn cancel(&self) -> Result<(), VerificationError> {
        let request = self.inner.request.clone();

        if request.is_done() || request.is_passive() || request.is_cancelled() {
            return Err(VerificationError::WrongState);
        }

        let handle = spawn_tokio!(async move { request.cancel().await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => {
                self.dismiss();
                Ok(())
            }
            Err(cancel_error) => {
                error!("Could not cancel verification request: {cancel_error}");
                Err(VerificationError::Server(Box::new(cancel_error)))
            }
        }
    }

    /// Accept the verification request, with the methods both sides
    /// support.
    pub async fn accept(&self) -> Result<(), VerificationError> {
        let request = self.inner.request.clone();

        let VerificationRequestState::Requested { their_methods, .. } = request.state() else {
            error!("Cannot accept verification that is not in the requested state");
            return Err(VerificationError::WrongState);
        };
        let methods = intersect_methods(self.inner.our_methods.clone(), &their_methods);

        let handle = spawn_tokio!(async move { request.accept_with_methods(methods).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|accept_error| {
                error!("Could not accept verification request: {accept_error}");
                VerificationError::Server(Box::new(accept_error))
            })
    }

    /// Go back to the state to choose a verification method.
    pub fn choose_method(&self) {
        self.set_state(VerificationState::Ready);
    }

    /// Whether the current SAS verification supports emoji.
    #[must_use]
    pub fn sas_supports_emoji(&self) -> bool {
        self.sas_verification()
            .is_some_and(|sas| sas.supports_emoji())
    }

    /// The list of emojis for the current SAS verification, if any.
    #[must_use]
    pub fn sas_emoji(&self) -> Option<[Emoji; 7]> {
        self.sas_verification()?.emoji()
    }

    /// The list of decimals for the current SAS verification, if any.
    #[must_use]
    pub fn sas_decimals(&self) -> Option<(u16, u16, u16)> {
        self.sas_verification()?.decimals()
    }

    /// The user wants to scan a QR code: the embedder opened its scanner.
    pub fn start_qr_code_scan(&self) {
        self.set_state(VerificationState::QrScan);
    }

    /// The QR code was scanned.
    pub async fn qr_code_scanned(&self, data: QrVerificationData) -> Result<(), VerificationError> {
        self.set_state(VerificationState::QrScanned);
        let request = self.inner.request.clone();

        let handle = spawn_tokio!(async move { request.scan_qr_code(data).await });

        match handle.await.expect("task was not aborted") {
            Ok(Some(_)) => Ok(()),
            Ok(None) => {
                error!("Could not validate scanned verification QR code: unknown reason");
                Err(VerificationError::WrongState)
            }
            Err(scan_error) => {
                error!("Could not validate scanned verification QR code: {scan_error}");
                Err(VerificationError::Server(Box::new(scan_error)))
            }
        }
    }

    /// Confirm that the QR code was scanned by the other party.
    pub async fn confirm_qr_code_scanned(&self) -> Result<(), VerificationError> {
        let Some(qr_verification) = self.qr_verification() else {
            error!("Cannot confirm QR Code scan without an ongoing QR verification");
            return Err(VerificationError::WrongState);
        };

        let handle = spawn_tokio!(async move { qr_verification.confirm().await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|confirm_error| {
                error!("Could not confirm scanned verification QR code: {confirm_error}");
                VerificationError::Server(Box::new(confirm_error))
            })
    }

    /// Start a SAS verification.
    pub async fn start_sas(&self) -> Result<(), VerificationError> {
        let request = self.inner.request.clone();
        let handle = spawn_tokio!(async move { request.start_sas().await });

        match handle.await.expect("task was not aborted") {
            Ok(Some(_)) => Ok(()),
            Ok(None) => {
                error!("Could not start SAS verification: unknown reason");
                Err(VerificationError::WrongState)
            }
            Err(start_error) => {
                error!("Could not start SAS verification: {start_error}");
                Err(VerificationError::Server(Box::new(start_error)))
            }
        }
    }

    /// The SAS data does not match.
    pub async fn sas_mismatch(&self) -> Result<(), VerificationError> {
        let Some(sas_verification) = self.sas_verification() else {
            error!("Cannot send SAS mismatch without an ongoing SAS verification");
            return Err(VerificationError::WrongState);
        };

        let handle = spawn_tokio!(async move { sas_verification.mismatch().await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|mismatch_error| {
                error!("Could not send SAS verification mismatch: {mismatch_error}");
                VerificationError::Server(Box::new(mismatch_error))
            })
    }

    /// The SAS data matches.
    pub async fn sas_match(&self) -> Result<(), VerificationError> {
        let Some(sas_verification) = self.sas_verification() else {
            error!("Cannot send SAS match without an ongoing SAS verification");
            return Err(VerificationError::WrongState);
        };

        let handle = spawn_tokio!(async move { sas_verification.confirm().await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|confirm_error| {
                error!("Could not send SAS verification match: {confirm_error}");
                VerificationError::Server(Box::new(confirm_error))
            })
    }

    /// The verification can be dismissed, and removed from its list.
    pub fn dismiss(&self) {
        self.inner.dismissed.set_if_not_eq(true);
    }
}

/// The list of ongoing verification requests.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct VerificationList {
    inner: Arc<VerificationListInner>,
}

#[derive(Debug)]
struct VerificationListInner {
    /// The session this list belongs to.
    session: WeakSession,
    /// The ongoing verification requests.
    list: Mutex<IndexMap<VerificationKey, IdentityVerification>>,
    /// Bumped whenever a verification is added or removed.
    changed: SharedObservable<u64>,
    /// The methods this side supports.
    supported_methods: Mutex<Vec<VerificationMethod>>,
    /// Whether the SDK's handlers are installed.
    initialized: AtomicBool,
}

impl VerificationList {
    /// Create the verification list of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        Self {
            inner: Arc::new(VerificationListInner {
                session,
                list: Mutex::new(IndexMap::new()),
                changed: SharedObservable::new(0),
                supported_methods: Mutex::new(DEFAULT_SUPPORTED_METHODS.to_vec()),
                initialized: AtomicBool::new(false),
            }),
        }
    }

    /// Declare the verification methods this side supports.
    ///
    /// The application adds `QrCodeScanV1` when it has a camera; an
    /// embedder that cannot show a QR code drops `QrCodeShowV1`.
    pub fn set_supported_methods(&self, methods: Vec<VerificationMethod>) {
        *self
            .inner
            .supported_methods
            .lock()
            .expect("mutex is not poisoned") = methods;
    }

    /// The verification methods this side supports.
    #[must_use]
    pub fn supported_methods(&self) -> Vec<VerificationMethod> {
        self.inner
            .supported_methods
            .lock()
            .expect("mutex is not poisoned")
            .clone()
    }

    /// A counter bumped whenever a verification is added or removed.
    pub fn subscribe_changed(&self) -> Subscriber<u64> {
        self.inner.changed.subscribe()
    }

    /// The ongoing verifications, in the order they arrived.
    #[must_use]
    pub fn snapshot(&self) -> Vec<IdentityVerification> {
        self.inner
            .list
            .lock()
            .expect("mutex is not poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Get the verification with the given key.
    #[must_use]
    pub fn get(&self, key: &VerificationKey) -> Option<IdentityVerification> {
        self.inner
            .list
            .lock()
            .expect("mutex is not poisoned")
            .get(key)
            .cloned()
    }

    /// The ongoing session verification, if any.
    #[must_use]
    pub fn ongoing_session_verification(&self) -> Option<IdentityVerification> {
        self.inner
            .list
            .lock()
            .expect("mutex is not poisoned")
            .values()
            .find(|v| v.is_self_verification() && !v.is_finished())
            .cloned()
    }

    /// The ongoing verification in the given room, if any.
    #[must_use]
    pub fn ongoing_room_verification(
        &self,
        room_id: &ruma::RoomId,
    ) -> Option<IdentityVerification> {
        self.inner
            .list
            .lock()
            .expect("mutex is not poisoned")
            .values()
            .find(|v| v.room_id().is_some_and(|id| id == room_id) && !v.is_finished())
            .cloned()
    }

    /// Initialize this list to listen to new verification requests.
    ///
    /// Only to-device requests from the account's own sessions are taken:
    /// verifying another user happens in a room.
    pub fn init(&self) {
        if self.inner.initialized.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        let client = session.client();

        let weak = Arc::downgrade(&self.inner);
        client.add_event_handler(
            move |ev: ToDeviceKeyVerificationRequestEvent, client: Client| {
                let weak = weak.clone();
                async move {
                    let Some(request) = client
                        .encryption()
                        .get_verification_request(&ev.sender, &ev.content.transaction_id)
                        .await
                    else {
                        // This might be normal if the request has already timed out.
                        debug!(
                            "To-device verification request `({}, {})` not found in the SDK",
                            ev.sender, ev.content.transaction_id
                        );
                        return;
                    };

                    if !request.is_self_verification() {
                        // We only support in-room verifications for other users.
                        debug!(
                            "To-device verification request `({}, {})` for other users is not supported",
                            ev.sender, ev.content.transaction_id
                        );
                        return;
                    }

                    if let Some(inner) = weak.upgrade() {
                        VerificationList { inner }
                            .add_to_device_request(request)
                            .await;
                    }
                }
            },
        );

        let weak = Arc::downgrade(&self.inner);
        client.add_event_handler(
            move |ev: OriginalSyncRoomMessageEvent, room: matrix_sdk::Room, client: Client| {
                let weak = weak.clone();
                async move {
                    let MessageType::VerificationRequest(_) = &ev.content.msgtype else {
                        return;
                    };
                    let Some(request) = client
                        .encryption()
                        .get_verification_request(&ev.sender, &ev.event_id)
                        .await
                    else {
                        // This might be normal if the request has already timed out.
                        debug!(
                            "In-room verification request `({}, {})` not found in the SDK",
                            ev.sender, ev.event_id
                        );
                        return;
                    };

                    if let Some(inner) = weak.upgrade() {
                        VerificationList { inner }
                            .add_in_room_request(request, room.room_id())
                            .await;
                    }
                }
            },
        );
    }

    /// Add a verification received via a to-device event.
    async fn add_to_device_request(&self, request: VerificationRequest) {
        if request.is_done() || request.is_cancelled() || request.is_passive() {
            // Ignore requests that are already finished.
            return;
        }

        let verification = IdentityVerification::new(request, self.supported_methods(), None).await;
        self.add(&verification);
    }

    /// Add a verification received via an in-room event.
    async fn add_in_room_request(&self, request: VerificationRequest, room_id: &ruma::RoomId) {
        if request.is_done() || request.is_cancelled() || request.is_passive() {
            // Ignore requests that are already finished.
            return;
        }

        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        let Some(room) = session.room_list().get(room_id) else {
            error!(
                "Room for verification request `({}, {})` not found",
                request.other_user_id(),
                request.flow_id()
            );
            return;
        };

        if matches!(
            room.matrix_room().state(),
            RoomState::Left | RoomState::Banned
        ) {
            // Ignore requests where the user is not in the room anymore.
            return;
        }

        let verification =
            IdentityVerification::new(request, self.supported_methods(), Some(&room)).await;
        self.add(&verification);
    }

    /// Add the given verification to the list.
    fn add(&self, verification: &IdentityVerification) {
        let key = verification.key();

        {
            let mut list = self.inner.list.lock().expect("mutex is not poisoned");
            // Don't add request that already exists.
            if list.contains_key(&key) {
                return;
            }
            list.insert(key.clone(), verification.clone());
        }

        // Remove it when it asks to be dismissed.
        let weak = Arc::downgrade(&self.inner);
        let mut dismissed = verification.subscribe_dismissed();
        RUNTIME.spawn(async move {
            loop {
                if dismissed.get() {
                    if let Some(inner) = weak.upgrade() {
                        VerificationList { inner }.remove(&key);
                    }
                    return;
                }
                if dismissed.next().await.is_none() {
                    return;
                }
            }
        });

        self.inner
            .changed
            .update(|count| *count = count.wrapping_add(1));
    }

    /// Remove the verification with the given key.
    pub fn remove(&self, key: &VerificationKey) {
        let removed = self
            .inner
            .list
            .lock()
            .expect("mutex is not poisoned")
            .shift_remove(key)
            .is_some();

        if removed {
            self.inner
                .changed
                .update(|count| *count = count.wrapping_add(1));
        }
    }

    /// Create and send a new verification request.
    ///
    /// If `user_id` is `None`, a new session verification is started for our
    /// own user and sent to other devices.
    pub async fn create(
        &self,
        user_id: Option<&UserId>,
    ) -> Result<IdentityVerification, VerificationError> {
        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(VerificationError::NoSession)?;
        let user_id = user_id.unwrap_or_else(|| session.user_id()).to_owned();

        let supported_methods = self.supported_methods();

        let Some(identity) = ensure_crypto_identity(&session, &user_id).await else {
            error!("Could not create identity verification: cryptographic identity not found");
            return Err(VerificationError::NoIdentity);
        };

        let handle = spawn_tokio!(async move {
            identity
                .request_verification_with_methods(supported_methods)
                .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(request) => {
                let room = if let Some(room_id) = request.room_id() {
                    let Some(room) = session.room_list().get(room_id) else {
                        error!(
                            "Room for verification request `({}, {})` not found",
                            request.other_user_id(),
                            request.flow_id()
                        );
                        return Err(VerificationError::NoRoom);
                    };
                    Some(room)
                } else {
                    None
                };

                let verification =
                    IdentityVerification::new(request, self.supported_methods(), room.as_ref())
                        .await;
                self.add(&verification);

                Ok(verification)
            }
            Err(create_error) => {
                error!("Could not create identity verification: {create_error}");
                Err(VerificationError::Request(Box::new(create_error)))
            }
        }
    }
}

/// The cryptographic identity of the given user, fetched from the
/// homeserver when the local one may be stale — the application's
/// `User::ensure_crypto_identity`.
///
/// When we get the remote crypto identity of a user manually, it is cached
/// locally but it is not kept up-to-date unless the user is tracked. That's
/// why it's important to only use the local crypto identity if the user is
/// tracked.
async fn ensure_crypto_identity(
    session: &super::Session,
    user_id: &UserId,
) -> Option<UserIdentity> {
    let encryption = session.client().encryption();

    let should_have_local = if user_id == session.user_id() {
        true
    } else {
        let encryption_clone = encryption.clone();
        let handle = spawn_tokio!(async move { encryption_clone.tracked_users().await });

        match handle.await.expect("task was not aborted") {
            Ok(tracked_users) => tracked_users.contains(user_id),
            Err(tracked_error) => {
                error!("Could not get tracked users: {tracked_error}");
                // We are not sure, but let us try to get the local user identity first.
                true
            }
        }
    };

    if should_have_local {
        let encryption_clone = encryption.clone();
        let local_user_id = user_id.to_owned();
        let handle =
            spawn_tokio!(async move { encryption_clone.get_user_identity(&local_user_id).await });

        match handle.await.expect("task was not aborted") {
            Ok(Some(identity)) => return Some(identity),
            Ok(None) => {}
            Err(local_error) => {
                error!("Could not get local crypto identity: {local_error}");
            }
        }
    }

    let remote_user_id = user_id.to_owned();
    let handle =
        spawn_tokio!(async move { encryption.request_user_identity(&remote_user_id).await });

    match handle.await.expect("task was not aborted") {
        Ok(identity) => identity,
        Err(remote_error) => {
            error!("Could not request remote crypto identity: {remote_error}");
            None
        }
    }
}

/// Get the intersection or our methods and their methods.
fn intersect_methods(
    our_methods: Vec<VerificationMethod>,
    their_methods: &[VerificationMethod],
) -> Vec<VerificationMethod> {
    let mut supported_methods = our_methods;

    supported_methods.retain(|m| match m {
        VerificationMethod::SasV1 => their_methods.contains(&VerificationMethod::SasV1),
        VerificationMethod::QrCodeScanV1 => {
            their_methods.contains(&VerificationMethod::QrCodeShowV1)
                && their_methods.contains(&VerificationMethod::ReciprocateV1)
        }
        VerificationMethod::QrCodeShowV1 => {
            their_methods.contains(&VerificationMethod::QrCodeScanV1)
                && their_methods.contains(&VerificationMethod::ReciprocateV1)
        }
        VerificationMethod::ReciprocateV1 => {
            (their_methods.contains(&VerificationMethod::QrCodeShowV1)
                || their_methods.contains(&VerificationMethod::QrCodeScanV1))
                && their_methods.contains(&VerificationMethod::ReciprocateV1)
        }
        _ => false,
    });

    supported_methods
}

/// Start listening for requests on the runtime, for a `Session` accessor
/// that cannot await.
pub(crate) fn spawn_init(list: &VerificationList) {
    let list = list.clone();
    RUNTIME.spawn(async move {
        list.init();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_intersect_as_the_application_intersects_them() {
        let ours = vec![
            VerificationMethod::SasV1,
            VerificationMethod::QrCodeShowV1,
            VerificationMethod::ReciprocateV1,
        ];

        // They can scan and reciprocate: we can show, and reciprocate.
        let theirs = [
            VerificationMethod::SasV1,
            VerificationMethod::QrCodeScanV1,
            VerificationMethod::ReciprocateV1,
        ];
        assert_eq!(
            intersect_methods(ours.clone(), &theirs),
            vec![
                VerificationMethod::SasV1,
                VerificationMethod::QrCodeShowV1,
                VerificationMethod::ReciprocateV1,
            ]
        );

        // They can only show: we cannot scan, so no QR flow — but
        // reciprocating stays, as the application keeps it, and the
        // `Ready` handling is what drops it from the choice.
        let theirs = [
            VerificationMethod::SasV1,
            VerificationMethod::QrCodeShowV1,
            VerificationMethod::ReciprocateV1,
        ];
        assert_eq!(
            intersect_methods(ours.clone(), &theirs),
            vec![VerificationMethod::SasV1, VerificationMethod::ReciprocateV1]
        );

        // No reciprocate on their side: QR is off the table.
        let theirs = [VerificationMethod::SasV1, VerificationMethod::QrCodeScanV1];
        assert_eq!(
            intersect_methods(ours, &theirs),
            vec![VerificationMethod::SasV1]
        );
    }
}
