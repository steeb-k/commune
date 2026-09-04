//! The security of a session: its crypto identity, its verification, and
//! account recovery.
//!
//! The headless counterpart of the application's `SessionSecurity`
//! (`src/session/security.rs`) — the three states it watches, and the
//! three flags it derives from the recovery state — plus the operations the
//! application's crypto setup views and encryption page perform on the
//! session: creating the crypto identity, enabling recovery and recovering,
//! and exporting and importing room keys. What stayed in the application is
//! the `AuthDialog` that answers the homeserver's authentication stages, and
//! every sentence.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use eyeball::{SharedObservable, Subscriber};
use futures_util::StreamExt;
use matrix_sdk::encryption::{
    VerificationState as SdkVerificationState,
    recovery::{RecoveryError as SdkRecoveryError, RecoveryState as SdkRecoveryState},
    secret_storage::SecretStorageError,
};
use ruma::api::client::uiaa::{AuthData, UiaaInfo};
use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

use super::WeakSession;
use crate::{RUNTIME, UserFacingError, spawn_tokio, utils::LoadingState};

/// The state of the crypto identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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

/// The state of the verification of the session.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SessionVerificationState {
    /// The state is not known yet.
    #[default]
    Unknown,
    /// The session is verified.
    Verified,
    /// The session is not verified.
    Unverified,
}

impl From<SdkVerificationState> for SessionVerificationState {
    fn from(value: SdkVerificationState) -> Self {
        match value {
            SdkVerificationState::Unknown => Self::Unknown,
            SdkVerificationState::Verified => Self::Verified,
            SdkVerificationState::Unverified => Self::Unverified,
        }
    }
}

/// The state of the recovery.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

impl From<SdkRecoveryState> for RecoveryState {
    fn from(value: SdkRecoveryState) -> Self {
        match value {
            SdkRecoveryState::Unknown => Self::Unknown,
            SdkRecoveryState::Disabled => Self::Disabled,
            SdkRecoveryState::Enabled => Self::Enabled,
            SdkRecoveryState::Incomplete => Self::Incomplete,
        }
    }
}

/// What can go wrong while creating the crypto identity.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    /// The session is gone.
    #[error("the session is no longer available")]
    NoSession,
    /// The homeserver wants an authentication stage completed first.
    ///
    /// Boxed because the info carries every flow the homeserver offers.
    #[error("the homeserver wants an authentication stage completed")]
    Uiaa(Box<UiaaInfo>),
    /// The homeserver asked for a stage this core cannot answer: the
    /// application opens a browser page for it.
    #[error("this homeserver asks for a sign-in step this app cannot answer")]
    UnsupportedAuth,
    /// The homeserver refused for another reason.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(Box<matrix_sdk::Error>),
}

impl UserFacingError for BootstrapError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NoSession => "The session is no longer available.".to_owned(),
            Self::Uiaa(_) | Self::Server(_) => "Could not create the crypto identity".to_owned(),
            Self::UnsupportedAuth => {
                "This homeserver asks for a step this app cannot answer yet.".to_owned()
            }
        }
    }
}

/// What can go wrong while recovering the account, or setting recovery up.
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    /// The session is gone.
    #[error("the session is no longer available")]
    NoSession,
    /// The recovery passphrase or key is invalid.
    #[error("the recovery passphrase or key is invalid")]
    InvalidKey,
    /// The recovery data could not be accessed.
    #[error(transparent)]
    Access(Box<SdkRecoveryError>),
    /// Recovery could not be enabled or reset.
    #[error(transparent)]
    Enable(Box<SdkRecoveryError>),
}

impl UserFacingError for RecoveryError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NoSession => "The session is no longer available.".to_owned(),
            Self::InvalidKey => "The recovery passphrase or key is invalid".to_owned(),
            Self::Access(_) => "Could not access recovery data".to_owned(),
            Self::Enable(_) => "Could not set up account recovery".to_owned(),
        }
    }
}

/// What recovering left the session with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Every secret is here.
    Complete,
    /// The recovery data was not complete: some secrets are still missing.
    Incomplete,
}

/// What can go wrong while exporting or importing room keys.
#[derive(Debug, thiserror::Error)]
pub enum RoomKeysError {
    /// The session is gone.
    #[error("the session is no longer available")]
    NoSession,
    /// The export failed.
    #[error(transparent)]
    Export(Box<dyn std::error::Error + Send + Sync>),
    /// The import failed.
    #[error(transparent)]
    Import(Box<dyn std::error::Error + Send + Sync>),
}

impl UserFacingError for RoomKeysError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NoSession => "The session is no longer available.".to_owned(),
            Self::Export(error) | Self::Import(error) => error.to_string(),
        }
    }
}

/// Information about the security of a session.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct SessionSecurity {
    inner: Arc<SessionSecurityInner>,
}

#[derive(Debug)]
struct SessionSecurityInner {
    /// The session this belongs to.
    session: WeakSession,
    /// The state of the crypto identity.
    crypto_identity_state: SharedObservable<CryptoIdentityState>,
    /// The state of the verification.
    verification_state: SharedObservable<SessionVerificationState>,
    /// The state of recovery.
    recovery_state: SharedObservable<RecoveryState>,
    /// Whether all the cross-signing keys are available.
    cross_signing_keys_available: SharedObservable<bool>,
    /// Whether the room keys backup is enabled.
    backup_enabled: SharedObservable<bool>,
    /// Whether the room keys backup exists on the homeserver.
    backup_exists_on_server: SharedObservable<bool>,
    /// How far the first read has got.
    state: SharedObservable<LoadingState>,
    /// The tasks following the SDK's streams.
    abort_handles: Mutex<Vec<AbortHandle>>,
}

impl Drop for SessionSecurityInner {
    fn drop(&mut self) {
        if let Ok(handles) = self.abort_handles.get_mut() {
            for handle in handles.drain(..) {
                handle.abort();
            }
        }
    }
}

impl SessionSecurity {
    /// Create the security of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        Self {
            inner: Arc::new(SessionSecurityInner {
                session,
                crypto_identity_state: SharedObservable::new(CryptoIdentityState::Unknown),
                verification_state: SharedObservable::new(SessionVerificationState::Unknown),
                recovery_state: SharedObservable::new(RecoveryState::Unknown),
                cross_signing_keys_available: SharedObservable::new(false),
                backup_enabled: SharedObservable::new(false),
                backup_exists_on_server: SharedObservable::new(false),
                state: SharedObservable::new(LoadingState::Initial),
                abort_handles: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Start following the session's security, and load its current state.
    ///
    /// The states are only meaningful once the encryption tasks have run,
    /// which the application's setup view waits for, so this does too.
    pub async fn load(&self) {
        if self.inner.state.get() != LoadingState::Initial {
            return;
        }
        self.inner.state.set_if_not_eq(LoadingState::Loading);

        let Some(session) = self.inner.session.upgrade() else {
            self.inner.state.set_if_not_eq(LoadingState::Error);
            return;
        };

        let encryption = session.client().encryption();
        spawn_tokio!(async move {
            encryption.wait_for_e2ee_initialization_tasks().await;
        })
        .await
        .expect("task was not aborted");

        self.watch_verification_state();
        self.watch_recovery_state();
        self.watch_crypto_identity_state().await;

        self.inner.state.set_if_not_eq(LoadingState::Ready);
    }

    /// Load the state unless it has already been loaded, waiting for a load
    /// in progress.
    pub async fn ensure_loaded(&self) {
        match self.inner.state.get() {
            LoadingState::Ready | LoadingState::Error => {}
            LoadingState::Initial => self.load().await,
            LoadingState::Loading => {
                let mut state = self.inner.state.subscribe();
                while !matches!(state.get(), LoadingState::Ready | LoadingState::Error) {
                    if state.next().await.is_none() {
                        break;
                    }
                }
            }
        }
    }

    /// The state of the crypto identity.
    #[must_use]
    pub fn crypto_identity_state(&self) -> CryptoIdentityState {
        self.inner.crypto_identity_state.get()
    }

    /// Subscribe to the state of the crypto identity.
    pub fn subscribe_crypto_identity_state(&self) -> Subscriber<CryptoIdentityState> {
        self.inner.crypto_identity_state.subscribe()
    }

    /// The state of the verification.
    #[must_use]
    pub fn verification_state(&self) -> SessionVerificationState {
        self.inner.verification_state.get()
    }

    /// Subscribe to the state of the verification.
    pub fn subscribe_verification_state(&self) -> Subscriber<SessionVerificationState> {
        self.inner.verification_state.subscribe()
    }

    /// The state of recovery.
    #[must_use]
    pub fn recovery_state(&self) -> RecoveryState {
        self.inner.recovery_state.get()
    }

    /// Subscribe to the state of recovery.
    pub fn subscribe_recovery_state(&self) -> Subscriber<RecoveryState> {
        self.inner.recovery_state.subscribe()
    }

    /// Whether all the cross-signing keys are available.
    #[must_use]
    pub fn cross_signing_keys_available(&self) -> bool {
        self.inner.cross_signing_keys_available.get()
    }

    /// Whether the room keys backup is enabled.
    #[must_use]
    pub fn backup_enabled(&self) -> bool {
        self.inner.backup_enabled.get()
    }

    /// Whether the room keys backup exists on the homeserver.
    #[must_use]
    pub fn backup_exists_on_server(&self) -> bool {
        self.inner.backup_exists_on_server.get()
    }

    /// Subscribe to whether all the cross-signing keys are available.
    pub fn subscribe_cross_signing_keys_available(&self) -> Subscriber<bool> {
        self.inner.cross_signing_keys_available.subscribe()
    }

    /// Subscribe to whether the room keys backup is enabled.
    pub fn subscribe_backup_enabled(&self) -> Subscriber<bool> {
        self.inner.backup_enabled.subscribe()
    }

    /// Subscribe to whether the room keys backup exists on the homeserver.
    pub fn subscribe_backup_exists_on_server(&self) -> Subscriber<bool> {
        self.inner.backup_exists_on_server.subscribe()
    }

    /// Listen to crypto identity changes: the identities and the devices of
    /// our own user.
    async fn watch_crypto_identity_state(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        let own_user_id = session.user_id().clone();
        let encryption = session.client().encryption();

        let encryption_clone = encryption.clone();
        let handle = spawn_tokio!(async move { encryption_clone.user_identities_stream().await });
        let identities_stream = match handle.await.expect("task was not aborted") {
            Ok(stream) => stream,
            Err(stream_error) => {
                error!("Could not get user identities stream: {stream_error}");
                // All method calls here have the same error, so we can return early.
                return;
            }
        };

        let weak = Arc::downgrade(&self.inner);
        let user_id = own_user_id.clone();
        let identities_handle = RUNTIME
            .spawn(identities_stream.for_each(move |updates| {
                let weak = weak.clone();
                let user_id = user_id.clone();
                async move {
                    if (updates.new.contains_key(&user_id)
                        || updates.changed.contains_key(&user_id))
                        && let Some(inner) = weak.upgrade()
                    {
                        SessionSecurity { inner }.load_crypto_identity_state().await;
                    }
                }
            }))
            .abort_handle();

        let handle = spawn_tokio!(async move { encryption.devices_stream().await });
        let devices_stream = match handle.await.expect("task was not aborted") {
            Ok(stream) => stream,
            Err(stream_error) => {
                error!("Could not get devices stream: {stream_error}");
                return;
            }
        };

        let weak = Arc::downgrade(&self.inner);
        let user_id = own_user_id;
        let devices_handle = RUNTIME
            .spawn(devices_stream.for_each(move |updates| {
                let weak = weak.clone();
                let user_id = user_id.clone();
                async move {
                    if (updates.new.contains_key(&user_id)
                        || updates.changed.contains_key(&user_id))
                        && let Some(inner) = weak.upgrade()
                    {
                        SessionSecurity { inner }.load_crypto_identity_state().await;
                    }
                }
            }))
            .abort_handle();

        self.inner
            .abort_handles
            .lock()
            .expect("mutex is not poisoned")
            .extend([identities_handle, devices_handle]);

        self.load_crypto_identity_state().await;
    }

    /// Load the crypto identity state.
    async fn load_crypto_identity_state(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        let client = session.client();
        let user_id = session.user_id().clone();

        let client_clone = client.clone();
        let identity_user_id = user_id.clone();
        let user_identity_handle = spawn_tokio!(async move {
            client_clone
                .encryption()
                .get_user_identity(&identity_user_id)
                .await
        });

        let has_identity = match user_identity_handle.await.expect("task was not aborted") {
            Ok(Some(_)) => true,
            Ok(None) => {
                debug!("No crypto user identity found");
                false
            }
            Err(identity_error) => {
                error!("Could not get crypto user identity: {identity_error}");
                false
            }
        };

        if !has_identity {
            self.inner
                .crypto_identity_state
                .set_if_not_eq(CryptoIdentityState::Missing);
            return;
        }

        let devices_handle =
            spawn_tokio!(async move { client.encryption().get_user_devices(&user_id).await });

        let own_device = session.device_id().clone();
        let has_other_sessions = match devices_handle.await.expect("task was not aborted") {
            Ok(devices) => devices
                .devices()
                .any(|d| d.device_id() != own_device && d.is_cross_signed_by_owner()),
            Err(devices_error) => {
                error!("Could not get user devices: {devices_error}");
                // If there are actually no other devices, the user can still
                // reset the crypto identity.
                true
            }
        };

        let state = if has_other_sessions {
            CryptoIdentityState::OtherSessions
        } else {
            CryptoIdentityState::LastManStanding
        };

        self.inner.crypto_identity_state.set_if_not_eq(state);
    }

    /// Listen to verification state changes.
    fn watch_verification_state(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        let mut stream = session.client().encryption().verification_state();
        // Get the current value right away.
        stream.reset();

        let weak = Arc::downgrade(&self.inner);
        let handle = RUNTIME
            .spawn(stream.for_each(move |state| {
                let weak = weak.clone();
                async move {
                    if let Some(inner) = weak.upgrade() {
                        inner.verification_state.set_if_not_eq(state.into());
                    }
                }
            }))
            .abort_handle();

        self.inner
            .abort_handles
            .lock()
            .expect("mutex is not poisoned")
            .push(handle);
    }

    /// Listen to recovery state changes.
    fn watch_recovery_state(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        let stream = session.client().encryption().recovery().state_stream();

        let weak = Arc::downgrade(&self.inner);
        let handle = RUNTIME
            .spawn(stream.for_each(move |state| {
                let weak = weak.clone();
                async move {
                    if let Some(inner) = weak.upgrade() {
                        SessionSecurity { inner }
                            .update_recovery_state(state.into())
                            .await;
                    }
                }
            }))
            .abort_handle();

        self.inner
            .abort_handles
            .lock()
            .expect("mutex is not poisoned")
            .push(handle);
    }

    /// Update the session for the given recovery state.
    async fn update_recovery_state(&self, state: RecoveryState) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        let (cross_signing_keys_available, backup_enabled, backup_exists_on_server) = if matches!(
            state,
            RecoveryState::Enabled
        ) {
            (true, true, true)
        } else {
            let encryption = session.client().encryption();
            let backups = encryption.backups();

            let handle = spawn_tokio!(async move { encryption.cross_signing_status().await });
            let cross_signing_keys_available = handle
                .await
                .expect("task was not aborted")
                .is_some_and(|status| status.is_complete());

            let handle = spawn_tokio!(async move {
                if backups.are_enabled().await {
                    (true, true)
                } else {
                    let backup_exists_on_server = match backups.exists_on_server().await {
                        Ok(exists) => exists,
                        Err(backup_error) => {
                            warn!(
                                "Could not request whether recovery backup exists on homeserver: {backup_error}"
                            );
                            // If the request failed, we have to try to delete the backup to
                            // avoid unsolvable errors.
                            true
                        }
                    };
                    (false, backup_exists_on_server)
                }
            });
            let (backup_enabled, backup_exists_on_server) =
                handle.await.expect("task was not aborted");

            (
                cross_signing_keys_available,
                backup_enabled,
                backup_exists_on_server,
            )
        };

        self.inner
            .cross_signing_keys_available
            .set_if_not_eq(cross_signing_keys_available);
        self.inner.backup_enabled.set_if_not_eq(backup_enabled);
        self.inner
            .backup_exists_on_server
            .set_if_not_eq(backup_exists_on_server);

        self.inner.recovery_state.set_if_not_eq(state);
    }

    /// Create the crypto identity — cross-signing — for the account,
    /// answering the homeserver's authentication with the given data.
    ///
    /// One request: a homeserver that wants a stage completed answers with
    /// [`BootstrapError::Uiaa`], and the caller chooses the stage with
    /// [`crate::login::AuthStage::next`] and calls again with its data —
    /// the application's `AuthDialog` loop, with the dialog left to the
    /// embedder.
    pub async fn bootstrap_cross_signing(
        &self,
        auth: Option<AuthData>,
    ) -> Result<(), BootstrapError> {
        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(BootstrapError::NoSession)?;
        let encryption = session.client().encryption();

        let handle = spawn_tokio!(async move { encryption.bootstrap_cross_signing(auth).await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => Ok(()),
            Err(bootstrap_error) => {
                if let Some(uiaa_info) = bootstrap_error.as_uiaa_response() {
                    return Err(BootstrapError::Uiaa(Box::new(uiaa_info.clone())));
                }

                error!("Could not bootstrap cross-signing: {bootstrap_error:?}");
                Err(BootstrapError::Server(Box::new(bootstrap_error)))
            }
        }
    }

    /// Reset the crypto identity: new cross-signing keys, signed by
    /// nobody yet. The application's recovery setup offers it when the
    /// old identity is lost.
    ///
    /// The homeserver guards it with user-interactive authentication;
    /// `auth` answers the stage the first call reported.
    pub async fn reset_cross_signing(&self, auth: Option<AuthData>) -> Result<(), BootstrapError> {
        use matrix_sdk::encryption::CrossSigningResetAuthType;

        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(BootstrapError::NoSession)?;
        let encryption = session.client().encryption();

        let handle = spawn_tokio!(async move {
            let Some(reset) = encryption
                .reset_cross_signing()
                .await
                .map_err(|error| BootstrapError::Server(Box::new(error)))?
            else {
                // No authentication was needed.
                return Ok(());
            };

            match reset.auth_type().clone() {
                CrossSigningResetAuthType::Uiaa(uiaa_info) => match auth {
                    Some(auth) => reset
                        .auth(Some(auth))
                        .await
                        .map_err(|error| BootstrapError::Server(Box::new(error))),
                    None => Err(BootstrapError::Uiaa(Box::new(uiaa_info))),
                },
                CrossSigningResetAuthType::OAuth(_) => Err(BootstrapError::UnsupportedAuth),
            }
        });

        handle.await.expect("task was not aborted")
    }

    /// Enable recovery, with the given passphrase when there is one,
    /// returning the recovery key.
    pub async fn enable_recovery(&self, passphrase: Option<&str>) -> Result<String, RecoveryError> {
        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(RecoveryError::NoSession)?;
        let recovery = session.client().encryption().recovery();
        let passphrase = passphrase.map(ToOwned::to_owned);

        let handle = spawn_tokio!(async move {
            let mut enable = recovery.enable();
            if let Some(passphrase) = passphrase.as_deref() {
                enable = enable.with_passphrase(passphrase);
            }

            enable.await
        });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|enable_error| {
                error!("Could not enable account recovery: {enable_error}");
                RecoveryError::Enable(Box::new(enable_error))
            })
    }

    /// Reset the recovery key, with the given passphrase when there is one,
    /// returning the new key.
    pub async fn reset_recovery_key(
        &self,
        passphrase: Option<&str>,
    ) -> Result<String, RecoveryError> {
        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(RecoveryError::NoSession)?;
        let recovery = session.client().encryption().recovery();
        let passphrase = passphrase.map(ToOwned::to_owned);

        let handle = spawn_tokio!(async move {
            let mut reset = recovery.reset_key();
            if let Some(passphrase) = passphrase.as_deref() {
                reset = reset.with_passphrase(passphrase);
            }

            reset.await
        });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|reset_error| {
                error!("Could not reset account recovery key: {reset_error}");
                RecoveryError::Enable(Box::new(reset_error))
            })
    }

    /// Recover the account's secrets with the given recovery key or
    /// passphrase.
    ///
    /// Even when recovery succeeds, the recovery data may not have been
    /// complete; the outcome says whether some secrets are still missing.
    pub async fn recover(&self, key: &str) -> Result<RecoveryOutcome, RecoveryError> {
        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(RecoveryError::NoSession)?;
        let encryption = session.client().encryption();
        let recovery = encryption.recovery();
        let key = key.to_owned();

        let handle = spawn_tokio!(async move { recovery.recover(&key).await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => {
                // Because the SDK uses multiple threads, we are only sure of
                // the SDK's recovery state at this point, not the session's.
                if encryption.recovery().state() == SdkRecoveryState::Incomplete {
                    Ok(RecoveryOutcome::Incomplete)
                } else {
                    Ok(RecoveryOutcome::Complete)
                }
            }
            Err(recover_error) => {
                error!("Could not recover account: {recover_error}");
                match recover_error {
                    SdkRecoveryError::SecretStorage(SecretStorageError::SecretStorageKey(_)) => {
                        Err(RecoveryError::InvalidKey)
                    }
                    other => Err(RecoveryError::Access(Box::new(other))),
                }
            }
        }
    }

    /// Export the room keys to the file at the given path, encrypted with
    /// the given passphrase.
    pub async fn export_room_keys(
        &self,
        path: PathBuf,
        passphrase: &str,
    ) -> Result<(), RoomKeysError> {
        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(RoomKeysError::NoSession)?;
        let encryption = session.client().encryption();
        let passphrase = passphrase.to_owned();

        let handle = spawn_tokio!(async move {
            encryption
                .export_room_keys(path, &passphrase, |_| true)
                .await
        });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|export_error| {
                error!("Could not export the room keys: {export_error}");
                RoomKeysError::Export(Box::new(export_error))
            })
    }

    /// Import the room keys from the file at the given path, encrypted with
    /// the given passphrase.
    ///
    /// Returns how many keys came in.
    pub async fn import_room_keys(
        &self,
        path: PathBuf,
        passphrase: &str,
    ) -> Result<usize, RoomKeysError> {
        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(RoomKeysError::NoSession)?;
        let encryption = session.client().encryption();
        let passphrase = passphrase.to_owned();

        let handle =
            spawn_tokio!(async move { encryption.import_room_keys(path, &passphrase).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|counts| counts.imported_count)
            .map_err(|import_error| {
                error!("Could not import the room keys: {import_error}");
                RoomKeysError::Import(Box::new(import_error))
            })
    }
}

/// Start loading the security of the given session on the runtime, for a
/// `Session` accessor that cannot await.
pub(crate) fn spawn_load(security: &SessionSecurity) {
    let security = security.clone();
    RUNTIME.spawn(async move {
        security.load().await;
    });
}
