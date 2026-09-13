#!/bin/bash
# Build libcommune_core.so for the Android ABIs and put the stripped
# copies where Gradle packages them. Run inside the WSL build environment:
#
#     ./build-core.sh                    # debug, x86_64 (the emulator)
#     ./build-core.sh --all              # debug, x86_64 + arm64
#     ./build-core.sh --release          # release, both ABIs
#     ./build-core.sh --release --arm64  # release, the device only
#
# Order matters for the last one: --release sets both ABIs, and --arm64 after
# it narrows them back down.
#
# `app/src/main/jniLibs` is one directory shared by every Gradle variant, not
# one per profile or ABI selection: a debug build and a release build both
# package whatever is sitting there. A run that only asks for one ABI (or an
# `--arm64`-only release run, say) leaves the other ABI's copy exactly as an
# earlier, possibly quite different, invocation left it. This script warns
# at the end when that happens, but does not rebuild the untouched one: run
# `--all` (debug) or plain `--release` (both ABIs, the default) rather than
# an ABI-narrowed flag when the other ABI's freshness actually matters, e.g.
# testing an x86_64 emulator build right after a device-only `--arm64` run.
#
# The Kotlin bindings are generated separately (see README.md): they
# change when the facade changes, the .so on every core change.
#
# `--features ffi` is not optional: the facade and the UniFFI scaffolding
# are behind it, so without it the cdylib exports nothing and the app
# fails at load time rather than at build time. The GTK application links
# the same crate with the feature off — see Track 3 of doc/kotlin-plan.md.
set -eu

NDK_VERSION=${NDK_VERSION:-27.2.12479018}
# Where the SDK lives. The default is the WSL build environment's layout,
# which is what this has always assumed; a runner sets ANDROID_SDK_ROOT to
# somewhere else entirely and would otherwise fail with a path nobody
# recognises. The NDK version stays pinned either way, because the .so this
# produces is shipped and ought to come out of the same toolchain wherever it
# was built.
ANDROID_SDK_ROOT=${ANDROID_SDK_ROOT:-$HOME/android/sdk}
NDK_BIN="$ANDROID_SDK_ROOT/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin"

if [ ! -d "$NDK_BIN" ]; then
    echo "build-core: no NDK $NDK_VERSION under $ANDROID_SDK_ROOT" >&2
    echo "build-core: set ANDROID_SDK_ROOT, or NDK_VERSION to one installed" >&2
    exit 1
fi
API=29

CORE_DIR="$(cd "$(dirname "$0")/../commune-core" && pwd)"
JNILIBS_DIR="$(cd "$(dirname "$0")" && pwd)/app/src/main/jniLibs"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/android/commune-target-kotlin}

profile=debug
profile_flag=()
targets=(x86_64-linux-android)

for arg in "$@"; do
  case "$arg" in
    --all) targets=(x86_64-linux-android aarch64-linux-android) ;;
    --release)
      profile=release
      profile_flag=(--release)
      targets=(x86_64-linux-android aarch64-linux-android)
      ;;
    # One ABI only. The release variant packages arm64-v8a alone, so a
    # release build that also does x86_64 spends half its time on a library
    # nothing ships — which is most of an hour on a two-core runner.
    --arm64) targets=(aarch64-linux-android) ;;
    --x86_64) targets=(x86_64-linux-android) ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

built_abis=()

for target in "${targets[@]}"; do
  case "$target" in
    x86_64-linux-android) abi=x86_64; prefix=x86_64-linux-android ;;
    aarch64-linux-android) abi=arm64-v8a; prefix=aarch64-linux-android ;;
  esac

  clang="$NDK_BIN/${prefix}${API}-clang"
  env_target=${target//-/_}

  echo "== $target ($profile) =="
  env \
    "CC_${env_target}=$clang" \
    "CXX_${env_target}=${clang}++" \
    "AR_${env_target}=$NDK_BIN/llvm-ar" \
    "RANLIB_${env_target}=$NDK_BIN/llvm-ranlib" \
    "CARGO_TARGET_$(echo "$env_target" | tr '[:lower:]' '[:upper:]')_LINKER=$clang" \
    cargo build --manifest-path "$CORE_DIR/Cargo.toml" --lib --features ffi --target "$target" "${profile_flag[@]}"

  mkdir -p "$JNILIBS_DIR/$abi"
  "$NDK_BIN/llvm-strip" \
    -o "$JNILIBS_DIR/$abi/libcommune_core.so" \
    "$CARGO_TARGET_DIR/$target/$profile/libcommune_core.so"
  ls -la "$JNILIBS_DIR/$abi/libcommune_core.so"
  built_abis+=("$abi")
done

# `jniLibs` is one directory shared by every Gradle variant: a debug build
# and a release build both package whatever is sitting in it, regardless of
# which profile or ABI selection last wrote there. Asking for one ABI (or
# one profile) leaves the other ABI's copy exactly as some earlier, possibly
# quite different, invocation left it — which is how a stale x86_64 library
# ended up inside an otherwise-fresh `--release` test build. Nothing here
# rebuilds the untouched one; this just makes sure that is not silent.
for abi in x86_64 arm64-v8a; do
  lib="$JNILIBS_DIR/$abi/libcommune_core.so"
  built=false
  for done_abi in "${built_abis[@]}"; do
    [ "$done_abi" = "$abi" ] && built=true
  done
  if [ "$built" = false ] && [ -f "$lib" ]; then
    echo "build-core: NOT rebuilt this run, left over from an earlier build: $lib" >&2
    ls -la "$lib" >&2
  fi
done
