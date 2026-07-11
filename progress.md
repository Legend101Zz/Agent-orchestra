# Progress Log

## Session 1 — 2026-07-10
- On v3-rust, clean, synced with origin. Planning files created.
- Review complete: `docs/reviews/2026-07-11-v3-rust-review.md`, verdict fix-first, pushed.

## Session 2 — 2026-07-11 (v4 planning)
- Researched advisor tool, BridgeSpace (16-pane grid + kanban, Tauri/Rust),
  claude-squad (tmux+worktrees), Claude Code agent teams (task list/mailbox),
  fulsomenko/kanban (ratatui, V view-cycling, atomic JSON), tui-term/portable-pty.
- Wrote v4 design spec + next-session prompt (see task_plan.md). Committed on v3-rust.
- Rev 2 after user review: flagship UI pivoted ratatui → Tauri 2 app (xterm.js
  terminal cards, SVG baton-line connectors, draggable SCORE kanban); default
  worker pool changed to hermes + pi/MiniMax-M3; researched Vibe Kanban,
  tauri-plugin-pty, hermes-agent to ground the pivot.
- Rev 3 (final): user rejected desktop app — TUI-only, but amazing. New
  architecture: orcd client-server daemon (zellij model) + ratatui/tachyonfx
  client; conductor-down recovery; Python fully deleted (fixtures first);
  remote = SSH attach; worktree-per-task in scope; ember+phosphor only;
  engineering standards (rustdoc/TDD/snapshot tests/benchmarks) mandated.

## Session 3 — 2026-07-11 (v4 implementation)
- User approved the rev-3 handoff and asked for exact execution.
- `main` already contains PR #1 plus the amended branch-order documentation;
  created `v4-bench` from local `main` at `4121170`.
- Preserving the existing untracked review/planning artifacts and extending
  them as durable handoff state.
- Phase-0 delegated audit completed as run
  `20260711-121239-audit-phase-0-only-for-p-725a`, correctly attributed to
  Codex; its claims are being independently verified before use.
- Phase 0 implemented test-first: the revised two-turn fake-pi fixture failed
  with final usage `10` instead of `12`, and the new subprocess timeout test
  failed to compile until the bounded helper existed. Both now pass.
- Quota HTTP connect/global timeouts are 15 seconds; the `security` lookup is
  killed and reaped after 10 seconds; transport failures still fail open.
- RPC completion now requires one `agent_end` for the initial prompt plus one
  for every successfully delivered follow-up, with the inbox drained again
  before deciding at the turn boundary.
- TUI quota and history retrieval now runs immediately on a named worker and
  then at the configured cache-TTL cadence; startup and rendering only drain
  non-blocking channel updates.
- Phase-0 gates: Python `92 passed`; Rust unit/integration/doc tests passed;
  fmt clean; clippy clean with `-D warnings`; rustdoc clean with `-D warnings`;
  release build clean; live smoke with the Rust binary first on PATH `10/10`.
  Smoke quota before/after: 84% five-hour, 32% weekly, level `ok`.
- A `deleg8` second-pass run (`20260711-122651-review-only-the-current-024a`)
  was explicitly attributed to Codex but killed after nine minutes without a
  report (exit 130; registry fallback estimate was implausibly amplified to
  14,116,214 tokens). It made no repository edits and was not retried or used.
