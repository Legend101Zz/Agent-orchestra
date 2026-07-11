#!/bin/sh
# Named-terminal evidence helper; uses only isolated temporary state.
ORC_HOME=/tmp/orc-ghostty-phase2
export ORC_HOME
exec /Users/comreton/Desktop/pi-orchestra/rust/target/release/pi-orchestra --theme ember home
