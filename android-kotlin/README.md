# commune-android

The Kotlin variant of Commune. The plan of record is `../doc/kotlin-plan.md`;
today this is the **walking skeleton** — the smallest app that loads the
Rust core over UniFFI.

## Building

The native core and the bindings come out of `../commune-core`:

```sh
# 1. The .so, per ABI (in the WSL build environment; NDK env as in
#    doc/kotlin-plan.md):
cargo build --lib --target x86_64-linux-android
llvm-strip -o libcommune_core.stripped.so .../libcommune_core.so

# 2. Copy it in (never committed — see .gitignore):
cp libcommune_core.stripped.so app/src/main/jniLibs/x86_64/libcommune_core.so

# 3. The bindings (committed under app/src/main/java/.../core/ for now;
#    a Gradle task should take this over):
cargo run --features cli --bin uniffi-bindgen -- \
    generate --library target/debug/commune_core.dll \
    --language kotlin --out-dir target/bindings

# 4. The APK (Gradle 9.3.1, AGP 9.1.0 — both in the build machine's cache):
ANDROID_HOME=$HOME/android/sdk gradle assembleDebug
```

Debug builds install as `io.github.steeb_k.commune.skeleton`, so they never
displace the GTK build (or its seeded test session) on a device. Release
builds keep the real id: adopting the GTK build's session in place is a
design goal.
