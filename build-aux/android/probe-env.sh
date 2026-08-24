#!/bin/sh
# Inventory the Android cross-build environment for the Commune port.
#
# Read-only: this script never installs, builds or modifies anything. It prints
# one row per dependency as `name | found | version | required | status` and
# ends with the list of things that have to be installed before an APK can be
# built.
#
# Unlike the macOS and Windows probes, this one does not run on the machine the
# app will run on. It runs on the Linux *build host* — WSL on this machine —
# because pixiewood, meson and the NDK are Linux tools. Nothing here is
# discovered through `pkg-config`: the GTK stack is not installed on the host at
# all, it is built from meson subprojects, once per architecture.
#
#     sh build-aux/android/probe-env.sh
#
# Paste the output into `doc/android.md` when the environment changes.

set -u

missing=''
optional_missing=''

SDK=${ANDROID_HOME:-$HOME/android/sdk}
NDK_VERSION=${ANDROID_NDK_VERSION:-27.2.12479018}
PIXIEWOOD=${PIXIEWOOD:-$HOME/src/gtk-android-builder/pixiewood}

# Record a name in a space-separated list, avoiding duplicates.
#
# The list is split on whitespace when it is printed, so names must not contain
# spaces.
remember() {
    _list=$1
    _name=$2
    for _entry in $(eval "printf '%s' \"\$$_list\""); do
        if [ "$_entry" = "$_name" ]; then
            return 0
        fi
    done
    eval "$_list=\"\${$_list} \$_name\""
}

row() {
    printf '%-28s | %-5s | %-30s | %-14s | %s\n' "$1" "$2" "$3" "$4" "$5"
}

header() {
    printf '\n%s\n' "$1"
    printf '%s\n' '--------------------------------------------------------------------------------'
}

table_header() {
    row 'name' 'found' 'version' 'required' 'status'
}

have() {
    command -v "$1" >/dev/null 2>&1
}

# tool_check <command> <required|optional> [version argument]
tool_check() {
    _cmd=$1
    _kind=$2
    _verarg=${3:---version}

    if ! have "$_cmd"; then
        row "$_cmd" 'no' '-' "$_kind" 'MISSING'
        if [ "$_kind" = 'required' ]; then
            remember missing "$_cmd"
        else
            remember optional_missing "$_cmd"
        fi
        return
    fi

    _ver=$("$_cmd" "$_verarg" 2>&1 | head -n 1)
    [ -n "$_ver" ] || _ver='(no version output)'
    row "$_cmd" 'yes' "$_ver" "$_kind" 'ok'
}

# perl_check <module>
perl_check() {
    _mod=$1
    if perl -M"$_mod" -e 1 >/dev/null 2>&1; then
        _ver=$(perl -M"$_mod" -e "print \$${_mod}::VERSION || 'unknown'" 2>/dev/null)
        [ -n "$_ver" ] || _ver='unknown'
        row "$_mod" 'yes' "$_ver" 'required' 'ok'
    else
        row "$_mod" 'no' '-' 'required' 'MISSING'
        remember missing "perl:$_mod"
    fi
}

# path_check <label> <path> <required|optional>
path_check() {
    _label=$1
    _path=$2
    _kind=$3

    if [ -e "$_path" ]; then
        row "$_label" 'yes' "$_path" "$_kind" 'ok'
    else
        row "$_label" 'no' "$_path" "$_kind" 'MISSING'
        if [ "$_kind" = 'required' ]; then
            remember missing "$_label"
        else
            remember optional_missing "$_label"
        fi
    fi
}

printf 'Commune Android build-host environment\n'
printf 'host: %s\n' "$(uname -srm)"
printf 'sdk:  %s\n' "$SDK"

header 'Build host tools'
table_header
tool_check meson required
tool_check ninja required
tool_check perl required -e
tool_check java required -version
tool_check git required
tool_check sassc required
tool_check glslc optional
tool_check xmllint optional
tool_check unzip required -v
tool_check python3 required

header 'meson features'
table_header
# `android_exe_type` is the kwarg that turns an executable into the shared
# object GTK's Java glue dlopens. It landed in meson 1.9, so stock meson is
# enough: pixiewood's README still points at a forked meson, but that predates
# the kwarg being upstream.
if have meson; then
    _mver=$(meson --version 2>/dev/null)
    # Compare by version rather than by grepping the installation: meson is
    # often in a venv the system python cannot import.
    _moldest=$(printf '1.9\n%s\n' "$_mver" | sort -V | head -n 1)
    if [ "$_moldest" = '1.9' ]; then
        row 'android_exe_type' 'yes' "$_mver" '>= 1.9' 'ok'
    else
        row 'android_exe_type' 'no' "$_mver" '>= 1.9' 'TOO OLD'
        remember missing 'meson>=1.9'
    fi
else
    row 'android_exe_type' 'no' '-' '>= 1.9' 'no meson'
    remember missing 'meson>=1.9'
fi

header 'Host GLib code generators'
table_header
# Cross-compiling uses two GLibs: the target one, built from the `main` wrap,
# and the *build machine's* generators, which meson takes from the host and
# bakes into build.ninja as absolute paths. That last part is why this belongs
# in a probe: a host whose tools are too old cannot be worked around with PATH
# or pkg-config shims, so it has to be caught before a build starts.
#
# The bar is `G_GNUC_FLAG_ENUM` (`Since: 2.88`), which libadwaita's
# adw-tab-view.h uses. An older glib-mkenums does not skip the type, it reads
# the macro as the type's *name* — generating ADW_TYPE_G_GNUC_FLAG_ENUM and
# failing hundreds of targets later on a symbol nobody ever wrote. Probe the
# behaviour rather than the version, because the behaviour is what breaks.
tool_check glib-mkenums required --version
tool_check glib-compile-resources required --version
tool_check glib-compile-schemas required --version

if have glib-mkenums; then
    _gver=$(glib-mkenums --version 2>/dev/null | sed -n '1s/.*version //p')
    _probe_hdr=$(mktemp 2>/dev/null) || _probe_hdr=''
    if [ -n "$_probe_hdr" ]; then
        cat > "$_probe_hdr" <<'PROBE_EOF'
typedef enum {
  PROBE_FLAG_NONE = 0,
  PROBE_FLAG_ONE  = 1 << 0,
} G_GNUC_FLAG_ENUM ProbeFlagEnum;
PROBE_EOF
        _probe_name=$(glib-mkenums --fhead '' --eprod '@EnumName@' "$_probe_hdr" 2>/dev/null \
            | grep -c '^ProbeFlagEnum$')
        rm -f "$_probe_hdr"
        if [ "$_probe_name" = '1' ]; then
            row 'G_GNUC_FLAG_ENUM' 'yes' "${_gver:-unknown}" '>= 2.88' 'ok'
        else
            row 'G_GNUC_FLAG_ENUM' 'no' "${_gver:-unknown}" '>= 2.88' 'TOO OLD'
            remember missing 'glib-mkenums>=2.88 (libadwaita will not build)'
        fi
    fi
fi

header 'Perl modules (pixiewood)'
table_header
perl_check Glib
perl_check IPC::Run
perl_check JSON
perl_check Set::Scalar
perl_check XML::LibXML
perl_check XML::LibXSLT

header 'Android SDK'
table_header
path_check 'sdk root' "$SDK" required
path_check 'cmdline-tools' "$SDK/cmdline-tools/latest/bin/sdkmanager" required
path_check 'platform android-35' "$SDK/platforms/android-35" required
path_check 'build-tools 35.0.0' "$SDK/build-tools/35.0.0" required
path_check 'ndk' "$SDK/ndk/$NDK_VERSION" required
# The NDK is host-specific. The Windows SDK next door carries only
# `prebuilt/windows-x86_64` and `.exe` build tools, so it cannot be shared with
# the Linux build host however convenient that would be.
path_check 'ndk linux toolchain' \
    "$SDK/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64" required
path_check 'ndk clang x86_64 api31' \
    "$SDK/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin/x86_64-linux-android31-clang" \
    required

header 'pixiewood'
table_header
path_check 'pixiewood' "$PIXIEWOOD" required
if [ -f "$PIXIEWOOD" ]; then
    _pver=$(perl "$PIXIEWOOD" --version 2>&1 | grep -v experimental | head -n 1)
    row 'pixiewood --version' 'yes' "${_pver:-unknown}" 'required' 'ok'
fi

header 'Rust (needed from S1 onwards, not by S0)'
table_header
tool_check cargo optional
tool_check rustc optional
tool_check cargo-ndk optional
if have rustup; then
    for _t in x86_64-linux-android aarch64-linux-android; do
        if rustup target list --installed 2>/dev/null | grep -qx "$_t"; then
            row "target $_t" 'yes' 'installed' 'optional' 'ok'
        else
            row "target $_t" 'no' '-' 'optional' 'MISSING'
            remember optional_missing "rust-target:$_t"
        fi
    done
else
    row 'rustup' 'no' '-' 'optional' 'MISSING'
    remember optional_missing 'rustup'
fi

header 'Device access'
table_header
# `adb` normally lives on the Windows side, where the emulator runs; the APK is
# installed from there over the `\\wsl$` share.
if have adb; then
    _devs=$(adb devices 2>/dev/null | sed -n '2,$p' | grep -c 'device$')
    row 'adb' 'yes' "$(adb --version 2>/dev/null | head -n 1)" 'optional' 'ok'
    if [ "${_devs:-0}" -gt 0 ]; then
        row 'devices attached' 'yes' "$_devs" 'optional' 'ok'
    else
        row 'devices attached' 'no' '0' 'optional' 'none attached'
    fi
else
    row 'adb' 'no' '-' 'optional' 'use the Windows adb'
    remember optional_missing 'adb'
fi

printf '\n'
if [ -n "$missing" ]; then
    printf 'needs installing/building:%s\n' "$missing"
else
    printf 'needs installing/building: nothing\n'
fi
if [ -n "$optional_missing" ]; then
    printf 'optional, missing:%s\n' "$optional_missing"
fi
