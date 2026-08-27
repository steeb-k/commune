# `build-aux/macos`

The scripts that create the macOS build environment and turn a build of it into
a `Commune.app`. The reasoning behind all of it — why conda-forge, what is
stubbed, what bit us — is in [`doc/macos.md`](../../doc/macos.md); this file is
just the map.

| Script | What it does |
| --- | --- |
| `setup-conda-macos.sh` | Creates the conda-forge GTK environment under `.conda-gtk/`, and builds the few things conda-forge does not package. |
| `probe-env.sh` | Read-only inventory of that environment. Prints one row per dependency and ends with what still has to be built. |
| `bundle.sh` | Assembles `Commune.app` from a configured Meson build directory. |
| `make-icns.sh` | Renders an app icon SVG into the `.icns` the bundle names. |
| `make-dmg.sh` | Wraps a built bundle in a compressed disk image. |
| `make-tarball.sh` | Wraps a built bundle in a `.tar.gz`. |

`Info.plist.in` is the bundle's metadata template. `bundle.sh` substitutes its
`@TOKEN@` placeholders rather than Meson, because two of the values — the
deployment target and the icon file name — are things the script works out while
it assembles the bundle.

## Building a bundle

From Meson, which fills in the application ID, version and profile that the
build directory was configured with:

```sh
meson compile -C _build macos-bundle    # Commune.app
meson compile -C _build macos-dmg       # ... and a .dmg
meson compile -C _build macos-tarball   # ... and a .tar.gz
```

All three targets exist only on darwin. Each assembles the bundle from scratch;
the last two then wrap it.

Or directly, which is the same thing with the values spelled out:

```sh
build-aux/macos/bundle.sh --build-dir _build --profile Stable --version 1.rc1 \
    --app-id io.github.steeb_k.Commune --tarball
```

The result lands in `<build-dir>/macos/`.

## Two things worth knowing before changing `bundle.sh`

**The layout is a contract.** `src/utils/app_bundle.rs` reads it back at
startup: it expects a small Unix prefix under `Contents/Resources` and points
GLib, GdkPixbuf, GStreamer and fontconfig at it with environment variables that
it sets before any of those libraries are initialized. Moving a directory here
means moving it there too.

**The `otool -L` audit at the end is the point of the exercise.** A bundle that
still names a path on the machine that built it works perfectly there and fails
on the first machine it is copied to, which is the hardest kind of failure to
notice. The audit, the dangling-symlink check and the `loaders.cache` guard all
exist because each of those went wrong once.
