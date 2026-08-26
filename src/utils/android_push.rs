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

use std::sync::atomic::{AtomicBool, Ordering};

use gtk::glib;
use jni::{
    JNIEnv,
    objects::{JByteArray, JClass, JObject, JString, JValue},
};
use tracing::{debug, info, warn};

use super::{
    DataType,
    android::{self, AndroidJniError},
};

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
        }
    }

    fn save(&self) {
        let keyfile = glib::KeyFile::new();
        for (key, value) in [
            ("token", &self.token),
            ("distributor", &self.distributor),
            ("endpoint", &self.endpoint),
        ] {
            if let Some(value) = value {
                keyfile.set_string(STATE_GROUP, key, value);
            }
        }

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

/// The body of [`init()`], so that one place reports what went wrong.
fn register() -> Result<(), AndroidJniError> {
    let distributors = distributors()?;

    let Some(distributor) = distributors.first() else {
        info!("No UnifiedPush distributor is installed; only the foreground service can deliver");
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
    _message: JByteArray,
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
            // The step 1 measurement, and step 2's cue to register a pusher.
            info!("UnifiedPush endpoint: {endpoint}");
            state.endpoint = Some(endpoint);
            state.save();
        }
        ACTION_MESSAGE => {
            // Step 3 turns this into a notification; today it is only proof of
            // arrival.
            info!("A push message arrived");
        }
        ACTION_REGISTRATION_FAILED => {
            let reason = jstring(&mut env, &reason);
            warn!(?reason, "UnifiedPush registration failed");
            // The spec wants a different token for the next attempt.
            state.token = None;
            state.endpoint = None;
            state.save();
        }
        ACTION_UNREGISTERED => {
            let replacement = jstring(&mut env, &use_distributor);
            info!(?replacement, "The UnifiedPush distributor unregistered us");
            state.token = None;
            state.endpoint = None;
            state.save();
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
