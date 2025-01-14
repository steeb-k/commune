project('fractal',
        'rust',
        version: '10.rc',
        license: 'GPL-3.0-or-later',
        meson_version: '>= 1.1')

i18n = import('i18n')
gnome = import('gnome')

base_id = 'org.gnome.Fractal'
application_id = base_id

major_version = '10'
pre_release_version = 'rc'

version = major_version
if pre_release_version != ''
  version += '.' + pre_release_version
endif
full_version = version

dependency('glib-2.0', version: '>= 2.76') # update when changing gtk version
dependency('gio-2.0', version: '>= 2.76') # always same version as glib
dependency('gtk4', version: '>= 4.16')
dependency('libadwaita-1', version: '>= 1.6')

# Please keep these dependencies sorted.
dependency('gstgtk4', version: '>= 0.13.0')
dependency('gstreamer-1.0', version: '>= 1.20')
dependency('gstreamer-app-1.0', version: '>= 1.20')
dependency('gstreamer-base-1.0', version: '>= 1.20')
dependency('gstreamer-pbutils-1.0', version: '>= 1.20')
dependency('gstreamer-play-1.0', version: '>= 1.20')
dependency('gstreamer-video-1.0', version: '>= 1.20')
dependency('gtksourceview-5', version: '>= 5.0.0')
dependency('libwebp', version: '>= 1.0.0')
dependency('openssl', version: '>= 1.0.1')
dependency('shumate-1.0', version: '>= 1.0.0')
dependency('sqlite3', version: '>= 3.24.0')

# Required by glycin crate
dependency('lcms2', version: '>=2.12.0')
dependency('libseccomp', version: '>= 2.5.0')

# Linux-only dependencies
if build_machine.system() == 'linux'
  dependency('libpipewire-0.3', version: '>= 0.3.0')
endif

glib_compile_resources = find_program('glib-compile-resources', required: true)
glib_compile_schemas = find_program('glib-compile-schemas', required: true)
desktop_file_validate = find_program('desktop-file-validate', required: false)
appstreamcli = find_program('appstreamcli', required: false)
cargo = find_program('cargo', required: true)
cargo_version = run_command(cargo, '--version', check: true).stdout().strip()
message(cargo_version)
rustc_version = run_command('rustc', '--version', check: true).stdout().strip()
message(rustc_version)

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
localedir = prefix / get_option('localedir')

datadir = prefix / get_option('datadir')
pkgdatadir = datadir / meson.project_name()
iconsdir = datadir / 'icons'
podir = meson.project_source_root() / 'po'
gettext_package = meson.project_name()

if get_option('profile') == 'development'
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

if profile == 'Devel'
  vcs_tag = run_command('git', 'rev-parse', '--short', 'HEAD').stdout().strip()
  if vcs_tag == ''
    devel_version = profile.to_lower()
  else
    devel_version = vcs_tag
  endif
  full_version += '-' + devel_version

  release_date = run_command('git', 'show', '-s', '--format=%cI').stdout().strip()

  # Setup pre-commit hook for ensuring coding style is always consistent
  message('Setting up git pre-commit hook..')
  run_command('cp', '-f', 'hooks/pre-commit.hook', '.git/hooks/pre-commit')
endif

subdir('data')
subdir('po')
subdir('src')

gnome.post_install(
  gtk_update_icon_cache: true,
  glib_compile_schemas: true,
  update_desktop_database: true,
)
