if not build_env_only
  # Compile Blueprint resources
  blp_files = []
  foreach line : fs.read('ui-blueprint-resources.in').splitlines()
    if not line.startswith('#') and line.strip() != ''
      blp_files += line
    endif
  endforeach

  blueprints = custom_target(
    'blueprints',
    input: blp_files,
    output: '.',
    command: [blueprint_compiler, 'batch-compile', '@OUTPUT@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
  )

  # Populate the GResource file for the compiled Blueprint files dynamically,
  # using the same file list
  compiled_ui_files = ''
  foreach file : blp_files
    compiled_ui_files += '\n    <file compressed="true" preprocess="xml-stripblanks">' + file.replace('.blp', '.ui') + '</file>'
  endforeach
  ui_resources_xml = configure_file(
    input: 'ui-resources.gresource.xml.in',
    output: 'ui-resources.gresource.xml',
    configuration: { 'UI_FILES': compiled_ui_files }
  )

  ui_resources = gnome.compile_resources(
    'ui-resources',
    ui_resources_xml,
    gresource_bundle: true,
    install: true,
    install_dir: pkgdatadir,
    dependencies: [blueprints, ui_resources_xml],
  )
endif

# Generate config.rs
global_conf = configuration_data()
global_conf.set_quoted('APP_ID', application_id)
global_conf.set('DISABLE_GLYCIN_SANDBOX', get_option('disable-glycin-sandbox').to_string())
global_conf.set_quoted('GETTEXT_PACKAGE', gettext_package)
global_conf.set_quoted('LOCALEDIR', localedir)
global_conf.set_quoted('PKGDATADIR', pkgdatadir)
global_conf.set('PROFILE', profile)
global_conf.set_quoted('VERSION', full_version)
config = configure_file(
  input: 'config.rs.in',
  output: 'config.rs',
  configuration: global_conf
)
run_command(
  'cp',
  meson.project_build_root() / 'src' / 'config.rs',
  meson.project_source_root() / 'src' / 'config.rs',
  check: true
)

# Cargo settings
cargo_options = [ '--manifest-path', meson.project_source_root() / 'Cargo.toml' ]
cargo_options += [ '--target-dir', meson.project_build_root() / 'src' ]

if profile == 'Devel'
  rust_target = 'debug'
  message('Building in debug mode')
else
  cargo_options += [ '--release' ]
  rust_target = 'release'
  message('Building in release mode')
endif

cargo_env = [ 'CARGO_HOME=' + meson.project_build_root() / 'cargo-home' ]

if not build_env_only
  # Build binary with cargo
  custom_target(
    'cargo-build',
    build_by_default: true,
    build_always_stale: true,
    output: meson.project_name(),
    console: true,
    install: true,
    install_dir: bindir,
    depends: [resources, ui_resources],
    command: [
      'env',
      cargo_env,
      cargo, 'build',
      cargo_options,
      '&&',
      'cp', 'src' / rust_target / meson.project_name(), '@OUTPUT@',
    ]
  )
endif

# Build docs with rustdoc
rustdoc_flags = ' '.join([
  '-Zunstable-options',
  '--enable-index-page',
  '--extern-html-root-url=gio=https://gtk-rs.org/gtk-rs-core/stable/latest/docs/',
  '--extern-html-root-url=glib=https://gtk-rs.org/gtk-rs-core/stable/latest/docs/',
  '--extern-html-root-url=gsk4=https://gtk-rs.org/gtk4-rs/stable/latest/docs/',
  '--extern-html-root-url=gdk4=https://gtk-rs.org/gtk4-rs/stable/latest/docs/',
  '--extern-html-root-url=gtk4=https://gtk-rs.org/gtk4-rs/stable/latest/docs/',
  '--extern-html-root-url=libadwaita=https://world.pages.gitlab.gnome.org/Rust/libadwaita-rs/stable/latest/docs/',
  '--cfg=docsrs',
  '-Dwarnings',
])
doc_env = ['RUSTDOCFLAGS=' + rustdoc_flags ]
doc_deps = [
  '--package=ruma',
  '--package=ruma-common',
  '--package=ruma-client-api',
  '--package=ruma-events',
  '--package=matrix-sdk',
  '--package=matrix-sdk-base',
  '--package=matrix-sdk-common',
  '--package=matrix-sdk-crypto',
  '--package=matrix-sdk-qrcode',
  '--package=matrix-sdk-sqlite',
  '--package=matrix-sdk-store-encryption',
  '--package=matrix-sdk-ui',
  '--package=fractal',
  '--package=ashpd',
]

custom_target(
  'cargo-doc',
  build_by_default: false,
  build_always_stale: true,
  output: 'doc',
  console: true,
  command: [
    'env', cargo_env + doc_env,
    cargo, 'doc',
    cargo_options + doc_deps + ['--no-deps', '-Zrustdoc-map'] ,
  ],
)
