project(
    'commune',
    version: '1.rc2',
    license: 'GPL-3.0-or-later',
    meson_version: '>= 1.4',
)

is_android = host_machine.system() == 'android'

# Meson never compiles any Rust itself: `src/meson.build` drives Cargo through
# `custom_target`, and the only reason to declare the language is that Meson
# then finds and reports a `rustc` up front instead of failing later.
#
# On Android that check asks the wrong question. Declaring the language makes
# Meson look for a `rustc` for the *host* machine, which the cross files
# pixiewood writes do not name, and it fails with `'rust' compiler binary not
# defined in cross file [binaries] section`. Cargo cross-compiles on its own
# with `--target` and needs nothing from Meson to do it.
#
# Android needs a C compiler instead, for exactly one file:
# `build-aux/android/stub.c`, the `main` that GTK's glue looks up.
if is_android
    add_languages('c', native: false)
else
    add_languages('rust', native: false)
endif

fs = import('fs')
gnome = import('gnome')
i18n = import('i18n')

base_id = 'io.github.steeb_k.Commune'
application_id = base_id

major_version = '1'
pre_release_version = 'rc2'

version = major_version
if pre_release_version != ''
    version += '.' + pre_release_version
endif
full_version = version

# The number of commits behind this build.
#
# The version cannot order two builds of the same release — every nightly in
# a series carries the same one — so `commune_core::updates` compares this
# when the versions match. It is the same number `build-aux/macos/bundle.sh`
# writes into `CFBundleVersion` and `android-kotlin` uses for `versionCode`,
# for the same reason and from the same command, so all three platforms
# order the same builds the same way.
#
# Read at configure time, which means it is as current as the last
# `meson setup`. That is exact for a release and for CI, both of which
# configure a fresh tree; a developer whose build directory has gone stale
# gets a number that is merely low, and a low build number can only make the
# updater offer something, never hide it.
build_number = run_command(
    'git',
    'rev-list',
    '--count',
    'HEAD',
    check: false,
).stdout().strip()
if build_number == ''
    build_number = '0'
endif

# Android links these into the shared object itself, because Cargo hands over a
# `staticlib` and Meson performs the one real link. Everywhere else they are
# assertions that the libraries exist, and Cargo's `-sys` crates do the linking.
glib_dep = dependency('glib-2.0', version: '>= 2.82')  # update when changing gtk version
gio_dep = dependency('gio-2.0', version: '>= 2.82')  # always same version as glib
gtk_dep = dependency('gtk4', version: '>= 4.20.2')
adw_dep = dependency('libadwaita-1', version: '>= 1.8.0')

# Please keep these dependencies sorted.
if not is_android
    # GStreamer is not cross-built for Android, so media playback, voice
    # messages and calls are all absent there and the crates that need it are
    # not in the dependency graph at all. See `doc/android.md`.
    dependency('gstreamer-1.0', version: '>= 1.20')
    dependency('gstreamer-app-1.0', version: '>= 1.20')
    dependency('gstreamer-base-1.0', version: '>= 1.20')
    dependency('gstreamer-pbutils-1.0', version: '>= 1.20')
    dependency('gstreamer-play-1.0', version: '>= 1.20')
    dependency('gstreamer-sdp-1.0', version: '>= 1.20')
    dependency('gstreamer-video-1.0', version: '>= 1.20')
    dependency('gstreamer-webrtc-1.0', version: '>= 1.20')
endif
# Android has no GtkSourceView package, so it builds the one in `subprojects/`.
# Everywhere else the fallback stays off: a missing development package should say
# so, not quietly start cloning and building GtkSourceView from git.
sourceview_dep = dependency(
    'gtksourceview-5',
    version: '>= 5.0.0',
    allow_fallback: is_android,
)
if is_android
    # Android is the only platform with no icon theme of its own. Elsewhere
    # adwaita-icon-theme is a package that is always installed; in an APK it is
    # there only if we put it there, and without it every icon that Commune
    # does not ship itself and GTK does not ship builtin draws as the
    # missing-icon placeholder. See `subprojects/adwaita-icon-theme.wrap` and
    # `doc/android.md`.
    #
    # `subproject()` rather than `dependency()` because it provides none: it is
    # data, so nothing would ever ask for it and nothing would ever build it.
    subproject('adwaita-icon-theme')
endif
if host_machine.system() == 'linux'
    # glycin decodes images on Linux. Every other platform uses the `image`
    # crate instead, see `src/utils/media/image/decoder/`.
    dependency('glycin-2', version: '>= 2.0.0')
    dependency('glycin-gtk4-2', version: '>= 2.0.0')
endif
if not is_android
    # `libwebp-sys` builds libwebp itself with `cc`, so this only ever asserted
    # that the system copy existed. Android has no system copy and does not need
    # one.
    dependency('libwebp', version: '>= 1.0.0')
    # libshumate wants libsoup-3.0, json-glib, libprotobuf-c and sqlite3, none of
    # which are cross-built, so Android has no map. See `doc/android.md`.
    dependency('shumate-1.0', version: '>= 1.1.0')
endif
if not is_android
    # Android builds its own; see the `rusqlite` entry in `Cargo.toml` and the
    # Android block below.
    dependency('sqlite3', version: '>= 3.24.0')
endif

if is_android
    cc = meson.get_compiler('c')

    # SQLite is deliberately absent from this list. Android has it, but not as
    # an NDK API — there is no `libsqlite3.so` anywhere in the sysroot — so
    # nothing native can link against the platform's copy. `Cargo.toml` turns on
    # `rusqlite`'s `bundled` feature for Android instead, which compiles SQLite
    # into the static library Cargo hands over, so by the time Meson links there
    # is nothing left to find.

    # `__android_log_write`, which is where `tracing` output goes. `dl` and `m`
    # are what the Rust standard library and the vendored C in `aws-lc-sys`,
    # `libwebp-sys` and now `libsqlite3-sys` expect to find.
    log_dep = cc.find_library('log')
    dl_dep = cc.find_library('dl')
    m_dep = cc.find_library('m')

    # GStreamer is not a wrap and pixiewood does not know how to build it. It
    # comes from the upstream Android binaries, pruned into something safe to
    # put on a pkg-config path by `build-aux/android/gstreamer-prefix.sh` —
    # read that script before changing any of this; it explains why the tarball
    # cannot be pointed at directly.
    #
    # The location is per-machine, so it is an environment variable with a
    # default, the same convention `build-aux/android/probe-env.sh` uses for the
    # SDK and the NDK. The prefix names its architectures the way Meson does.
    gst_prefix = run_command(
        'sh',
        '-c',
        'echo "${GSTREAMER_ANDROID_PREFIX:-$HOME/android/gst-android}"',
        check: true,
    ).stdout().strip() / host_machine.cpu_family()
    gst_pkgconfig = gst_prefix / 'lib' / 'pkgconfig'

    if not fs.exists(gst_pkgconfig)
        error(
            'No GStreamer prefix at ' + gst_pkgconfig + '. Extract the Android '
            + 'binaries from https://gstreamer.freedesktop.org/data/pkg/android/ '
            + 'and run build-aux/android/gstreamer-prefix.sh over them, or set '
            + '$GSTREAMER_ANDROID_PREFIX. See doc/android-media-plan.md.',
        )
    endif

    # Not `dependency()`. Meson would run pkg-config with its own environment,
    # which for a cross build sets `PKG_CONFIG_SYSROOT_DIR` from the NDK sysroot
    # and would rewrite these absolute paths, and there is nowhere to tell it
    # about the two search directories in the first place. The search path is
    # the whole subtlety here — `meson-uninstalled` first so that pixiewood wins
    # `glib-2.0`, GStreamer second — so it is built explicitly, once, and the
    # same string is handed to Cargo below.
    uninstalled = meson.project_build_root() / 'meson-uninstalled'
    pkgconfig_path = uninstalled + ':' + gst_pkgconfig
    pkgconfig_exe = find_program('pkg-config', required: true)
    pkgconfig_env = {
        'PKG_CONFIG_LIBDIR': pkgconfig_path,
        'PKG_CONFIG_PATH': pkgconfig_path,
        'PKG_CONFIG_ALLOW_CROSS': '1',
    }

    # `--static`: every GStreamer library here is an archive, so their private
    # dependencies — orc, zlib, `-latomic`, `-llog` — have to come along.
    #
    # One entry per GStreamer crate the Rust builds against, because each of
    # those is a set of `gst_*` calls that this link has to answer. The plugin
    # list below names plugins, not libraries, and nothing in it reaches these
    # archives: a missing entry here is a wall of undefined symbols at the very
    # end of a seven-minute build.
    #
    # `gstreamer-gl-1.0` has no crate of its own in `Cargo.toml`. It is here for
    # `gst-plugin-gtk4`, which depends on `gstreamer-gl` unconditionally and so
    # references `gst_gl_*` whether or not any of its GL features are on. It
    # brings `-lGLESv2` and `-lEGL` with it.
    gst_modules = [
        'gstreamer-1.0',
        'gstreamer-app-1.0',
        'gstreamer-gl-1.0',
        'gstreamer-pbutils-1.0',
        'gstreamer-play-1.0',
        'gstreamer-video-1.0',
    ]

    # Nothing is discovered in a static build. The plugins listed in
    # `build-aux/android/gstreamer-plugins` are the only elements that will
    # exist, and the same list generates both the archives linked here and the
    # C that registers them, so the two cannot drift apart.
    sh = find_program('sh')
    gst_plugins_script = files('build-aux/android/gstreamer-static-plugins.sh')
    gst_plugins_list = files('build-aux/android/gstreamer-plugins')

    gst_dep = declare_dependency(
        compile_args: run_command(
            pkgconfig_exe,
            '--cflags',
            gst_modules,
            env: pkgconfig_env,
            check: true,
        ).stdout().split(),
        link_args: run_command(
            pkgconfig_exe,
            '--libs',
            '--static',
            gst_modules,
            env: pkgconfig_env,
            check: true,
        ).stdout().split() + run_command(
            sh,
            gst_plugins_script,
            gst_prefix,
            gst_plugins_list,
            '--libs',
            check: true,
        ).stdout().split(),
    )

    gst_static_plugins_c = custom_target(
        'gstreamer-static-plugins',
        input: gst_plugins_list,
        output: 'gstreamer-static-plugins.c',
        command: [sh, gst_plugins_script, gst_prefix, '@INPUT@', '--source'],
        capture: true,
    )
    message(
        'GStreamer ' + run_command(
            pkgconfig_exe,
            '--modversion',
            'gstreamer-1.0',
            env: pkgconfig_env,
            check: true,
        ).stdout().strip() + ' from ' + gst_prefix,
    )

    android_deps = [
        adw_dep,
        dl_dep,
        gio_dep,
        glib_dep,
        gst_dep,
        gtk_dep,
        log_dep,
        m_dep,
        sourceview_dep,
    ]
endif

# When the `build-env` profile is used, we only want to set up the build
# environment for the sandbox, we will not try to compile the app, so we can
# remove some build steps.
build_env_only = get_option('profile') == 'build-env'

cargo = find_program('cargo', required: true)
cargo_version = run_command(cargo, '--version', check: true).stdout().strip()
message(cargo_version)
rustc_version = run_command('rustc', '--version', check: true).stdout().strip()
message(rustc_version)

if not build_env_only
    glib_compile_resources = find_program(
        'glib-compile-resources',
        required: true,
    )
    glib_compile_schemas = find_program('glib-compile-schemas', required: true)
    desktop_file_validate = find_program(
        'desktop-file-validate',
        required: false,
    )
    appstreamcli = find_program('appstreamcli', required: false)

    blueprint_compiler = find_program('blueprint-compiler', required: true)
    blueprint_compiler_version = run_command(
        'blueprint-compiler',
        '--version',
        check: true,
    ).stdout().strip()
    message('blueprint-compiler ' + blueprint_compiler_version)
endif

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
localedir = prefix / get_option('localedir')

datadir = prefix / get_option('datadir')
pkgdatadir = datadir / meson.project_name()
iconsdir = datadir / 'icons'
podir = meson.project_source_root() / 'po'
gettext_package = meson.project_name()

if get_option('profile') == 'development' or build_env_only
    profile = 'Devel'
    application_id += '.Devel'
elif get_option('profile') == 'hack'
    profile = 'Devel'
    application_id += '.Hack'
elif get_option('profile') == 'beta'
    profile = 'Beta'
else
    profile = 'Stable'
endif

cargo_target_dir = meson.project_build_root() / 'cargo-target'
cargo_home = meson.project_build_root() / 'cargo-home'

cargo_env = ['CARGO_TARGET_DIR=' + cargo_target_dir]

if is_android
    # Meson knows how to cross-compile and Cargo does not know that Meson is
    # doing it, so everything Cargo needs to target Android has to be handed
    # over explicitly. Without this it would quietly build for the machine doing
    # the building and produce an archive that cannot be linked.
    cpu = host_machine.cpu_family()
    if cpu == 'x86_64'
        rust_triple = 'x86_64-linux-android'
    elif cpu == 'aarch64'
        rust_triple = 'aarch64-linux-android'
    elif cpu == 'x86'
        rust_triple = 'i686-linux-android'
    elif cpu == 'arm'
        rust_triple = 'armv7-linux-androideabi'
    else
        error('No Rust target for Android CPU family "' + cpu + '"')
    endif

    # Cargo spells these with underscores and in capitals.
    triple_env = rust_triple.replace('-', '_')
    triple_upper = triple_env.to_upper()

    # The NDK's clang, as Meson resolved it from pixiewood's cross file, and the
    # rest of the toolchain that sits beside it. The `cc` crates in `aws-lc-sys`,
    # `libsqlite3-sys` and `libwebp-sys` compile real C and need all of it.
    ndk_bin = fs.parent(cc.cmd_array()[0])

    # Cargo's `-sys` crates resolve GTK and friends through pkg-config, and the
    # only description of the ones being built here is the `-uninstalled.pc`
    # files Meson writes at configure time — before ninja compiles anything,
    # which is exactly why a `staticlib` works. `PKG_CONFIG_LIBDIR` *replaces*
    # the search path rather than extending it, so the host's own `/usr` `.pc`
    # files cannot leak into an Android build and quietly satisfy something.
    #
    # `pkgconfig_path` is the same string Meson resolved `gstreamer-1.0` with
    # above, so `gstreamer-sys` sees exactly what Meson linked: GStreamer from
    # the pruned prefix, GLib from pixiewood.
    cargo_env += [
        'PKG_CONFIG_PATH=' + pkgconfig_path,
        'PKG_CONFIG_LIBDIR=' + pkgconfig_path,
        'PKG_CONFIG_ALLOW_CROSS=1',
        'CC_' + triple_env + '=' + cc.cmd_array()[0],
        'AR_' + triple_env + '=' + ndk_bin / 'llvm-ar',
        'CARGO_TARGET_' + triple_upper + '_LINKER=' + cc.cmd_array()[0],
    ]

    cargo_build_options_android = [
        '--target',
        rust_triple,
        # `[profile.release] debug = true` is deliberate for desktop, where the
        # symbols cost disk and buy backtraces. Here they are ruinous: they made
        # a 3.4 GB static library and a 1.6 GB APK, which Gradle then reported it
        # could not strip. An Android build takes its diagnostics from logcat
        # instead, so nothing is lost by leaving them out.
        '--config',
        'profile.release.debug=false',
    ]
endif

# Keeping the registry and the git checkouts inside the build directory makes a
# build self-contained, which is worth having — but not at the cost of not
# building at all. Windows still limits a path to 260 characters for anything
# that has not opted out, and the libgit2 that cargo checks git dependencies out
# with is one of those: `matrix-rust-sdk` has a test fixture whose name alone is
# 96 characters, and rooting the checkout three directories deeper than the
# default pushes it over. `core.longpaths` does not help, and neither does
# `net.git-fetch-with-cli`, which changes how cargo fetches but not how it
# checks out. So Windows uses the user's own `CARGO_HOME`.
if host_machine.system() != 'windows'
    cargo_env += 'CARGO_HOME=' + cargo_home
endif

if host_machine.system() == 'darwin' or host_machine.system() == 'windows'
    # Without these, `gettext-sys` compiles and statically links its own vendored
    # copy of libintl. Point it at the one in the prefix instead, so the binary
    # links the same libintl as everything else it loads.
    gettext_prefix = glib_dep.get_variable(pkgconfig: 'prefix')

    if fs.exists(gettext_prefix / 'include' / 'libintl.h')
        cargo_env += 'GETTEXT_SYSTEM=1'
        cargo_env += 'GETTEXT_DIR=' + gettext_prefix
    else
        message(
            'No libintl.h in ' + gettext_prefix + ', gettext-rs will build its vendored copy',
        )
    endif
endif

if host_machine.system() == 'windows'
    # The name a user reads in the Properties dialog and in Task Manager, and
    # the icon Explorer, the taskbar and Alt-Tab draw. Windows keeps both inside
    # the executable, so `build.rs` compiles them in and this is where it is
    # told what to compile — the profile decides which of the two icons it is,
    # and only Meson knows the profile.
    if profile == 'Stable'
        windows_app_name = 'Commune'
        windows_appicon = 'assets/appicon.svg'
    else
        windows_app_name = 'Commune ' + profile
        windows_appicon = 'assets/appicon-devel.svg'
    endif

    windows_icon = meson.project_build_root() / 'windows' / 'commune.ico'
    run_command(
        'bash',
        meson.project_source_root() / 'build-aux' / 'windows' / 'make-ico.sh',
        meson.project_source_root() / windows_appicon,
        windows_icon,
        check: true,
    )

    cargo_env += 'COMMUNE_WINDOWS_ICON=' + windows_icon
    cargo_env += 'COMMUNE_WINDOWS_NAME=' + windows_app_name
endif

if profile == 'Devel'
    vcs_tag = run_command('git', 'rev-parse', '--short', 'HEAD', check: false).stdout().strip()
    if vcs_tag == ''
        devel_version = profile.to_lower()
    else
        devel_version = vcs_tag
    endif
    full_version += '-' + devel_version

    release_date = run_command('git', 'show', '-s', '--format=%cI', check: true).stdout().strip()

    # Setup pre-commit hook for ensuring coding style is always consistent
    message('Compiling pre-commit binary…')
    run_command(
        cargo,
        'build',
        '--manifest-path',
        'hooks/checks/Cargo.toml',
        check: true,
        env: cargo_env,
    )
    cp_bin_result = run_command(
        'cp',
        '-f',
        cargo_target_dir / 'debug/checks',
        'hooks/checks-bin',
        check: false,
    )
    cp_hook_result = run_command(
        'cp',
        '-f',
        'hooks/pre-commit.hook',
        '.git/hooks/pre-commit',
        check: false,
    )

    if cp_bin_result.returncode() == 0 and cp_hook_result.returncode() == 0
        message('Pre-commit hook installed')
    else
        if cp_bin_result.returncode() != 0
            message(
                'Could not install pre-commit binary: ' + cp_bin_result.stderr(),
            )
        endif
        if cp_hook_result.returncode() != 0
            message(
                'Could not install pre-commit hook: ' + cp_hook_result.stderr(),
            )
        endif
    endif
endif

# Generate nextest config file
# We generate it dynamically to put the test report inside the build directory,
# like all other artefacts
nextest_store_dir = meson.project_build_root() / 'nextest'
nextest_data = configuration_data()
nextest_data.set_quoted('STORE_DIR', nextest_store_dir)
nextest_config = configure_file(
    input: 'nextest.toml.in',
    output: 'nextest.toml',
    configuration: nextest_data,
)

if not build_env_only
    subdir('data')
    subdir('po')
endif

subdir('src')

# macOS packaging. There is nothing to gate on a profile here: the bundle is
# assembled out of a staged `meson install` of whatever this build directory was
# configured as, and `bundle.sh` names and identifies the result accordingly.
if host_machine.system() == 'darwin'
    macos_bundle_command = [
        meson.project_source_root() / 'build-aux' / 'macos' / 'bundle.sh',
        '--build-dir', meson.project_build_root(),
        '--app-id', application_id,
        '--version', version,
        '--profile', profile,
        '--executable', meson.project_name(),
    ]

    run_target('macos-bundle', command: macos_bundle_command)
    run_target('macos-dmg', command: macos_bundle_command + ['--dmg'])
    run_target('macos-tarball', command: macos_bundle_command + ['--tarball'])
endif

# Windows packaging. Same reasoning as the darwin block above: the folder is
# assembled out of a staged `meson install` of whatever this build directory was
# configured as, and `bundle.sh` names the result after the profile.
if host_machine.system() == 'windows'
    windows_bundle_command = [
        'bash',
        meson.project_source_root() / 'build-aux' / 'windows' / 'bundle.sh',
        '--build-dir', meson.project_build_root(),
        '--app-id', application_id,
        '--app-name', 'Commune',
        '--version', version,
        '--profile', profile,
        '--executable', meson.project_name(),
    ]

    run_target('windows-bundle', command: windows_bundle_command)
    run_target('windows-zip', command: windows_bundle_command + ['--zip'])
endif

gnome.post_install(
    gtk_update_icon_cache: true,
    glib_compile_schemas: true,
    # Only Linux installs a desktop file, and only Linux has an
    # update-desktop-database to register it with.
    update_desktop_database: host_machine.system() == 'linux',
)
