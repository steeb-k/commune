project(
    'commune',
    'rust',
    version: '1.rc1',
    license: 'GPL-3.0-or-later',
    meson_version: '>= 1.4',
)

fs = import('fs')
gnome = import('gnome')
i18n = import('i18n')

base_id = 'io.github.steeb_k.Commune'
application_id = base_id

major_version = '1'
pre_release_version = 'rc1'

version = major_version
if pre_release_version != ''
    version += '.' + pre_release_version
endif
full_version = version

glib_dep = dependency('glib-2.0', version: '>= 2.82')  # update when changing gtk version
dependency('gio-2.0', version: '>= 2.82')  # always same version as glib
dependency('gtk4', version: '>= 4.20.2')
dependency('libadwaita-1', version: '>= 1.8.0')

# Please keep these dependencies sorted.
dependency('gstreamer-1.0', version: '>= 1.20')
dependency('gstreamer-app-1.0', version: '>= 1.20')
dependency('gstreamer-base-1.0', version: '>= 1.20')
dependency('gstreamer-pbutils-1.0', version: '>= 1.20')
dependency('gstreamer-play-1.0', version: '>= 1.20')
dependency('gstreamer-sdp-1.0', version: '>= 1.20')
dependency('gstreamer-video-1.0', version: '>= 1.20')
dependency('gstreamer-webrtc-1.0', version: '>= 1.20')
dependency('gtksourceview-5', version: '>= 5.0.0')
if host_machine.system() == 'linux'
    # glycin decodes images on Linux. Every other platform uses the `image`
    # crate instead, see `src/utils/media/image/decoder/`.
    dependency('glycin-2', version: '>= 2.0.0')
    dependency('glycin-gtk4-2', version: '>= 2.0.0')
endif
dependency('libwebp', version: '>= 1.0.0')
dependency('shumate-1.0', version: '>= 1.1.0')
dependency('sqlite3', version: '>= 3.24.0')

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
