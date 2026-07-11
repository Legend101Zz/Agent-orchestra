# Next-session prompt — pi-orchestra v4 "Bench" (rev 2: Tauri app)

Copy everything below the line into a fresh session started in
`/Users/comreton/Desktop/pi-orchestra`. Handing this prompt to a session
constitutes approval of the design document it references.

---

You are building **pi-orchestra v4 "Bench"** — the multi-harness orchestration
workspace: a Tauri 2 desktop app where one brain harness and its workers run
as floating terminal cards on a stage, connected by animated Bézier "baton
lines", next to a draggable kanban SCORE board. The approved design (rev 2) is
`docs/superpowers/specs/2026-07-11-pi-orchestra-v4-bench-design.md`; read it
first and treat it as the contract — especially the presentation-layer
decision (Tauri app is the flagship; the ratatui `orc top` is frozen as
fallback) and the worker defaults (pool = **hermes + pi/MiniMax-M3**, user may
choose claude/codex/any configured harness). Then read, in order:

1. `docs/reviews/2026-07-11-v3-rust-review.md` — fix-first verdict; findings
   1–3 (+6) are your Phase 0.
2. `docs/superpowers/specs/2026-07-11-orc-v3-rust-design.md` — inherited
   constraints (atomic registry, tolerant models, fail-open quota, theme
   discipline).
3. `docs/notes/2026-07-11-codex-orchestrate-friction.md` — dogfood lessons
   (log amplification, session grouping, worker untrust).
4. `README.md`, `skills/orchestrate/SKILL.md`, `skills/pi-delegate/SKILL.md`,
   `codex/AGENTS-block.md`.
5. Skim `rust/crates/` — orc-core, orc-cli, orc-tui are your base.

## Hard rules

- Phase 0 happens on `v3-rust`; after its gates pass, merge `v3-rust` → `main`
  (no force-push; the review authorizes this merge only after fixes 1–3 are
  verified), then create `v4-bench` from `main` for all v4 work. If a Phase 0
  gate fails in a way you cannot fix cleanly, stop and report instead of
  merging.
- Never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`, or `~/.local/bin/orc` mid-development.
- Registry, sessions, tasks: plain JSON, atomic temp+fsync+rename, additive
  fields only; Python `orc_pkg` must keep reading everything (parity tests
  prove it).
- **Single-writer rule:** the app frontend never writes files. Every mutation
  (launch, kill, steer, task move) goes through orc-core command functions,
  recorded with its actor (`brain` | `human`).
- **Prime directive:** never hinder a harness. Focused terminal cards get
  verbatim input; app chords must not collide with harness keymaps (audit in
  the spike); `cmd+w`/`cmd+q` are intercepted with confirm while panes are
  live; orc never proxies harness API traffic.
- TDD for core logic (failing test first); small conventional commits; push
  the branch as verified slices land. Worker output is untrusted — verify
  every claim against files/tests before acting on it.
- Design bar: this must not look like AI-template slop (no generic card grids,
  no cream+terracotta or black+acid-green defaults). Follow the spec's
  ember-derived material palette, typography plan, and the baton-line
  signature. Screenshot and actually look at every visual milestone; iterate
  until it would not be mistaken for a template.

## Phases (each ends with its gate green + a commit + push)

**Phase 0 — fix-first (on `v3-rust`).**
1. `quota.rs`: connect + global timeouts (15 s) on the ureq call; 10 s timeout
   on the `security` keychain subprocess.
2. `runner.rs`: count delivered prompts; finish the RPC loop only when
   `agent_end` count ≥ 1 + delivered prompts (idle watchdog stays as
   backstop; drain pending inbox prompts before deciding). Fix the fake-pi
   fixture in `tests/test_rust_parity.py::test_rpc_send_delivers_once_and_acks`
   to emit one `agent_end` per prompt — the current shape masks the bug.
3. `orc-tui`: move quota + history fetch off the startup/render path onto a
   worker thread on the cache-TTL cadence.
   Gate: full pytest, cargo test / clippy -D warnings / fmt --check, and
   `tests/live_smoke.sh` with the Rust binary first on PATH → 10/10. Merge to
   `main`, branch `v4-bench`.

**Phase 1 — app spike (go/no-go, on `v4-bench`).**
Scaffold `app/` (Tauri 2, React+TS+Vite): two xterm.js (WebGL addon) panes
running Claude Code and hermes interactively via portable-pty, plus one
hardcoded SVG Bézier between them that pulses when a watched file changes.
Measure/verify: redraw fidelity for full-screen harness TUIs, paste, mouse,
resize, IPC throughput under output floods (batch PTY output across the IPC
boundary), key passthrough (including kitty keyboard protocol and bracketed
paste), quit interception. Also run `hermes --help` (and docs) to record its
headless/JSON/usage-reporting shape for the Phase 5 adapter. Write the verdict
to `docs/notes/2026-07-11-app-spike.md`. If Tauri disappoints: pivot to the
same UI served locally (`orc serve` + WebSocket) and record why — do not
grind silently on a failing approach.

**Phase 2 — HOME + STAGE.** `session.rs` + records under
`~/.orchestra/sessions/<id>/`; HOME shelf (session cards, receipts, attention
badges); three-step new-session flow (brain → worker pool [hermes + pi-m3
preselected, any configured harness selectable] → cwd); STAGE ensemble layout
(conductor center-left, players arced) with drag/resize/zoom and per-session
layout persistence; pane↔run attribution via `ORC_SESSION`/`ORC_PANE_ID`;
harness registry + `default_workers` in `config.json`; design-token +
typography pass reviewed via screenshots; `pi-orchestra` launcher in
`install.sh`; Rust CLI becomes the default install (Python behind
`--python`).

**Phase 3 — tasks + SCORE + skills.** `tasks.rs` (file-per-task, statuses
backlog/assigned/running/review/done/dropped, `depends_on`, `assignee_run`,
`history` with actor); `orc task add|assign|start|review|done|drop|move|list
--json`; SCORE kanban with drag-to-move (through `orc task move`, actor
human); card→stage flying link; update `orchestrate`, `pi-delegate`,
`codex/AGENTS-block.md`: board maintenance, "pi-orchestra" trigger alias,
worker-pool choice (offer `default_workers`, never assume), `ORC_PANE_ID`
awareness, the five wording fixes from the v3 review's process section; add a
hermes instructions block if hermes reads an AGENTS.md-equivalent; `orc top`
gains read-only task rendering only.

**Phase 4 — baton lines + RUNS.** fs-watcher event stream → directional
pulses on real dispatch/steer/complete/handoff with timestamped labels
settling into a timeline; `prefers-reduced-motion` degrade; RUNS view (dense
instrument panel: quota meters, run table, search) over the same snapshot
logic as the TUI.

**Phase 5 — adapters + docs + dogfood.** Adapter seam in `runner.rs` with
capability flags (`steerable`, `exact_usage`); **hermes adapter first** (from
the Phase 1 shape notes; if hermes has no headless mode, it stays an
interactive stage pane — never fake exact usage), then claude
(`claude -p --output-format stream-json`) and codex (`codex exec --json`)
best-effort. README + guide rewrite around the app with screenshots/recording.
**Dogfood gate:** build at least one Phase 5 deliverable by saying
"orchestrate" with the brain hosted on the STAGE and tasks tracked on the
SCORE; keep a friction log in `docs/notes/` — it is a deliverable.

Cut order under pressure: claude/codex adapters → RUNS view (TUI covers it) →
layout presets beyond solo/duet/quartet → texture/typography extras. Never
cut: Phase 0, passthrough integrity, single-writer mutations, fail-open
quota, actor-attributed task moves, the hermes+pi default pool, baton lines
(static-line degrade acceptable; absence is not).

## Verification culture

Before claiming any phase done: run the full gates (pytest, cargo suite, and
from Phase 1 on `npm run build` + typecheck in `app/ui`), exercise the feature
live with real harnesses, and capture evidence — screenshots for anything
visual, looked at, not just taken. Update `task_plan.md`/`progress.md` as you
go so a cleared session can resume. End with a shipped/cut/risks summary and
leave merge decisions beyond Phase 0 to the user.
