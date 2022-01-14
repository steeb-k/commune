#!/bin/bash
# Source: https://gitlab.gnome.org/GNOME/fractal/blob/master/hooks/pre-commit.hook

# Usage info
show_help() {
cat << EOF
Run conformity checks on the current Rust project.

If a dependency is not found, helps the user to install it.

USAGE: ${0##*/} [OPTIONS]

OPTIONS:
    -f, --force-install     Install missing dependencies without asking
    -v, --verbose           Use verbose output
    -h, --help              Display this help and exit

ERROR CODES:
    1                       Check failed
    2                       Missing dependency
EOF
}

# Initialize variables
verbose=0
force_install=0

# Check arguments
while [[ "$1" ]]; do case $1 in
    -f | --force-install )
        force_install=1
        ;;
    -v | --verbose )
        verbose=1
        ;;
    -h | --help )
        show_help
        exit 0
        ;;
    *)
        show_help >&2
        exit 1
esac; shift; done

install_rustfmt() {
    if ! which rustup &> /dev/null; then
        curl https://sh.rustup.rs -sSf  | sh -s -- -y
        export PATH=$PATH:$HOME/.cargo/bin
        if ! which rustup &> /dev/null; then
            echo "Failed to install rustup."
            exit 2
        fi
    fi

    echo "Installing rustfmt…"
    rustup component add rustfmt
}

if ! which cargo >/dev/null 2>&1 || ! cargo fmt --help >/dev/null 2>&1; then
    echo "Unable to check Fractal’s code style, because rustfmt could not be run."

    if [[ $force_install -eq 1 ]]; then
        install_rustfmt
    elif [ ! -t 1 ]; then
        # No input is possible
        exit 2
    else
        echo ""
        echo "y: Install rustfmt via rustup"
        echo "N: Don't install rustfmt"
        echo ""
        while true
        do
            echo -n "Install rustfmt? [y/N]: "; read yn < /dev/tty
            case $yn in
                [Yy]* ) install_rustfmt; break;;
                [Nn]* | "" ) exit 2 >/dev/null 2>&1;;
                * ) echo "Invalid input";;
            esac
        done
    fi
fi

if [[ $verbose -eq 1 ]]; then
    rustc -Vv && cargo -Vv
    cargo fmt --version
fi

echo "--Checking style--"
cargo fmt --all -- --color=always --check
if test $? != 0; then
    echo "--Checking style fail--"
    echo "Please fix the above issues, either manually or by running: cargo fmt --all"

    exit 1
else
    echo "--Checking style pass--"
fi
