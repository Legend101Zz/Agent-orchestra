# v4 Bench — Phase 4 then Phase 5 execution prompt

Work only in `/Volumes/Mrigesh SSD/pi-orchestra`.

Start from the current verified remote `main` at
`69da971742e68ec23d3e456cfa74aa8fd82d3a8f`. First run `git fetch origin`,
prove local `main`, `origin/main`, and `git ls-remote origin refs/heads/main`,
then create a new branch from `origin/main` (for example
`v4-phase4-phase5`). Do not reuse `v4-bench`, force-push, or merge unrelated
history.

You are authorized to complete **Phase 4 followed by Phase 5 only**, make
small conventional commits, merge the verified completed branch to `main`, and
push `main` after every required acceptance gate passes. `findings.md` may now
be read, updated, and committed when genuinely useful; otherwise preserve it.

Do not start a Phase 6, web surface, or any unapproved feature. Never modify
`~/.pi/agent/*`, `~/.claude/settings.json`, `~/.codex/config.toml`, or the
live installed target behind `~/.local/bin/orc`. Every build must use an
isolated `CARGO_TARGET_DIR`.

## Required reading and preflight

Before editing, read in full:

- the approved v4 design;
- the v3 Rust review;
- Phase 2 and Phase 3 evidence notes;
- current `README.md`, `docs/guide.html`, `progress.md`, and `task_plan.md`;
- current Rust crates/tests; skills; `codex/AGENTS-block.md`; shell helpers;
  installer and uninstaller; and this prompt;
- `findings.md` if it contains useful current constraints.

Capture checksums of protected paths before work and reproduce them after the
final gate. If a platform-managed session log changes by itself, do not restore
or edit it; record the exact before/after discrepancy honestly. Preserve the
pre-existing Codex configuration checksum
`f0a989ad75b992ef16d4feb1ccba245634dc233aaaf05738362cac303b1ef31c` as an
observed baseline, not something to "fix".

Use the existing Rust-only runtime/package/test plumbing. Public modules/APIs
need rustdoc. There must be no production `unwrap`/`expect` in `orc-core` or
`orcd`, no busy loops, and bounded concurrency/memory everywhere.

## Phase 4A — real brain-to-worker control plane (user-required)

This is the first Phase 4 deliverable and gates the visual polish. A brain
launched in a Bench session must know, from the start, its `ORC_SESSION`,
`ORC_PANE_ID`, configured worker panes/harnesses, durable task board, and how
to delegate. The user must not need to manually copy a brain response into a
worker pane for a normal supported delegation.

Implement a bounded, durable daemon/core command path that lets the brain:

1. create/assign/start a task with explicit actor/session;
2. select an available worker pane/harness;
3. dispatch a worker brief;
4. record task, actor, pane/run linkage, delivery status and errors; and
5. show confirmed state in SCORE and STAGE across detach/reattach.

Do **not** fake this with a visual pane, hidden provider proxy, unbounded
polling, unsupported terminal keystroke injection, or invented adapter
capabilities. Focused pane bytes remain raw and verbatim. A worker is only
shown as having received a task after delivery is confirmed. A worker missing
its executable or capability (for example `pi-m3` when no local `pi` exists)
must be explicitly unavailable, never silently selected or claimed as active.

Update the source skills, Codex AGENTS block, and shell/install propagation so
that Claude/Codex/Hermes launched with `ORC_SESSION` automatically re-orient,
see the configured worker offer, and use this delegation path when the user
asks to use `pi-orchestra`. The worker pool remains an offer, not an
assumption. Do not modify a user's existing global configuration beyond the
installer's safe owned blocks/symlinks.

Add temporary-Git, fake-worker, daemon protocol, failed/missing-worker,
confirmed-delivery, task history, SCORE/STAGE linkage, and detach/reattach
tests. Dogfood this with Claude as the brain and Hermes as a worker. Do not
implement provider adapters, API proxying, or fake exact usage here; those are
Phase 5.

## Phase 4B — friendly first launch and Phase 4 visuals

The initial `pi-orchestra` screen must feel welcoming rather than like a bare
operator console. Improve HOME without adding a browser/UI framework:

- render a deliberate, compact ASCII/glyph title treatment for
  `PI ORCHESTRA` on first launch, using only theme tokens and no emoji;
- make the empty state plainly explain the next action (`n` to create a
  session), what a brain and worker are, and that workers are editable offers;
- keep a short, always-visible bottom key legend appropriate to the active
  view (HOME, STAGE, SCORE, RUNS);
- implement `?` as an in-app help overlay/page with the first-use flow,
  leader key, detach/reattach, task board, worker availability, and recovery
  instructions; escape or `?` closes it;
- use clear plain-language errors when an executable is missing or a worker
  cannot receive a task;
- preserve all raw focused-pane input and do not cover pane cells with chrome.

Also complete the approved Phase 4 scope: event stream directional baton
pulses with per-kind shape/tempo and static/reduced-motion degradation,
session-open settling where justified, view transitions, reduced-motion
behavior, and the real event-driven RUNS ledger port. Do not claim animation
or measurements not demonstrated by tests/evidence.

Provide reproducible TestBackend snapshots for ember and phosphor at both wide
and exactly `72x30`, including first-launch HOME, help, unavailable worker,
confirmed delegation, SCORE, STAGE, and RUNS. Capture/inspect real terminal
evidence where practical; do not overclaim macOS process-only observations as
image evidence.

Measure and record Phase 4 performance honestly: idle CPU, event-driven
refresh behavior, bounded refresh under worker output, and any changed
input/render latency. Keep network/subprocess work off the render path.

## Phase 5 — verified adapters, docs, and dogfood

Only after every Phase 4 acceptance test and evidence item is green:

- inspect local command help and actual capabilities before implementing each
  adapter; Hermes is first;
- implement adapters with honest capability flags and degradation; never fake
  exact usage, steering, or headless delivery;
- make `orc run` operational only when a local `pi` executable/configuration
  is genuinely available; otherwise document the blocker and keep the worker
  visibly unavailable;
- update README and guide with actual install, first-use, workers, control,
  recovery, and availability instructions;
- run the approved dogfood workflow from the Bench, record an honest friction
  log, and include it in the evidence.

## Verification and shipping

Run, with isolated target directories:

```sh
cd rust
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo fmt --check
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo clippy --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo test --locked
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo build --locked --release
```

After recording the gate results and before ending the session, remove this
isolated build directory to reclaim its temporary artifacts:

```sh
rm -rf /tmp/pi-orchestra-v4-final
```

Do not remove the repository's `rust/target`, any evidence, or any other
temporary directory; this cleanup is limited to the exact isolated target
directory above.

Also verify no Python runtime/package/test plumbing, production
`orc-core`/`orcd` unwrap/expect absence, installer idempotence and an actual
isolated-HOME install/uninstall, protected checksums, reproducible visual
evidence, and `git diff --check`.

Write a new dated Phase 4/5 evidence note with exact commands/results,
snapshots/screenshots, measurements, caveats, worker-capability proof, commits,
and dogfood friction. Update `README.md`, `docs/guide.html`, `progress.md`, and
`task_plan.md` honestly. Push the branch, merge to `main` only after all gates
pass, push `main`, then prove local HEAD, `origin/main`, and
`git ls-remote origin refs/heads/main` are identical.

End with shipped, intentionally cut, residual risks, and stop.
