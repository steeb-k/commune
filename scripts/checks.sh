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

# Style helpers
act="\e[1;32m"
err="\e[1;31m"
pos="\e[32m"
neg="\e[31m"
res="\e[0m"

# Common styled strings
Installing="${act}Installing${res}"
Checking="  ${act}Checking${res}"
Failed="    ${err}Failed${res}"
error="${err}error:${res}"
invalid="${neg}Invalid input${res}"
ok="${pos}ok${res}"
fail="${neg}fail${res}"

# Initialize variables
force_install=0
verbose=0

# Check if rustup is available.
# Argument:
#   '-i' to install if missing.
check_rustup() {
    if ! which rustup &> /dev/null; then
        if [[ "$1" == '-i' ]]; then
            echo -e "$Installing rustup…"
            curl https://sh.rustup.rs -sSf  | sh -s -- -y
            export PATH=$PATH:$HOME/.cargo/bin
            if ! which rustup &> /dev/null; then
                echo -e "$Failed to install rustup"
                exit 2
            fi
        else
            exit 2
        fi
    fi
}

# Install cargo via rustup.
install_cargo() {
    check_rustup -i
    if ! which cargo >/dev/null 2>&1; then
        echo -e "$Failed to install cargo"
        exit 2
    fi
}

# Check if cargo is available. If not, ask to install it.
check_cargo() {
    if ! which cargo >/dev/null 2>&1; then
        echo "Unable to find cargo for pre-commit checks"

        if [[ $force_install -eq 1 ]]; then
            install_cargo
        elif [ ! -t 1 ]; then
            exit 2
        elif check_rustup; then
            echo -e "$error rustup is installed but the cargo command isn't available"
            exit 2
        else
            echo ""
            echo "y: Install cargo via rustup"
            echo "N: Don't install cargo and abort checks"
            echo ""
            while true; do
                echo -n "Install cargo? [y/N]: "; read yn < /dev/tty
                case $yn in
                    [Yy]* )
                        install_cargo
                        break
                        ;;
                    [Nn]* | "" )
                        exit 2
                        ;;
                    * ) 
                        echo $invalid
                        ;;
                esac
            done
        fi
    fi

    if [[ $verbose -eq 1 ]]; then
        echo ""
        rustc -Vv && cargo -Vv
        echo ""
    fi
}

# Install rustfmt with rustup.
install_rustfmt() {
    check_rustup -i

    echo -e "$Installing rustfmt…"
    rustup component add rustfmt
    if ! cargo fmt --version >/dev/null 2>&1; then
        echo -e "$Failed to install rustfmt"
        exit 2
    fi
}

# Run rustfmt to enforce code style.
run_rustfmt() {
    if ! cargo fmt --version >/dev/null 2>&1; then
        echo "Unable to check Fractal’s code style, because rustfmt could not be run"

        if [[ $force_install -eq 1 ]]; then
            install_rustfmt
        elif [ ! -t 1 ]; then
            exit 2
        else
            echo ""
            echo "y: Install rustfmt via rustup"
            echo "N: Don't install rustfmt and abort checks"
            echo ""
            while true; do
                echo -n "Install rustfmt? [y/N]: "; read yn < /dev/tty
                case $yn in
                    [Yy]* ) 
                        install_rustfmt
                        break
                        ;;
                    [Nn]* | "" )
                        exit 2
                        ;;
                    * ) 
                        echo $invalid
                        ;;
                esac
            done
        fi
    fi
    
    echo -e "$Checking code style…"

    if [[ $verbose -eq 1 ]]; then
        echo ""
        cargo fmt --version
        echo ""
    fi

    if ! cargo fmt --all -- --check; then
        echo -e "  Checking code style result: $fail"
        echo "Please fix the above issues, either manually or by running: cargo fmt --all"
        exit 1
    else
        echo -e "  Checking code style result: $ok"
    fi
}

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

# Run
check_cargo
run_rustfmt
