#!/bin/sh
# Fixture worker — FLAG-PROMPT invocation style (issue #6, AC1).
#
# Emulates claude/hermes/pi headless forms where the brief is the final
# positional argument, after one or more leading flags:
#     claude -p [--output-format stream-json --verbose] "<brief>"
#     hermes -z "<brief>"
#     pi -p --no-session [--mode json] "<brief>"
#
# It receives the brief and returns a confirmed receipt (exit 0) naming the
# argv it saw, the brief, and the working directory pi-orchestra spawned it in —
# so a dispatch test can prove the adapter delivered the prompt and set the cwd.
brief=""
for arg in "$@"; do
	brief="$arg"
done
echo "flag-style receipt argv: $*"
echo "flag-style receipt brief: ${brief}"
echo "flag-style receipt cwd: $(pwd)"
exit 0
