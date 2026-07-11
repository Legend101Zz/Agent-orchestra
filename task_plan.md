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
