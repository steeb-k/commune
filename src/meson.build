global_conf = configuration_data()
global_conf.set_quoted('APP_ID', application_id)
global_conf.set_quoted('PKGDATADIR', pkgdatadir)
global_conf.set_quoted('PROFILE', profile)
global_conf.set_quoted('VERSION', version + version_suffix)
global_conf.set_quoted('GETTEXT_PACKAGE', gettext_package)
global_conf.set_quoted('LOCALEDIR', localedir)
config = configure_file(
  input: 'config.rs.in',
  output: 'config.rs',
  configuration: global_conf
)
# Copy the config.rs output to the source directory.
run_command(
  'cp',
  meson.build_root() / 'src' / 'config.rs',
  meson.source_root() / 'src' / 'config.rs',
  check: true
)

sources = files(
  'application.rs',
  'config.rs',
  'main.rs',
  'window.rs',
  'login.rs',
  'secret.rs',
  'utils.rs',
  'session/user.rs',
  'session/mod.rs',
  'session/categories/categories.rs',
  'session/categories/category.rs',
  'session/categories/category_type.rs',
  'session/categories/mod.rs',
  'session/content/content.rs',
  'session/content/divider_row.rs',
  'session/content/item_row.rs',
  'session/content/message_row.rs',
  'session/content/mod.rs',
  'session/content/state_row.rs',
  'session/room/event.rs',
  'session/room/highlight_flags.rs',
  'session/room/item.rs',
  'session/room/mod.rs',
  'session/room/room.rs',
  'session/room/timeline.rs',
  'session/sidebar/mod.rs',
  'session/sidebar/row.rs',
  'session/sidebar/room_row.rs',
  'session/sidebar/sidebar.rs',
)

custom_target(
  'cargo-build',
  build_by_default: true,
  input: sources,
  output: meson.project_name(),
  console: true,
  install: true,
  install_dir: bindir,
  depends: resources,
  command: [
    cargo_script,
    meson.build_root(),
    meson.source_root(),
    '@OUTPUT@',
    profile,
    meson.project_name(),
  ]
)
