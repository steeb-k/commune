# Releasing Commune

Commune's version number tracks the Fractal release it is based on, so that it is always obvious
which upstream tree a build came from. A release of Commune that adds nothing from upstream keeps
the major version and bumps the pre-release part.

## Before making a new release

* Rebase onto the upstream release you intend to ship, following [`doc/fork.md`](doc/fork.md) and
  the rebase guides in [`doc/image-packs.md`](doc/image-packs.md) and
  [`doc/rebrand.md`](doc/rebrand.md).
* Check that the `matrix-sdk`, `matrix-sdk-store-encryption` and `ruma` pins in `Cargo.toml` moved.
  These carry the protocol and cryptography fixes and are the whole reason for tracking upstream.
* Build both Flatpak manifests and check that the app starts, signs in, and that an existing
  session still restores. See [`doc/flatpak.md`](doc/flatpak.md).

## Making a new release

1. Make a single release commit, described [below](#release-commit-content).
2. [Create a signed tag](#creating-a-signed-tag) on that commit.
3. Push the tag and create a release on GitHub for it.
4. [Publish the build](#publishing-a-build).

## Release commit content

* Update `/meson.build`:
  * Change the version on L4, it must look the same as it would in the app, with a
    `major_version.pre_release_version` format.
  * Change the `major_version` and `pre_release_version` below it. For stable versions,
    `pre_release_version` should be an empty string.
* Update `/Cargo.toml`: change the `version`, using a semver format.
* Update `/data/io.github.steeb_k.Commune.metainfo.xml.in.in`:
  * Add a new `release` entry at the top of the `releases`:
    * Its `version` should use the `major_version~pre_release_version` format.
    * For stable versions, its `type` should be `stable`, otherwise it should be `development`.
  * Remove all the `development` entries for stable releases.
* If there were visible changes in the UI, update the screenshots in `/screenshots`. They should
  follow [Flathub's quality guidelines](https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines/quality-guidelines#screenshots),
  and they must be of Commune — the ones inherited from the fork are of Fractal.

Then run the validators, which are the same ones CI runs:

```sh
meson setup _build --prefix=~/.local -Dprofile=development
ninja -C _build data/io.github.steeb_k.Commune.Devel.desktop \
                data/io.github.steeb_k.Commune.Devel.metainfo.xml
meson test -C _build validate-desktop validate-appdata validate-gschema
```

## Creating a signed tag

Creating a signed tag is not mandatory but is good practice. To do so, use this command:

```sh
git tag -s V
```

With `V` being the version to tag, in the format `major_version.pre_release_version`.

You will be prompted for a tag message. This message doesn't really matter so something like
`Release Commune V` should suffice.

## Publishing a build

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
