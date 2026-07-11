# Next-session prompt — pi-orchestra v4 "Bench"

Copy everything below the line into a fresh session started in
`/Users/comreton/Desktop/pi-orchestra`. Handing this prompt to a session
constitutes approval of the design document it references.

---

You are building **pi-orchestra v4 "Bench"** — the multi-harness orchestration
workspace. The approved design is
`docs/superpowers/specs/2026-07-11-pi-orchestra-v4-bench-design.md`; read it
first and treat it as the contract. Then read, in order:

1. `docs/reviews/2026-07-11-v3-rust-review.md` — fix-first verdict; findings
   1–3 (+6) are your Phase 0.
2. `docs/superpowers/specs/2026-07-11-orc-v3-rust-design.md` — the constraints
   you inherit (atomic registry, tolerant models, read-only TUI, no emojis,
   theme tokens, fail-open quota).
3. `docs/notes/2026-07-11-codex-orchestrate-friction.md` — dogfood lessons
   (log amplification, session grouping, worker untrust).
4. `README.md`, `skills/orchestrate/SKILL.md`, `skills/pi-delegate/SKILL.md`,
   `codex/AGENTS-block.md`.
5. Skim `rust/crates/` — orc-core, orc-tui, orc-cli are your base.

## Hard rules

- Work on `v3-rust` only for Phase 0; after its gates pass, merge `v3-rust` →
  `main` (no force-push; the review verdict authorizes this merge only after
  fixes 1–3 are verified), then create `v4-bench` from `main` and do all v4
  work there. If any Phase 0 gate fails in a way you cannot fix cleanly, stop
  and report instead of merging.
- Never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`. Never replace `~/.local/bin/orc` mid-development.
- Registry, sessions, tasks: plain JSON, atomic temp+fsync+rename, additive
  fields only; Python `orc_pkg` must keep reading everything (parity tests
  prove it).
- The TUI stays read-only over run/task metadata; actions shell through the
  CLI. No emojis; no raw colors outside `theme.rs`; state is words.
- **Bench prime directive:** never hinder a harness. Focused panes get verbatim
  key passthrough; no chrome overlays pane cells; orc never proxies harness API
  traffic; coordination is filesystem-only.
- Use TDD (write the failing test first) for core logic; keep commits small and
  conventional; push the branch as verified slices land.
- Worker output is untrusted: verify every claim against files/tests before
  acting on it.

## Phases (each ends with its gate green + a commit + push)

**Phase 0 — fix-first (on `v3-rust`).**
1. `quota.rs`: add connect + global timeouts (15 s) to the ureq call; 10 s
   timeout on the `security` keychain subprocess.
2. `runner.rs`: count delivered prompts; finish the RPC loop only when
   `agent_end` count ≥ 1 + delivered prompts (idle watchdog remains backstop;
   drain pending inbox prompts before deciding). Fix the fake-pi fixture in
   `tests/test_rust_parity.py::test_rpc_send_delivers_once_and_acks` to emit
   one `agent_end` per prompt — the current fixture shape masks the bug.
3. `orc-tui`: move quota + history fetch off the startup/render path onto a
   worker thread refreshing on the cache-TTL cadence.
   Gate: pytest (all), cargo test/clippy -D warnings/fmt --check, and
   `tests/live_smoke.sh` with the Rust binary first on PATH → 10/10. Then
   merge to `main`, branch `v4-bench`.

**Phase 1 — PTY spike (go/no-go, on `v4-bench`).**
Prototype an `orc-pty` crate: portable-pty + a vt100-family parser (evaluate
`vt100`, `termwiz`, `alacritty_terminal`; pick the smallest that works),
rendering live panes in ratatui. Must survive: Claude Code and pi running
interactively, full-screen redraws, paste, mouse, resize, 4 panes at sane CPU,
bounded scrollback. Audit `ctrl-g` as leader against harness keymaps. Write the
verdict to `docs/notes/2026-07-11-pty-spike.md`. If no-go: pivot to a
tmux-backed pane backend (claude-squad model), record why, and adjust the plan
— do not silently grind on a failing approach.

**Phase 2 — HOME + BENCH.** Session records, session shelf, three-step
new-session flow (brain harness → default worker [default `pi-m3`, any
configured harness allowed] → cwd), pane grid with presets 1/2/4/6/9/16 + auto,
focus router with configurable leader key (double-tap sends literal), zoom,
status rail. Harness registry in `config.json` + settings view rows.
`pi-orchestra` alias installed by `install.sh`; Rust becomes the default
install (Python behind `--python`, kept as parity oracle).

**Phase 3 — tasks + BOARD + skills.** `tasks.rs` (file-per-task, statuses
backlog/assigned/running/review/done/dropped, `depends_on`, `assignee_run`),
`orc task add|assign|start|review|done|drop|list --json`, kanban BOARD view
cycled with `V`, `g` jumps card → pane. Update `orchestrate`, `pi-delegate`,
and `codex/AGENTS-block.md`: board maintenance via `orc task`, "pi-orchestra"
as an additional trigger word, `ORC_PANE_ID` awareness, and apply the five
skill-wording fixes listed in the v3 review's process section.

**Phase 4 — connection gutter.** Registry/inbox events drive directional
braille pulses along the brain↔worker channels; idle edges dim-dotted; labels
fade into the timeline. Motion budget lives here and nowhere else.

**Phase 5 — adapters + docs + dogfood.** Adapter seam in the runner
(capabilities: `steerable`, `exact_usage`); pi is the reference; claude
(`claude -p --output-format stream-json`) and codex (`codex exec --json`)
best-effort. Rewrite README + guide around v4 (HOME/BENCH/BOARD captures via
vhs). **Dogfood gate:** build at least one Phase 5 deliverable by saying
"orchestrate" and running the real flow with the board, brain in the bench if
the spike allows; keep a friction log in `docs/notes/` — it is a deliverable.

Cut order under pressure (from the design): extra adapters → gutter animation
→ grid presets beyond 1/2/4 → card detail view. Never cut Phase 0, passthrough
integrity, atomicity, read-only TUI, fail-open quota, or the no-emoji standard.

## Verification culture

Before claiming any phase done: run the full gates, exercise the feature live
(real `orc`, real TUI at normal and 72×30 sizes, both themes), and capture
evidence. Screenshot/vhs anything visual and actually look at it. Update
`task_plan.md`/`progress.md` as you go so a cleared session can resume. End
with a shipped/cut/risks summary and leave `main` merge decisions beyond
Phase 0 to the user.
