//! UnifiedPush on Android: registration with a distributor, and the receiving
//! end of its broadcasts.
//!
//! The foreground service (`utils::android_sync_service`) keeps messages
//! arriving for six hours a day; this is the beginning of the real answer.
//! UnifiedPush (spec `AND_3.1.0`) is a distributor application — ntfy, for one
//! — that holds the device's one push connection and talks to applications in
//! broadcasts: Commune sends `REGISTER` with a token, the distributor answers
//! `NEW_ENDPOINT` with an HTTPS endpoint, the homeserver's push gateway POSTs
//! to that endpoint, and every push thereafter arrives as a `MESSAGE`
//! broadcast on `PushReceiver.java`, which forwards everything here through
//! `nativeReceive`. The measurements behind the design — the gateway chain,
//! the plaintext payload, the pushkey self-cleaning — are in
//! `doc/android-push-plan.md`.
//!
//! Three things about the shape of this are worth knowing.
//!
//! **The receiver trusts nothing.** It is exported — the distributor is
//! another application — so anything on the device can send it a
//! `NEW_ENDPOINT` or a `MESSAGE`. The token is what makes that harmless: it
//! never leaves this device except toward the chosen distributor, and a
//! broadcast carrying a token that is not the stored one is dropped here,
//! with a warning, before anything reads the rest of it.
//!
//! **Discovery is a `PackageManager` query, selection is not built yet.**
//! Every `AND_3` distributor exposes an activity on `unifiedpush://link`
//! (the `<queries>` entry in `patch-manifest.sh` is what makes other packages
//! visible to the query at all). Until the first-time-setup screen exists —
//! step 5 of the plan — the first distributor found is the one used, which is
//! correct on a device with one and arbitrary on a device with several.
//!
//! **State survives the process, in a file.** The token has to: it is the
//! identity of the registration, a fresh one per run would pile up dead
//! registrations on the distributor, and the broadcasts it validates arrive
//! in processes started long after the one that registered. It lives beside
//! the secret store in `DataType::Persistent` — which on Android is
//! `no_backup`, out of reach of both the system backup and the directory
//! GTK's glue wipes (see `doc/android.md`).

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use gtk::{
    glib::{self, GString},
    prelude::*,
};
use jni::{
    JNIEnv,
    objects::{JByteArray, JClass, JObject, JString, JValue},
};
use matrix_sdk::{Client, reqwest::Url};
use matrix_sdk_ui::notification_client::{
    NotificationClient, NotificationItem, NotificationProcessSetup, NotificationStatus,
};
use ruma::{
    OwnedEventId, OwnedRoomId,
    api::client::push::{PusherIds, PusherInit, PusherKind, get_pushers},
    push::{HttpPusherData, PushFormat},
};
use tracing::{debug, info, warn};

use super::{
    DataType,
    android::{self, AndroidJniError},
    http::{self, HttpError},
};
use crate::{Application, session::Session, session_list::SessionInfoExt, spawn, spawn_tokio};

/// The actions a connector sends a distributor.
const ACTION_REGISTER: &str = "org.unifiedpush.android.distributor.REGISTER";
const ACTION_MESSAGE_ACK: &str = "org.unifiedpush.android.distributor.MESSAGE_ACK";

/// The actions a distributor sends a connector, answered in [`nativeReceive`].
const ACTION_NEW_ENDPOINT: &str = "org.unifiedpush.android.connector.NEW_ENDPOINT";
const ACTION_MESSAGE: &str = "org.unifiedpush.android.connector.MESSAGE";
const ACTION_REGISTRATION_FAILED: &str = "org.unifiedpush.android.connector.REGISTRATION_FAILED";
const ACTION_UNREGISTERED: &str = "org.unifiedpush.android.connector.UNREGISTERED";
const ACTION_TEMP_UNAVAILABLE: &str = "org.unifiedpush.android.connector.TEMP_UNAVAILABLE";

/// The deep link every distributor exposes an activity for, and therefore what
/// finding one means querying for.
const DISTRIBUTOR_LINK: &str = "unifiedpush://link";

/// Where the registration state lives, under
/// [`DataType::Persistent`][super::DataType].
const STATE_FILE: &str = "unifiedpush";
const STATE_GROUP: &str = "unifiedpush";

/// Whether registration has been attempted this run.
///
/// Registering again with the same token is documented as harmless — the
/// distributor answers with a fresh `NEW_ENDPOINT` — but once per run is
/// enough to keep the endpoint current.
static REGISTERED: AtomicBool = AtomicBool::new(false);

/// The registration state that has to survive the process.
#[derive(Debug, Default)]
struct State {
    /// The token identifying this registration, a UUID minted here.
    token: Option<String>,
    /// The package name of the distributor the token was registered with.
    distributor: Option<String>,
    /// The endpoint the distributor last announced.
    endpoint: Option<String>,
    /// The endpoint before that, kept until its pusher has been removed.
    ///
    /// Only pushkeys this device has held may ever be deleted: the `app_id`
    /// is the same for every Commune on Android, so "everything under our
    /// `app_id`" on the homeserver includes the user's other devices.
    previous_endpoint: Option<String>,
    /// Whether the endpoint's pusher registration last succeeded.
    ///
    /// What separates "a distributor answered" from "the homeserver will
    /// POST": the endpoint may have no Matrix gateway, or the registration
    /// may have failed. The foreground service keeps running until this is
    /// true — see [`service_needed()`].
    pusher_ok: bool,
}

impl State {
    fn path() -> std::path::PathBuf {
        DataType::Persistent.dir_path().join(STATE_FILE)
    }

    fn load() -> Self {
        let keyfile = glib::KeyFile::new();
        if keyfile
            .load_from_file(Self::path(), glib::KeyFileFlags::NONE)
            .is_err()
        {
            // Not there yet, which is what a first run looks like.
            return Self::default();
        }

        let string = |key: &str| keyfile.string(STATE_GROUP, key).ok().map(Into::into);

        Self {
            token: string("token"),
            distributor: string("distributor"),
            endpoint: string("endpoint"),
            previous_endpoint: string("previous_endpoint"),
            pusher_ok: keyfile.boolean(STATE_GROUP, "pusher_ok").unwrap_or(false),
        }
    }

    fn save(&self) {
        let keyfile = glib::KeyFile::new();
        for (key, value) in [
            ("token", &self.token),
            ("distributor", &self.distributor),
            ("endpoint", &self.endpoint),
            ("previous_endpoint", &self.previous_endpoint),
        ] {
            if let Some(value) = value {
                keyfile.set_string(STATE_GROUP, key, value);
            }
        }
        keyfile.set_boolean(STATE_GROUP, "pusher_ok", self.pusher_ok);

        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = keyfile.save_to_file(&path) {
            warn!("Could not save the push registration state: {error}");
        }
    }
}

/// Register with a UnifiedPush distributor, if one is installed.
///
/// Called when the main window is presented, like the notification setup, and
/// once per run. Must be called on the GTK thread with the application on
/// screen — the first `Context` capture needs the window.
///
/// This is step 1 scaffolding: it registers with the first distributor found,
/// unconditionally. Which distributor, whether at all, and what the endpoint
/// is then registered as a pusher for are steps 2, 4 and 5 of
/// `doc/android-push-plan.md`.
pub(crate) fn init() {
    if REGISTERED.swap(true, Ordering::Relaxed) {
        return;
    }

    if let Err(error) = register() {
        warn!("Could not register with a UnifiedPush distributor: {error}");
    }
}

/// Whether the foreground service is needed for delivery.
///
/// The mode setting decides ("service" and "push" are the user's overrides,
/// step 5 of the plan); on "auto", the service runs exactly when push does not
/// deliver — no endpoint, or an endpoint whose pusher registration has not
/// succeeded. Callers re-evaluate on present and whenever
/// [`notify_delivery_changed()`] fires; a transition that happens in the
/// background takes effect on the next present, because the service may only
/// be started from the foreground.
pub(crate) fn service_needed(settings: &gtk::gio::Settings) -> bool {
    match settings.string("background-delivery").as_str() {
        "service" => true,
        "push" => false,
        _ => {
            let state = State::load();
            !(state.endpoint.is_some() && state.pusher_ok)
        }
    }
}

/// Have the application re-evaluate how background delivery happens.
///
/// Callable from any thread; the decision belongs to the main context.
fn notify_delivery_changed() {
    glib::MainContext::default().invoke(|| {
        Application::default().update_background_delivery();
    });
}

/// The body of [`init()`], so that one place reports what went wrong.
fn register() -> Result<(), AndroidJniError> {
    let distributors = distributors()?;

    let Some(distributor) = distributors.first() else {
        let state = State::load();
        if state.distributor.is_some() || state.endpoint.is_some() {
            // The distributor this registration lived on was uninstalled. The
            // token is unreachable and the endpoint is dead; the pusher on the
            // homeserver cleans itself up through the gateway's pushkey
            // rejection, measured in step 0 of the plan.
            info!("The UnifiedPush distributor is gone; back to the foreground service");
            State::default().save();
            notify_delivery_changed();
        } else {
            info!(
                "No UnifiedPush distributor is installed; only the foreground service can deliver"
            );
        }
        return Ok(());
    };
    info!("UnifiedPush distributors: {distributors:?}, registering with {distributor}");

    let mut state = State::load();
    if state.distributor.as_deref() != Some(distributor.as_str()) {
        // A new distributor means a new registration; a token is its identity,
        // so it must not be reused across them.
        state.token = None;
    }
    let token = state
        .token
        .get_or_insert_with(|| glib::uuid_string_random().into())
        .clone();
    state.distributor = Some(distributor.clone());
    // Saved before the broadcast goes out: the answer can arrive in a process
    // this one knows nothing about, and the token is what that process
    // validates it with.
    state.save();

    let context = android::application_context()?;

    android::with_env(|env| {
        let intent = env.new_object("android/content/Intent", "()V", &[])?;
        let action = JObject::from(env.new_string(ACTION_REGISTER)?);
        env.call_method(
            &intent,
            "setAction",
            "(Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&action)],
        )?;

        // Explicit, so no other application can answer a broadcast that
        // carries the token.
        let package = JObject::from(env.new_string(distributor)?);
        env.call_method(
            &intent,
            "setPackage",
            "(Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&package)],
        )?;

        // `Intent.FLAG_INCLUDE_STOPPED_PACKAGES`. A freshly installed
        // distributor that has never been opened is in the stopped state, and
        // a broadcast without this flag is silently not delivered to it —
        // measured on the emulator: registration against a never-opened ntfy
        // produced nothing, and the identical broadcast after opening it once
        // answered in 178 ms. Being installed is what makes it the user's
        // distributor; having been opened should not be part of the contract.
        env.call_method(
            &intent,
            "addFlags",
            "(I)Landroid/content/Intent;",
            &[JValue::Int(0x0000_0020)],
        )?;

        put_string(env, &intent, "token", &token)?;

        // AND_2 distributors read the sender's package out of an extra; AND_3
        // ones are told it by the platform below. Carrying both costs one
        // extra and works with either.
        let own_package = env
            .call_method(
                context.as_obj(),
                "getPackageName",
                "()Ljava/lang/String;",
                &[],
            )?
            .l()?;
        let name = JObject::from(env.new_string("application")?);
        env.call_method(
            &intent,
            "putExtra",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&name), JValue::Object(&own_package)],
        )?;

        send_broadcast(env, context.as_obj(), &intent)?;
        debug!("Sent a UnifiedPush registration to {distributor}");

        Ok(())
    })
}

/// The package names of the installed UnifiedPush distributors.
fn distributors() -> Result<Vec<String>, AndroidJniError> {
    let context = android::application_context()?;

    android::with_env(|env| {
        let uri = JObject::from(env.new_string(DISTRIBUTOR_LINK)?);
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&uri)],
            )?
            .l()?;

        let view = JObject::from(env.new_string("android.intent.action.VIEW")?);
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(&view), JValue::Object(&uri)],
        )?;

        let manager = env
            .call_method(
                context.as_obj(),
                "getPackageManager",
                "()Landroid/content/pm/PackageManager;",
                &[],
            )?
            .l()?;

        // The `int`-flags overload is deprecated in favor of one taking a
        // `ResolveInfoFlags`, but it is not going anywhere and the replacement
        // is three more lookups for the same list.
        let list = env
            .call_method(
                &manager,
                "queryIntentActivities",
                "(Landroid/content/Intent;I)Ljava/util/List;",
                &[JValue::Object(&intent), JValue::Int(0)],
            )?
            .l()?;

        let size = env.call_method(&list, "size", "()I", &[])?.i()?;
        let mut found = Vec::new();

        for i in 0..size {
            let info = env
                .call_method(&list, "get", "(I)Ljava/lang/Object;", &[JValue::Int(i)])?
                .l()?;
            let activity = env
                .get_field(&info, "activityInfo", "Landroid/content/pm/ActivityInfo;")?
                .l()?;
            let package = env
                .get_field(&activity, "packageName", "Ljava/lang/String;")?
                .l()?;
            found.push(String::from(env.get_string(&JString::from(package))?));
        }

        Ok(found)
    })
}

/// The pusher `app_id` — one per platform, so a homeserver (and the user, in
/// another client's session list) can tell this apart from a future FCM build,
/// and so that removing this device's pusher can never touch another's: what
/// distinguishes two Androids under the same `app_id` is the pushkey alone.
const PUSHER_APP_ID: &str = "io.github.steeb_k.commune.android";

/// The path of the Matrix push gateway on the endpoint's server, per the
/// UnifiedPush Matrix convention; answering on it is what makes an endpoint
/// usable as a pushkey. Measured against ntfy in step 0 of the plan.
const GATEWAY_PATH: &str = "/_matrix/push/v1/notify";

/// The errors that can occur keeping a session's pusher current.
#[derive(Debug, thiserror::Error)]
enum PusherError {
    /// A request to the homeserver failed.
    #[error(transparent)]
    Matrix(#[from] matrix_sdk::Error),
    /// The pusher listing failed.
    #[error(transparent)]
    MatrixHttp(#[from] matrix_sdk::HttpError),
    /// The gateway probe failed on the wire.
    #[error(transparent)]
    Probe(#[from] HttpError),
    /// The endpoint, or what its server answered, is not usable.
    #[error("{0}")]
    Gateway(String),
}

/// Make sure the account behind `client` has a pusher for the current
/// endpoint, and no pusher for an endpoint this device has abandoned.
///
/// Must be called from the tokio runtime. Called when a session becomes ready
/// — once per run per session, which also repairs a registration a previous
/// run left half-done — and for every session when the endpoint changes.
///
/// With no endpoint this does nothing, which is what makes it safe to call
/// unconditionally: no distributor, no pusher, and the foreground service
/// remains the only delivery.
pub(crate) async fn ensure_pusher(client: Client) {
    // The two callers race on a fresh registration — the endpoint arriving
    // and the session becoming ready happen within milliseconds of each other
    // — and each would see no pusher and register one. An upsert makes that
    // harmless on the homeserver and wasteful on the wire (measured: the same
    // registration logged twice, 23 ms apart). One at a time; the loser
    // re-reads and finds the work done.
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _serialized = ONE_AT_A_TIME.lock().await;

    let ok = match try_ensure_pusher(&client).await {
        Ok(()) => true,
        Err(error) => {
            warn!("Could not keep the push endpoint registered as a pusher: {error}");
            false
        }
    };

    // What the delivery mode reads. Only meaningful with an endpoint — a
    // no-op run against no endpoint proves nothing either way — and only
    // worth a save and a re-evaluation when it changes.
    let mut state = State::load();
    if state.endpoint.is_some() && state.pusher_ok != ok {
        state.pusher_ok = ok;
        state.save();
        notify_delivery_changed();
    }
}

/// The body of [`ensure_pusher()`], so that one place reports what went wrong.
async fn try_ensure_pusher(client: &Client) -> Result<(), PusherError> {
    let state = State::load();
    let Some(endpoint) = state.endpoint else {
        debug!("No push endpoint to register as a pusher");
        return Ok(());
    };

    let pushers = client.send(get_pushers::v3::Request::new()).await?.pushers;
    let ours: Vec<&str> = pushers
        .iter()
        .filter(|pusher| pusher.ids.app_id == PUSHER_APP_ID)
        .map(|pusher| pusher.ids.pushkey.as_str())
        .collect();

    // The pusher of an endpoint this registration has moved off — and only
    // that one. See the note on [`State::previous_endpoint`].
    if let Some(previous) = &state.previous_endpoint
        && previous != &endpoint
        && ours.contains(&previous.as_str())
    {
        client
            .pusher()
            .delete(PusherIds::new(previous.clone(), PUSHER_APP_ID.to_owned()))
            .await?;
        info!("Removed the pusher of a superseded push endpoint");
    }

    if ours.contains(&endpoint.as_str()) {
        debug!("The push endpoint is already registered as a pusher");
        return Ok(());
    }

    let gateway = matrix_gateway(&endpoint).await?;

    let mut data = HttpPusherData::new(gateway);
    // Mandatory for content, not merely preferred: events are E2EE, so a full
    // payload would carry ciphertext at best — and the metadata that does not
    // need to travel, still would.
    data.format = Some(PushFormat::EventIdOnly);

    let pusher = PusherInit {
        ids: PusherIds::new(endpoint.clone(), PUSHER_APP_ID.to_owned()),
        kind: PusherKind::Http(data),
        app_display_name: "Commune".to_owned(),
        // Shown in other clients' session lists; not translated because it
        // describes this installation to readers elsewhere, in whatever
        // language they use.
        device_display_name: "Commune on Android".to_owned(),
        profile_tag: None,
        lang: pusher_lang(),
    };

    client.pusher().set(pusher.into(), false).await?;
    info!("Registered the push endpoint as a pusher");

    Ok(())
}

/// The URL of the Matrix push gateway serving `endpoint`, confirmed to be one.
///
/// The UnifiedPush convention is that the gateway lives on the endpoint's own
/// server — with ntfy they are the same service — and announces itself at
/// [`GATEWAY_PATH`]. An endpoint whose server does not answer there gets no
/// pusher at all: the alternative would be routing every notification through
/// some third-party gateway the user never chose.
async fn matrix_gateway(endpoint: &str) -> Result<String, PusherError> {
    let url = Url::parse(endpoint)
        .and_then(|url| url.join(GATEWAY_PATH))
        .map_err(|error| PusherError::Gateway(format!("Unusable endpoint URL: {error}")))?;

    let body = http::fetch(url.as_str(), 4096).await?;
    let answer: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|error| PusherError::Gateway(format!("Unreadable gateway answer: {error}")))?;

    if answer["unifiedpush"]["gateway"] != "matrix" {
        return Err(PusherError::Gateway(
            "The endpoint's server has no Matrix push gateway".to_owned(),
        ));
    }

    Ok(url.into())
}

/// The language for the pusher, from the session locale.
///
/// `language_names()` speaks glibc (`en_US.UTF-8`, `C`); the pusher field
/// wants the shape of a language tag (`en`, `en-US`).
fn pusher_lang() -> String {
    let names = glib::language_names();
    let name = names.first().map_or("C", GString::as_str);
    let base = name.split(['.', '@']).next().unwrap_or_default();

    if base.is_empty() || base == "C" || base == "POSIX" {
        return "en".to_owned();
    }
    base.replace('_', "-")
}

/// Remove the pushers this device holds on the account behind `client`.
///
/// Must be called from the tokio runtime, and before the session is logged
/// out — the pusher belongs to the account, not to the device, so nothing
/// removes it implicitly, and after logout there is no token to remove it
/// with. Best-effort: the homeserver refuses to delete a pusher that is not
/// there, which is the common case for a session that never had one.
pub(crate) async fn remove_pusher(client: Client) {
    let state = State::load();

    for endpoint in [state.endpoint, state.previous_endpoint]
        .into_iter()
        .flatten()
    {
        if let Err(error) = client
            .pusher()
            .delete(PusherIds::new(endpoint, PUSHER_APP_ID.to_owned()))
            .await
        {
            debug!("Could not remove a pusher while logging out: {error}");
        }
    }
}

/// Re-register the current endpoint as a pusher, for every session.
///
/// Callable from any thread — the session list belongs to the main context,
/// so that is where the walk happens; each session's work then goes to tokio.
fn ensure_pushers_of_sessions() {
    glib::MainContext::default().invoke(|| {
        for object in Application::default().session_list().snapshot() {
            let Ok(session) = object.downcast::<Session>() else {
                continue;
            };
            let client = session.client();
            spawn_tokio!(async move {
                ensure_pusher(client).await;
            });
        }
    });
}

/// Send `intent` as a broadcast, sharing this application's identity with the
/// receiving distributor where the platform can.
///
/// `AND_3` distributors on API 34 and later learn who is registering from the
/// platform, through the share-identity broadcast option. Below 34 the spec
/// wants a `PendingIntent` dance this does not perform — the `application`
/// extra above is what such a distributor gets, which is the `AND_2` way.
fn send_broadcast(
    env: &mut jni::AttachGuard<'_>,
    context: &JObject,
    intent: &JObject,
) -> Result<(), AndroidJniError> {
    let sdk = env
        .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
        .i()?;

    if sdk < 34 {
        env.call_method(
            context,
            "sendBroadcast",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(intent)],
        )?;
        return Ok(());
    }

    let options = env
        .call_static_method(
            "android/app/BroadcastOptions",
            "makeBasic",
            "()Landroid/app/BroadcastOptions;",
            &[],
        )?
        .l()?;
    let options = env
        .call_method(
            &options,
            "setShareIdentityEnabled",
            "(Z)Landroid/app/BroadcastOptions;",
            &[JValue::Bool(u8::from(true))],
        )?
        .l()?;
    let bundle = env
        .call_method(&options, "toBundle", "()Landroid/os/Bundle;", &[])?
        .l()?;

    env.call_method(
        context,
        "sendBroadcast",
        "(Landroid/content/Intent;Ljava/lang/String;Landroid/os/Bundle;)V",
        &[
            JValue::Object(intent),
            JValue::Object(&JObject::null()),
            JValue::Object(&bundle),
        ],
    )?;

    Ok(())
}

/// Put a string extra on the given `Intent`.
fn put_string(
    env: &mut jni::AttachGuard<'_>,
    intent: &JObject,
    name: &str,
    value: &str,
) -> Result<(), AndroidJniError> {
    let name = JObject::from(env.new_string(name)?);
    let value = JObject::from(env.new_string(value)?);

    env.call_method(
        intent,
        "putExtra",
        "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&name), JValue::Object(&value)],
    )?;

    Ok(())
}

/// Acknowledge receipt of a broadcast the distributor marked with an `id`.
///
/// `AND_3` requires it, and it is what lets a distributor stop redelivering.
fn send_ack(
    env: &mut JNIEnv,
    context: &JObject,
    distributor: &str,
    token: &str,
    id: &str,
) -> Result<(), AndroidJniError> {
    let intent = env.new_object("android/content/Intent", "()V", &[])?;
    let action = JObject::from(env.new_string(ACTION_MESSAGE_ACK)?);
    env.call_method(
        &intent,
        "setAction",
        "(Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&action)],
    )?;
    let package = JObject::from(env.new_string(distributor)?);
    env.call_method(
        &intent,
        "setPackage",
        "(Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&package)],
    )?;

    for (name, value) in [("token", token), ("id", id)] {
        let name = JObject::from(env.new_string(name)?);
        let value = JObject::from(env.new_string(value)?);
        env.call_method(
            &intent,
            "putExtra",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&name), JValue::Object(&value)],
        )?;
    }

    env.call_method(
        context,
        "sendBroadcast",
        "(Landroid/content/Intent;)V",
        &[JValue::Object(&intent)],
    )?;

    Ok(())
}

/// The body of a Matrix push, as the gateway POSTs it and the distributor
/// hands it over.
///
/// This is the plaintext arm only — ntfy's gateway forwards the notify JSON
/// as it is, measured in step 0 of the plan. A gateway that encrypts
/// (RFC 8291, `aes128gcm`) would need a decryption arm here that nothing
/// currently exercises; such a payload fails the parse and is logged.
#[derive(Debug, serde::Deserialize)]
struct PushPayload {
    notification: PushNotification,
}

/// The half of the notify body the wake needs.
///
/// Both fields are absent on a badge-only push — the unread count changing,
/// typically because the events were read elsewhere. `event_id_only` means
/// there is nothing else worth naming here.
#[derive(Debug, serde::Deserialize)]
struct PushNotification {
    event_id: Option<OwnedEventId>,
    room_id: Option<OwnedRoomId>,
}

/// Act on the payload of one push message.
fn receive_message(bytes: &[u8]) {
    match serde_json::from_slice::<PushPayload>(bytes) {
        Ok(payload) => match (payload.notification.room_id, payload.notification.event_id) {
            (Some(room_id), Some(event_id)) => {
                info!("A push message arrived");
                debug!("Pushed event {event_id} in room {room_id}");
                deliver_pushed_event(room_id, event_id);
            }
            _ => {
                // Nothing to fetch and nothing to show; the unread counts on
                // the notifications already posted are not re-rendered.
                debug!("A badge-only push arrived");
            }
        },
        Err(error) => warn!("Could not parse a push message payload: {error}"),
    }
}

/// Turn a pushed event into a posted notification, through the same
/// `Notifications::show_push()` a synced event takes — one formatter, one tap
/// path, and the same deterministic tag, so whichever of the push and the
/// sync arrives second replaces rather than duplicates.
///
/// Callable from any thread; the work starts on the main context because the
/// session list lives there.
fn deliver_pushed_event(room_id: OwnedRoomId, event_id: OwnedEventId) {
    glib::MainContext::default().invoke(move || {
        spawn!(async move {
            show_pushed_event(room_id, event_id).await;
        });
    });
}

/// The main-context half of delivering a pushed event.
async fn show_pushed_event(room_id: OwnedRoomId, event_id: OwnedEventId) {
    // Find the session that knows the room. In a process the push itself
    // started — the ordinary case — the sessions are still restoring, so this
    // waits for them, bounded well inside the ten seconds the freezer allows
    // a woken process (measured; see `doc/android.md` S5b).
    let mut found = None;
    for _ in 0..16 {
        for object in Application::default().session_list().snapshot() {
            let Ok(session) = object.downcast::<Session>() else {
                continue;
            };
            if let Some(room) = session.client().get_room(&room_id) {
                found = Some((session, room));
                break;
            }
        }
        if found.is_some() {
            break;
        }
        glib::timeout_future(Duration::from_millis(500)).await;
    }

    let Some((session, matrix_room)) = found else {
        warn!("No session knows the room of a pushed event");
        return;
    };
    debug!(
        session = session.session_id(),
        "Fetching a pushed event for a session"
    );

    let client = session.client();
    let fetch_room_id = room_id.clone();
    // Bounded, and not generously: a process frozen mid-fetch comes back with
    // its sockets dead, and a request on a dead socket that has no timeout of
    // its own hangs forever — measured, silently, before this was one. The
    // timeout turns that into a warning on thaw.
    let handle = spawn_tokio!(async move {
        tokio::time::timeout(
            Duration::from_secs(25),
            fetch_pushed_event(client, fetch_room_id, event_id),
        )
        .await
    });

    // The room was only needed as proof this session is the right one; what
    // is shown comes from the fetched item, which is what keeps the wake off
    // the room models and their idle-priority initialization.
    drop(matrix_room);

    match handle.await.expect("task was not aborted") {
        Ok(Ok(Some(item))) => {
            debug!("A pushed event was fetched; showing it");
            session
                .notifications()
                .show_pushed_item(room_id, item)
                .await;
        }
        Ok(Ok(None)) => {}
        Ok(Err(error)) => warn!("Could not fetch a pushed event: {error}"),
        Err(_) => warn!("Fetching a pushed event timed out"),
    }
}

/// Fetch, and where needed decrypt, the one event a push names.
///
/// Must be called from the tokio runtime. `None` means the event should not
/// become a notification — filtered out by push rules, redacted, or gone.
///
/// On the process setup: `SingleProcess` wants the SDK's own `SyncService`,
/// which Commune does not run — it has its own sync loop. `MultipleProcesses`
/// is the constructible truth: a cross-process store lock that nothing here
/// ever contends, since the push wake and the application share one process.
async fn fetch_pushed_event(
    client: Client,
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
) -> Result<Option<NotificationItem>, matrix_sdk_ui::notification_client::Error> {
    let notification_client =
        NotificationClient::new(client, NotificationProcessSetup::MultipleProcesses).await?;

    // Not `get_notification()`: that tries a short-lived sliding sync first,
    // which is more requests and more machinery than the freezer's budget
    // likes, and against a homeserver without sliding sync it errors rather
    // than falling through. `/context` is one request, still retries
    // decryption, and is where the full path falls back to anyway.
    let status = notification_client
        .get_notification_with_context(&room_id, &event_id)
        .await?;

    match status {
        NotificationStatus::Event(item) => Ok(Some(*item)),
        NotificationStatus::EventFilteredOut => {
            debug!("The push rules filtered a pushed event out");
            Ok(None)
        }
        NotificationStatus::EventRedacted => {
            debug!("A pushed event was redacted");
            Ok(None)
        }
        NotificationStatus::EventNotFound => {
            warn!("A pushed event could not be found on the homeserver");
            Ok(None)
        }
    }
}

/// The string behind a possibly-null Java string.
fn jstring(env: &mut JNIEnv, string: &JString) -> Option<String> {
    if string.is_null() {
        return None;
    }
    env.get_string(string).ok().map(String::from)
}

/// The receiving end of every distributor broadcast, called by
/// `PushReceiver.java`.
///
/// Runs on whatever thread Android delivers broadcasts on, in whatever process
/// state the broadcast found — a running Commune, or one started moments ago
/// for exactly this. The name is the JNI encoding of the Java class and
/// method, and `build-aux/android/stub.c` is what keeps the symbol in the
/// link.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_gtk_android_PushReceiver_nativeReceive(
    mut env: JNIEnv,
    _class: JClass,
    context: JObject,
    action: JString,
    token: JString,
    endpoint: JString,
    reason: JString,
    use_distributor: JString,
    id: JString,
    message: JByteArray,
) {
    if let Err(error) = android::seed_from_jni(&mut env, &context) {
        warn!("Could not capture the Java side from a push broadcast: {error}");
    }

    let Some(action) = jstring(&mut env, &action) else {
        return;
    };
    let token = jstring(&mut env, &token);
    let id = jstring(&mut env, &id);

    let mut state = State::load();

    // The proof the broadcast is from the distributor the token went to, and
    // the first thing checked — see the module comment. `REGISTRATION_FAILED`
    // and `UNREGISTERED` are held to it too: they clear state, and anything on
    // the device being able to unregister us would be a denial of service.
    if state.token.is_none() || token != state.token {
        warn!("Dropped a UnifiedPush broadcast that did not carry the registration token");
        return;
    }
    // Both validated non-empty above; kept aside because the match arms below
    // may clear the state they came from, and the acknowledgement still has to
    // name them.
    let registration_token = state.token.clone();
    let distributor = state.distributor.clone();

    match action.as_str() {
        ACTION_NEW_ENDPOINT => {
            let Some(endpoint) = jstring(&mut env, &endpoint) else {
                warn!("A UnifiedPush endpoint announcement carried no endpoint");
                return;
            };
            info!("UnifiedPush endpoint: {endpoint}");
            if state.endpoint.as_deref() != Some(endpoint.as_str()) {
                // The old endpoint's pusher has to be found and removed later,
                // and this is the last moment its address is known. The new
                // one is not registered anywhere yet; the reconciliation below
                // flips this back.
                state.previous_endpoint = state.endpoint.take();
                state.pusher_ok = false;
            }
            state.endpoint = Some(endpoint);
            state.save();
            ensure_pushers_of_sessions();
        }
        ACTION_MESSAGE => {
            if message.is_null() {
                warn!("A push message arrived with no payload");
                return;
            }
            match env.convert_byte_array(&message) {
                Ok(bytes) => receive_message(&bytes),
                Err(error) => warn!("Could not read a push message payload: {error}"),
            }
        }
        ACTION_REGISTRATION_FAILED => {
            let reason = jstring(&mut env, &reason);
            warn!(?reason, "UnifiedPush registration failed");
            // The spec wants a different token for the next attempt.
            state.token = None;
            state.endpoint = None;
            state.pusher_ok = false;
            state.save();
            notify_delivery_changed();
        }
        ACTION_UNREGISTERED => {
            let replacement = jstring(&mut env, &use_distributor);
            info!(?replacement, "The UnifiedPush distributor unregistered us");
            state.token = None;
            state.endpoint = None;
            state.pusher_ok = false;
            state.save();
            notify_delivery_changed();
        }
        ACTION_TEMP_UNAVAILABLE => {
            debug!("The UnifiedPush distributor reports its push server unavailable");
        }
        other => {
            debug!("Ignoring an unexpected UnifiedPush action: {other}");
        }
    }

    // Acknowledged last: an `id` means the distributor redelivers until told
    // otherwise, and told-otherwise should mean the broadcast was acted on.
    if let (Some(id), Some(token), Some(distributor)) = (id, registration_token, distributor)
        && let Err(error) = send_ack(&mut env, &context, &distributor, &token, &id)
    {
        warn!("Could not acknowledge a UnifiedPush broadcast: {error}");
    }

    // A pending exception aborts the process on the next JNI call from this
    // thread, and returning to Java with one throws out of `onReceive`. The
    // trace lands in logcat either way; keeping the failure to a failed call
    // is this side's job, as in `android::with_env`.
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}
