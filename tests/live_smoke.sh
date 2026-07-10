#!/usr/bin/env bash
# Live smoke: real pi + MiniMax calls. Run manually; prints PASS/FAIL per check.
# Costs a few cents of coding-plan quota.
set -uo pipefail
export ORC_SMOKE_PATH="$PATH"
pass=0; fail=0
check() { local name="$1"; shift; if "$@"; then echo "PASS: $name"; ((pass++)); else echo "FAIL: $name"; ((fail++)); fi }

echo "--- 1: raw pi PONG"
out1=$(timeout 240 pi -p --offline --provider minimax --model MiniMax-M3 --no-session "Reply with the single word: PONG" 2>&1)
check "1 pi PONG" grep -qi "PONG" <<<"$out1"

echo "--- 2: deleg8 PONG (via orc, registered)"
out2=$(zsh -ic 'export PATH="$ORC_SMOKE_PATH"; deleg8 "Reply with the single word: PONG"' 2>&1)
check "2 deleg8 PONG" grep -qi "PONG" <<<"$out2"

echo "--- 3: model identity"
out3=$(timeout 240 pi -p --offline --provider minimax --model MiniMax-M3 --no-session "What model are you? Reply with just your model id." 2>&1)
echo "   model says: $(tail -1 <<<"$out3")"
check "3 model id mentions minimax/M3" grep -qiE "minimax|m3" <<<"$out3"

echo "--- 4: real agentic task via deleg8 (file listing)"
out4=$(zsh -ic 'export PATH="$ORC_SMOKE_PATH"; deleg8 "List every file in the current directory recursively (excluding .venv and .git), grouped by extension, with counts. Output as markdown." "'"$PWD"'"' 2>&1)
check "4 recursive listing mentions py" grep -qi "py" <<<"$out4"

echo "--- 5: skills + codex block in place"
check "5a skill pi-delegate" test -f "$HOME/.claude/skills/pi-delegate/SKILL.md"
check "5b skill orchestrate" test -f "$HOME/.claude/skills/orchestrate/SKILL.md"
check "5c codex block" grep -qF "pi-orchestra:begin" "$HOME/.codex/AGENTS.md"

echo "--- 6: quota endpoint"
orc quota; qc=$?
check "6 orc quota exit 0/2" test "$qc" -eq 0 -o "$qc" -eq 2

echo "--- 7: background run + kill"
rid=$(orc run "Think very carefully and at extreme length about the philosophy of orchestration, then write a 10000 word essay" --bg)
sleep 3
orc kill "$rid" >/dev/null
sleep 1
st=$(orc show "$rid" 2>/dev/null | python3 -c 'import json,sys; print(json.loads(sys.stdin.read().split("--- output.log")[0])["status"])')
check "7 bg run killed (status=$st)" test "$st" = "killed"

echo "--- 8: registry populated"
check "8 registry has >=3 runs" test "$(orc list --json | python3 -c 'import json,sys;print(len(json.loads(sys.stdin.read())))')" -ge 3

echo; echo "== $pass passed, $fail failed =="
exit "$fail"
