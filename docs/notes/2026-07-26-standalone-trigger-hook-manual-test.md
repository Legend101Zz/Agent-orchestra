# Manual test — standalone `delegate:` trigger via the Claude Code hook

Date: 2026-07-26 · Issue: #10 (V1-8 standalone integrations v2) · Actor: code-puppy

This is the documented manual test for acceptance check #3: *a Claude Code
session with the hook, given a `delegate:` prompt, invokes `pio`.* It has two
parts — a **reproducible non-interactive simulation** (run it anywhere, exact
output below) and the **live Claude Code procedure** (what to do in a real
session, since an interactive TUI can't run in CI).

The hook is `shell/claude-userpromptsubmit-hook.py`. `install.sh` links it to
`~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py` and prints the
registration snippet; it never edits `~/.claude/settings.json` (a protected
file), so you opt in yourself.

## Prerequisites

- `./install.sh` has been run (links `pio`, `pio-mcp`, the skills, and the hook).
- `pio` is on `PATH` (or export `PIO_BIN=/path/to/pio`).
- `python3` is available (macOS/Linux dev default).

## Register the hook (one time, in your own settings)

Add to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "UserPromptSubmit": [
      { "hooks": [ { "type": "command",
          "command": "~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py" } ] }
    ]
  }
}
```

For a **project-scoped** test you can instead drop the same block in
`<project>/.claude/settings.json` — neither is a pi-orchestra-owned file, so you
stay in control.

## Part A — reproducible simulation (no live session needed)

The hook reads the same JSON on stdin that Claude Code's UserPromptSubmit event
delivers. Feed it a `delegate:` prompt and watch it invoke `pio` and inject
routing context:

```bash
python3 -c 'import json; print(json.dumps({
  "session_id":"demo","hook_event_name":"UserPromptSubmit",
  "prompt":"delegate: find every TODO in this repo with file:line"}))' \
| ~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py
```

Observed result (stdout is the JSON context Claude ingests; stderr is the
transcript acknowledgment; exit is always 0 so the prompt is never lost):

```
# stderr:
pi-orchestra: delegate: detected — routing through pio.

# stdout (additionalContext, unescaped for readability):
pi-orchestra trigger detected: `delegate:`. You (the conductor) are casting a
spell — route this through pi-orchestra instead of doing the heavy work inline.

Quota (relayed verbatim):
  MiniMax quota: unknown — no MiniMax key in Keychain or auth.json

delegate: — one bounded hand-off to one worker.
  Preferred (MCP): call the `orch_delegate` tool with a task contract
  {harness, session, title, objective, acceptance_checks}.
  CLI equivalent:
    pio session create --brain claude --worker <harness>   # once; note the id
    pio orch delegate <harness> --session <id> \
      --title "<what>" --objective "<done-when>" \
      --check "<acceptance check>" --json
  Observe with `orch_status` / `pio orch status <T>`; wait with `orch_await`...
  Only a `confirmed` dispatch means the worker received the brief...

Single-harness honesty: ... "One capable harness detected. Parallel
cross-harness deliberation is unavailable. Running a sequential plan with
self-review." ...

Register the MCP tools once with `pio mcp print-config --format claude` ...
```

The line `MiniMax quota: unknown — …` is the proof that the hook **invoked
`pio`** (it ran `pio quota` and relayed its output verbatim). On a machine with
a MiniMax key this is the real quota; if quota were BLOCKED (exit 3) the hook
appends an explicit "ask the user before delegating; do NOT pass --force" note.

Control cases (no trigger → the hook stays completely silent, exit 0):

```bash
# ordinary prose — no colon, or the keyword welded into another word:
printf '{"prompt":"please delegate this task"}'   | ~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py ; echo "exit=$?"
printf '{"prompt":"redelegate: not a trigger"}'   | ~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py ; echo "exit=$?"
# → no stdout, exit=0 for both
```

Grammar parity with the in-pane source of truth (`orc_pty::trigger`) is guarded
by a self-test:

```bash
~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py --selftest
# → selftest: all 22 grammar checks passed
```

## Part B — live Claude Code procedure

1. Register the hook (above) and start `claude` in any repo.
2. Type a prompt beginning with the spell, e.g.
   `delegate: summarize the architecture in src/ and list the entry points`.
3. **Expected:** before Claude answers, the hook runs. You'll see the
   acknowledgment `pi-orchestra: delegate: detected — routing through pio.` in
   the transcript, and Claude receives the injected context — so instead of
   reading `src/` itself it sets up the delegation:
   `pio session create --brain claude --worker <harness>` then
   `pio orch delegate <harness> --session <id> --title … --objective … --check …`
   (or the `orch_delegate` MCP tool if you registered it with
   `pio mcp print-config --format claude`).
4. **Quota relay:** any `ORC WARNING` / `ORC BLOCKED` line from `pio quota` is
   surfaced verbatim; on a block, Claude asks you before spending tokens.
5. **`orchestrate:`** routes the same way into the multi-worker plan
   (`orch_plan`→`orch_delegate`→`orch_status`/`orch_await`→`orch_review`→
   `orch_finish`); **`deliberate:`** honestly reports that the judged panel is a
   V2 feature and offers a delegate / sequential-self-review fallback.

## Pass criteria

- [x] A `delegate:` prompt causes the hook to invoke `pio` (the relayed
  `MiniMax quota:` line proves it) and inject the exact CLI/MCP invocation.
- [x] Non-triggers (`please delegate this`, `redelegate:`, `Delegate:`) produce
  no output and never block the prompt.
- [x] `--selftest` passes (grammar parity with `orc_pty::trigger`).
- [x] Registering the hook touches only your own `settings.json`; `install.sh`
  leaves all protected-config checksums unchanged (see the install AC1 evidence).
