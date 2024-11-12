# Stylesheets
# We accept grass (a Rust SASS compiler) or sass (the official dart SASS compiler).
sass_bin = find_program('grass', required: false)
sass_options = []
if not sass_bin.found()
  # Require SASS as we need at least one compiler.
  sass_bin = find_program('sass')

  if sass_bin.found()
    # these options are not supported by grass.
    sass_options += ['--no-error-css', '--no-source-map']
  endif
endif

scss_files = [
  'style',
  'style-hc',
]

# Keep ordered alphabetically.
scss_deps = files([
  'stylesheet/_common.scss',
  'stylesheet/_components.scss',
  'stylesheet/_config.scss',
  'stylesheet/_login.scss',
  'stylesheet/_room_details.scss',
  'stylesheet/_room_history.scss',
  'stylesheet/_session_view.scss',
  'stylesheet/_vendor.scss',
])

stylesheet_deps = []

foreach scss: scss_files
  stylesheet_deps += custom_target('@0@.scss'.format(scss),
    input: '@0@.scss'.format(scss),
    output: '@0@.css'.format(scss),
    command: [
      sass_bin, sass_options, '@INPUT@', '@OUTPUT@',
    ],
    depend_files: scss_deps,
  )
endforeach


# Resources
resources = gnome.compile_resources(
  'resources',
  'resources.gresource.xml',
  gresource_bundle: true,
  install: true,
  install_dir: pkgdatadir,
  dependencies: stylesheet_deps,
)
