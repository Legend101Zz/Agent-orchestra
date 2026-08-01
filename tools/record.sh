#!/usr/bin/env bash
# Record the README media with VHS, against a scratch ORC_HOME.
#
# Written for issue #14. The pre-existing tapes under tools/ are all v4-era:
# they point at /tmp/pi-orchestra-phase6 and a theme set that no longer matches
# the code, so they cannot be re-run to produce current media.
#
# Everything happens in a throwaway ORC_HOME and a throwaway working directory,
# so a recording can never touch your real ~/.orchestra or show your own repos.
#
# Usage:
#   tools/record.sh              # record every tape
#   tools/record.sh hero-nocturne
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/rust/target/release"
STAGE_HOME="${ORC_RECORD_HOME:-/tmp/pio-record-home}"
STAGE_CWD="${ORC_RECORD_CWD:-/tmp/pio-record-demo}"

command -v vhs >/dev/null || { echo "record.sh: vhs is not installed (brew install vhs)" >&2; exit 1; }
for b in pi-orchestra piod pio; do
  [ -x "$BIN/$b" ] || { echo "record.sh: missing $BIN/$b — run: (cd rust && cargo build --release --locked)" >&2; exit 1; }
done

COMMIT="$(git -C "$ROOT" rev-parse --short HEAD)"
echo "==> recording at commit $COMMIT"
"$BIN/pio" version

SHIM="/tmp/pio-record-shim"

reset_state() {
  # A fresh daemon per tape: piod persists across runs and would otherwise
  # serve panes from a previous recording. Matched on the scratch socket so
  # this can never touch a daemon serving the real ~/.orchestra.
  pkill -f "piod --home $STAGE_HOME" 2>/dev/null || true
  sleep 1
  rm -rf "$STAGE_HOME" "$STAGE_CWD"
  mkdir -p "$STAGE_HOME" "$STAGE_CWD"
  chmod 700 "$STAGE_HOME"

  # The client spawns its daemon by the pre-#17 name `orcd`: it looks for a
  # sibling called `orcd` and otherwise falls back to `orcd` on PATH
  # (orc-app/src/main.rs:66-72). A build tree contains `piod`, not `orcd`, so
  # without this shim a locally built client silently starts the *installed*
  # daemon and then refuses on the build handshake. Filed separately; shimmed
  # here so a recording captures the build it says it captures.
  mkdir -p "$SHIM"
  ln -sf "$BIN/piod" "$SHIM/orcd"
  # A demo working directory with something in it, so the panes are not
  # sitting in an empty folder.
  printf '# demo\n\nA scratch repository for the pi-orchestra README recording.\n' \
    > "$STAGE_CWD/README.md"
  printf 'def add(a, b):\n    return a + b\n' > "$STAGE_CWD/app.py"
  git -C "$STAGE_CWD" init -q 2>/dev/null || true
}

record() {
  local tape="$1"
  echo "==> $tape"
  reset_state
  mkdir -p "$ROOT/docs/media"
  # VHS 0.11 cannot parse absolute paths in Output/Screenshot, so the tapes use
  # repo-relative paths and vhs runs from the repo root.
  ( cd "$ROOT" && PIO_BIN="$BIN" vhs "tools/$tape.tape" )
}

if [ $# -gt 0 ]; then
  for t in "$@"; do record "$t"; done
else
  record hero-nocturne
  record stage-phosphor
fi

pkill -f "piod --socket $STAGE_HOME/orcd.sock" 2>/dev/null || true
echo
echo "==> done. Recorded at $COMMIT. Output in docs/media/."
