# pi-orchestra v4 Bench Phase 2 evidence

**Date:** 2026-07-11  
**Branch:** `v4-bench`  
**Scope:** Phase 2 only; Phase 3 was not started.

## Ground truth and protected state

The session began in `/Users/comreton/Desktop/pi-orchestra` on `v4-bench` at
`59b2e4aae85b599c7eb914f946f67454de974128`. Local and
`origin/v4-bench` both contained the remote-verified Phase 1 commit `91624a0`.
`git status --short --branch` showed only the pre-existing untracked
`findings.md`; it was not read, changed, staged, or committed.

Protected checksums before Phase 2:

```text
33ecb4a6c902fdacfb6085c7673438d1b95777413dc19a68e6c7bcb6ddaa7ce3  ~/.pi/agent/settings.json
83709fd8b25ad3f656aa3e1fd0860a26d939239a3ec610e4ed88ec077bdba491  ~/.claude/settings.json
e5b9f8acd517ac565d8b7a1a87c4b8b446a453f2bd8ec817d3141f6a18eea461  ~/.codex/config.toml
d230ae0ba42c1bd9283609df839f0b605d7db3cd92fc0e3bb0b18b4c946691d5  ~/.local/bin/orc
```

The same four checksums were reproduced at the final gate. The last path is a
pre-existing symlink into `rust/target/release/orc`; release builds temporarily
changed the followed target bytes without changing the symlink. The original
Phase 1 target was rebuilt at the same repository path and the branch was
returned to `v4-bench`, restoring the exact checksum. Final verification builds
therefore used an isolated `CARGO_TARGET_DIR`.

## Baseline and shipped slices

Before editing, the Python suite passed all 92 tests. Rust formatting, clippy
with `-D warnings`, all tests, and rustdoc with `-D warnings` were green.

Phase 2 was split into independently gated conventional commits:

- `89379a0 test: capture Python compatibility oracle`
- `0cac6bf feat: productionize Bench daemon`
- `15d00d2 feat: ship Bench HOME STAGE and recovery`
- `69a8a40 refactor: remove Python runtime and install Rust only`

No Tokio dependency was added; bounded threads and blocking socket waits met
the correctness and idle requirements.

## 2A — compatibility oracle before deletion

`tools/capture_phase2_compat.py` captured the live Python behavior before any
Python removal. The immutable Rust-owned corpus is
`rust/crates/orc-core/tests/fixtures/python-v3/`. It includes current,
legacy/missing-field, exact-usage, killed, orphaned, RPC `agent_end`,
session-linked, retry, handoff, corrupt, truncated, CJK, combining-mark, and
wide-character cases plus normalized list/show/stats/quota outputs and exit
behavior. The fixture README records capture normalization and invariants.

Rust compatibility tests consume the complete oracle, tolerate corrupt and
legacy siblings, and verify that unknown additive fields survive. The Rust
fake-pi integration suite covers JSON and RPC, one `agent_end` per delivered
turn, one-time prompt acknowledgement, exact usage, signal/kill behavior, idle
timeout 124, context exhaustion, retry/handoff linkage, and quota
warn/block/unknown fail-open. The Python 92-test suite and all Rust parity tests
were green before the deletion gate was opened.

## 2B — daemon, protocol, and bounded operation

The production daemon uses a private per-user socket directory (`0700`) and
socket (`0600`), checks socket type/owner/listener before stale removal,
enforces protocol/version errors and client limits, and supports clients with
different requested sizes. Malformed and oversized requests return explicit
protocol errors. First client use starts the daemon on demand; detach leaves
PTY children running and attach replays bounded canonical state.

Registry, session, layout, and PID records are plain additive JSON written by
daemon/core paths with temp-file, flush, `sync_all`, and rename. Restart reap
requires the recorded PID, process group, process start identity, and command
to match; tests prove a reused unrelated PID is not killed. Structured daemon
logging is bounded by rotation and retention, and normal detach is not logged
as a warning.

Regression coverage includes protocol round trips, mismatch/malformed/size
errors, permissions, client caps and multi-client sizing, detach/replay,
atomic records, exact restart/reap identity, bounded output coalescing, compact
snapshots, synchronized output, lost wakeups, raw PTY bytes, and a four-pane
flood.

### Required two-hour flood soak

The production soak ran the required four panes, each repeatedly emitting
1,024 lines and then pausing 50 ms. The CSV contains 615 process samples and
its final sample is at **7,608 seconds** (2 h 6 m 48 s), so the required
two-hour flood duration passed.

| Point | daemon CPU | daemon RSS |
|---|---:|---:|
| start | 21.2% | 31,168 KiB |
| peak | 36.5% | 52,672 KiB |
| end (7,608 s) | 22.8% | 33,520 KiB |

The captured metrics snapshot already showed 471,380 bytes in 35,742 output
chunks, four emitted snapshots, and **33,392 coalesced updates** across the
four panes with one attached replay watcher. Canonical PTY bytes/screen state
were not dropped; intermediate client updates were coalesced under pressure.
RSS finished only 2,352 KiB above its start and 19,152 KiB below its observed
peak, so the run showed no unbounded memory growth.

The user accidentally interrupted the harness after the 7,608-second sample
while its final `snapshot-once` redirection was in progress. Consequently the
duration and CPU/RSS resource gate is a pass, but `end-snapshot.json` is empty
and there is no post-run metrics total; the coalescing count above is the
captured in-run metrics value, not a fabricated final count. Unit/integration
flood tests separately prove bounded canonical state and coalescing behavior.

## 2C — HOME, STAGE, raw input, and measurements

HOME provides a durable session shelf and the required brain → editable worker
pool → cwd flow. Hermes and pi/MiniMax-M3 are visibly preselected defaults;
Claude, Codex, and configured harnesses remain valid. The additive atomic
harness registry includes command, args, `resume_args`, roles, adapter,
default workers, max workers, leader key, reduced motion, and theme. Only
`ember` and `phosphor` are accepted.

STAGE renders floating arc-corner cards, half-block shadows, brass focus,
ensemble layout, focus, swap, mouse drag, resize, and zoom-to-solo. Layout
mutations go through daemon/core commands and persist per session. Every
launched pane receives `ORC_SESSION` and `ORC_PANE_ID`. `attach` reconnects and
replays. `orc top` opens a plainly labelled RUNS shell placeholder; the full
RUNS port remains outside Phase 2.

The focused-pane input router preserves raw kitty extended-key sequences and
bracketed paste, translates mouse coordinates relative to pane content, and
uses `ctrl-g` as the only leader; a double tap forwards a literal control-G.
SIGWINCH drives resize without polling.

Release measurements on the development M-series Mac:

- Unix-socket round trip, 5,000 samples: p50 13 µs, p95 17 µs, p99 42 µs,
  maximum 72 µs.
- PTY input to visible replay, 100 samples: p50 3.770 ms, p95 4.000 ms,
  p99 4.363 ms, maximum 4.601 ms.
- Five idle samples: daemon 0.0% CPU and client 0.0% CPU; observed RSS was
  6,832 KiB and 2,848 KiB respectively.

TestBackend assertions cover both themes, wide dimensions, and exactly 72×30.
The inspected captures are:

- `docs/v4-phase2-shell.gif`: HOME, three-step flow, wide ember STAGE,
  zoom/swap, detach, and reattach.
- `docs/v4-phase2-narrow-phosphor.gif`: flow and STAGE at exactly 72×30 in
  phosphor, including zoom.

Real kitty 0.47.4 launched the Phase 2 app and held an active isolated
`orcd.sock`, confirmed by process and Unix-socket inspection. The signed
Ghostty app launched with the Phase 2 helper command, but macOS Ghostty did not
spawn that child through either `-e` or `--command`; no Phase 2 socket existed.
The local UI-control capability also refused Ghostty access for safety. This is
therefore recorded only as Ghostty process evidence, not as image or active
socket evidence. Visual assertions come from the inspected VHS captures.

## 2D — conductor-down recovery

Brain death preserves the session, workers, and last screen and renders the
words `CONDUCTOR DOWN` with elapsed time. `R` respawns only when the configured
harness has `resume_args`, using the same cwd, `ORC_SESSION`, and
`ORC_PANE_ID`; otherwise the client states `RESUME NOT SUPPORTED`. Durable
attention/reorientation state remains visible after restart. Tests cover
supported and unsupported resume, worker survival, last-screen retention,
repeated crashes, and daemon restart around a dead conductor.

## 2E — Rust-only runtime and isolated install

Only after 2A–2D were green, the Python package, virtual environment,
packaging, pytest plumbing/tests, and Python-only demo/capture/seed helpers
were deleted. The compatibility fixtures remain. Runtime documentation and
the guide now describe one Rust implementation; older design/review documents
retain historical language for auditability.

`install.sh` performs a locked Rust release build and safely links `orc`,
`orcd`, and `pi-orchestra`; `uninstall.sh` removes all three while preserving
`~/.orchestra` by default. The install integration test and an actual
install/uninstall run passed in an isolated temporary HOME and bin directory,
using the existing toolchain roots and never the user's live bin directory.

## Final gate

The final gate was run with an isolated target directory:

```sh
cd rust
CARGO_TARGET_DIR=/tmp/orc-phase2-final-target cargo fmt --check
CARGO_TARGET_DIR=/tmp/orc-phase2-final-target cargo clippy --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/orc-phase2-final-target cargo test
CARGO_TARGET_DIR=/tmp/orc-phase2-final-target RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
CARGO_TARGET_DIR=/tmp/orc-phase2-final-target cargo build --release --locked
```

All five commands passed. The test run included the compatibility CLI/oracle,
fake-pi, isolated installer, daemon protocol/permission/client/restart/flood,
HOME/STAGE theme and dimension, raw input, mouse, attribution/layout, and
recovery suites. A repository search found no runtime Python files or package,
virtualenv, pytest, or Python test plumbing. An audit of `orc-daemon/src` and
`orc-core/src` found no `unwrap`/`expect` outside `#[cfg(test)]` sections.
`git diff --check` passed, the actual isolated-HOME install/uninstall passed,
and all protected checksums remained exact.

`findings.md` remained the only unrelated untracked path. Phase 3 code,
worktrees, tasks, and SCORE were not started, and `v4-bench` was not merged to
`main`.
