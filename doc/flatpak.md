# Building and distributing the Flatpak

Two manifests, both building the working tree:

| Manifest | Application ID | Meson profile |
| --- | --- | --- |
| `build-aux/io.github.steeb_k.Commune.json` | `io.github.steeb_k.Commune` | default (Stable) |
| `build-aux/io.github.steeb_k.Commune.Devel.json` | `io.github.steeb_k.Commune.Devel` | `development` |

They install and run side by side, and both are separate from any Fractal
Flatpak — different IDs, so different `~/.var/app` trees and different
portal permissions.

## Building locally

```sh
flatpak install --user flathub org.gnome.Platform//50 org.gnome.Sdk//50

flatpak-builder --user --install --force-clean \
    build-flatpak build-aux/io.github.steeb_k.Commune.json
flatpak run io.github.steeb_k.Commune
```

`build-flatpak/` and `.flatpak-builder/` are gitignored.

The Devel manifest additionally runs the test suite during the build
(`"run-tests": true`) and passes `-Dprofile=development`, which is what picks
up the blue icon, the `.Devel` suffix and debug logging.

## Building on a machine short of memory

The release profile in `Cargo.toml` sets `codegen-units = 1`, `lto = "thin"`
and `debug = true`. That combination is memory hungry, and cargo defaults to
one job per core, so the last stretch of the build — where the app crate
compiles alongside `matrix-sdk-ui`, `libadwaita` and the rest — can peak
around 6.5 GB. On a 16 GB machine with a browser open, the kernel OOM killer
takes out `rustc`, and `flatpak-builder` reports it as:

```text
Error: module commune: Child process killed by signal 15
```

That message names no cause, so check for the real one before suspecting the
manifest:

```sh
journalctl -b | grep -iE 'Killed process|oom-kill'
```

Capping cargo's parallelism helps but is not on its own enough. `cargo` is
invoked without `-j` by `src/meson.build`, so it honours `CARGO_BUILD_JOBS`
from the environment; copy the manifest, add the env to the `commune` module,
and build from the copy:

```json
"build-options": { "env": { "CARGO_BUILD_JOBS": "3" } }
```

At three jobs the build still died here, and the journal showed a _single_
`rustc` holding 3.7 GB when the kernel picked it. The heavy crates —
`matrix-sdk-ui` and its neighbours — want around 4 GB each on their own, so no
job count saves a machine that does not have that much free. Either build when
the machine is otherwise idle, or drop the debug info, which is the largest
part of it and costs nothing but a less useful backtrace:

```json
"build-options": { "env": {
    "CARGO_BUILD_JOBS": "3",
    "CARGO_PROFILE_RELEASE_DEBUG": "false"
} }
```

None of this belongs in the committed manifests. It is a property of the
machine doing the build, not of the app, and hard-coding it would throttle
every other builder — including Flathub's — and strip the debug info from
published builds.

## The runtime version

The manifests target `org.gnome.Platform//50`, not `master`. Upstream Fractal
tracks the nightly runtime because it develops against unreleased GTK;
Commune has no reason to, and a released runtime is what Flathub wants.
GNOME 50 ships GTK 4.22, libadwaita 1.9, GLib 2.88, GtkSourceView 5.20 and
glycin 2.1, all comfortably past the minimums in `meson.build`.

If a rebase raises those minimums past what GNOME 50 provides, bump
`runtime-version` in both manifests and the `GNOME_STABLE_VERSION` and
`FREEDESKTOP_RUNTIME_BRANCH` variables in `.gitlab-ci.yml` together.

## Publishing

Both manifests use a `"type": "dir"` source pointing at `../`, which builds
whatever is in the working tree. That is right for developing and wrong for
distributing: a published build has to be reproducible from a commit.

To publish, replace the source of the `commune` module with a pinned Git
source:

```json
"sources": [
    {
        "type": "git",
        "url": "https://github.com/steeb-k/commune.git",
        "tag": "v1.rc1",
        "commit": "<the commit the tag points at>"
    }
]
```

Then either build into a local repository and serve it, or submit the stable
manifest to Flathub. Flathub additionally requires:

* The screenshot URLs in
  `data/io.github.steeb_k.Commune.metainfo.xml.in.in` to resolve. They point
  at `raw.githubusercontent.com/steeb-k/commune/main/screenshots/`, so the
  repository has to exist and be public, and the images have to be retaken
  from Commune rather than inherited from Fractal.
* An `<update_contact>` in the metainfo, if you are willing to publish an
  address there.
* `flatpak-builder-lint` to pass. `.gitlab-ci/flatpak-builder-lint-exceptions.json`
  carries the exceptions inherited from upstream, keyed by application ID.

The linter ships in the `org.flatpak.Builder` Flatpak:

```sh
flatpak run --command=flatpak-builder-lint org.flatpak.Builder \
    manifest build-aux/io.github.steeb_k.Commune.json
```

As of the rebrand it reports exactly one error, `appid-url-not-reachable`,
because `https://github.com/steeb-k/commune` is still a 404. The linter checks
that the repository an `io.github.<user>.<App>` ID claims actually exists, so
that error clears by pushing the repository and not before. Nothing else in
the manifest is flagged.
