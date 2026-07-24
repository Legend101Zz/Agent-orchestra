#!/bin/sh
# Fixture worker — SUBCOMMAND-PROMPT invocation style (issue #6, AC1).
#
# Emulates codex/opencode headless forms where the FIRST argument is a
# subcommand and the brief is the final positional argument:
#     codex exec [--json] [-C <dir>] "<brief>"
#     opencode run [--format json] [--dir <dir>] "<brief>"
#
# It receives the brief and returns a confirmed receipt (exit 0) naming the
# subcommand, the argv it saw, the brief, and the working directory — so a
# dispatch test can prove the adapter chose the subcommand style and delivered
# the prompt into the right cwd.
sub="$1"
brief=""
for arg in "$@"; do
	brief="$arg"
done
echo "subcommand-style receipt sub: ${sub}"
echo "subcommand-style receipt argv: $*"
echo "subcommand-style receipt brief: ${brief}"
echo "subcommand-style receipt cwd: $(pwd)"
exit 0
