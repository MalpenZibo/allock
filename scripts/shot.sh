#!/usr/bin/env bash
# One scene of the lock or the greeter drawn without a compositor (nothing
# is locked, no window opens) and written to a PNG.
#   scripts/shot.sh --list
#   scripts/shot.sh <scene> <out.png> [key=value]...
# key=value are the keys of the [appearance] table (mode, density, shape,
# material, motion, theme, ...). The clock, the machine and the accounts are
# fixed, so the same arguments give the same pixels. e.g.
#   scripts/shot.sh lock/card /tmp/card.png mode=light material=glass
# Needs a GPU adapter; the first run builds in release.
set -euo pipefail
export ALLOCK_SHOT_CWD="$PWD"
ALLOCK_SHOT="$(printf '%s\n' "$@")"
export ALLOCK_SHOT
cd "$(dirname "$0")/.."
cargo test --release --quiet --bin allock -- --ignored --exact ui::snapshots::shot --nocapture 2>&1 \
    | grep -vE '^(running 1 test|test ui::snapshots::shot .*|test result: .*|\.)?$'
