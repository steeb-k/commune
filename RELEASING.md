# Releasing Fractal

## Before making a new release

- Update the dependencies (crates or system libraries) and migrate from deprecated APIs.
- Make the `build-stable` CI jobs use the latest stable GNOME runtime.

## Making a new stable release

1. If this is a new major version, create a new `fractal-M` branch, where `M` is the major version
  number.
2. Create a [release merge request](#release-merge-request-content) against the major version
  branch.
3. After the MR is merged, [create a tag](#creating-a-signed-tag) on the last commit of the major
  version branch.
4. Create a release on GitLab for that tag.
5. Make a fast-forward merge of the major version branch to `main`.
6. [Publish the new version on Flathub and Flathub beta](#publishing-a-version-on-flathub).

## Making a new beta release

1. Create a [release merge request](#release-merge-request-content) against `main`.
2. After the MR is merged, [create a tag](#creating-a-signed-tag) on the last commit of `main`.
3. Create a release on GitLab for that tag.
4. [Publish the new version on Flathub beta](#publishing-a-version-on-flathub).

## Release merge request content

_To represent conditional list items, this section will start items with "**stable.**" to mean "if
this is a stable release"._

Make a single release commit containing the following changes:

- Update `/meson.build`:
  - Change the version on L3, it must look the same as it would in the app, with a
    `major_version.pre_release_version` format.
  - Change the `major_version` and `pre_release_version` on L13-14. For stable versions,
    `pre_release_version` should be an empty string.
- Update `/Cargo.toml`: change the `version`, using a semver format.
- Update `/README.md`:
  - **stable.** update the current stable version and its release date.
  - Update the current beta version. For stable versions, put `(same as stable)` instead of the
    release date.
- Update `/data/org.gnome.Fractal.metainfo.xml.in.in`:
  - Add a new `release` entry at the top of the `releases`:
    - Its `version` should use the `major_version~pre_release_version` format.
    - For stable versions, its `type` should be `stable`, otherwise it should be `development`.
  - **stable.** remove all the `development` entries.
  - **stable.** update the paths of the screenshots to point to the major version branch.
- **stable.** If there were visible changes in the UI, update the screenshots in `/screenshots`.
  They should follow [Flathub's quality guidelines](https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines/quality-guidelines#screenshots),
  with the following window sizes:
  - `main.png`: 760×550.
  - `adaptive.png`: 360×600.
  - `media-history.png`: 500×540.

A good practice in this merge request is to launch the `build-stable` CI jobs to make sure that
Fractal builds with the stable Flatpak runtime.

## Creating a signed tag

Creating a signed tag is not mandatory but is good practice. To do so, use this command:

```sh
git tag -s V
```

With `V` being the version to tag, in the format `major_version.pre_release_version`.

You will be prompted for a tag message. This message doesn't really matter so something like
`Release Fractal V` should suffice.

## Publishing a version on Flathub

Publishing a version of Fractal on Flathub is done via its [Flathub repository on GitHub](https://github.com/flathub/org.gnome.Fractal/).
A permission from the Flathub team granted to your GitHub account is necessary to merge PRs on this
repository and interact with the buildbot, but anyone can open a PR.

1. Open a PR against the correct branch. For a stable build, work against the `master` branch, for a
beta build, work against the `beta` branch. It must contain a commit that updates the manifest to:
  - Use the latest GNOME runtime.
  - Make sure that the Flatpak dependencies are the same as in the nightly manifest, and using the
    same version.
  - Build the latest version of Fractal, identified by its tag _and_ commit hash. 
2. When the PR is opened, a CI job will update the `cargo-sources.json` file with the latest Rust
  dependencies for Fractal and add a commit to the PR.
3. The Flathub buildbot will likely launch 2 builds, one for when the PR was opened, and another one
  after the `cargo-sources.json` commit. You can stop any of those 2 builds, as long as the
  remaining one uses the latest commit as the `flathub_revision` in the `Build Properties` tab.
4. Once the build succeeds, test the generated Flatpak as instructed and watch for obvious errors.
  If there are no issues, merge the PR.
5. To watch the next steps, go to https://buildbot.flathub.org/. You should see a job for Fractal
  start soon, unless there is a queue in `Builds > Pending Buildrequests`. Open that job and wait
  for it to complete. When it is complete, there will be instructions again to test that build.
6. If everything is fine with the build, click the `Publish` button to publish the build right away.
  If this is not done, the build will be published anyway after 3 to 4 hours. There can be some time
  before the publish job is complete and `flatpak update` offers to update the app.

## Launching a build manually for Flathub

In most cases, this should not be necessary. Flathub launches builds automatically for PRs and after
a PR is merged. If those builds fail, there is a `Rebuild` button to trigger a new build. However in
some cases, if the previous builds are not available anymore, if is necessary to trigger a build
manually:

1. Go to https://buildbot.flathub.org/ and log in.
2. Click on `Start build`.
3. Enter only the App ID. It is `org.gnome.Fractal` for the stable branch, and
  `org.gnome.Fractal/beta` for the beta branch.
4. If you only want to trigger a test build, i.e. one that will not be published in the end, check
  the corresponding setting.
5. Click on `Start build`.
