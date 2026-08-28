//! Encrypting the stored session with a key held in the Android Keystore.
//!
//! # Why this is JNI and not a Rust crypto library
//!
//! The point of the Keystore is that the key never exists as bytes we can hold.
//! It is generated inside the Keystore, backed by the device's secure hardware
//! where there is any, and the API hands back a handle rather than key
//! material. So the encryption has to happen on the Java side too: there is
//! nothing to give a Rust cipher. Every call below is a step of that.
//!
//! What this buys over a passphrase of our own: the key cannot be copied off
//! the device, and it cannot be used without the app's UID. A file taken off a
//! rooted phone, or out of a backup, is ciphertext with no key anywhere near
//! it.
//!
//! # The scheme
//!
//! AES-256-GCM, one key, aliased [`KEY_ALIAS`], generated on first use and
//! reused afterwards. GCM gives authentication as well as secrecy, so a
//! tampered file fails to decrypt rather than decrypting to something else.
//!
//! The IV is chosen by the Keystore rather than by us —
//! `setRandomizedEncryptionRequired` is on by default and refuses a
//! caller-supplied one, which is the safer arrangement anyway, since reusing an
//! IV with GCM is catastrophic. It comes back from the `Cipher` after `init`
//! and is stored in front of the ciphertext:
//!
//! ```text
//! [ 1 byte version ][ 1 byte IV length ][ IV ][ ciphertext + GCM tag ]
//! ```
//!
//! The version byte is there so that a future scheme can be told apart from
//! this one rather than being fed to it.

use jni::{
    AttachGuard,
    objects::{JByteArray, JObject, JValue},
};
use zeroize::Zeroizing;

use crate::platform::android::{self, AndroidJniError};

/// The alias the key is stored under in the Keystore.
///
/// Spelled out rather than derived from `CARGO_PKG_NAME`: the application
/// derives it and gets `commune.secrets.v1`, and this crate is named
/// `commune-core` — deriving here would seal under a different alias and
/// silently break the same-application-id session adoption that
/// `doc/kotlin-plan.md` promises. The Keystore is per-application, so the
/// Devel and Hack profiles never see this key anyway.
const KEY_ALIAS: &str = "commune.secrets.v1";

/// The Keystore provider name. Not a real file, a virtual provider.
const PROVIDER: &str = "AndroidKeyStore";

/// The transformation used for both directions.
const TRANSFORMATION: &str = "AES/GCM/NoPadding";

/// The size of the GCM authentication tag, in bits.
const TAG_BITS: i32 = 128;

/// The size of the key, in bits.
const KEY_BITS: i32 = 256;

/// The version byte written in front of every payload.
const VERSION: u8 = 1;

/// `KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT`.
///
/// Constants rather than field lookups because these are `static final int`s
/// whose values are part of the platform API and will not change.
const PURPOSE_ENCRYPT_DECRYPT: i32 = 1 | 2;

/// `Cipher.ENCRYPT_MODE` and `Cipher.DECRYPT_MODE`.
const ENCRYPT_MODE: i32 = 1;
const DECRYPT_MODE: i32 = 2;

/// The errors that can occur using the Keystore.
#[derive(Debug, thiserror::Error)]
pub(super) enum KeystoreError {
    /// The Java side could not be reached at all.
    #[error(transparent)]
    Jni(#[from] AndroidJniError),

    /// A JNI call failed.
    #[error(transparent)]
    Java(#[from] jni::errors::Error),

    /// The stored payload is not something this version wrote.
    #[error("stored secret is malformed or from a newer version")]
    Malformed,
}

/// Encrypt the given plaintext with the Keystore key, creating it if this is
/// the first time.
pub(super) fn encrypt(plaintext: &[u8]) -> Result<Vec<u8>, KeystoreError> {
    android::with_env(|env| {
        let key = get_or_create_key(env)?;

        let cipher = cipher_instance(env)?;
        env.call_method(
            &cipher,
            "init",
            "(ILjava/security/Key;)V",
            &[JValue::Int(ENCRYPT_MODE), JValue::Object(&key)],
        )?;

        // The Keystore picks the IV. Asking for a specific one is refused, and
        // taking the one it chose is what keeps GCM safe across calls.
        let iv = env.call_method(&cipher, "getIV", "()[B", &[])?.l()?;
        let iv = env.convert_byte_array(JByteArray::from(iv))?;

        let input = env.byte_array_from_slice(plaintext)?;
        let ciphertext = env
            .call_method(
                &cipher,
                "doFinal",
                "([B)[B",
                &[JValue::Object(&JObject::from(input))],
            )?
            .l()?;
        let ciphertext = env.convert_byte_array(JByteArray::from(ciphertext))?;

        let iv_len = u8::try_from(iv.len()).map_err(|_| KeystoreError::Malformed)?;

        let mut payload = Vec::with_capacity(2 + iv.len() + ciphertext.len());
        payload.push(VERSION);
        payload.push(iv_len);
        payload.extend_from_slice(&iv);
        payload.extend_from_slice(&ciphertext);

        Ok(payload)
    })
}

/// Decrypt a payload written by [`encrypt()`].
pub(super) fn decrypt(payload: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeystoreError> {
    let (iv, ciphertext) = split_payload(payload)?;

    android::with_env(|env| {
        let key = get_or_create_key(env)?;

        // `GCMParameterSpec(tagLengthBits, iv)` is how the IV is given back for
        // decryption; the Keystore only refuses a *caller-chosen* IV when
        // encrypting.
        let iv_array = env.byte_array_from_slice(iv)?;
        let spec = env.new_object(
            "javax/crypto/spec/GCMParameterSpec",
            "(I[B)V",
            &[
                JValue::Int(TAG_BITS),
                JValue::Object(&JObject::from(iv_array)),
            ],
        )?;

        let cipher = cipher_instance(env)?;
        env.call_method(
            &cipher,
            "init",
            "(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V",
            &[
                JValue::Int(DECRYPT_MODE),
                JValue::Object(&key),
                JValue::Object(&spec),
            ],
        )?;

        let input = env.byte_array_from_slice(ciphertext)?;
        let plaintext = env
            .call_method(
                &cipher,
                "doFinal",
                "([B)[B",
                &[JValue::Object(&JObject::from(input))],
            )?
            .l()?;

        Ok(Zeroizing::new(
            env.convert_byte_array(JByteArray::from(plaintext))?,
        ))
    })
}

/// Split a stored payload into its IV and ciphertext.
fn split_payload(payload: &[u8]) -> Result<(&[u8], &[u8]), KeystoreError> {
    let (&version, rest) = payload.split_first().ok_or(KeystoreError::Malformed)?;
    if version != VERSION {
        return Err(KeystoreError::Malformed);
    }

    let (&iv_len, rest) = rest.split_first().ok_or(KeystoreError::Malformed)?;
    let iv_len = usize::from(iv_len);

    if rest.len() <= iv_len {
        return Err(KeystoreError::Malformed);
    }

    Ok(rest.split_at(iv_len))
}

/// `Cipher.getInstance("AES/GCM/NoPadding")`.
fn cipher_instance<'a>(env: &mut AttachGuard<'a>) -> Result<JObject<'a>, KeystoreError> {
    let transformation = env.new_string(TRANSFORMATION)?;

    Ok(env
        .call_static_method(
            "javax/crypto/Cipher",
            "getInstance",
            "(Ljava/lang/String;)Ljavax/crypto/Cipher;",
            &[JValue::Object(&JObject::from(transformation))],
        )?
        .l()?)
}

/// The key for [`KEY_ALIAS`], generating it if the Keystore does not have it
/// yet.
fn get_or_create_key<'a>(env: &mut AttachGuard<'a>) -> Result<JObject<'a>, KeystoreError> {
    if let Some(key) = load_key(env)? {
        return Ok(key);
    }

    create_key(env)
}

/// Load the key for [`KEY_ALIAS`] from the Keystore, if it is there.
fn load_key<'a>(env: &mut AttachGuard<'a>) -> Result<Option<JObject<'a>>, KeystoreError> {
    let provider = env.new_string(PROVIDER)?;
    let store = env
        .call_static_method(
            "java/security/KeyStore",
            "getInstance",
            "(Ljava/lang/String;)Ljava/security/KeyStore;",
            &[JValue::Object(&JObject::from(provider))],
        )?
        .l()?;

    // `load(null)` is how an `AndroidKeyStore` is opened: there is no file and
    // no password, the argument exists for the general `KeyStore` API.
    env.call_method(
        &store,
        "load",
        "(Ljava/security/KeyStore$LoadStoreParameter;)V",
        &[JValue::Object(&JObject::null())],
    )?;

    let alias = env.new_string(KEY_ALIAS)?;
    let key = env
        .call_method(
            &store,
            "getKey",
            "(Ljava/lang/String;[C)Ljava/security/Key;",
            &[
                JValue::Object(&JObject::from(alias)),
                JValue::Object(&JObject::null()),
            ],
        )?
        .l()?;

    Ok((!key.is_null()).then_some(key))
}

/// Generate a new key for [`KEY_ALIAS`] in the Keystore.
fn create_key<'a>(env: &mut AttachGuard<'a>) -> Result<JObject<'a>, KeystoreError> {
    let alias = env.new_string(KEY_ALIAS)?;

    // `new KeyGenParameterSpec.Builder(alias, PURPOSE_ENCRYPT | PURPOSE_DECRYPT)`
    let builder = env.new_object(
        "android/security/keystore/KeyGenParameterSpec$Builder",
        "(Ljava/lang/String;I)V",
        &[
            JValue::Object(&JObject::from(alias)),
            JValue::Int(PURPOSE_ENCRYPT_DECRYPT),
        ],
    )?;

    // Each of these returns the builder, which is discarded: the builder mutates
    // itself and returning `this` is only for chaining.
    let block_modes = string_array(env, &["GCM"])?;
    env.call_method(
        &builder,
        "setBlockModes",
        "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
        &[JValue::Object(&block_modes)],
    )?;

    let paddings = string_array(env, &["NoPadding"])?;
    env.call_method(
        &builder,
        "setEncryptionPaddings",
        "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
        &[JValue::Object(&paddings)],
    )?;

    env.call_method(
        &builder,
        "setKeySize",
        "(I)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
        &[JValue::Int(KEY_BITS)],
    )?;

    let spec = env
        .call_method(
            &builder,
            "build",
            "()Landroid/security/keystore/KeyGenParameterSpec;",
            &[],
        )?
        .l()?;

    let algorithm = env.new_string("AES")?;
    let provider = env.new_string(PROVIDER)?;
    let generator = env
        .call_static_method(
            "javax/crypto/KeyGenerator",
            "getInstance",
            "(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;",
            &[
                JValue::Object(&JObject::from(algorithm)),
                JValue::Object(&JObject::from(provider)),
            ],
        )?
        .l()?;

    env.call_method(
        &generator,
        "init",
        "(Ljava/security/spec/AlgorithmParameterSpec;)V",
        &[JValue::Object(&spec)],
    )?;

    // `generateKey` both returns the key and stores it under the alias, so
    // there is nothing further to do to persist it.
    Ok(env
        .call_method(&generator, "generateKey", "()Ljavax/crypto/SecretKey;", &[])?
        .l()?)
}

/// Build a `String[]` from the given strings.
fn string_array<'a>(
    env: &mut AttachGuard<'a>,
    values: &[&str],
) -> Result<JObject<'a>, KeystoreError> {
    let len = i32::try_from(values.len()).map_err(|_| KeystoreError::Malformed)?;
    let empty = env.new_string("")?;
    let array = env.new_object_array(len, "java/lang/String", &empty)?;

    for (index, value) in values.iter().enumerate() {
        let value = env.new_string(value)?;
        let index = i32::try_from(index).map_err(|_| KeystoreError::Malformed)?;
        env.set_object_array_element(&array, index, &value)?;
    }

    Ok(JObject::from(array))
}
