# The rebrand to Commune

This file is the ledger for turning Fractal into Commune: what was renamed,
what was deliberately left alone, and how to resolve the conflicts this
causes on a rebase. See `fork.md` for why this tree is a fork at all.

## The identifiers

| | Value |
| --- | --- |
| Application ID | `io.github.steeb_k.Commune` |
| Development ID | `io.github.steeb_k.Commune.Devel` |
| Meson project / Cargo package / binary | `commune` |
| Gettext domain | `commune` |
| GSettings path | `/io/github/steeb_k/Commune/<profile>/` |
| Session data | `~/.local/share/commune`, `~/.local/share/commune-Devel` |
| Cache | `~/.cache/commune`, `~/.cache/commune-Devel` |

The ID uses `steeb_k`, with an underscore, because an application ID is also
a D-Bus name and D-Bus name elements cannot contain a hyphen. The GitHub
account it claims is `steeb-k`; Flathub accepts an `io.github.<user>.<App>`
ID when the source lives under that account.

## Most of the rebrand is one line of Meson

`meson.build` already threads a single `base_id` through the desktop file,
the metainfo, the GSettings schema and the D-Bus service, and derives
`gettext_package` from `meson.project_name()`. Two things follow from the
project name alone and needed no code change:

* `pkgdatadir`, and so `PKGDATADIR` and both gresource paths in `config.rs`.
* `AppProfile::dir_name()` in `src/application.rs`, which builds the session
  data and cache directory names out of `GETTEXT_PACKAGE`. Renaming the Meson
  project is what moves `~/.local/share/fractal` to `~/.local/share/commune`.

The keyring is separated by `APP_ID`, which `src/secret/linux.rs` writes as
the `xdg:schema` attribute of every stored session. Nothing else was needed
to keep the two apps' sessions apart.

## What was deliberately not renamed

**The GResource prefix stays `/org/gnome/Fractal/`.** It appears in more than
200 lines across roughly 150 files, almost all of them
`#[template(resource = "/org/gnome/Fractal/ui/...")]` attributes. It is
compiled into the app's own gresource bundle, is namespaced per install by
`PKGDATADIR`, and is never shown to anyone. Rewriting it would turn every
file in `src/` into a rebase conflict candidate in exchange for nothing a
user can see. `Application::properties()` sets `resource-base-path` to it
explicitly, so GTK never derives it from the application ID.

For the same reason `data/resources/icons/scalable/apps/org.gnome.Fractal.svg`
keeps its name. Its **contents** are the Commune icon; only the filename is
inherited, and it is only ever referenced through that unchanged prefix, by
`data/resources/resources.gresource.xml` and `src/login/local_server.rs`.

**The composer mention tags stay `<org.gnome.fractal.mention>`**
(`src/session_view/room_history/message_toolbar/composer_state.rs`). They
are internal markers inside the composer buffer and are stripped before a
message is sent, so they never reach the wire.

**Upstream URLs in comments and tests stay.** `src/utils/matrix/mod.rs` cites
the Fractal issue a workaround came from, and `src/utils/string/tests.rs` uses
a `gitlab.gnome.org/World/fractal` URL as link-detection test data. Both are
correct as they are.

## What was renamed, and where

Build and packaging, all of it ours to keep on a rebase:

* `meson.build` — project name, `base_id`; `meson.options` — description.
* `Cargo.toml` — package name; `src/meson.build` — `--package=commune`.
* `data/io.github.steeb_k.Commune.{desktop.in.in,gschema.xml.in,metainfo.xml.in.in,service.in}`
  and `data/meson.build`, which now derives the service input from `base_id`.
* `data/icons/io.github.steeb_k.Commune{,.Devel,-symbolic}.svg`. The first two
  are byte copies of `assets/appicon.svg` and `assets/appicon-devel.svg`,
  which are the sources; copy them again rather than editing the installed
  files. The Devel one carries GNOME's hazard tape. Nothing draws that tape
  for you — a `.Devel` icon is an ordinary hand-drawn file, and
  `data/icons/meson.build` simply installs it under the real application ID
  when the profile is Devel. The symbolic one is drawn separately at 16px,
  because the interlocking bars do not survive being scaled down that far.

  All three carry no `clipPath`, `mask` or `filter`, and new shapes must keep
  it that way. QtSvg renders only SVG Tiny 1.2 and silently ignores
  `clip-path`, so the earlier versions — which built the bubble by clipping
  full-bleed rectangles — came out as edge-to-edge squares in Qt-based
  launchers and panels while looking correct everywhere GTK draws them, since
  librsvg handles the full spec. The shapes are now trimmed to the silhouette
  as literal geometry instead. `assets/appicon.svg` is also the content of
  `data/resources/icons/scalable/apps/org.gnome.Fractal.svg`, so a change to
  the artwork is three files, not one.
* `build-aux/io.github.steeb_k.Commune{,.Devel}.json` — see `flatpak.md`.
* `po/POTFILES.in`, `.gitattributes`, `.gitlab-ci.yml`,
  `.gitlab-ci/flatpak-builder-lint-exceptions.json`.
* `fractal.doap` was deleted. It exists only to describe a project to GNOME's
  infrastructure, which this one is not part of.

User-visible strings in `src/`, which is the whole of the divergence a rebase
has to carry inside the code:

* `application.rs` — `APP_NAME`, `APP_HOMEPAGE_URL`, and the About dialog.
* `main.rs` — `set_application_name`, the default `RUST_LOG` filter (which
  matches on the crate name and so had to follow the Cargo rename), and the
  rustdoc logo URLs.
* `login/method_page.rs` — `initial_device_display_name`, the name other
  people see for this device in a Matrix room.
* `secret/linux.rs` — the label of the keyring item.
* `login/local_server.rs` — the OAuth "you can go back now" page.
* `identity_verification_view/no_supported_methods_page.rs` — four strings.
* `account_settings/encryption_page/import_export_keys_subpage.rs` — the
  default filename for exported room keys.
* `window.blp`, `error_page.blp`, `login/greeter.blp`,
  `login/session_setup_view.blp`, `session_view/sidebar/mod.blp`.

Two removals follow from the About dialog no longer offering Fractal's Matrix
room as its support URL: the `connect_activate_link` handler that opened that
room inside the app, and `SessionList::has_session_ready`, which the handler
was the only caller of. If upstream grows another caller, take it back.

One wire-format identifier changed: the image packs room account data event
is now `io.github.steeb_k.Commune.image_packs_room`
(`src/session/image_packs/events.rs`). It was renamed without a migration, so
a pack room configured under the old type is not found and a new one is
created on demand. See `image-packs.md`.

## Rebasing

The renamed files conflict as delete/add pairs against any upstream change to
their `org.gnome.Fractal` originals. Take the upstream change, apply it to the
`io.github.steeb_k.Commune` file, and drop the original.

For everything under `src/`, the rule is the ordinary one: take upstream, then
re-apply the string changes listed above. The list is short by design — if it
starts growing, that is a sign a change should have gone through `base_id` or
`meson.project_name()` instead.

The About dialog is the one place where taking upstream wholesale is wrong in
a way that is not obvious. It credits the Fractal contributors in a
`add_credit_section`, and that credit must survive: nearly all of the code
this app runs is theirs.
