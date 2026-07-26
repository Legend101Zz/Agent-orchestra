#!/usr/bin/env python3
"""pi-orchestra standalone trigger hook for Claude Code (UserPromptSubmit).

Claude Code is a closed UI: pi-orchestra cannot re-color its terminal the way it
highlights the trigger grammar inside a hosted pane (issue #9). The spec's
answer for standalone harnesses is a *hook / status acknowledgment* — so this
script runs on every submitted prompt, and when the conductor casts a spell
(``delegate:`` / ``orchestrate:`` / ``deliberate:``) it:

1. invokes ``pio`` for a bounded, read-only quota check and relays any
   ``ORC WARNING`` / ``ORC BLOCKED`` / ``ORC NOTE`` line verbatim (the quota
   guarantee the skills already promise); and
2. injects context that tells the conductor the *exact* ``pio`` CLI / MCP
   invocation for the detected verb, plus the single-harness honesty rule.

It never edits the prompt away and never blocks: it always exits 0 with additive
context, so a false negative just means "no help this turn", never a lost
prompt.

## Registration (manual, opt-in — never written by install.sh)

install.sh links this script to a stable path and prints the snippet; add it to
your *own* ``~/.claude/settings.json`` (a protected file pi-orchestra never
touches):

    {
      "hooks": {
        "UserPromptSubmit": [
          { "hooks": [ { "type": "command",
              "command": "~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py" } ] }
        ]
      }
    }

## Grammar source of truth

The detection here mirrors ``orc_pty::trigger`` (rust/crates/orc-pty/src/trigger.rs):
a keyword at a word boundary (line start or after a non-alphanumeric char)
immediately followed by ``:``; case-sensitive; fires mid-line and more than
once; ``redelegate:``/``delegated:``/``Delegate:``/colon-less ``delegate`` stay
quiet. That module is the authority; ``--selftest`` here guards against drift
(the Rust crate cannot be imported from a hook). Run it with:

    ./claude-userpromptsubmit-hook.py --selftest
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys

# Keywords in the same stable order as orc_pty::trigger::Trigger::ALL.
TRIGGERS: tuple[str, ...] = ("delegate", "orchestrate", "deliberate")


def scan_line(line: str) -> list[tuple[str, int]]:
    """Return ``(keyword, char_start)`` for every spell on one line, L-to-R.

    A faithful port of ``orc_pty::trigger::scan_line``: word-boundary anchored,
    colon-required, case-sensitive, non-overlapping, mid-line, repeatable.
    """
    matches: list[tuple[str, int]] = []
    index = 0
    length = len(line)
    while index < length:
        at_boundary = index == 0 or not line[index - 1].isalnum()
        if at_boundary:
            matched = _match_keyword_at(line, index)
            if matched is not None:
                keyword = matched
                matches.append((keyword, index))
                index += len(keyword) + 1  # skip keyword and its colon
                continue
        index += 1
    return matches


def _match_keyword_at(line: str, index: int) -> str | None:
    """Match a bare keyword + ``:`` starting exactly at ``index``."""
    for keyword in TRIGGERS:
        end = index + len(keyword)
        if line[index:end] == keyword and line[end : end + 1] == ":":
            return keyword
    return None


def detect(prompt: str) -> list[str]:
    """Unique triggers across the whole prompt, in first-seen order."""
    seen: list[str] = []
    for line in prompt.splitlines():
        for keyword, _ in scan_line(line):
            if keyword not in seen:
                seen.append(keyword)
    return seen


def find_pio() -> str | None:
    """Locate the ``pio`` binary: ``$PIO_BIN`` → PATH → ``~/.local/bin/pio``."""
    override = os.environ.get("PIO_BIN")
    if override and os.access(override, os.X_OK):
        return override
    found = shutil.which("pio")
    if found:
        return found
    fallback = os.path.expanduser("~/.local/bin/pio")
    return fallback if os.access(fallback, os.X_OK) else None


def quota_relay(pio: str | None) -> list[str]:
    """Run a bounded, read-only ``pio quota`` and relay ORC lines verbatim.

    Never raises: a missing binary, a timeout, or a non-zero exit degrades to an
    honest note so the hook stays fast and non-blocking. Bound with
    ``ORC_HOOK_QUOTA_TIMEOUT`` seconds (default 6).
    """
    if pio is None:
        return [
            "pio was not found on PATH — install pi-orchestra (install.sh) or set"
            " $PIO_BIN. Delegation is unavailable until then."
        ]
    try:
        timeout = float(os.environ.get("ORC_HOOK_QUOTA_TIMEOUT", "6"))
    except ValueError:
        timeout = 6.0
    try:
        done = subprocess.run(
            [pio, "quota"],
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return ["quota check timed out; run `pio quota` yourself before delegating."]
    except OSError as err:
        return [f"could not run `pio quota` ({err}); check your pi-orchestra install."]

    lines: list[str] = []
    combined = f"{done.stdout}\n{done.stderr}"
    for raw in combined.splitlines():
        text = raw.strip()
        if text.startswith(("ORC WARNING", "ORC BLOCKED", "ORC NOTE", "MiniMax quota")):
            lines.append(text)
    if done.returncode == 3:
        lines.append(
            "Quota is BLOCKED (pio quota exit 3). Ask the user before delegating;"
            " do NOT pass --force unless they say so."
        )
    if not lines:
        lines.append(f"pio quota ran (exit {done.returncode}); no quota advisory to relay.")
    return lines


# --- Per-verb routing guidance (the exact pio / MCP invocation) --------------

_SINGLE_HARNESS = (
    "Single-harness honesty: if only one capable harness is installed, do NOT "
    "claim cross-harness diversity — say \"One capable harness detected. Parallel "
    "cross-harness deliberation is unavailable. Running a sequential plan with "
    "self-review.\" and proceed sequentially."
)

_CONFIRMED = (
    "Only a `confirmed` dispatch means the worker received the brief; a missing "
    "executable, absent capability, stopped pane, timeout, or non-zero exit is "
    "unavailable/failed and must be reported as such."
)


def guidance(verbs: list[str]) -> list[str]:
    """Instruction blocks for each detected verb (exact invocations)."""
    blocks: list[str] = []
    if "delegate" in verbs:
        blocks.append(
            "delegate: — one bounded hand-off to one worker.\n"
            "  Preferred (MCP): call the `orch_delegate` tool with a task "
            "contract {harness, session, title, objective, acceptance_checks}.\n"
            "  CLI equivalent:\n"
            "    pio session create --brain claude --worker <harness>   # once; note the id\n"
            "    pio orch delegate <harness> --session <id> \\\n"
            "      --title \"<what>\" --objective \"<done-when>\" \\\n"
            "      --check \"<acceptance check>\" --json\n"
            "  Observe with `orch_status` / `pio orch status <T>`; wait with "
            "`orch_await` / `pio orch await <T>`.\n"
            f"  {_CONFIRMED}"
        )
    if "orchestrate" in verbs:
        blocks.append(
            "orchestrate: — dependency-aware decomposition across the bench.\n"
            "  1. Quota first (relayed above). If BLOCKED, ask the user before delegating.\n"
            "  2. Decompose into independent chunks; never exceed "
            "`max_parallel_workers` (~/.orchestra/config.json, default 3).\n"
            "  3. Per chunk: `orch_plan` then `orch_delegate` (or `pio orch plan` "
            "/ `pio orch delegate <harness> --session <id> ...`).\n"
            "  4. Watch with `orch_status`/`orch_await`; move with "
            "`orch_review` then `orch_finish` (or `pio orch review|finish <T>`).\n"
            "  5. Verify each worker's output against real files, synthesize "
            "yourself, and report per-worker status + `tokens.total`/`cost_usd` "
            "+ `pio stats` + post-run `pio quota`."
        )
    if "deliberate" in verbs:
        blocks.append(
            "deliberate: — a parallel panel / MoA is a V2 mode and is NOT "
            "available in V1. Do not fake a panel or invent judges.\n"
            "  Say so honestly, then offer a real fallback: a `delegate:`-style "
            "single hand-off, or a sequential self-review plan (plan → implement "
            "→ independent review) via the `orch_*` tools. "
            "Ask which the user prefers."
        )
    blocks.append(_SINGLE_HARNESS)
    blocks.append(
        "Register the MCP tools once with `pio mcp print-config --format claude` "
        "(prints a `.mcp.json` snippet; it never edits protected config). Without "
        "them, the `pio orch ...` CLI verbs are the equivalent surface."
    )
    return blocks


def build_context(verbs: list[str], quota_lines: list[str]) -> str:
    """Assemble the additive context injected back into the conductor."""
    spells = ", ".join(f"`{verb}:`" for verb in verbs)
    header = (
        f"pi-orchestra trigger detected: {spells}. You (the conductor) are "
        "casting a spell — route this through pi-orchestra instead of doing the "
        "heavy work inline."
    )
    quota_block = "Quota (relayed verbatim):\n" + "\n".join(
        f"  {line}" for line in quota_lines
    )
    return "\n\n".join([header, quota_block, *guidance(verbs)])


def run_hook(stdin_text: str) -> int:
    """Process one UserPromptSubmit payload. Always returns 0 (never blocks)."""
    try:
        payload = json.loads(stdin_text) if stdin_text.strip() else {}
    except json.JSONDecodeError:
        # Malformed payload: stay silent rather than corrupt the turn.
        return 0
    prompt = payload.get("prompt", "")
    if not isinstance(prompt, str):
        return 0
    verbs = detect(prompt)
    if not verbs:
        return 0  # ordinary prose passes through untouched
    pio = find_pio()
    context = build_context(verbs, quota_relay(pio))
    output = {
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": context,
        }
    }
    print(json.dumps(output))
    # A short human acknowledgment on stderr shows up in the transcript.
    spells = ", ".join(f"{verb}:" for verb in verbs)
    print(f"pi-orchestra: {spells} detected — routing through pio.", file=sys.stderr)
    return 0


# --- Self-test: parity with orc_pty::trigger (drift guard) -------------------


def _selftest() -> int:
    checks: list[tuple[str, bool]] = []

    def expect(name: str, condition: bool) -> None:
        checks.append((name, condition))

    for keyword in TRIGGERS:
        expect(f"{keyword} fires anchored", detect(f"{keyword}: do it") == [keyword])
    expect("mid-line fires", detect("first orchestrate: later") == ["orchestrate"])
    expect("indented fires", scan_line("    delegate: x")[0] == ("delegate", 4))
    expect(
        "multiple on a line",
        [k for k, _ in scan_line("delegate: a , delegate: b and orchestrate: c")]
        == ["delegate", "delegate", "orchestrate"],
    )
    for prompt in ("\u276f ", "> ", "$ ", "% ", "# ", "\u279c ", ">>> "):
        expect(f"prompt {prompt!r} fires", detect(f"{prompt}delegate: go") == ["delegate"])
    expect("redelegate quiet", detect("redelegate: nope") == [])
    expect("predelegate quiet", detect("predelegate: nope") == [])
    expect("suffix quiet", detect("delegated: past") == [])
    expect("typo quiet", detect("delegatex: nope") == [])
    expect("no-colon quiet", detect("please delegate this") == [])
    expect("case-sensitive quiet", detect("Delegate: cap") == [])
    expect("shout quiet", detect("ORCHESTRATE: loud") == [])
    expect("blank quiet", detect("") == [] and detect("   ") == [])
    expect("prompt+redelegate quiet", detect("\u276f redelegate: nope") == [])

    failures = [name for name, ok in checks if not ok]
    for name, ok in checks:
        print(f"  {'ok  ' if ok else 'FAIL'} {name}")
    if failures:
        print(f"selftest: {len(failures)} FAILED", file=sys.stderr)
        return 1
    print(f"selftest: all {len(checks)} grammar checks passed")
    return 0


def main(argv: list[str]) -> int:
    if "--selftest" in argv:
        return _selftest()
    if "-h" in argv or "--help" in argv:
        print(__doc__)
        return 0
    return run_hook(sys.stdin.read())


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
