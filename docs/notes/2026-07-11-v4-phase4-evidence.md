# v4 Phase 4 evidence

**Date:** 2026-07-11

**Branch:** `v4-phase4`

**Scope:** Phase 4 only; Phase 5 was not started.

## Baseline and protected state

After `git fetch origin`, local `main`, `origin/main`, and
`git ls-remote origin refs/heads/main` all resolved to
`a685133ed2def3de94b36de5028f5b214655a91a`. The requested older commit
`69da971742e68ec23d3e456cfa74aa8fd82d3a8f` is its direct ancestor; the user
explicitly authorized using current remote `main`. The fresh branch was
`v4-phase4`; `v4-bench` was not reused.

Before work, `~/.codex/config.toml` actually hashed to
`d71ad59868ec7bb3e212dd5a847f91043ae56862062b56017112d48cb28fd00f`, while
the Phase 3 handoff recorded
`f0a989ad75b992ef16d4feb1ccba245634dc233aaaf05738362cac303b1ef31c`.
Neither value was treated as something to repair. No protected configuration
or live installed target was edited or built.

## Phase 4A control plane

The new bounded dispatch path is available through core, daemon protocol, and
`orc dispatch send/list/show`. It requires explicit actor, session, task, and
harness, accepts explicit pane/run linkage, and uses only declared
non-interactive harness capability. Local `hermes --help` proved `-z/--oneshot`
before that default was added. No PTY keystroke injection, adapter, provider
proxy, or exact-usage fabrication was added.

Prompts and captured output are capped at 16 KiB and every invocation has a
bounded timeout. Missing executable, missing capability, stopped/mismatched
pane, timeout, and harness failure are explicit. Only exit-zero confirmed
delivery writes `delivery_confirmed` and pane linkage into the durable task;
failures write `delivery_failed` without claiming receipt. The daemon control
sequence wakes attached clients, which refresh SCORE/STAGE across detach and
reattach.

Tests cover temporary Git, fake worker, missing executable/capability, bounded
prompt, confirmed delivery, actor/pane/run linkage, task history, protocol,
CLI, and daemon detach/replay. Bench panes now receive `ORC_SESSION`,
`ORC_PANE_ID`, `ORC_WORKERS`, and `ORC_DELEGATE_HINT`. Source skills, the owned
Codex AGENTS block, shell helper, and installer propagation teach the explicit
task add/assign/start then dispatch flow.

## Phase 4B and snapshots

HOME has a theme-token glyph title, next-action copy, brain/worker explanation,
and editable-offer language. `?` opens a help page covering first use, leader,
detach/reattach, SCORE, availability, and recovery; `?` or escape closes it.
HOME, STAGE, SCORE, and RUNS retain an always-visible active-view legend without
covering pane content.

The v3 RUNS ledger is embedded in the Bench client. Baton rendering has four
bounded profiles (settle, dispatch, complete, failed) with different tempo,
width, and direction; reduced motion suppresses its frame clock and leaves the
static filament. Session attach uses the settle profile. TestBackend coverage
renders ember and phosphor at 150x44 and exactly 72x30 for HOME, help, SCORE,
STAGE confirmed linkage, and RUNS. Missing/failed delivery is asserted through
core/protocol/client summaries rather than an invented successful pane.

## Dogfood and performance

Actual Claude `-p` generated a bounded worker brief for Hermes. Actual
`hermes -z` consumed it and returned exactly `PHASE4_DOGFOOD_OK`. Both calls
were bounded by a 120-second process timeout. This proves the real local
Claude-to-Hermes capability shape; the fake-worker suite separately proves the
durable control-plane bookkeeping deterministically. No exact usage claim is
made because adapters remain Phase 5.

Release measurements used
`CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-phase4-final-target` and an isolated
daemon/socket:

- 5,000 socket pings: p50 13 us, p95 15 us, p99 16 us, max 162 us.
- 100 visible-input samples: p50 3.365 ms, p95 3.544 ms, p99 3.628 ms,
  max 3.645 ms.
- Idle after settling: daemon 0.0% CPU, 7,536 KiB RSS.
- 20,000-line bounded burst: 268,890 bytes, 19,825 chunks, 20 snapshots,
  19,819 coalesced generations; sampled daemon 10.7% CPU and 14,112 KiB RSS.

The burst is evidence for bounded refresh/coalescing, not a replacement claim
for the Phase 2 two-hour soak. Network and worker subprocess calls remain off
the render path.

## Delegated review accounting

The required large-corpus read run
`20260711-201921-read-the-phase-4-require-77d0` finished with 150,953 exact
tokens and $0.022012, but carried `context_exhausted`; its map was used only as
an untrusted lead. The Phase 4A implementation run
`20260711-202307-implement-phase-4a-only-3304` finished with 144,495 exact
tokens and $0.0099015. Its patch was independently reviewed, corrected for
task linkage and pane availability, and retested.

## Acceptance commands

Every Cargo invocation used an isolated `CARGO_TARGET_DIR`. The final gate is:

```sh
cd rust
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-phase4-final-target cargo fmt --check
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-phase4-final-target cargo clippy --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-phase4-final-target cargo test --locked
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-phase4-final-target RUSTDOCFLAGS='-D warnings' cargo doc --no-deps
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-phase4-final-target cargo build --release --locked
```

The evidence note is completed only after those commands, isolated-HOME
install/reinstall/uninstall, source audits, protected checksum comparison,
merge, and remote-main proof pass. Phase 5 and Phase 6 remain unstarted.
