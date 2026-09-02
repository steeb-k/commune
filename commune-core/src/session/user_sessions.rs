//! The account's other sessions — its devices — headless.
//!
//! The application's `session/user_sessions_list/` without the `GObject`s,
//! the `SortListModel` or the authentication dialog. Four things the
//! application does and the facade did not, all of them behaviour:
//!
//! * **It follows the list.** `devices_stream()` reports device updates, and
//!   the application reloads on any that names this user — or on an empty one,
//!   which is how a disconnection arrives without saying whose. The facade
//!   fetched once per call.
//! * **It merges two sources.** `/devices` knows the display name, the
//!   last-seen time and the IP; the crypto store knows whether cross-signing
//!   vouches for the device. A device can be in either alone, and the
//!   application shows it either way.
//! * **It degrades.** When `/devices` fails but the crypto store answers, the
//!   application lists what it has. The facade returned an error and showed
//!   nothing.
//! * **It breaks ties.** The other sessions sort by last-seen descending and
//!   then by device ID, so devices the server never dated keep a stable order.
//!   The facade's sort had no tiebreak, so they shuffled.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use eyeball::{SharedObservable, Subscriber};
use futures_util::StreamExt;
use matrix_sdk::encryption::identities::UserDevices;
use ruma::{DeviceId, OwnedDeviceId, api::client::device::Device as DeviceData};
use tokio::task::AbortHandle;
use tracing::error;

use super::WeakSession;
use crate::{RUNTIME, UserFacingError, spawn_tokio, utils::LoadingState};

/// What can go wrong while managing the account's sessions.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// The session this list belongs to is gone.
    #[error("the session is no longer available")]
    NoSession,
    /// Neither the device API nor the crypto store would answer.
    #[error("the sessions could not be loaded")]
    NotLoaded,
    /// The homeserver wants the account's password before it will do this,
    /// and none was given.
    ///
    /// The application answers this with its authentication dialog, which
    /// speaks more than passwords; an embedder that has only a password
    /// field gets to ask for it rather than being told the request failed.
    #[error("the homeserver asks for the account password")]
    NeedsPassword,
    /// The homeserver asked for a stage this core cannot answer.
    #[error("this homeserver asks for a sign-in step this app cannot answer")]
    UnsupportedAuth,
    /// The homeserver refused.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl UserFacingError for DeviceError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NoSession => "The session is no longer available.".to_owned(),
            Self::NotLoaded => "Could not load the sessions.".to_owned(),
            Self::NeedsPassword => "Enter your password to confirm.".to_owned(),
            Self::UnsupportedAuth => {
                "This homeserver asks for a step this app cannot answer yet.".to_owned()
            }
            Self::Server(error) => error.to_string(),
        }
    }
}

/// One of the account's sessions, as both sources describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The ID of the device.
    pub device_id: OwnedDeviceId,
    /// Its display name, when one is set.
    pub display_name: Option<String>,
    /// Whether it is the session running this code.
    pub is_current: bool,
    /// Whether cross-signing vouches for it.
    ///
    /// False for a device with no cryptographic identity at all, which is
    /// what the application shows too.
    pub is_verified: bool,
    /// When it was last seen, in milliseconds since the epoch.
    pub last_seen_ts: Option<u64>,
    /// The IP it was last seen from.
    pub last_seen_ip: Option<String>,
}

/// The account's sessions.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct UserSessions {
    inner: Arc<UserSessionsInner>,
}

#[derive(Debug)]
struct UserSessionsInner {
    /// The session these belong to.
    session: WeakSession,
    /// The devices, this one first and the rest as the application sorts
    /// them.
    devices: SharedObservable<Vec<Device>>,
    /// How far the first read has got.
    state: SharedObservable<LoadingState>,
    /// The task watching the SDK for device updates.
    watch_handle: Mutex<Option<AbortHandle>>,
}

impl Drop for UserSessionsInner {
    fn drop(&mut self) {
        if let Ok(Some(handle)) = self.watch_handle.get_mut().map(Option::take) {
            handle.abort();
        }
    }
}

impl UserSessions {
    /// Create the session list of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        Self {
            inner: Arc::new(UserSessionsInner {
                session,
                devices: SharedObservable::new(Vec::new()),
                state: SharedObservable::new(LoadingState::Initial),
                watch_handle: Mutex::new(None),
            }),
        }
    }

    /// Read the list, and follow it from there.
    pub async fn load(&self) {
        self.watch().await;
        self.reload().await;
    }

    /// Read the list unless it has already been read.
    pub async fn ensure_loaded(&self) {
        if self.inner.state.get() == LoadingState::Ready {
            return;
        }
        self.load().await;
    }

    /// How far the first read has got.
    #[must_use]
    pub fn state(&self) -> LoadingState {
        self.inner.state.get()
    }

    /// Subscribe to how far the read has got.
    pub fn subscribe_state(&self) -> Subscriber<LoadingState> {
        self.inner.state.subscribe()
    }

    /// A snapshot of the account's sessions.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Device> {
        self.inner.devices.get()
    }

    /// The current list, and every version of it that follows.
    pub fn subscribe(&self) -> Subscriber<Vec<Device>> {
        self.inner.devices.subscribe()
    }

    /// Rename one of the account's sessions.
    pub async fn rename(&self, device_id: &DeviceId, name: &str) -> Result<(), DeviceError> {
        let session = self.inner.session.upgrade().ok_or(DeviceError::NoSession)?;
        let client = session.client();
        let owned = device_id.to_owned();
        // The application does not trim here; the name is whatever the
        // person typed, and a homeserver takes it as given.
        let name = name.to_owned();

        spawn_tokio!(async move { client.rename_device(&owned, &name).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| DeviceError::Server(Box::new(error.into())))?;

        self.reload().await;
        Ok(())
    }

    /// Sign another of the account's sessions out.
    ///
    /// The homeserver almost always demands the account password for this.
    /// Called without one, the first attempt learns that and reports
    /// [`DeviceError::NeedsPassword`] rather than failing, so the embedder
    /// can ask; called with one, the second attempt answers the stage.
    pub async fn sign_out(
        &self,
        device_id: &DeviceId,
        password: Option<&str>,
    ) -> Result<(), DeviceError> {
        use ruma::api::client::uiaa::{AuthData, MatrixUserIdentifier, Password};

        let session = self.inner.session.upgrade().ok_or(DeviceError::NoSession)?;
        let client = session.client();
        let user_id = session.user_id().to_string();
        let devices = vec![device_id.to_owned()];
        let password = password.map(ToOwned::to_owned);

        let result = spawn_tokio!(async move {
            match client.delete_devices(&devices, None).await {
                Ok(_) => Ok(()),
                Err(error) => {
                    let Some(info) = error.as_uiaa_response() else {
                        return Err(DeviceError::Server(Box::new(error.into())));
                    };
                    // The stage list says what this homeserver will take.
                    // Only the password stage can be answered here.
                    let wants_password = info.flows.iter().any(|flow| {
                        flow.stages
                            .iter()
                            .any(|stage| stage.as_str() == "m.login.password")
                    });
                    if !wants_password {
                        return Err(DeviceError::UnsupportedAuth);
                    }
                    let Some(password) = password else {
                        return Err(DeviceError::NeedsPassword);
                    };

                    let auth = AuthData::Password(ruma::assign!(
                        Password::new(MatrixUserIdentifier::new(user_id).into(), password),
                        { session: info.session.clone() }
                    ));
                    client
                        .delete_devices(&devices, Some(auth))
                        .await
                        .map(|_| ())
                        .map_err(|error| DeviceError::Server(Box::new(error.into())))
                }
            }
        })
        .await
        .expect("task was not aborted");

        result?;
        self.reload().await;
        Ok(())
    }

    /// Watch the SDK for device updates, replacing any previous watch.
    async fn watch(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        let user_id = session.user_id().clone();

        let stream = match session.client().encryption().devices_stream().await {
            Ok(stream) => stream,
            Err(error) => {
                error!("Could not access the user sessions stream: {error}");
                return;
            }
        };

        let weak = Arc::downgrade(&self.inner);
        let handle = RUNTIME
            .spawn(async move {
                let mut stream = std::pin::pin!(stream);
                while let Some(updates) = stream.next().await {
                    // An update about somebody else is not ours to spend a
                    // request on — but an empty one is how a disconnection
                    // arrives, and it does not say whose, so it is taken.
                    if !updates.new.contains_key(&user_id)
                        && !updates.changed.contains_key(&user_id)
                        && (!updates.new.is_empty() || !updates.changed.is_empty())
                    {
                        continue;
                    }
                    let Some(inner) = weak.upgrade() else { break };
                    Self { inner }.reload().await;
                }
            })
            .abort_handle();

        if let Some(previous) = self
            .inner
            .watch_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// Read both sources and publish the merged list.
    async fn reload(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        // Loading twice at once would spend two round trips to reach the
        // same answer, as the application's own guard says.
        if self.inner.state.get() == LoadingState::Loading {
            return;
        }
        self.inner.state.set(LoadingState::Loading);

        let client = session.client();
        let user_id = session.user_id().clone();
        let own_device_id = session.device_id().clone();

        let (api, crypto) = spawn_tokio!(async move {
            let crypto = match client.encryption().get_user_devices(&user_id).await {
                Ok(devices) => Some(devices),
                Err(error) => {
                    error!("Could not get crypto sessions for {user_id}: {error}");
                    None
                }
            };
            let api = match client.devices().await {
                Ok(response) => Some(response.devices),
                Err(error) => {
                    error!("Could not get the sessions list for {user_id}: {error}");
                    None
                }
            };
            (api, crypto)
        })
        .await
        .expect("task was not aborted");

        if api.is_none() && crypto.is_none() {
            self.inner.state.set(LoadingState::Error);
            return;
        }

        let devices = merge(api, crypto.as_ref(), &own_device_id);
        self.inner.devices.set_if_not_eq(devices);
        self.inner.state.set(LoadingState::Ready);
    }
}

/// Merge what the two sources know into one list, sorted as the
/// application presents it: this session first, then by last-seen
/// descending, then by device ID so that undated devices keep still.
fn merge(
    api: Option<Vec<DeviceData>>,
    crypto: Option<&UserDevices>,
    own_device_id: &DeviceId,
) -> Vec<Device> {
    let mut api: HashMap<OwnedDeviceId, DeviceData> = api
        .into_iter()
        .flatten()
        .map(|device| (device.device_id.clone(), device))
        .collect();

    let mut devices = Vec::with_capacity(api.len());

    // The devices with a cryptographic identity first, taking the API's
    // half of each where there is one.
    for device in crypto.into_iter().flat_map(UserDevices::devices) {
        let data = api.remove(device.device_id());
        devices.push(Device {
            device_id: device.device_id().to_owned(),
            display_name: data.as_ref().and_then(|data| data.display_name.clone()),
            is_current: device.device_id() == own_device_id,
            is_verified: device.is_verified(),
            last_seen_ts: data
                .as_ref()
                .and_then(|data| data.last_seen_ts)
                .map(|ts| ts.0.into()),
            last_seen_ip: data.and_then(|data| data.last_seen_ip),
        });
    }

    // Whatever the API listed and the crypto store did not is a device
    // that does not support encryption.
    for data in api.into_values() {
        devices.push(Device {
            is_current: data.device_id == own_device_id,
            device_id: data.device_id,
            display_name: data.display_name,
            is_verified: false,
            last_seen_ts: data.last_seen_ts.map(|ts| ts.0.into()),
            last_seen_ip: data.last_seen_ip,
        });
    }

    devices.sort_by(|a, b| {
        b.is_current
            .cmp(&a.is_current)
            .then_with(|| b.last_seen_ts.cmp(&a.last_seen_ts))
            .then_with(|| a.device_id.cmp(&b.device_id))
    });

    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_device(id: &str, last_seen: Option<u64>) -> DeviceData {
        let mut device = DeviceData::new(id.into());
        device.last_seen_ts = last_seen.map(|ts| {
            ruma::MilliSecondsSinceUnixEpoch(ruma::UInt::new(ts).expect("test value fits"))
        });
        device
    }

    /// This session leads, whatever the server said about when it was
    /// last seen.
    #[test]
    fn the_current_session_comes_first() {
        let api = vec![
            api_device("OLD", Some(10)),
            api_device("MINE", Some(1)),
            api_device("NEW", Some(20)),
        ];
        let devices = merge(Some(api), None, "MINE".into());

        assert_eq!(devices[0].device_id, "MINE");
        assert!(devices[0].is_current);
        assert_eq!(devices[1].device_id, "NEW");
        assert_eq!(devices[2].device_id, "OLD");
    }

    /// The tiebreak the facade did not have: two devices the server never
    /// dated must not shuffle between reads.
    #[test]
    fn undated_devices_are_ordered_by_id() {
        let api = vec![
            api_device("ZZZ", None),
            api_device("AAA", None),
            api_device("MMM", None),
        ];
        let devices = merge(Some(api), None, "OTHER".into());

        let ids: Vec<_> = devices.iter().map(|d| d.device_id.as_str()).collect();
        assert_eq!(ids, ["AAA", "MMM", "ZZZ"]);
    }

    /// A dated device outranks an undated one.
    #[test]
    fn a_dated_device_comes_before_an_undated_one() {
        let api = vec![api_device("AAA", None), api_device("ZZZ", Some(5))];
        let devices = merge(Some(api), None, "OTHER".into());

        assert_eq!(devices[0].device_id, "ZZZ");
        assert_eq!(devices[1].device_id, "AAA");
    }

    /// A device the API listed and the crypto store did not is one that
    /// does not support encryption — it is still a session the account
    /// has, and it is still listed.
    #[test]
    fn a_device_without_a_crypto_identity_is_still_listed() {
        let api = vec![api_device("PLAIN", Some(1))];
        let devices = merge(Some(api), None, "OTHER".into());

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_id, "PLAIN");
        assert!(!devices[0].is_verified);
    }
}
