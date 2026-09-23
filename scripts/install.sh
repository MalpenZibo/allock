#!/usr/bin/env bash
# Build allock and install it system-wide.
#
#   scripts/install.sh            binary + PAM policy (lock screen)
#   scripts/install.sh --greeter  also the greeter's own config, wallpaper,
#                                 cache directory and niri config
#
# Run it as yourself, not as root: it builds with your toolchain and asks sudo
# only for the steps that write outside your home. It never edits
# /etc/greetd/config.toml or your own configs — see docs/SETUP.md for those.
#
# Options:
#   --greeter    install the greeter files as well
#   --force      overwrite greeter files that already exist
#   --prefix P   install under P instead of /usr/local
#   --wallpaper F  the greeter's wallpaper (default: GREETER_WALLPAPER below)

set -euo pipefail

PREFIX=/usr/local
# One fixed picture for the greeter, copied once at install: the session's
# wallpaper rotates through a library the greeter cannot read, and keeping a
# second library in step with it is not worth it. Falls back to the
# `[background] path` of your allock config when this file is missing.
GREETER_WALLPAPER="$HOME/.config/niri-wallpaper/library/Abstract - Nature.jpg"
GREETER=0
FORCE=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --greeter) GREETER=1 ;;
        --force) FORCE=1 ;;
        --prefix) PREFIX="$2"; shift ;;
        --wallpaper) GREETER_WALLPAPER="$2"; shift ;;
        -h | --help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

if [[ $EUID -eq 0 ]]; then
    echo "Run this as your own user; it uses sudo where it needs root." >&2
    exit 1
fi

REPO="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$PREFIX/bin/allock"
SHARE="$PREFIX/share/allock"
GREETD=/etc/greetd
GREETER_CACHE=/var/cache/allock

step() { printf '\n==> %s\n' "$*"; }

# Write stdin to a root-owned file, unless it exists and --force was not given.
put() {
    local dest="$1" mode="$2"
    if [[ -e "$dest" && $FORCE -eq 0 ]]; then
        echo "    keeping existing $dest (--force to overwrite)"
        cat >/dev/null
        return
    fi
    sudo install -Dm"$mode" /dev/stdin "$dest"
    echo "    wrote $dest"
}

step "Building (release)"
cargo build --release --manifest-path "$REPO/Cargo.toml"

step "Installing the binary"
sudo install -Dm755 "$REPO/target/release/allock" "$BIN"
echo "    $BIN"

step "Installing the PAM policy"
if [[ -e /etc/pam.d/system-auth ]]; then
    sudo install -Dm644 "$REPO/pam/allock" /etc/pam.d/allock
else
    # Debian and derivatives name the shared stack common-auth.
    sed 's/system-auth/common-auth/' "$REPO/pam/allock" | sudo install -Dm644 /dev/stdin /etc/pam.d/allock
fi
echo "    /etc/pam.d/allock"

if ! fc-list | grep -qi "Symbols Nerd Font"; then
    echo "    note: no 'Symbols Nerd Font' installed; icons will render as boxes"
fi

if [[ $GREETER -eq 0 ]]; then
    printf '\nDone. Lock mode is ready: run `allock --preview` to try it, then see docs/SETUP.md.\n'
    exit 0
fi

# ---------------------------------------------------------------------------
# Greeter
# ---------------------------------------------------------------------------

if ! id greeter >/dev/null 2>&1; then
    echo "No 'greeter' user: install greetd first (it creates one)." >&2
    exit 1
fi
if ! command -v niri >/dev/null; then
    echo "The greeter runs inside niri, which is not installed." >&2
    exit 1
fi

USER_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/allock/config.toml"
NIRI_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/niri/config.kdl"
ASHELL_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/ashell/config.toml"

# The first `key = "value"` or `key = { ... }` line in a TOML file, verbatim.
toml_line() { grep -m1 -E "^[[:space:]]*$1[[:space:]]*=" "$2" 2>/dev/null || true; }
# The first quoted value after `key` in a KDL file.
kdl_value() { grep -m1 -E "^[[:space:]]*$1[[:space:]]+\"" "$2" 2>/dev/null | sed -E 's/^[^"]*"([^"]*)".*/\1/' || true; }

step "Copying what the greeter cannot read from your home"
# The greeter runs as the `greeter` user, whose `~` is not yours: anything the
# lock screen reads from your home has to be copied somewhere it can reach.

WALLPAPER_LINE=""
wallpaper_src="$GREETER_WALLPAPER"
if [[ ! -r "$wallpaper_src" ]]; then
    wallpaper_src="$(toml_line path "$USER_CONFIG" | sed -E 's/^[^"]*"([^"]*)".*/\1/')"
    wallpaper_src="${wallpaper_src/#\~/$HOME}"
fi
if [[ -n "$wallpaper_src" && -r "$wallpaper_src" ]]; then
    resolved="$(readlink -f "$wallpaper_src")"
    ext="${resolved##*.}"
    dest="$SHARE/greeter-wallpaper.${ext,,}"
    sudo install -Dm644 "$resolved" "$dest"
    WALLPAPER_LINE="path = \"$dest\""
    echo "    wallpaper: $resolved -> $dest"
else
    echo "    no readable wallpaper ($GREETER_WALLPAPER, or [background] path in $USER_CONFIG); the greeter uses a plain colour"
fi

ASHELL_LINE='ashell_config = ""'
if [[ -r "$ASHELL_CONFIG" ]]; then
    sudo install -Dm644 "$ASHELL_CONFIG" "$GREETD/allock-ashell.toml"
    ASHELL_LINE="ashell_config = \"$GREETD/allock-ashell.toml\""
    echo "    ashell colours: $ASHELL_CONFIG -> $GREETD/allock-ashell.toml"
fi

MAIN_OUTPUT_LINE="$(toml_line main_output "$USER_CONFIG")"
[[ -n "$MAIN_OUTPUT_LINE" ]] || MAIN_OUTPUT_LINE='main_output = "All"'

step "Writing the greeter's allock config"
put "$GREETD/allock.toml" 644 <<EOF
# allock's config in greeter mode. Written by scripts/install.sh.
$ASHELL_LINE
$MAIN_OUTPUT_LINE

[background]
$WALLPAPER_LINE

[indicators]
battery = true

[greeter]
# Start on the last account and session. Kept in $GREETER_CACHE, which only
# the greeter user can read.
remember = true
EOF

step "Creating the greeter's cache ($GREETER_CACHE)"
sudo install -d -m700 -o greeter -g greeter "$GREETER_CACHE"

step "Writing the greeter's niri config"
layout="$(kdl_value layout "$NIRI_CONFIG")"
options="$(kdl_value options "$NIRI_CONFIG")"
cursor_theme="$(kdl_value xcursor-theme "$NIRI_CONFIG")"
CURSOR_BLOCK=""
if [[ -n "$cursor_theme" && -d "/usr/share/icons/$cursor_theme" ]]; then
    CURSOR_BLOCK="cursor {
    xcursor-theme \"$cursor_theme\"
}"
fi
put "$GREETD/allock-niri.kdl" 644 <<EOF
// niri hosting allock as the greetd greeter. Written by scripts/install.sh.
//
// greetd starts the chosen session only after the greeter process exits, and
// the process it waits on is this niri. So niri quits as soon as allock does,
// which allock does once greetd has accepted the session.

input {
    keyboard {
        xkb {
            layout "${layout:-us}"
            options "${options}"
        }
        numlock
    }
}

$CURSOR_BLOCK

hotkey-overlay {
    skip-at-startup
}

spawn-sh-at-startup "XDG_CACHE_HOME=$GREETER_CACHE $BIN --greeter -c $GREETD/allock.toml; niri msg action quit --skip-confirmation"
EOF
if ! niri validate -c "$GREETD/allock-niri.kdl" >/dev/null 2>&1; then
    echo "    warning: niri rejects $GREETD/allock-niri.kdl; run: niri validate -c $GREETD/allock-niri.kdl" >&2
fi

cat <<EOF

Done. To make it your login screen, set in $GREETD/config.toml:

    [default_session]
    command = "niri -c $GREETD/allock-niri.kdl"
    user = "greeter"

and read docs/SETUP.md about [initial_session] before rebooting.
EOF
