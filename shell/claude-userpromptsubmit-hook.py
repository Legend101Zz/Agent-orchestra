#!/usr/bin/env python3
"""pi-orchestra standalone trigger hook for Claude Code (UserPromptSubmit).

Claude Code is a closed UI: pi-orchestra cannot re-color its terminal the way it
highlights the trigger grammar inside a hosted pane (issue #9). The spec's
answer for standalone harnesses is a *hook / status acknowledgment* — so this
script runs on every submitted prompt, and when the conductor casts a spell
(``delegate:`` / ``orchestrate:`` / ``deliberate:``) it:

1. invokes ``pio quota --json`` (bounded; it delegates nothing and only
   refreshes its own ``~/.orchestra`` quota cache) and renders the reported
   level as the ``ORC WARNING`` / ``ORC BLOCKED`` / ``ORC NOTE`` line the
   skills already promise; and
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
import tempfile

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


# `pio quota` exit codes, from orc-cli's `quota_exit`: ok / warn / block / other.
_EXIT_LEVEL = {0: "ok", 2: "warn", 3: "block"}


def _pct(value: object) -> str:
    """Render a percentage the way `pio` does: 20.0 -> `20`, missing -> `?`."""
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return f"{value:g}"
    return "?"


def quota_advisory(payload: object, returncode: int) -> list[str]:
    """Render `pio quota --json` into the ORC advisory lines the skills promise.

    The *level* is authoritative — `orc_core::quota` computed it against the
    user's configured thresholds; this only renders it, mirroring the wording of
    ``quota::gate()`` so one vocabulary reaches the conductor whether the news
    arrives via `pio run` or via this hook.

    Falls back to the exit code when the payload is unusable, and never claims
    "no advisory" for a non-ok level.
    """
    level = None
    five = weekly = reason = None
    if isinstance(payload, dict):
        raw_level = payload.get("level")
        if isinstance(raw_level, str):
            level = raw_level
        five, weekly = payload.get("five_hour_pct"), payload.get("weekly_pct")
        reason = payload.get("reason")
    if level is None:
        # Unparseable output: trust the exit code rather than invent calm.
        level = _EXIT_LEVEL.get(returncode, "unknown")
        if reason is None:
            reason = f"could not parse `pio quota --json` (exit {returncode})"

    if level == "warn":
        return [
            f"ORC WARNING: MiniMax quota low — 5h window {_pct(five)}% /"
            f" weekly {_pct(weekly)}% remaining. Consider pausing delegation.",
            "Tell the user this before spending tokens.",
        ]
    if level == "block":
        return [
            f"ORC BLOCKED: MiniMax quota below block threshold (5h {_pct(five)}%,"
            f" weekly {_pct(weekly)}%).",
            "Ask the user before delegating; do NOT pass --force unless they say so.",
        ]
    if level == "ok":
        return [
            f"Quota ok — 5h window {_pct(five)}% / weekly {_pct(weekly)}% remaining."
        ]
    detail = reason if isinstance(reason, str) and reason else "no reason reported"
    return [
        f"ORC NOTE: quota unknown ({detail}) — proceeding, but the guard cannot"
        " protect this delegation."
    ]


def quota_relay(pio: str | None) -> list[str]:
    """Run a bounded `pio quota --json` and relay the advisory for its level.

    Not a mutation of anything the conductor owns: it delegates nothing and only
    refreshes pi-orchestra's own `~/.orchestra` quota cache. Never raises — a
    missing binary, a timeout, or unparseable output degrades to an honest note
    so the hook stays fast and can't wedge a prompt. Bound with
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
            [pio, "quota", "--json"],
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return ["quota check timed out; run `pio quota` yourself before delegating."]
    except OSError as err:
        return [f"could not run `pio quota` ({err}); check your pi-orchestra install."]

    try:
        payload: object = json.loads(done.stdout)
    except json.JSONDecodeError:
        payload = None
    lines = quota_advisory(payload, done.returncode)
    # Anything pio itself marked as an ORC advisory is relayed verbatim on top.
    for raw in f"{done.stdout}\n{done.stderr}".splitlines():
        text = raw.strip()
        if text.startswith(("ORC WARNING", "ORC BLOCKED", "ORC NOTE")):
            lines.append(text)
    return lines


# --- Where am I? (issue #45) --------------------------------------------------


class Seat:
    """Where this conductor is sitting, when it is sitting anywhere.

    A brain launched into a pi-orchestra pane already carries `ORC_SESSION`,
    `ORC_PANE_ID` and `ORC_WORKERS` in its environment (the daemon puts them
    there). Nothing used to read them, so the injected guidance told a seated
    brain to `pio session create` — a *second*, pane-less session whose
    dispatches went to headless workers nobody could see, while the three
    panes on screen sat idle. That is issue #45.

    The environment says *that* we are seated; the durable session record says
    *with whom*, and is the only fresh answer (`ORC_WORKERS` is frozen at
    launch and never learns that a worker died). We prefer the record and fall
    back to the environment, because a degraded seat is still worth far more
    than a wrong `session create`.
    """

    def __init__(self, session: str, pane: str | None, workers_env: str | None) -> None:
        self.session = session
        self.pane = pane
        self.workers: list[dict[str, str]] = []
        self.role: str | None = None
        self.cwd: str | None = None
        self.brain: str | None = None
        self.note: str | None = None
        for pane_id, harness in _parse_worker_offer(workers_env):
            self.workers.append(
                {"id": pane_id, "harness": harness, "state": "unknown"}
            )
        if pane and pane.endswith("-brain"):
            self.role = "brain"
        elif pane and "-worker-" in pane:
            self.role = "worker"

    def adopt_record(self, record: object) -> None:
        """Overlay the durable session record; ignore anything unusable."""
        if not isinstance(record, dict):
            return
        cwd, brain = record.get("cwd"), record.get("brain")
        self.cwd = cwd if isinstance(cwd, str) else self.cwd
        self.brain = brain if isinstance(brain, str) else self.brain
        panes = record.get("panes")
        if not isinstance(panes, list):
            return
        seated = []
        for entry in panes:
            if not isinstance(entry, dict):
                continue
            if entry.get("id") == self.pane and isinstance(entry.get("role"), str):
                self.role = entry["role"]
            if entry.get("role") != "worker":
                continue
            seated.append(
                {
                    "id": str(entry.get("id", "?")),
                    "harness": str(entry.get("harness", "?")),
                    "state": str(entry.get("state", "unknown")),
                }
            )
        # Only replace the frozen env offer once the record really answered.
        if seated or any(entry.get("role") == "worker" for entry in panes if isinstance(entry, dict)):
            self.workers = seated

    def running_workers(self) -> list[dict[str, str]]:
        return [w for w in self.workers if w["state"] in ("running", "unknown")]

    def lines(self) -> list[str]:
        """The seat, stated plainly, as the first thing the conductor reads."""
        out = [
            "YOU ARE ALREADY INSIDE A pi-orchestra SESSION. The panes on screen"
            " are the bench — dispatch into them, and the user watches it happen.",
            f"  session:  {self.session}   <- REUSE THIS. Do NOT run `pio session create`.",
            f"  your pane: {self.pane or 'unknown'}"
            f"   role={self.role or 'unknown'}"
            + (f"   harness={self.brain}" if self.brain else ""),
            f"  cwd:      {self.cwd or os.getcwd()}",
        ]
        if self.workers:
            out.append("  workers seated with you:")
            out.extend(
                f"    {w['harness']:<10} {w['state']:<9} {w['id']}" for w in self.workers
            )
        else:
            out.append(
                "  workers seated with you: none recorded — check `pio session show"
                " --json` before promising a delegation."
            )
        if self.note:
            out.append(f"  note: {self.note}")
        out.append(
            "Creating a second session is the one thing that breaks this: a new"
            " session has no panes, so dispatch falls back to a headless worker,"
            " the board of the session you are sitting in never changes, and STAGE"
            " never moves. It would still 'work' — invisibly, to a worker the user"
            " cannot see."
        )
        return out


def _parse_worker_offer(raw: str | None) -> list[tuple[str, str]]:
    """Parse `ORC_WORKERS`: `<pane-id>=<harness>` pairs, comma separated."""
    offers: list[tuple[str, str]] = []
    for chunk in (raw or "").split(","):
        pane_id, separator, harness = chunk.partition("=")
        if separator and pane_id.strip() and harness.strip():
            offers.append((pane_id.strip(), harness.strip()))
    return offers


def read_seat(pio: str | None) -> Seat | None:
    """Detect the seat from the environment, then enrich it from `pio`.

    Never raises and never blocks for long: the hook's contract is that a
    false negative costs one turn of help, never a prompt.
    """
    session = os.environ.get("ORC_SESSION", "").strip()
    if not session:
        return None
    seat = Seat(session, os.environ.get("ORC_PANE_ID") or None, os.environ.get("ORC_WORKERS"))
    if pio is None:
        seat.note = "pio not found, so this is the environment's account, not the session record."
        return seat
    try:
        timeout = float(os.environ.get("ORC_HOOK_QUOTA_TIMEOUT", "6"))
    except ValueError:
        timeout = 6.0
    try:
        done = subprocess.run(
            [pio, "session", "show", session, "--json"],
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
        seat.adopt_record(json.loads(done.stdout))
    except (subprocess.TimeoutExpired, OSError, json.JSONDecodeError):
        seat.note = (
            "could not read the session record; the seat below comes from the"
            " environment and worker states may be stale."
        )
    return seat


# --- Per-verb routing guidance (the exact pio / MCP invocation) --------------

_SINGLE_HARNESS = (
    "Single-harness honesty: if only one capable harness is installed, do NOT "
    "claim cross-harness diversity — say \"One capable harness detected. Parallel "
    "cross-harness deliberation is unavailable. Running a sequential plan with "
    "self-review.\" and proceed sequentially."
)

_CONFIRMED = (
    "Only a `confirmed` dispatch means the worker received the brief; "
    "`confirmed` + `running` is not completion. A missing executable, absent "
    "capability, or stopped pane is unavailable; report a later timeout or "
    "non-zero exit as a worker execution failure."
)


_COLLECT = (
    "Delegate returns as soon as the worker has the brief, while the worker "
    "keeps running — so it returning is NOT the answer, and its silence is not "
    "failure. COLLECT THE RESULT before you report anything: block with "
    "`orch_await` / `pio orch await <T> --session <id> --json` for the answer, "
    "usage and exit code, or poll `orch_status` / `pio orch status <T>`, and "
    "read the worker's stream with `pio show <run>`. Reporting \"no output\" "
    "without awaiting is how a six-second success gets reported as silence."
)


def _seated_delegate_block(seat: Seat) -> str:
    """`delegate:` for a conductor that is already sitting on the bench."""
    running = seat.running_workers()
    harness = running[0]["harness"] if running else "<harness>"
    offered = ", ".join(sorted({worker["harness"] for worker in running})) or "none"
    return (
        "delegate: — one bounded hand-off to a worker ALREADY SEATED WITH YOU.\n"
        f"  Seated and running: {offered}.\n"
        "  Reuse this session. Dispatch selects the running worker pane whose\n"
        "  harness matches, so you do NOT need --pane:\n"
        f'    pio orch delegate {harness} --session "$ORC_SESSION" \\\n'
        '      --title "<what>" --objective "<done-when>" \\\n'
        '      --check "<acceptance check>" --json\n'
        "  MCP equivalent: `orch_delegate` with session set to $ORC_SESSION.\n"
        "  Do NOT run `pio session create` — you are in a session. And do not\n"
        "  spawn your own subagents for this: the user cast `delegate:` to move\n"
        "  work onto the bench they are watching.\n"
        "  If the cwd is not a git repository, a contracted task cannot take a\n"
        "  worktree; either delegate from a repo, or send an uncontracted brief\n"
        "  (`pio task add` → `assign` → `start` → `pio dispatch send`), which\n"
        "  needs no isolation.\n"
        f"  {_COLLECT}\n"
        "  `--dispatch-timeout` bounds the background worker; contract "
        "`--timeout` is metadata only.\n"
        f"  {_CONFIRMED}"
    )


def _standalone_delegate_block() -> str:
    """`delegate:` outside pi-orchestra: there is no bench, so make one."""
    return (
        "delegate: — one bounded hand-off to one worker.\n"
        "  You are NOT inside a pi-orchestra session (no $ORC_SESSION), so\n"
        "  create one first — this is the standalone path only.\n"
        "  Preferred (MCP): call the `orch_delegate` tool with a task "
        "contract {harness, session, title, objective, acceptance_checks}.\n"
        "  CLI equivalent:\n"
        "    pio session create --brain claude --worker <harness>   # once; note the id\n"
        "    pio orch delegate <harness> --session <id> \\\n"
        '      --title "<what>" --objective "<done-when>" \\\n'
        '      --check "<acceptance check>" --json\n'
        "  Run it from inside a git repository: a contracted task takes an\n"
        "  isolated worktree, and outside a repo it cannot. For a task that\n"
        "  changes no files, `pio task add` → `assign` → `start` →\n"
        "  `pio dispatch send <T> <harness> \"<brief>\"` needs no worktree.\n"
        f"  {_COLLECT}\n"
        "  `--dispatch-timeout` bounds the background worker; contract "
        "`--timeout` is metadata only.\n"
        f"  {_CONFIRMED}"
    )


def guidance(verbs: list[str], seat: Seat | None = None) -> list[str]:
    """Instruction blocks for each detected verb (exact invocations)."""
    blocks: list[str] = []
    if "delegate" in verbs:
        blocks.append(
            _seated_delegate_block(seat) if seat else _standalone_delegate_block()
        )
    if "orchestrate" in verbs:
        session_ref = '"$ORC_SESSION"' if seat else "<id>"
        blocks.append(
            "orchestrate: — dependency-aware decomposition across the bench.\n"
            + (
                "  0. You are already in a session — reuse "
                f"{session_ref}. Do NOT create one, and do NOT export a new\n"
                "     ORC_SESSION: the bench you would orchestrate is the one on"
                " screen.\n"
                if seat
                else "  0. No $ORC_SESSION: create one with `pio session create`"
                " first.\n"
            )
            + "  1. Quota first (relayed above). If BLOCKED, ask the user before delegating.\n"
            "  2. Decompose into independent chunks; never exceed "
            "`max_parallel_workers` (~/.orchestra/config.json, default 3)"
            + (
                " — and never more chunks in flight than there are seated"
                " workers.\n"
                if seat
                else ".\n"
            )
            + "  3. Per chunk: `orch_plan` then `orch_delegate` (or `pio orch plan` "
            f"/ `pio orch delegate <harness> --session {session_ref} ...`).\n"
            "  4. Watch with `orch_status`/`orch_await`; move with "
            "`orch_review` then `orch_finish` (or `pio orch review|finish <T>`). "
            "Await every chunk before reporting — delivery is not completion.\n"
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


def build_context(
    verbs: list[str], quota_lines: list[str], seat: Seat | None = None
) -> str:
    """Assemble the additive context injected back into the conductor.

    When we are seated, the seat leads. Everything after it is a detail of
    *how* to delegate; where you already are decides whether any of it is
    right at all.
    """
    spells = ", ".join(f"`{verb}:`" for verb in verbs)
    header = (
        f"pi-orchestra trigger detected: {spells}. You (the conductor) are "
        "casting a spell — route this through pi-orchestra instead of doing the "
        "heavy work inline."
    )
    quota_block = "Quota (from `pio quota`):\n" + "\n".join(
        f"  {line}" for line in quota_lines
    )
    parts = [header]
    if seat is not None:
        parts.append("\n".join(seat.lines()))
    parts.append(quota_block)
    parts.extend(guidance(verbs, seat))
    return "\n\n".join(parts)


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
    seat = read_seat(pio)
    context = build_context(verbs, quota_relay(pio), seat)
    output = {
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": context,
        }
    }
    print(json.dumps(output))
    # A short human acknowledgment on stderr shows up in the transcript.
    spells = ", ".join(f"{verb}:" for verb in verbs)
    where = f" into session {seat.session}" if seat else ""
    print(
        f"pi-orchestra: {spells} detected — routing through pio{where}.",
        file=sys.stderr,
    )
    return 0


# --- Self-test: grammar parity + quota-relay coverage (drift guard) -------------------


def _quota_checks(expect) -> None:
    """Every quota level must reach the conductor — one case per level.

    The bug this guards: the relay used to grep `pio quota`'s *human* output for
    `ORC WARNING`, which that command never prints (those come from
    `quota::gate()` via `pio run`/`dispatch`), so a WARN-level quota was reported
    as "no advisory to relay". Each level is checked twice — once through the
    pure renderer, once end-to-end through a real subprocess against a stub
    `pio`, so a regression in either layer fails here.
    """
    levels = {
        "ok": ({"level": "ok", "five_hour_pct": 80.0, "weekly_pct": 90.0}, 0, "Quota ok"),
        "warn": (
            {"level": "warn", "five_hour_pct": 20.0, "weekly_pct": 90.0},
            2,
            "ORC WARNING",
        ),
        "block": (
            {"level": "block", "five_hour_pct": 5.0, "weekly_pct": 90.0},
            3,
            "ORC BLOCKED",
        ),
        "unknown": (
            {"level": "unknown", "reason": "no MiniMax key"},
            4,
            "ORC NOTE",
        ),
    }
    for name, (payload, code, marker) in levels.items():
        rendered = " ".join(quota_advisory(payload, code))
        expect(f"quota {name} renders {marker}", marker in rendered)
        expect(f"quota {name} never says 'no advisory'", "no quota advisory" not in rendered)
    # Percentages must survive, not just the marker.
    warn = " ".join(quota_advisory(levels["warn"][0], 2))
    expect("quota warn keeps the numbers", "20%" in warn and "90%" in warn)
    # Unparseable output must fall back to the exit code, never to calm.
    expect("bad json + exit 2 still warns", "ORC WARNING" in " ".join(quota_advisory(None, 2)))
    expect("bad json + exit 3 still blocks", "ORC BLOCKED" in " ".join(quota_advisory(None, 3)))
    expect("bad json + odd exit is unknown", "ORC NOTE" in " ".join(quota_advisory(None, 9)))
    expect("missing pio is honest", "not found" in " ".join(quota_relay(None)))

    # End-to-end: a stub `pio` proves the subprocess path parses --json output.
    with tempfile.TemporaryDirectory() as tmp:
        for name, (payload, code, marker) in levels.items():
            stub = os.path.join(tmp, f"pio-{name}")
            with open(stub, "w", encoding="utf-8") as handle:
                handle.write(
                    "#!/bin/sh\n"
                    f"printf '%s' '{json.dumps(payload)}'\n"
                    f"exit {code}\n"
                )
            os.chmod(stub, 0o755)
            relayed = " ".join(quota_relay(stub))
            expect(f"quota {name} end-to-end via stub pio", marker in relayed)


SEATED_RECORD = {
    "id": "bin-1785416854-0000",
    "brain": "claude",
    "cwd": "/Users/comreton/.local/bin",
    "panes": [
        {
            "id": "bin-1785416854-0000-brain",
            "harness": "claude",
            "role": "brain",
            "state": "running",
        },
        {
            "id": "bin-1785416854-0000-worker-1",
            "harness": "hermes",
            "role": "worker",
            "state": "running",
        },
        {
            "id": "bin-1785416854-0000-worker-2",
            "harness": "pi-m3",
            "role": "worker",
            "state": "stopped",
        },
    ],
}


def _stub_pio(directory: str, name: str, stdout: str, code: int = 0) -> str:
    """A fake `pio` that answers one command with fixed output."""
    path = os.path.join(directory, name)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(f"#!/bin/sh\nprintf '%s' '{stdout}'\nexit {code}\n")
    os.chmod(path, 0o755)
    return path


def _with_env(**overrides: str | None):
    """Set env vars for one block, restoring whatever was there before."""
    previous = {key: os.environ.get(key) for key in overrides}

    def restore() -> None:
        for key, value in previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value

    for key, value in overrides.items():
        if value is None:
            os.environ.pop(key, None)
        else:
            os.environ[key] = value
    return restore


def _seat_checks(expect) -> None:
    """The two paths are told apart by the environment, never by a guess.

    Issue #45: a conductor seated in a pane must reuse `$ORC_SESSION`; a
    standalone one must still create a session exactly as it does today. Both
    are pinned here so neither can quietly become the other.
    """
    restore = _with_env(ORC_SESSION=None, ORC_PANE_ID=None, ORC_WORKERS=None)
    try:
        # --- Standalone: no ORC_SESSION, so no seat and the old recipe stands.
        expect("standalone has no seat", read_seat(None) is None)
        standalone = build_context(["delegate"], ["Quota ok"], None)
        expect("standalone still creates a session", "pio session create" in standalone)
        expect("standalone says it is standalone", "NOT inside a pi-orchestra" in standalone)
        expect("standalone names how to collect", "orch await" in standalone)
        expect(
            "standalone states the git precondition",
            "git repository" in standalone,
        )

        # --- Seated: the environment alone is enough to detect the seat.
        restore()
        restore = _with_env(
            ORC_SESSION="bin-1785416854-0000",
            ORC_PANE_ID="bin-1785416854-0000-brain",
            ORC_WORKERS="bin-1785416854-0000-worker-1=hermes,"
            "bin-1785416854-0000-worker-2=pi-m3",
        )
        env_only = read_seat(None)
        expect("seated detected from env alone", env_only is not None)
        expect("env seat keeps the session id", env_only.session == "bin-1785416854-0000")
        expect("env seat derives the brain role", env_only.role == "brain")
        expect(
            "env seat parses ORC_WORKERS",
            [w["harness"] for w in env_only.workers] == ["hermes", "pi-m3"],
        )
        expect("env-only seat says so", "pio not found" in (env_only.note or ""))

        # --- Seated, enriched by the real session record through a stub pio.
        with tempfile.TemporaryDirectory() as tmp:
            stub = _stub_pio(tmp, "pio-seated", json.dumps(SEATED_RECORD))
            seat = read_seat(stub)
            expect("record seat has no degradation note", seat.note is None)
            expect("record seat learns the cwd", seat.cwd == "/Users/comreton/.local/bin")
            expect("record seat learns the role", seat.role == "brain")
            expect(
                "record seat learns worker state",
                [(w["harness"], w["state"]) for w in seat.workers]
                == [("hermes", "running"), ("pi-m3", "stopped")],
            )
            expect(
                "a stopped worker is not offered",
                [w["harness"] for w in seat.running_workers()] == ["hermes"],
            )

            context = build_context(["delegate"], ["Quota ok"], seat)
            # AC2: session, pane, role, cwd and seated workers, all named.
            for label, needle in (
                ("session", "bin-1785416854-0000"),
                ("pane", "bin-1785416854-0000-brain"),
                ("role", "role=brain"),
                ("cwd", "/Users/comreton/.local/bin"),
                ("workers", "hermes"),
            ):
                expect(f"seated context names the {label}", needle in context)
            expect("seated context says reuse", "REUSE THIS" in context)
            expect(
                "seated context forbids session create",
                "Do NOT run `pio session create`" in context,
            )
            expect(
                "seated context never teaches session create",
                "pio session create" not in context.replace(
                    "Do NOT run `pio session create`", ""
                ),
            )
            expect(
                "seated context reuses $ORC_SESSION",
                '--session "$ORC_SESSION"' in context,
            )
            expect(
                "seated context picks a seated harness",
                "pio orch delegate hermes" in context,
            )
            expect("seated context names how to collect", "orch await" in context)
            expect(
                "seated context warns STAGE would not move",
                "STAGE" in context and "never moves" in context,
            )
            # The seat leads: it must precede the routing detail.
            expect(
                "the seat is stated before the recipe",
                context.index("YOU ARE ALREADY INSIDE")
                < context.index("pio orch delegate hermes"),
            )
            # orchestrate: must not mint a second session either.
            orchestrated = build_context(["orchestrate"], ["Quota ok"], seat)
            expect(
                "seated orchestrate reuses the session",
                "do NOT export a new" in orchestrated,
            )

        # --- A pio that cannot answer degrades honestly, never silently.
        with tempfile.TemporaryDirectory() as tmp:
            broken = _stub_pio(tmp, "pio-broken", "not json at all", 1)
            degraded = read_seat(broken)
            expect("an unreadable record still yields a seat", degraded is not None)
            expect(
                "and says the seat is the environment's account",
                "environment" in (degraded.note or ""),
            )
            expect(
                "and still refuses to create a session",
                "Do NOT run `pio session create`"
                in build_context(["delegate"], ["Quota ok"], degraded),
            )

        # --- A worker pane is seated too; the role must not be assumed.
        restore()
        restore = _with_env(
            ORC_SESSION="bin-1785416854-0000",
            ORC_PANE_ID="bin-1785416854-0000-worker-1",
            ORC_WORKERS=None,
        )
        worker_seat = read_seat(None)
        expect("a worker pane reports the worker role", worker_seat.role == "worker")
    finally:
        restore()


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
    delegate_help = "\n".join(guidance(["delegate"]))
    expect(
        "delegate guidance teaches background status/await",
        "returns immediately" in delegate_help
        and "orch_status" in delegate_help
        and "orch_await" in delegate_help,
    )
    expect(
        "delegate guidance distinguishes timeout flags",
        "--dispatch-timeout" in delegate_help
        and "contract `--timeout` is metadata" in delegate_help,
    )

    _quota_checks(expect)
    _seat_checks(expect)

    failures = [name for name, ok in checks if not ok]
    for name, ok in checks:
        print(f"  {'ok  ' if ok else 'FAIL'} {name}")
    if failures:
        print(f"selftest: {len(failures)} FAILED", file=sys.stderr)
        return 1
    print(f"selftest: all {len(checks)} grammar + quota checks passed")
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
