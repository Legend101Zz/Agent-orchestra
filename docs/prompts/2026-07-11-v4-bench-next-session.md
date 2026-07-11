# Next-session prompt — pi-orchestra v4 "Bench" (rev 3: TUI-only, daemon)

Copy everything below the line into a fresh session started in
`/Users/comreton/Desktop/pi-orchestra`. Handing this prompt to a session
constitutes approval of the design document it references.

---

You are building **pi-orchestra v4 "Bench"** — a terminal-only orchestration
workspace: one brain harness and its workers as floating pane cards on a
ratatui STAGE, connected by braille-Bézier "baton lines" that pulse on real
events, beside a kanban SCORE board with worktree-isolated tasks — all hosted
by a client-server daemon (`orcd`) so neither a UI crash nor a brain crash
ends the session. The approved design (rev 3) is
`docs/superpowers/specs/2026-07-11-pi-orchestra-v4-bench-design.md`; read it
first and treat it as the contract — especially: TUI-only (no Tauri, no web),
the orcd client-server architecture, conductor-down recovery, full Python
deletion, worktree-per-task, worker pool defaulting to **hermes +
pi/MiniMax-M3** (user may choose claude/codex/any configured harness), ember +
phosphor themes only, and the engineering-standards section. Then read, in
order:

1. `docs/reviews/2026-07-11-v3-rust-review.md` — fix-first verdict; findings
   1–3 (+6) are your Phase 0.
2. `docs/superpowers/specs/2026-07-11-orc-v3-rust-design.md` — inherited
   constraints (atomic registry, tolerant models, fail-open quota, no-emoji /
   theme-token discipline).
3. `docs/notes/2026-07-11-codex-orchestrate-friction.md` — dogfood lessons.
4. `README.md`, `skills/orchestrate/SKILL.md`, `skills/pi-delegate/SKILL.md`,
   `codex/AGENTS-block.md`.
5. Skim `rust/crates/` — orc-core, orc-cli, orc-tui are your base; orc-tui's
   theme tokens, glyph kit, and dashboard get ported into the new client.

## Hard rules

- `v3-rust` was already merged to `main` (PR #1) **before** the fix-first
  items landed, so the review's P0 defects are live on `main`. Create
  `v4-bench` from `main` immediately and do ALL work there, starting with
  Phase 0 as its first commits. Never force-push; if a Phase 0 gate fails in
  a way you cannot fix cleanly, stop and report.
- Never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`, or `~/.local/bin/orc` mid-development.
- Registry, sessions, tasks: plain JSON, atomic temp+fsync+rename, additive
  fields only. Before deleting Python, capture golden fixtures that pin the
  JSON/CLI contract (including legacy/corrupt/CJK metas) — they replace the
  parity oracle forever.
- **Single-writer rule:** the client never writes files; every mutation
  (launch, kill, steer, task move, worktree merge) goes through orc-core
  command paths, recorded with actor (`brain` | `human`).
- **Prime directive:** never hinder a harness. Verbatim input to the focused
  pane (kitty keyboard protocol, bracketed paste, mouse forwarded); one
  leader key (`ctrl-g`, double-tap = literal); chrome never overlays pane
  cells; no API-traffic proxying.
- **Engineering standards (non-negotiable):** TDD for core logic;
  `#![warn(missing_docs)]` + rustdoc on all public items and modules;
  `cargo doc --no-deps` warning-free; clippy `-D warnings`; no
  `unwrap`/`expect` in orcd/orc-core outside tests; `thiserror` in libs,
  `anyhow` in bins; `tracing` in the daemon; bounded memory everywhere; UI
  snapshot tests (TestBackend) for every view, both themes, wide + 72×30.
- Small conventional commits; push verified slices. Worker output is
  untrusted — verify claims against files/tests before acting.
- Design bar: the user must feel amazed that this is a terminal — but no
  AI-slop and no stock-ratatui look. Ember + phosphor tokens only, state as
  words, no emojis. Screenshot (or vhs) every visual milestone and actually
  look at it; iterate until it could not be mistaken for a template. Smooth
  means measured: frame time, input latency, and idle CPU are benchmarked
  and recorded, not asserted.

## Phases (each ends with its gates green + a commit + push)

**Phase 0 — fix-first (first commits on `v4-bench`).**
1. `quota.rs`: connect + global timeouts (15 s) on the ureq call; 10 s
   timeout on the `security` keychain subprocess.
2. `runner.rs`: count delivered prompts; finish the RPC loop only when
   `agent_end` count ≥ 1 + delivered prompts (idle watchdog stays backstop;
   drain pending inbox prompts before deciding). Fix the fake-pi fixture in
   `tests/test_rust_parity.py::test_rpc_send_delivers_once_and_acks` to emit
   one `agent_end` per prompt — the current shape masks the bug.
3. `orc-tui`: move quota + history fetch off the startup/render path onto a
   worker thread on the cache-TTL cadence.
   Gate: full pytest (Python still exists in Phase 0), cargo
   test/clippy/fmt, `tests/live_smoke.sh` with the Rust binary first on
   PATH → 10/10. (No merge step — `v3-rust` is already in `main`; these
   fixes are the first commits on `v4-bench`.)

**Phase 1 — spike (go/no-go, on `v4-bench`).**
Prototype the vertical slice: `orcd` skeleton (unix socket, hosts two PTYs
running Claude Code and hermes live), client rendering both panes — run a
vt-parser bake-off (`vt100` vs `termwiz` vs `alacritty_terminal`) and pick
with reasons — plus one braille Bézier between the panes with a tachyonfx
pulse, synchronized-output frames, adaptive frame clock (event-driven idle,
60 fps only during animation/PTY activity). Measure and record: full-screen
harness TUI fidelity, input latency (< 16 ms budget), CPU at 4 panes under
output flood, idle CPU (~0), detach/reattach with screen replay. Also record
hermes's headless/JSON/usage shape (`hermes --help`, docs) for Phase 5.
Verdict to `docs/notes/2026-07-11-tui-spike.md`. **Fallback if PTY fidelity
fails:** companion mode — brain stays in the user's own terminal; the client
shows workers, board, and flow. Decide explicitly; never grind silently.

**Phase 2 — daemon + client shell + HOME + STAGE + Python deletion.**
Productionize `orcd` (orc-proto versioned messages, multi-client attach,
reap-on-restart via pid records, tracing, soak test with a flooding pane);
client compositor: floating cards (arc-glyph corners, half-block shadows,
brass focus edge-light), ensemble layout with keyboard swap + mouse drag,
resize, zoom-to-solo, per-session layout persistence; HOME shelf +
three-step new session (brain → worker pool [hermes + pi-m3 preselected] →
cwd); `ORC_SESSION`/`ORC_PANE_ID` attribution; conductor-down basic
recovery (dead brain pane shows last screen + `R` respawns with
`resume_args` from the harness registry). Python removal, in this order:
golden fixtures captured → fake-pi integration suite ported to Rust test
helpers → delete `orc_pkg/`, `.venv`, `pyproject.toml`,
`requirements.txt` → `install.sh`/`uninstall.sh` Rust-only, installing the
`pi-orchestra` launcher; `orc top` becomes an alias for the client's RUNS
view.

**Phase 3 — tasks + worktrees + SCORE + skills.**
`tasks.rs` (file-per-task, statuses backlog/assigned/running/review/done/
dropped, `depends_on`, `assignee_run`, actor `history`); `orc task
add|assign|start|review|done|drop|move|diff|merge|list --json` with
`--isolate`; worktree lifecycle: create at
`~/.orchestra/worktrees/<session>/<task-id>` on branch
`orc/<session-slug>/<task-id>`, worker cwd = worktree, `diff` for review,
`merge` squash-merges and prunes, `drop` prunes, non-git cwd degrades with
plain words; SCORE board (keyboard + mouse drag → `orc task move`, actor
human; diff stats on review cards; `g` flies to the stage card). Update
`orchestrate`, `pi-delegate`, `codex/AGENTS-block.md`: board maintenance,
"pi-orchestra" trigger alias, worker-pool choice (offer `default_workers`,
never assume), `ORC_PANE_ID` awareness, brain-resume re-orientation note,
the five wording fixes from the v3 review; add a hermes block if hermes
reads an AGENTS.md-equivalent.

**Phase 4 — baton lines + RUNS + polish.**
Event stream → directional pulses with per-kind shape/tempo (dispatch,
steering delivery, completion, handoff) and labels settling into the
timeline; session-open settle animation; view transitions (`V` cycles
STAGE/SCORE/RUNS); reduced-motion config path; RUNS view = the v3 instrument
panel ported and event-driven; frame/latency/idle benchmarks recorded in the
README beside the v3 CLI numbers.

**Phase 5 — adapters + docs + dogfood.**
Adapter seam with capability flags (`steerable`, `exact_usage`); **hermes
adapter first** (from Phase 1 notes; if no headless mode, hermes remains an
interactive stage pane — never fake exact usage), then claude
(`claude -p --output-format stream-json`) and codex (`codex exec --json`)
best-effort. README + guide rewrite around the client (vhs captures of
HOME/STAGE/SCORE). **Dogfood gate:** build at least one Phase 5 deliverable
by saying "orchestrate" with the brain on the STAGE and worktree-isolated
tasks on the SCORE; keep a friction log in `docs/notes/` — it is a
deliverable.

Cut order under pressure: claude/codex adapters → view transitions + settle
animations (keep event pulses) → mouse drag on STAGE (keyboard swap stays) →
RUNS extras beyond the v3 port. Never cut: Phase 0, the daemon +
detach/reattach, conductor-down recovery, passthrough integrity,
single-writer mutations, actor-attributed moves, worktree isolation, the
hermes+pi default pool, fail-open quota, baton lines (static degrade
acceptable; absence is not), the engineering standards.

## Verification culture

Before claiming any phase done: run every gate (fmt, clippy -D warnings,
tests, doc build; pytest only while Python exists), exercise the feature live
with real harnesses in a real terminal (Ghostty and at least one of
kitty/Alacritty/WezTerm; wide and 72×30; both themes), and capture evidence —
screenshots/vhs actually looked at, latency and CPU actually measured. Update
`task_plan.md`/`progress.md` as you go so a cleared session can resume. End
with a shipped/cut/risks summary and leave merge decisions beyond Phase 0 to
the user.
