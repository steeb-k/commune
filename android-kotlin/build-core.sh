#!/bin/bash
# Build libcommune_core.so for the Android ABIs and put the stripped
# copies where Gradle packages them. Run inside the WSL build environment:
#
#     ./build-core.sh            # debug, x86_64 (the emulator)
#     ./build-core.sh --all      # debug, x86_64 + arm64
#     ./build-core.sh --release  # release, both ABIs
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
NDK_BIN="$HOME/android/sdk/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin"
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
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

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
done
