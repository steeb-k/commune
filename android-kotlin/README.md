# commune-android

The Kotlin variant of Commune. The plan of record is `../doc/kotlin-plan.md`;
today this is the **walking skeleton** — the smallest app that loads the
Rust core over UniFFI.

## Building

The native core and the bindings come out of `../commune-core`:

Everything below needs `--features ffi`: the facade and the UniFFI
scaffolding are behind it, so a build without it exports nothing. The GTK
application links the same crate with the feature off — see Track 3 of
`../doc/kotlin-plan.md`. `--features cli` implies it.

```sh
# 1. The .so, per ABI (in the WSL build environment; NDK env as in
#    doc/kotlin-plan.md):
cargo build --lib --features ffi --target x86_64-linux-android
llvm-strip -o libcommune_core.stripped.so .../libcommune_core.so

# 2. Copy it in (never committed — see .gitignore):
cp libcommune_core.stripped.so app/src/main/jniLibs/x86_64/libcommune_core.so

# 3. The bindings (committed under app/src/main/java/.../core/ for now;
#    a Gradle task should take this over):
cargo run --features cli --bin uniffi-bindgen -- \
    generate --library target/debug/commune_core.dll \
    --language kotlin --out-dir target/bindings

# 4. The APK, through the wrapper:
ANDROID_HOME=$HOME/android/sdk ./gradlew assembleDebug
```

`build-core.sh --all` does steps 1 and 2 for both ABIs in one go, which is
what Track 3's per-commit gate runs.

### Signing, and why it matters more now

The release variant signs with `keystore.properties` and the keystore it names
when both are present, and falls back to the debug key when they are not — so
a checkout without them still builds something installable, it just cannot
upgrade a released copy.

That fallback is the whole point. Android replaces an installed application
only with one signed by the same key, and the in-app updater rests entirely on
the system enforcing that. The keystore is therefore the one file here that
cannot be regenerated: losing it means every installation has to be
uninstalled by hand before it can be replaced. It is git-ignored, it is in the
`ANDROID_KEYSTORE_BASE64` repository secret, and it should be backed up
offline as well.

The first APK signed with it cannot install over the debug-keyed copies that
came before, including the one on the development Pixel — that uninstall is
paid once per device and costs that device's adopted session.

`versionName` is read from the workspace `Cargo.toml` and `versionCode` is
`git rev-list --count HEAD`, so neither is typed here any more. They used to
be, and had drifted: this app called itself 0.1.0 while the desktop shipped
1.0.0-rc1.

See [`../doc/updates.md`](../doc/updates.md) for the updater itself.

### Why there is a wrapper

`gradlew` and `gradle/wrapper/` are committed, `gradle-wrapper.jar`
included, which is how a Gradle wrapper is meant to be shipped: the point of
it is that a checkout builds with nothing installed but a JDK, and a wrapper
you have to fetch first cannot do that.

It is here because its absence was not free. This README used to say
`gradle assembleDebug`, and there is no `gradle` on the build machine's
PATH — so the instruction had never worked from a clean shell, and the APK
went unbuilt through a change to `build.gradle.kts` that did not compile.
The wrapper pins 9.3.1, the version the build was developed against, instead
of whatever a given machine happens to have.

### The emoji dataset

`app/src/main/assets/emoji.json` feeds the emoji chooser, the one behind
"More Reactions". It is generated, not hand-kept, by `gen-emoji-data.py`
from the emojibase dataset GTK's own chooser is built from:

```sh
curl -sSLO https://cdn.jsdelivr.net/npm/emojibase-data@latest/en/data.json
python3 gen-emoji-data.py data.json
```

Regenerate it when a new Unicode emoji release lands in emojibase.

### The KLIPY API key

The GIF search needs one, and it is a credential: put it in
`local.properties` beside this file, the only thing here that git ignores.

```properties
communeKlipyApiKey=…
```

**Not `gradle.properties`** — that one is tracked, and a credential in it is
a credential committed. `~/.gradle/gradle.properties` and
`-PcommuneKlipyApiKey=…` work as well. With no key the GIF search is inert
rather than broken, exactly as an empty `klipy-api-key` Meson option leaves
it on the desktop. `../doc/gif-search.md` has the whole story, including how
the key came to be published once.

Debug builds install as `io.github.steeb_k.commune.skeleton`, so they never
displace the GTK build (or its seeded test session) on a device. Release
builds keep the real id: adopting the GTK build's session in place is a
design goal.

### Signing for the device

A release APK is signed with the debug keystore of whichever machine ran
Gradle, and Android only upgrades a package in place — `pm install -r`,
session kept — when the new APK carries the same key as the installed one.
The phone was first installed from a WSL build, so its package carries the
WSL user's `~/.android/debug.keystore` (SHA-256 `56:A8:1A:A4…`). A Windows
Gradle build signs with the Windows user's keystore (`93:DE:BB:43…`) and the
install is refused with `INSTALL_FAILED_UPDATE_INCOMPATIBLE`; uninstalling to
get past that would destroy the adopted session. So either build in WSL, or
re-sign a Windows build before pushing it:

```sh
BT=/c/Android/Sdk/build-tools/36.0.0
cp //wsl.localhost/archlinux/home/steeb/.android/debug.keystore /tmp/wsl-debug.keystore
/apksigner.bat sign --ks /tmp/wsl-debug.keystore --ks-pass pass:android \n    --ks-key-alias androiddebugkey --key-pass pass:android app-release.apk
/apksigner.bat verify --print-certs app-release.apk   # expect 56a81aa4…
```

The emulator was first installed from a Windows build and carries the
Windows key, so an unsigned-over Windows build is fine there and nowhere
else.
