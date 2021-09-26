# Contributing

## Workflow

[Fractal](https://gitlab.gnome.org/GNOME/fractal/) follows the [GNOME Newcomers workflow](https://wiki.gnome.org/Newcomers/).
Follow these pages to learn how to contribute.

We only support building with [flatpak](https://flatpak.org/) using [GNOME Builder](https://wiki.gnome.org/Apps/Builder).
If you insist on using any other method, they may work, but you will be on your own.

You will need to manually install the latest version of the Rust extension of the FreeDesktop SDK.
To do this, just run `flatpak install --user org.freedesktop.Sdk.Extension.rust-stable//21.08`

## Merge requests

We expect all code contributions to be correctly formatted.
Before submitting a merge request, please run `cargo fmt` on your branch to ensure this is the case.
It is also recommended to run `cargo clippy` as that will catch common errors and improve the quality of your submissions.
