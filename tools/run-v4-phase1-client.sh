#!/usr/bin/env bash
set -euo pipefail
cd /Users/comreton/Desktop/pi-orchestra
exec env -u NO_COLOR rust/target/release/pi-orchestra \
  --socket /tmp/orcd-real-spike.sock \
  --theme "${1:-ember}"
