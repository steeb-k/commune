project(
    'commune',
    'rust',
    version: '14.1',
    license: 'GPL-3.0-or-later',
    meson_version: '>= 1.4',
)

fs = import('fs')
gnome = import('gnome')
i18n = import('i18n')

base_id = 'io.github.steeb_k.Commune'
application_id = base_id

major_version = '14.1'
pre_release_version = ''

version = major_version
if pre_release_version != ''
    version += '.' + pre_release_version
endif
full_version = version

glib_dep = dependency('glib-2.0', version: '>= 2.82')  # update when changing gtk version
dependency('gio-2.0', version: '>= 2.82')  # always same version as glib
dependency('gtk4', version: '>= 4.20.2')
dependency('libadwaita-1', version: '>= 1.8.0')

is_android = host_machine.system() == 'android'

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
dependency('gtksourceview-5', version: '>= 5.0.0', allow_fallback: is_android)
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
    # Android ships `libsqlite3.so` in the NDK sysroot but no `.pc` file to find
    # it with, so this assertion cannot be made there. `libsqlite3-sys` does not
    # vendor a copy — it has no `cc` dependency — so the link still needs that
    # system library to resolve; see `doc/android.md`.
    dependency('sqlite3', version: '>= 3.24.0')
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

cargo_env = [
    'CARGO_HOME=' + cargo_home,
    'CARGO_TARGET_DIR=' + cargo_target_dir,
]

if host_machine.system() == 'darwin'
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

gnome.post_install(
    gtk_update_icon_cache: true,
    glib_compile_schemas: true,
    # There is no desktop file to register on macOS, and no
    # update-desktop-database to register it with.
    update_desktop_database: host_machine.system() != 'darwin',
)
