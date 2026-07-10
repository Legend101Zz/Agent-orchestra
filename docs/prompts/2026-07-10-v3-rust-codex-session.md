# NEXT SESSION PROMPT — orc v3: ideate with me, then orchestrate a Rust rebuild

Paste everything below the line into a fresh **Codex (GPT) session** started in
`/Users/comreton/Desktop/pi-orchestra`. This session is deliberately also a live
test of pi-orchestra's own delegation stack driven by a Codex brain.

---

## Who you are

You are the senior product-engineer who *owns* this tool and uses it daily. You are
opinionated about terminal UX, allergic to generic-looking TUIs, and honest about
what's weak in your own product. This session has two jobs: (1) redesign the UI/UX
of `orc top` around what's actually missing, and (2) migrate the stack to **Rust**
for speed — and while doing it, (3) exercise the **orchestrate** flow for real, as
its first serious test from a Codex brain.

## Ground rules (non-negotiable)

- **Branch**: create `v3-rust` off `main`. Every commit goes there. Never commit to
  `main`, never force-push, never merge — a separate judge session decides that.
  Push the branch to `origin` as you go.
- **Protected**: never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`, or the user's `~/.local/bin/orc` symlink.
- **Registry compatibility is sacred**: `~/.orchestra/runs/<id>/{meta.json,
  output.log, inbox/}` — plain JSON, atomic temp-file+rename writes, single writer
  per meta.json, TUI read-only. Anything Rust writes must stay readable by the
  Python `orc`, and vice versa. Old metas (no `session`, no exact `tokens`) must
  still render.
- Read first: `README.md`, `docs/guide.html`,
  `docs/superpowers/specs/2026-07-10-pi-orchestra-design.md`, and skim
  `orc_pkg/` — it is the reference implementation you are porting.
- Attribute your delegations: `--brain codex` (the AGENTS.md block already says so).

## Phase 0 — ideate with the user FIRST (no code, no branch yet)

Think like the developer who built this and has to live with it. Study the current
TUI (`orc top`, screenshots in `docs/*.svg`, keys in README). Then present the user
your honest critique and a feature proposal list, and refine it **one question at a
time** until you converge on a prioritized v3 feature list. Seed material — known
gaps and warts to react to (verify, don't assume):

- You cannot **steer a running worker**: the inbox protocol supports
  `{"type":"prompt","message":…}` follow-ups (designed as Phase 2 in the original
  spec) but it was never built into the TUI or CLI.
- No **retry** action for a failed/stalled worker (exit 124 happens; MiniMax stalls
  ~50% of long calls some days) — you have to re-type the task.
- No **budgets or alerts**: nothing says "this session has burned $0.40, stop".
- No **notifications** when a background worker finishes/fails (macOS `osascript`
  is available).
- The activity strip counts run *starts*, not tokens; Codex brain-side numbers are
  inflated by cached input; the delegated-value multiple prices estimates as
  all-input. All labeled, but could be better.
- Table is rebuilt every 2 s (flicker risk at scale); no virtualization or paging
  for hundreds of runs; no cross-run output search; no timeline/replay of a session.
- Empty first-run state is dead space; warn/block thresholds and themes are edited
  by hand in `~/.orchestra/config.json` — no in-TUI settings.
- Startup pays Python+venv tax (~0.5–1 s); `orc top` is pinned to the repo venv.

Ask what they want, propose what you'd build, disagree where you have a better
idea. Then write the agreed design to
`docs/superpowers/specs/2026-07-11-orc-v3-rust-design.md` (features, Rust
architecture, parity strategy, phasing) and get an explicit "approved" from the
user **before touching code**. Only then create the branch.

## Phase 1 — orchestrate the heavy lifting (this is the live test)

Use the orchestrate flow — quota check first (`orc quota`, relay any
`ORC WARNING`/`ORC BLOCKED` verbatim), then
`export ORC_SESSION="orch-$(date +%Y%m%d-%H%M%S)-v3"`, then ≤3 parallel
`orc run "chunk" --bg --brain codex` workers, monitored via `orc list --json`
(tell the user to keep `orc top` open — the swarm shows as one session). Good
worker-sized chunks:

- repo-wide inventories (every registry field written/read, every CLI flag, every
  key binding and theme token — with file:line refs) to feed the port;
- first drafts of Rust modules from interface+test specs you write;
- documentation drafts.

Rules learned dogfooding this repo: worker output is **untrusted** (observed bug
rate is real: invented JSON wrappers, dropped quotes) — review and fix everything
yourself; retry a failed worker once with a tighter prompt, then do it yourself;
prefer `--thinking low` for tightly-specified drafts; the idle watchdog (exit 124)
is load-bearing. Keep a running log of every friction point with the skill wording,
`orc` ergonomics, or worker quality in
`docs/notes/2026-07-11-codex-orchestrate-friction.md` — the judge will read it.

## Phase 2 — Rust core (`rust/` cargo workspace)

- Stack: `clap` + `serde`/`serde_json` + `anyhow`; async only if you can justify
  it. Binary named `orc`, built to `rust/target/release/orc`, **not** installed
  over the symlink yet.
- Port with exact behavior parity: registry (atomic writes, orphan reconcile by
  PID liveness), quota (Keychain via `security` subprocess → fallback
  `~/.pi/agent/auth.json`; `GET /v1/token_plan/remains`, coding plan =
  `model_name:"general"`; 60 s cache; history append), runner (`pi -p --mode json`
  and `--mode rpc`, always `--offline`; hold rpc stdin open until `agent_end`;
  exact usage from `agent_end`; SIGTERM-trap → exit 143 → status `killed`; idle
  watchdog → exit 124; quota gate exit 3), `list/show/kill/stats`, sessions.
- **Parity tests**: golden fixtures — run the Python suite's fake-pi scenarios
  against the Rust binary too; add a round-trip test (Rust writes a run, Python
  `orc list` reads it; Python writes, Rust reads). The existing
  `.venv/bin/python -m pytest -q` suite must stay green on the branch.
- **Benchmarks**: `hyperfine 'python -m orc_pkg list' 'rust/target/release/orc
  list'` (and `quota --json` cached) — record the table in the branch README.
  The speedup is the migration's reason to exist; measure it, don't assert it.

## Phase 3 — ratatui TUI (the revamp)

Rebuild the control plane in `ratatui` + `crossterm` with the v2 feature set as
the floor — gradient quota meters + braille history, delegated-value hero tile,
session tree, live log tail, drill-in Flow/Conversation/Log/Meta, `ember` and
`phosphor` themes from a single token module, mouse + vim keys — plus the Phase 0
features the user picked. The anti-slop directive still applies in full: no stock
ratatui look, no default borders-and-white-text, every color from theme tokens.
Capture demo output with `vhs` (or `termshot`) into `docs/` for the judge.

## Phase 4 — quality bar & handoff

- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, Python suite,
  then `tests/live_smoke.sh` once with the Rust binary first on PATH (costs cents).
- Update README + guide **on the branch**; `install.sh` gains an opt-in `--rust`
  flag but defaults to Python.
- Small conventional commits; push `v3-rust`; end with a handoff summary: what
  shipped, what's cut, benchmark table, open risks, and what the judge should
  scrutinize. Include `orc stats` output as the cost receipt for the session.

Do not merge. The judge decides.
