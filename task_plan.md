# Task Plan: v4 "Bench" implementation (2026-07-11)

The v3 review and v4 design are approved. PR #1 merged `v3-rust` before the
fix-first findings landed, so implementation now proceeds entirely on
`v4-bench`, starting with those fixes.

## Goal
Ship pi-orchestra v4 Bench according to the approved rev-3 design, with every
phase gated, committed, pushed, and evidenced.

**Current handoff:** Phase 5 is complete on `v4-phase5` pending final gates,
evidence, intentional commit, merge, and remote-main proof. Do not start
Phase 6, a web UI, or a provider proxy.

## Phases
- [x] Read the approved design, v3 review/spec, friction log, README, skills,
      AGENTS block, and Phase-0 Rust surfaces
- [x] Create `v4-bench` from the current `main`
- [x] Phase 0: quota/subprocess timeouts, steering turn accounting, async TUI
      quota refresh; full Python/Rust/live-smoke gates; commit and push
- [x] Phase 1: PTY/vt-parser/daemon/client spike, measurements, explicit verdict
- [x] Phase 2: daemon, protocol, client shell, HOME/STAGE, recovery, fixtures,
      Python deletion, Rust-only install
- [x] Phase 3: tasks, worktrees, SCORE, skills
- [x] Phase 4: confirmed dispatch, baton event motion, RUNS port, polish,
      measured performance
- [x] Phase 5: verified Hermes/pi capability handling, documentation, and
      Bench dogfood with recorded friction

## Session 8 — 2026-07-12 polish + real-use pass (user request)
Goal: make the first-run experience beautiful, prove the tool on a real
project with both MiniMax workers, capture real screenshots, ship a
professional README, verify/clean the installed build, and push each step
directly to main.

- [x] 6. Audit installed build vs main; identify stale artifacts
      (result: install is current with f8c00ec; stale
      `~/.local/bin/orc.pi-orchestra.bak` → deleted Desktop path)
- [x] 7. HOME start page redesign: animated avatar (Claude-style pulse) via
      the existing tick loop, banner + framed launch flow, honors
      reduced_motion; TestBackend snapshots both themes, wide + 72x30;
      full gates (fmt, clippy -D warnings, tests, doc -D warnings,
      locked release build)
- [x] 8. Reinstall via ./install.sh, remove stale .bak link, verify
      `orc version` and links
- [x] 9. Real dogfood: temp project backend task, session with hermes +
      pi-m3 workers, confirmed dispatch to both, screenshots captured;
      bugs logged in findings.md Session-8 section
- [x] 10. README rewrite: purpose, architecture, features, install, first
      use, guide, real screenshots, troubleshooting; remove stale
      phase-log prose and dead references
- [x] 11. Commit+push to main incrementally (uncommitted prompt/tool
      fixes first) — 1bd0de8, 22f7dda, f443ca1 all on origin/main

## Errors Encountered (Session 8)
| Error | Attempt | Resolution |
|-------|---------|------------|
| runs_watcher test failed once under full parallel cargo test | 1 | passes isolated and on both full reruns; logged as flaky (findings B1) |
| VHS tape: ctrl-g h from SCORE did not navigate HOME | 1 | retook shelf shot via STAGE; logged as UX bug B3 |
| Write to README blocked (not yet read) | 1 | read file head first, then overwrote |
