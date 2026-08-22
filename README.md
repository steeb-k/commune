<div align="center">

<img
    src="assets/appicon.svg"
    alt=""
    width="128"
    height="128"
/>

# Commune

</div>

Commune is a Matrix messaging app written in Rust. Its interface is optimized for collaboration in
large groups, such as free software projects, and will fit all screens, big or small.

Commune is a fork of [Fractal](https://gitlab.gnome.org/World/fractal), the Matrix client for GNOME.
It is not affiliated with or endorsed by the Fractal project or the GNOME project, and issues with
it should not be reported to either. See [`doc/fork.md`](doc/fork.md) for what that means in
practice.

Highlights:

* Find rooms to discuss your favorite topics, or talk privately to people, securely thanks to
  end-to-end encryption
* Send rich formatted messages, files, or your current location
* Reply to specific messages, react with emoji, edit or remove messages
* View images, and play audio and video directly in the conversation
* Send and manage custom sticker and emoticon packs
* See who has read messages, and who is typing
* Log into multiple accounts at once (with Single-Sign On support)

## Contents

<!-- toc -->
* [Installing alongside Fractal](#installing-alongside-fractal)
* [Building](#building)
* [Runtime Dependencies](#runtime-dependencies)
* [Security Best Practices](#security-best-practices)
* [Contributing](#contributing)
* [The origin of Commune](#the-origin-of-commune)
<!-- /toc -->

## Installing alongside Fractal

Commune is built to coexist with Fractal rather than replace it. Nothing is shared between the two
apps:

| | Fractal | Commune |
| --- | --- | --- |
| Application ID | `org.gnome.Fractal` | `io.github.steeb_k.Commune` |
| Binary | `fractal` | `commune` |
| Settings (dconf) | `/org/gnome/Fractal/Stable/` | `/io/github/steeb_k/Commune/Stable/` |
| Session data | `~/.local/share/fractal` | `~/.local/share/commune` |
| Cache | `~/.cache/fractal` | `~/.cache/commune` |
| Keyring items | `xdg:schema` = `org.gnome.Fractal` | `xdg:schema` = `io.github.steeb_k.Commune` |

Installing one has no effect on the other, and signing in to one does not sign you in to the other.
The development builds (`.Devel`) of each are likewise separate from their stable counterparts.

## Building

### Flatpak

Flatpak is the recommended way to build and install Commune. You need `flatpak` and
`flatpak-builder`, and the GNOME 50 runtime:

```sh
flatpak install --user flathub org.gnome.Platform//50 org.gnome.Sdk//50
```

Then, from the repository root:

```sh
# Stable build
flatpak-builder --user --install --force-clean \
    build-flatpak build-aux/io.github.steeb_k.Commune.json
flatpak run io.github.steeb_k.Commune

# Development build, installed and run side by side with the above
flatpak-builder --user --install --force-clean \
    build-flatpak-devel build-aux/io.github.steeb_k.Commune.Devel.json
flatpak run io.github.steeb_k.Commune.Devel
```

Both manifests build the working tree (`"type": "dir"`). Publishing to a repository requires
swapping that source for a `git` source pinned to a tag — see [`doc/flatpak.md`](doc/flatpak.md).

### Meson

To build against the libraries on the host instead, you need the dependencies checked by
`meson.build`, plus `blueprint-compiler`, `sass` and a Rust toolchain:

```sh
meson setup _build --prefix=~/.local -Dprofile=development
meson install -C _build
commune
```

Note that a host build installs into the same prefix as anything else you have installed there; the
Flatpak builds are what keep Commune fully self-contained.

### macOS

The GTK stack comes from a conda-forge environment that
`build-aux/macos/setup-conda-macos.sh` creates — **not** from Homebrew, which would stamp the build
machine's OS version as the deployment floor. With that in place the usual Meson build works, and
three extra targets package it:

```sh
meson setup _build-release -Dprofile=default
meson compile -C _build-release macos-bundle    # Commune.app
meson compile -C _build-release macos-tarball   # ... and a .tar.gz
meson compile -C _build-release macos-dmg       # ... and a .dmg
```

The result is a relocatable, ad-hoc signed `Commune.app` in `_build-release/macos/`. Prefer the
tarball for handing to anyone: a browser tags a downloaded `.dmg` with `com.apple.quarantine`, and
Gatekeeper refuses a quarantined app that is not signed with a Developer ID.

The menu bar, the Cmd-key shortcuts, `matrix:` links and notifications are all in place but have
not been exercised by hand yet; camera QR scanning and location sharing are stubbed. See
[`doc/macos.md`](doc/macos.md) for the full story, the environment probe, the list of what still
has to be tested, and what is stubbed.

## Runtime Dependencies

On top of the dependencies required at build time and checked by Meson, Commune depends on the
following dependencies at runtime:

* xdg-desktop-portal and its backends: some functionalities are dependent on the following portals,
  and a permission will be asked when necessary, but Commune should work without them:
  * Secret: this portal or a Secret Service is required, see [storing secrets](#storing-secrets).
  * Camera: scan QR codes during verification.
  * Location: send the user’s location in a conversation.
  * Settings: get the 12h/24h time format system preference.
* GStreamer plugins:
  * gst-plugin-gtk4 (gstgtk4): required to preview videos in the timeline and to present the output
    of the camera.
  * libgstpipewire with the `pipewiredeviceprovider`: used to list and access the cameras.

On macOS none of the portals apply. Secrets go to the Keychain, the 12h/24h format is read from the
locale at startup, and location sharing and camera QR scanning are not available. GStreamer is
still needed, including gst-plugin-gtk4; `build-aux/macos/setup-conda-macos.sh` builds that plugin
from source because conda-forge does not package it.

### Storing secrets

Commune doesn’t store your **password**, but it stores your **access token** and the **passphrase**
used to encrypt the database and the local cache.

The Commune Flatpaks use the [Secret **Portal**](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Secret.html)
to store those secrets. If you are using GNOME this should just work. If you are using a different
desktop environment or are facing issues, make sure `xdg-desktop-portal` is installed along with a
service that provides the [Secret portal backend interface](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.Secret.html),
like gnome-keyring or KWallet (since version 6.2).

Any version that is not sandboxed relies on software that implements the [Secret Service API](https://www.freedesktop.org/wiki/Specifications/secret-storage-spec/)
to store those secrets. Therefore, you need to have software providing that service on your system,
like gnome-keyring, pass with [pass_secret_service](https://github.com/mdellweg/pass_secret_service/),
or KWallet. Once again, if you are using GNOME this should just work.

If you prefer to use software that only implements the Secret Service API while using the Flatpaks,
you need to make sure that no service implementing the Secret portal backend interface is running,
and you need to allow Commune to access the D-Bus service with this command:

```sh
flatpak override --user --talk-name=org.freedesktop.secrets io.github.steeb_k.Commune
```

_For the development version, change the application ID to `io.github.steeb_k.Commune.Devel`._

Or with [Flatseal](https://flathub.org/apps/details/com.github.tchx84.Flatseal), by adding
`org.freedesktop.secrets` in the **Session Bus** > **Talk** list of Commune.

## Security Best Practices

You should use a strong **password** that is hard to guess to protect the secrets stored on your
device, whether the password is used directly to unlock your secrets (with a password manager for
example) or if it is used to open your user session and your secrets are unlocked automatically
(which is normally the case with a GNOME session).

Furthermore, make sure to lock your system when stepping away from the computer since an unlocked
computer can allow other people to access your private communications and your secrets.

## Contributing

Please follow our [contributing guidelines](CONTRIBUTING.md).

The translations under `po/` were written for Fractal by the GNOME translation team on
[Damned Lies](https://l10n.gnome.org/). Strings that Commune has not changed are still translated by
that work; strings that mention the application name are not, and show in English until they are
translated again.

The names of the emoji displayed during verification come from [the Matrix specification repository](https://github.com/matrix-org/matrix-spec/tree/main/data-definitions).
They are translated on [Element’s translation platform](https://translate.element.io/projects/matrix-doc/sas-emoji-v1).

## The origin of Commune

Commune is a fork of Fractal 14.1, taken in August 2026. Almost all of the code it runs was written
by the Fractal contributors, and the About dialog credits them.

Fractal itself is a rewrite, built on the [matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk)
and [GTK4](https://gtk.org/), of an earlier GTK3 application of the same name. That one began as a
fork of Fest <https://github.com/fest-im/fest>, formerly called ruma-gtk, and before that was called
guillotine. The name Fractal was proposed by Regina Bíró.
