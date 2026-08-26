# Releasing Commune

Commune's version number is its own line, restarted at 1.rc1 after three feature rounds shipped
under the 14.1 inherited from Fractal (`34bc0e16` has the reasoning). Release candidates count
1.rc1, 1.rc2, … until a plain 1, and patch releases follow as 1.1. The same version wears three
spellings — `major.pre_release` in Meson and the app, semver (`1.0.0-rc1`) in `Cargo.toml`, and a
tilde (`1~rc1`) in the metainfo, which is the form appstream orders before the stable release.
Which upstream tree a build came from is [`doc/fork.md`](doc/fork.md)'s job, not the version's.

## Before making a new release

* Decide what the release carries. Commune releases on its own cadence — when the tree has
  something worth shipping — not when Fractal does. Upstream is a maintenance input, not a
  schedule: check that the `matrix-sdk`, `matrix-sdk-store-encryption` and `ruma` pins in
  `Cargo.toml` are current enough to carry the latest protocol and cryptography fixes, and if they
  are stale, do the rebase or the security cherry-picks [`doc/fork.md`](doc/fork.md) describes
  first.
* Run the full sweep on the tree being released: `meson install -C _build`,
  `cargo clippy --all-targets`, `cargo test -j 2 --bin commune`, and `hooks/checks-bin`. There is
  no CI on the GitHub repository — the inherited `.gitlab-ci.yml` never runs there — so these
  local checks are the whole gate.
* Check that `hooks/doc-freshness` reports the pages level with the code, and that
  [`doc/eyeball-tests.md`](doc/eyeball-tests.md) has no unstruck entries for anything the release
  ships.
* Build both Flatpak manifests and check that the app starts, signs in, and that an existing
  session still restores. See [`doc/flatpak.md`](doc/flatpak.md).

## Making a new release

1. Make a single release commit, described [below](#release-commit-content).
2. [Create a tag](#creating-a-tag) on that commit and push it.
3. Create a release on GitHub for the tag, and attach the
   [macOS and Windows artefacts](#macos-and-windows) to it.
4. [Publish the Flatpak](#publishing-the-flatpak).

## Release commit content

* Update `/meson.build`:
  * Change the version on L4, it must look the same as it would in the app, with a
    `major_version.pre_release_version` format.
  * Change the `major_version` and `pre_release_version` below it. For stable versions,
    `pre_release_version` should be an empty string.
* Update `/Cargo.toml`: change the `version`, using a semver format, and let a build carry the
  bump into `Cargo.lock`.
* Update `/data/io.github.steeb_k.Commune.metainfo.xml.in.in`:
  * Add a new `release` entry at the top of the `releases`:
    * Its `version` should use the `major_version~pre_release_version` format.
    * For stable versions, its `type` should be `stable`, otherwise it should be `development`.
  * The current entry's feature list grows a bullet in the same commit as each feature that lands
    (`fe8aa4e1` started this), so writing the notes at release time should mean reviewing them,
    not reconstructing three months from the log.
  * Remove all the `development` entries for stable releases.
* If there were visible changes in the UI, update the screenshots in `/screenshots`. They should
  follow [Flathub's quality guidelines](https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines/quality-guidelines#screenshots),
  and they must be of Commune — the ones inherited from the fork are of Fractal.

Then run the validators:

```sh
meson setup _build --prefix=~/.local -Dprofile=development
ninja -C _build data/io.github.steeb_k.Commune.Devel.desktop \
                data/io.github.steeb_k.Commune.Devel.metainfo.xml
meson test -C _build validate-desktop validate-appdata validate-gschema
```

## Creating a tag

Tags carry a `v` prefix — `v1.rc1` — which is the form the pinned Flatpak source in
[`doc/flatpak.md`](doc/flatpak.md) expects. Each release candidate gets its own tag; a tag is
never moved or reused, because GitHub releases and anyone who fetched it keep the old commit
silently.

Signing the tag is not mandatory but is good practice:

```sh
git tag -s vV
```

With `V` being the version, in the `major_version.pre_release_version` format. You will be
prompted for a tag message; something like `Release Commune V` suffices.

## Publishing the Flatpak

Both manifests in `build-aux/` build the working tree. A published build must instead come from the
tag, so swap the `commune` module's source for a pinned Git source before building — the exact form
is in [`doc/flatpak.md`](doc/flatpak.md).

### To a repository you host

```sh
flatpak-builder --repo=<repo-dir> --force-clean \
    build-flatpak build-aux/io.github.steeb_k.Commune.json
flatpak build-sign <repo-dir> --gpg-sign=<key>
flatpak build-update-repo <repo-dir> --gpg-sign=<key>
```

Serve `<repo-dir>` over HTTP and publish a `.flatpakrepo` file pointing at it, so that a user can
`flatpak remote-add` it once and then get updates normally.

### To Flathub

Flathub hosting requires a public repository at `github.com/steeb-k/commune`, because that is what
the `io.github.steeb_k.*` application ID claims. Submission is a PR against
[flathub/flathub](https://github.com/flathub/flathub), after which Flathub creates a
`flathub/io.github.steeb_k.Commune` repository that holds the published manifest; later releases are
PRs against that repository.

`flatpak-builder-lint` must pass on the manifest and on the built repository. The exceptions
inherited from Fractal are in `.gitlab-ci/flatpak-builder-lint-exceptions.json`, keyed by
application ID. The remaining prerequisites — reachable screenshot URLs, an `<update_contact>` —
are listed in [`doc/flatpak.md`](doc/flatpak.md).

More details are in the Flathub docs about [maintenance](https://docs.flathub.org/docs/for-app-authors/maintenance)
and [updates](https://docs.flathub.org/docs/for-app-authors/updates).

## macOS and Windows

Both ports package from a build directory configured as `-Dprofile=default`, which is what names
the bundle `Commune` rather than `Commune Devel` and gives it the plain icon. Each platform's
ledger carries the details and the traps — notably the release profile's memory appetite on
macOS, and `Info.plist` accepting only the numeric prefix of the version.

On a Mac ([`doc/macos.md`](doc/macos.md)):

```sh
meson compile -C _build macos-dmg       # Commune.app and a .dmg in <build-dir>/macos/
meson compile -C _build macos-tarball   # ... or a .tar.gz
```

On Windows ([`doc/windows.md`](doc/windows.md)), from MSYS2 and then PowerShell:

```sh
meson compile -C _build windows-bundle
pwsh -File build-aux\windows\build-msi.ps1 -BundleDir "_build\windows\Commune" -Profile Stable
```

Attach the `.dmg` (or `.tar.gz`) and the `.msi` to the GitHub release for the tag.
