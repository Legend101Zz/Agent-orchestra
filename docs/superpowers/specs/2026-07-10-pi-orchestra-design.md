# pi-orchestra — MiniMax M3 worker delegation + control plane

**Date:** 2026-07-10
**Status:** Draft — awaiting user review
**Project home:** `~/Desktop/pi-orchestra` (git repo; installs via symlinks so everything stays versioned)

## Problem

The user runs Claude Code (Anthropic subscription) and Codex (OpenAI subscription) as
interactive "brains." Heavy, token-expensive, long-context work should be offloaded to
**pi** (`@earendil-works/pi-coding-agent`) running **MiniMax-M3** (1M context,
$0.30/$1.20 per 1M tokens, Anthropic-compatible API, on a coding-plan quota with 5-hour
rolling + weekly windows).

Requirements, in the user's words:

1. Brains delegate heavy work to pi/M3 (one-shot and streaming modes).
2. Simple tasks stay in the brain — **orchestration only activates on an explicit
   keyword** ("orchestrate").
3. A **control plane / visual dashboard** shows all delegated sessions and lets the user
   control them (view output, kill).
4. The brain **tracks remaining MiniMax quota**, warns the user when low, and can decide
   not to delegate.
5. **Must not break** existing harness configs: plain `claude` stays on the Anthropic
   subscription, plain `codex` stays on OpenAI, pi's own config stays untouched.

## Current state (verified 2026-07-10)

- pi 0.80.3 at `~/.local/bin/pi`; `~/.pi/agent/auth.json` has a `minimax` `api_key`
  entry; `settings.json` defaults: provider `minimax`, model `MiniMax-M3`.
- pi CLI supports `-p`, `--mode rpc|json`, `--no-session`, `--provider`, `--model`.
- `~/.zshrc` already has `pix() { pi --provider minimax --model MiniMax-M3 "$@"; }`.
- MiniMax key also lives in macOS Keychain as `minimax_api_key`.
- Codex has a `[profiles.minimax]` in `~/.codex/config.toml` and an existing
  `~/.codex/AGENTS.md`.
- `~/.claude/skills/` is empty.
- MiniMax exposes a quota endpoint: `GET https://api.minimax.io/v1/token_plan/remains`
  (Bearer key; requires a coding-plan subscription key). Returns remaining quota for the
  5-hour and weekly windows.

## Approaches considered

**1. Thin file-based substrate + custom mini dashboard (chosen).** A single `orc` CLI
wraps pi, registers every delegated run in a plain-JSON registry under `~/.orchestra/`,
checks quota before spawning, and serves a small localhost dashboard reading that
registry. Everything is additive; no harness config is edited. Inspectable with `cat`.

**2. Adopt an existing control plane** (Vibe Kanban, Conductor, Claude Squad). Polished
UIs, but they manage *parallel peer sessions* — none understand the brain→cheap-worker
delegation pattern, pi, or MiniMax quota. Vibe Kanban's company shut down (community
maintained). Could complement later; doesn't replace the substrate.

**3. MCP mailbox server (dev.to CC2CC pattern, full duplex).** Push instead of poll,
richer two-way comms — but requires registering MCP servers in each harness config
(exactly what the user wants untouched), uses experimental MCP channels, and adds a
daemon. The registry format below is forward-compatible with this upgrade.

Chosen: **Approach 1**, borrowing Approach 3's mechanics (plain-JSON messages, per-run
inbox directories, atomic temp-file+rename writes).

## Architecture

```
Claude Code (brain, subscription)          Codex (brain, subscription)
   │  skill: pi-delegate (auto, heavy tasks)   │  AGENTS.md sections (same content)
   │  skill: orchestrate (keyword-gated)       │
   └───────────────┬───────────────────────────┘
                   ▼  bash
              orc CLI  (Python 3 stdlib, single file)
                   │  quota check → spawn pi → tee output → update registry
                   ▼
   pi -p / --mode rpc  --provider minimax --model MiniMax-M3 --no-session
                   │
                   ▼
        ~/.orchestra/   (plain-JSON registry, atomic writes)
        ├── runs/<run-id>/meta.json      status, pid, task, brain, timings, tokens
        ├── runs/<run-id>/output.log     worker stdout/stderr
        ├── runs/<run-id>/inbox/*.json   control messages (kill, follow-up)
        ├── quota.json                   last /remains snapshot (60s cache)
        └── config.json                  thresholds, port
                   ▲
        orc ui  →  localhost dashboard (Phase 2): live run table, log tail,
                   kill buttons, quota bars (5h + weekly)
```

## Components

### 1. `orc` CLI (`pi-orchestra/bin/orc`, symlinked into `~/.local/bin/orc`)

Python 3, stdlib only (argparse, json, subprocess, urllib, http.server). Subcommands:

- `orc run "task" [--cwd DIR] [--name N] [--brain claude|codex|human] [--bg] [--force]`
  — create `runs/<UTC-timestamp>-<slug>/`, write `meta.json` (status `running`), check
  quota (cached), spawn
  `pi -p --provider minimax --model MiniMax-M3 --no-session "task"` in `--cwd`, tee
  stdout+stderr to `output.log` **and** to the caller's stdout, then set status
  `done`/`failed` with exit code and timings. Foreground by default so the calling brain
  reads the result inline; `--bg` detaches and prints the run id.
- `orc rpc "task" [--cwd DIR]` — same registration, but `pi --mode rpc`; streams events
  to stdout and log. Watches `inbox/` between events for `{"type":"kill"}` (terminate;
  Phase 1) and `{"type":"prompt","message":...}` (one follow-up; Phase 2). Ctrl+C kills
  cleanly.
- `orc list [--json]` — table of runs (id, brain, status, age, task excerpt).
  Reconciles stale `running` entries by checking PID liveness.
- `orc show <id> [--tail N]` — meta + last N log lines.
- `orc kill <id>` — SIGTERM the recorded PID; status `killed`.
- `orc quota [--json]` — GET `/v1/token_plan/remains` with the key (Keychain
  `minimax_api_key` first, fallback `~/.pi/agent/auth.json`). Caches to `quota.json`
  for 60 s. Exit codes: 0 = ok, 2 = below warn threshold, 3 = below block threshold,
  4 = endpoint unavailable (unknown).
- `orc ui [--port 7777]` — Phase 2 dashboard server (see §4).

**Quota guard:** before each spawn, `orc run`/`orc rpc` consult the cached quota.
Below **warn** threshold (default 25 % of the 5-hour window): print a prominent
`ORC WARNING: MiniMax 5h window at N% — consider pausing delegation` line to stderr
(the brain sees it and must relay it to the user; the skills say so). Below **block**
threshold (default 10 %): refuse to spawn unless `--force`. Quota endpoint failure
never blocks work — warn `quota unknown` and proceed. Thresholds in
`~/.orchestra/config.json`.

**Fallback if `/remains` rejects the key** (it requires a coding-plan subscription
key): `orc quota` reports `unknown` and `orc` falls back to local accounting — best-
effort token usage per run parsed from pi's JSON output mode if available, else
chars/4 estimate of the log — displayed as "estimated spend today" instead of
"remaining." Implementation will test the endpoint with the real key first.

### 2. Registry (`~/.orchestra/`)

Plain JSON, one directory per run, all writes atomic (write temp file in same dir,
`os.replace`). Single writer per `meta.json` (the `orc` process that owns the run);
the dashboard is read-only except for dropping messages into `inbox/`. `meta.json`
fields: `id, task, brain, cwd, provider, model, pid, status
(running|done|failed|killed), started_at, ended_at, exit_code, tokens {input, output,
estimated}`.

### 3. Shell helpers + skills (the brain-facing layer)

Appended to `~/.zshrc` (a marked block; `pix` untouched):

- `deleg8 "task" [cwd]` → `orc run "task" --cwd "${2:-$PWD}" --brain "${ORC_BRAIN:-human}"`
- `pi-rpc "task"` → `orc rpc "task"`

Skills (live in the repo, symlinked to `~/.claude/skills/`):

- **`pi-delegate`** — auto-triggering; content per the user's original spec (when to
  delegate: 10+ files, large inputs, batch ops, refactors, cheap second pass; when not
  to; how to call `deleg8`/`pi-rpc`; treat worker output as untrusted). Additions: relay
  any `ORC WARNING` quota lines to the user; on worker error retry once with a more
  focused prompt, then stop.
- **`orchestrate`** — description states it triggers **only** when the user explicitly
  says "orchestrate" (or "orchestrated"). Flow: run `orc quota` and report it → decompose
  the task into worker-sized chunks → launch parallel `orc run --bg` workers (bounded,
  default ≤ 3 concurrent) → poll `orc list --json` → collect and verify outputs →
  synthesize. Mention the dashboard URL (`orc ui`) so the user can watch. Set
  `ORC_BRAIN=claude`.

**Codex:** append the same two sections (`## pi-delegate`, `## orchestrate`, with
`ORC_BRAIN=codex`) to the existing `~/.codex/AGENTS.md` under a marked
`<!-- pi-orchestra begin/end -->` block, after backing the file up. No `config.toml`
changes.

### 4. Dashboard (`orc ui`) — Phase 2

Single-file server on `localhost:7777` using `http.server` (no dependencies, never
binds beyond loopback). Endpoints: `/` (inlined HTML/JS/CSS), `/api/runs`,
`/api/runs/<id>/log?tail=200`, `/api/quota`, `POST /api/runs/<id>/kill` (writes a kill
message to the run's inbox / signals the PID). UI: auto-refreshing (2 s poll) run table
grouped by brain, status chips, expandable log tail, kill button per running run, and
two quota bars (5-hour and weekly windows) with warn/block markers. A "new task" form
that shells out to `orc run --bg` is included; **launching a full brain session from the
panel is out of scope for v1** (needs a TTY; noted as a Phase 3 idea via
`osascript`/Terminal.app).

## Phasing

- **Phase 1 (build first):** `orc` core (`run`, `rpc`, `list`, `show`, `kill`,
  `quota`), registry, `deleg8`/`pi-rpc`, both skills, Codex AGENTS.md block,
  end-to-end tests, cheat sheet. Everything testable from the terminal.
- **Phase 2:** `orc ui` dashboard; rpc inbox steering (follow-up prompts).
- **Phase 3 (ideas, not committed):** launch brain sessions from the panel; MCP push
  upgrade of the registry; advisor pattern (worker consults the brain mid-task, per
  Anthropic's advisor-tool docs); Vibe Kanban integration.

## Error handling

- Worker non-zero exit → status `failed`, stderr relayed; skills retry once, focused.
- Quota endpoint failure → warn, never block.
- Orphaned `running` runs (machine slept, pi crashed) → reconciled by PID check in
  `orc list`/dashboard.
- Concurrent access → single-writer meta.json + atomic renames; no locks needed.
- `orc kill` on an already-dead PID → idempotent, status still transitions.

## Testing (definition of done for Phase 1)

1. `pi -p "Reply with the single word: PONG"` → PONG.
2. `deleg8 "Reply with the single word: PONG"` → PONG **and** a registry entry appears.
3. `pi -p "What model are you? Reply with just your model id."` → MiniMax-M3 (or
   truthful equivalent).
4. `deleg8 "List every file in the current directory recursively, grouped by extension,
   with counts. Output as markdown."` in a real project dir → sensible markdown.
5. `ls -la ~/.claude/skills/pi-delegate/SKILL.md` and `orchestrate/SKILL.md` → present;
   `~/.codex/AGENTS.md` contains the marked block.
6. `orc quota` → real numbers (or documented fallback if the key isn't a subscription
   key).
7. `orc run --bg` + `orc kill` on a long task → status `killed`, process gone.
8. Regression: plain `claude` and plain `codex` still start on their subscriptions;
   `pix` still works; `~/.pi/agent/*`, `~/.claude/settings.json`, `~/.codex/config.toml`
   unmodified (checksum before/after).

## Constraints

- Never invent or move API keys; read from Keychain/auth.json only.
- All changes additive: new repo, new symlinks, marked appends to `~/.zshrc` and
  `~/.codex/AGENTS.md` (with backups). One-command uninstall documented in the README.
- Delegation always passes explicit `--provider minimax --model MiniMax-M3` and
  `--no-session`.
- Cost expectations (for the cheat sheet): typical delegation ~50k in / 5k out ≈ $0.02;
  a 500k-token repo scan ≈ $0.17 — and on the coding plan these draw from the prepaid
  window rather than billing per-token.

## Assumptions made while the user was away (please veto on review)

1. **Phasing** — design everything now; build Phase 1 first, dashboard second.
2. **Build vs adopt** — custom thin dashboard over adopting Vibe Kanban/Conductor.
3. **"Control" scope v1** — view + kill + one follow-up message to rpc runs; launching
   brain sessions from the panel deferred to Phase 3.
4. **Quota key** — assumed the Keychain key is a coding-plan subscription key; the
   `/remains` endpoint will be verified during implementation, with the estimation
   fallback if not.
5. **Keyword** — the literal words "orchestrate"/"orchestrated" gate the orchestrate
   skill; plain heavy-task delegation via `pi-delegate` still auto-triggers (as the
   user's original prompt specified).
6. **`orc` implemented in Python 3 stdlib** (already on macOS) rather than pure bash —
   JSON handling and the Phase-2 HTTP server make bash a poor fit.
