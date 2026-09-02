//! The account's push registration, and its notifications settings.
//!
//! The settings are in [`settings`]; the rest of this file is the push
//! registration.
//!
//! The authority is `src/utils/android_push.rs` — the GTK Android port's
//! own `UnifiedPush` implementation, which is the mature version of exactly
//! this and which the facade's transcription lost three things from.
//!
//! Two things it does that this does not, and both are deliberate rather
//! than dropped — see [`Session::set_push_gateway`] for the push format,
//! which is a contract with the embedder's notification code and was
//! settled by keeping the payload, and for the pusher this must not
//! delete.

use ruma::api::client::push::{Pusher, PusherIds, PusherInit, PusherKind, get_pushers, set_pusher};
use tracing::{debug, info};

mod body;
mod settings;

pub(crate) use self::settings::spawn_load;
pub use self::{
    body::NotificationBody,
    settings::{
        NotificationsError, NotificationsGlobalSetting, NotificationsRoomSetting,
        NotificationsSettings, NotificationsSpecialRule,
    },
};
use crate::{UserFacingError, config, spawn_tokio};

/// What can go wrong while registering for push.
#[derive(Debug, thiserror::Error)]
pub enum PushError {
    /// The homeserver refused.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl UserFacingError for PushError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder renders an SDK error itself; this is the
            // fallback the Kotlin side gets.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// The application id a pusher registered before the fix below was aimed
/// at.
///
/// It was once hard-coded to the debug build's, so a device that
/// registered then holds a pusher under that id pointing at the very
/// endpoint about to be registered again — two pushers, one endpoint,
/// every notification twice.
const LEGACY_APP_ID: &str = "io.github.steeb_k.commune.skeleton";

impl super::Session {
    /// Point the homeserver's push at the given gateway.
    ///
    /// `gateway_url` is the Matrix push gateway
    /// (`…/_matrix/push/v1/notify`) and `pushkey` the endpoint that
    /// identifies this device.
    ///
    /// **The push format is left unset, and that was decided rather than
    /// overlooked.** `src/utils/android_push.rs` sets
    /// `PushFormat::EventIdOnly` and calls it mandatory, because otherwise
    /// the homeserver POSTs the whole event to the gateway — for an
    /// unencrypted room, the sender, the room and the body, through a
    /// third-party service the user chose only as a wake-up. The
    /// application can afford that narrowing because its Android
    /// notification path fetches the event by ID afterwards. The Kotlin
    /// application cannot: `Push.kt` posts straight from the gateway
    /// payload — it reads `type` to keep a call push from becoming a
    /// message notification, and `sender`, `room_name` and `content.body`
    /// to have anything to say — so narrowing the format here would empty
    /// its notifications and revive a bug its own comment records fixing.
    ///
    /// **Settled 1 September 2026: keep the payload.** The notification
    /// arrives complete and instantly, without waking a sync to fetch what
    /// the push already carried, and what it discloses to the gateway is
    /// accepted. An encrypted room discloses nothing but its metadata in
    /// any case, since the body is ciphertext. **Whoever changes this must
    /// change `Push.kt` in the same commit**, or every notification on
    /// Android silently becomes "Commune / New message" and a call push
    /// starts posting one.
    ///
    /// **A pusher this account holds under our application id but with
    /// another pushkey is not ours to remove**, however stale it looks.
    /// The application id is the same for every install of this client, so
    /// "everything under our app id" on the homeserver is also the user's
    /// other phones — the application's own note on `previous_endpoint`
    /// says so, and it deletes only the endpoint *this* registration moved
    /// off, which it knows because it wrote it down. The core keeps no such
    /// record, so it removes nothing it cannot prove is its own; an
    /// endpoint that changes without the old one being retired leaves a
    /// pusher the homeserver keeps `POST`ing to, and closing that wants
    /// somewhere to remember the previous endpoint.
    pub async fn set_push_gateway(
        &self,
        gateway_url: &str,
        pushkey: &str,
    ) -> Result<(), PushError> {
        use ruma::push::HttpPusherData;

        let client = self.client();
        let gateway_url = gateway_url.to_owned();
        let pushkey = pushkey.to_owned();

        spawn_tokio!(async move {
            // The legacy registration goes first, and it is keyed on this
            // device's own pushkey, so it can only ever be ours.
            if config::app_id() != LEGACY_APP_ID {
                let legacy = PusherIds::new(pushkey.clone(), LEGACY_APP_ID.to_owned());
                if let Err(legacy_error) =
                    client.send(set_pusher::v3::Request::delete(legacy)).await
                {
                    debug!("No legacy pusher to remove: {legacy_error}");
                }
            }

            // Registering the same endpoint again is a request that
            // changes nothing, and the application skips it too.
            let already = client
                .send(get_pushers::v3::Request::new())
                .await
                .is_ok_and(|response| {
                    response.pushers.iter().any(|pusher| {
                        pusher.ids.app_id == config::app_id() && pusher.ids.pushkey == pushkey
                    })
                });
            if already {
                debug!("The push endpoint is already registered as a pusher");
                return Ok(());
            }

            // No `format`, which the specification reads as "send the
            // whole event". Deliberate — see the note on this method, and
            // do not narrow it without changing `Push.kt` too.
            let data = HttpPusherData::new(gateway_url);

            let pusher: Pusher = PusherInit {
                ids: PusherIds::new(pushkey, config::app_id().to_owned()),
                kind: PusherKind::Http(data),
                app_display_name: config::app_name().to_owned(),
                // Shown in other clients' session lists, so it describes
                // this installation to readers elsewhere and is not
                // translated. The embedder says which platform it is.
                device_display_name: config::device_display_name().to_owned(),
                profile_tag: None,
                // Moot beside `EventIdOnly`: the homeserver sends no text
                // to localise. The application computes one from the
                // desktop's locale.
                lang: "en".to_owned(),
            }
            .into();

            client
                .send(set_pusher::v3::Request::post(pusher))
                .await
                .map(|_| ())
                .map_err(|error| PushError::Server(Box::new(error.into())))?;

            info!("Registered the push endpoint as a pusher");
            Ok(())
        })
        .await
        .expect("task was not aborted")
    }

    /// Remove the pusher with the given pushkey, so the homeserver stops
    /// pushing to it.
    pub async fn remove_push_gateway(&self, pushkey: &str) -> Result<(), PushError> {
        let client = self.client();
        let ids = PusherIds::new(pushkey.to_owned(), config::app_id().to_owned());

        spawn_tokio!(async move { client.send(set_pusher::v3::Request::delete(ids)).await })
            .await
            .expect("task was not aborted")
            .map(|_| ())
            .map_err(|error| PushError::Server(Box::new(error.into())))
    }
}
