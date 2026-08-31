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
