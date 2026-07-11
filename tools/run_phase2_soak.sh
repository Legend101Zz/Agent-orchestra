#!/usr/bin/env bash
# Reproducible Phase-2 four-pane flooding soak. Production sign-off requires >=7200s.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DURATION="${1:-7200}"
OUT="${2:-$ROOT/docs/notes/phase2-soak-data}"
if [ "$DURATION" -lt 7200 ] && [ "${ORC_SOAK_TEST:-0}" != 1 ]; then
  echo "refusing to label ${DURATION}s as the required hours-long soak" >&2
  exit 2
fi

mkdir -p "$OUT"
HOME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/orcd-soak.XXXXXX")"
SOCKET="$HOME_DIR/orcd.sock"
ORCD="$ROOT/rust/target/release/orcd"
APP="$ROOT/rust/target/release/pi-orchestra"

cleanup() {
  if [ -n "${WATCH_PID:-}" ]; then kill "$WATCH_PID" 2>/dev/null || true; fi
  if [ -n "${SAMPLE_PID:-}" ]; then kill "$SAMPLE_PID" 2>/dev/null || true; fi
  if [ -n "${DAEMON_PID:-}" ]; then kill "$DAEMON_PID" 2>/dev/null || true; fi
}
trap cleanup EXIT INT TERM

cargo build --manifest-path "$ROOT/rust/Cargo.toml" --release --locked -q
"$ORCD" --home "$HOME_DIR" --socket "$SOCKET" \
  --pane "sh -c 'while :; do i=0; while [ \$i -lt 1024 ]; do echo flood-one; i=\$((i+1)); done; sleep 0.05; done'" \
  --pane "sh -c 'while :; do i=0; while [ \$i -lt 1024 ]; do echo flood-two; i=\$((i+1)); done; sleep 0.05; done'" \
  --pane "sh -c 'while :; do i=0; while [ \$i -lt 1024 ]; do echo flood-three; i=\$((i+1)); done; sleep 0.05; done'" \
  --pane "sh -c 'while :; do i=0; while [ \$i -lt 1024 ]; do echo flood-four; i=\$((i+1)); done; sleep 0.05; done'" &
DAEMON_PID=$!

for _ in $(seq 1 200); do
  [ -S "$SOCKET" ] && break
  sleep 0.05
done
[ -S "$SOCKET" ]

"$APP" --socket "$SOCKET" --snapshot-once > "$OUT/start-snapshot.json"
"$APP" --socket "$SOCKET" --metrics > "$OUT/start-metrics.json"
printf 'elapsed_sec,cpu_pct,rss_kib\n' > "$OUT/samples.csv"
START_EPOCH=$(date +%s)
(
  while kill -0 "$DAEMON_PID" 2>/dev/null; do
    NOW=$(date +%s)
    PS_ROW=$(ps -o %cpu=,rss= -p "$DAEMON_PID" | awk '{$1=$1; print}')
    [ -n "$PS_ROW" ] || break
    printf '%s,%s,%s\n' "$((NOW - START_EPOCH))" ${PS_ROW} >> "$OUT/samples.csv"
    sleep 10
  done
) &
SAMPLE_PID=$!
(
  while kill -0 "$DAEMON_PID" 2>/dev/null; do
    "$APP" --socket "$SOCKET" --snapshot-once >/dev/null 2>&1 || break
    sleep 0.1
  done
) &
WATCH_PID=$!

DEADLINE=$((START_EPOCH + DURATION))
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  kill -0 "$DAEMON_PID"
  sleep 30
done

"$APP" --socket "$SOCKET" --snapshot-once > "$OUT/end-snapshot.json"
"$APP" --socket "$SOCKET" --metrics > "$OUT/end-metrics.json"
END_EPOCH=$(date +%s)
kill "$WATCH_PID" "$SAMPLE_PID" 2>/dev/null || true
wait "$WATCH_PID" 2>/dev/null || true
wait "$SAMPLE_PID" 2>/dev/null || true
unset WATCH_PID SAMPLE_PID
kill "$DAEMON_PID"
wait "$DAEMON_PID" 2>/dev/null || true
unset DAEMON_PID

# A clean restart executes the exact identity-validated reap invariant.
"$ORCD" --home "$HOME_DIR" --socket "$SOCKET" &
REAPER_PID=$!
for _ in $(seq 1 200); do
  [ -S "$SOCKET" ] && break
  sleep 0.05
done
kill "$REAPER_PID"
wait "$REAPER_PID" 2>/dev/null || true

CPU_PEAK=$(awk -F, 'NR>1 && $2>m {m=$2} END {print m+0}' "$OUT/samples.csv")
RSS_PEAK=$(awk -F, 'NR>1 && $3>m {m=$3} END {print m+0}' "$OUT/samples.csv")
printf '%s\n' "{\"requested_seconds\":$DURATION,\"actual_seconds\":$((END_EPOCH - START_EPOCH)),\"producer\":\"four line floods: 1024 lines then 50ms pause\",\"sample_interval_seconds\":10,\"daemon_cpu_peak_pct\":$CPU_PEAK,\"daemon_rss_peak_kib\":$RSS_PEAK,\"result\":\"completed\"}" > "$OUT/summary.json"
rm -rf "$HOME_DIR"
trap - EXIT INT TERM
