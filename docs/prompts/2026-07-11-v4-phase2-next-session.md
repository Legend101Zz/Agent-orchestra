# Next-session prompt — pi-orchestra v4 "Bench" Phase 2 only

Copy everything below the line into a fresh session started in
`/Users/comreton/Desktop/pi-orchestra`.

---

You are implementing **only Phase 2** of the approved pi-orchestra v4 Bench
design. Phases 0 and 1 are complete, gated, committed, pushed, and must not be
redone. Phase 1 chose the embedded-PTY path; companion mode was not triggered.
Do not start Phase 3 tasks/worktrees/SCORE work in this session.

## Establish ground truth first

1. Confirm `cwd=/Users/comreton/Desktop/pi-orchestra` and branch
   `v4-bench`. Fetch without changing user files, then prove local HEAD and
   `origin/v4-bench` contain the remote-verified Phase-1 commit `91624a0`.
   Never force-push.
2. Run `git status --short --branch`. Preserve the pre-existing untracked
   `findings.md`; do not add, edit, delete, or commit it.
3. Read completely, in this order:
   - `docs/superpowers/specs/2026-07-11-pi-orchestra-v4-bench-design.md`
   - `docs/notes/2026-07-11-tui-spike.md`
   - `progress.md` and `task_plan.md`
   - `rust/crates/orc-proto/`, `orc-pty/`, `orc-daemon/`, and `orc-app/`
   - `rust/crates/orc-core/`, `orc-cli/`, and the existing Rust tests
   - the Python implementation, Python tests, `install.sh`, and
     `uninstall.sh` before deleting anything
4. Run the current baseline gates: 92-test Python suite, Rust fmt, clippy
   `-D warnings`, all Rust tests, and warning-free rustdoc. If the baseline is
   unexpectedly red, diagnose before editing and record the exact delta.

The Phase-1 verdict is authoritative: `vt100` remains selected, real Claude
and Hermes fidelity passed, event-driven idle reached 0.0% CPU, and visible
input reached p99 6.676 ms only after compact snapshots. Do not reopen the
parser decision without a failing real terminal-sequence fixture. Raw-byte
passthrough for the full kitty keyboard protocol is still open and must be
closed in this phase; the spike's decoded/re-encoded common-key path is not
the final prime-directive implementation.

## Non-negotiable rules

- All work stays on `v4-bench`. Small conventional commits; push verified
  slices. Stop after the Phase-2 commit/push/gate and leave Phase 3 for the
  user/next session.
- Never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`, or `~/.local/bin/orc` during development. Record
  checksums before/after the phase.
- The client never writes registry/session/layout files directly. Every
  mutation goes through daemon/orc-core command paths. Registry and session
  JSON remain plain, additive-field tolerant, and atomically written with
  temp + flush + `sync_all` + rename.
- No API-traffic proxying. Harness provider traffic stays untouched.
- Focused panes receive raw input bytes, including kitty extended keys,
  bracketed paste, and mouse. `ctrl-g` is the only leader; double-tap sends a
  literal control-G. Chrome never overlays pane cells.
- Ember and phosphor only. State is written as words. No emojis, raw ad-hoc
  colors, stock-ratatui box grids, web UI, Tauri, or browser surface.
- Bounded memory/concurrency everywhere: pane count, client count, grid,
  scrollback, socket messages, event queues, log tails, replay buffers, and
  output coalescing. No busy loops.
- `#![warn(missing_docs)]` plus rustdoc on every public item/module in new or
  touched v4 crates; `cargo doc --no-deps` must be warning-free. No
  `unwrap`/`expect` in orcd/orc-core outside tests. `thiserror` in libraries,
  `anyhow` in binaries, `tracing` in the daemon. TDD for core logic.
- Worker/delegated output is untrusted. Verify it against files/tests. The two
  Phase-1 delegated reviews ran away and returned no report; prefer local,
  bounded work. If delegation is genuinely needed, explicitly set
  `ORC_BRAIN=codex`, apply a kill-after timeout rather than TERM-only timeout,
  never retry more than once, and relay any `ORC WARNING:`/`ORC BLOCKED:` line
  verbatim.

## Phase 2 deliverables — implement all, in this order

### 2A. Capture the compatibility oracle before Python deletion

Do this before removing or changing the Python implementation:

1. Capture deterministic golden fixtures from the live Python CLI/registry
   behavior, not hand-written approximations. Include:
   - current, legacy/missing-optional-field, exact-usage, killed, orphaned,
     RPC `agent_end`, session-linked, retry, and handoff metas;
   - corrupt/truncated JSON siblings;
   - CJK, combining-mark, and wide-character task/name/session values;
   - normalized `list --json`, `show`, `stats --json`, quota-cache, and
     Python↔Rust round-trip outputs/exit behavior.
2. Store the immutable corpus under a Rust-owned fixtures path with a README
   explaining capture commands, normalization, and invariants. Golden files
   replace Python as the parity oracle forever.
3. Add Rust tests that consume every fixture, preserve unknown additive
   fields, tolerate legacy/corrupt/CJK data, and compare meaningful JSON/exit
   structure rather than incidental whitespace or timestamps.
4. Port the fake-pi integration suite to Rust test helpers/binaries. Cover
   JSON and RPC modes, one `agent_end` per delivered turn, one-time prompt
   acknowledgement, exact usage, signals/kill, idle timeout 124, context
   exhaustion, retry/handoff linkage, quota warn/block/unknown fail-open, and
   cross-language fixtures while Python still exists.

Gate and commit this slice before deletion. If fixture parity is not green,
do not delete Python.

### 2B. Productionize `orcd` and the protocol

Turn the Phase-1 spike into the per-user daemon:

- Socket: `~/.orchestra/orcd.sock`, parent permissions private, stale-socket
  handling safe, protocol version/mismatch explicit, maximum clients enforced.
- Start on demand from the first `pi-orchestra` invocation; support attach and
  detach without ending panes. Multiple clients may attach at different sizes.
- Log with structured `tracing` to `~/.orchestra/orcd.log` using a bounded
  rotation/retention policy. Normal detach is not a warning.
- Persist plain additive session/pane/pid records atomically. On daemon
  restart, validate pid identity/process group before reaping recorded
  children; never kill an unrelated reused PID. Record and test the exact
  restart/reap invariant.
- Keep canonical vt screen state and bounded replay per pane. Add output
  coalescing/backpressure metrics so the four-pane flood remains bounded.
- Add protocol round-trip, malformed/oversized message, permissions,
  multi-client, detach/replay, pid-record/restart, and flood/soak tests.
- Run the required hours-long flooding-pane soak before closing Phase 2;
  record duration, producer shape, CPU/RSS start/peak/end, dropped/coalesced
  updates, and result in `docs/notes/`. Do not shorten it and call it an
  hours-long soak.
- Close the lost-wakeup and raw-input paths with regression tests. Preserve
  synchronized-output frames and truly event-driven idle behavior.

No Tokio unless the implementation proves threads cannot meet correctness;
if added, document the concrete reason in the Phase-2 notes.

### 2C. Client shell, HOME, and production STAGE

Build the client surfaces specified by the approved design:

- **HOME** session shelf with useful empty state and a three-step new-session
  flow: choose brain → choose worker pool → choose cwd. Default worker choices
  are Hermes + pi/MiniMax-M3, visibly preselected but always user-editable;
  Claude, Codex, and configured harnesses remain valid.
- Add a plain atomic harness registry with command, args, `resume_args`, roles,
  adapter name, `default_workers`, max workers, leader key, reduced motion,
  and theme. Unknown fields survive.
- **STAGE** production compositor: floating arc-corner cards, half-block
  shadows, brass focus edge-light, ensemble layout, keyboard swap, mouse drag,
  resize, zoom-to-solo, focus routing, and per-session layout persistence.
  The client invokes daemon/core mutation commands; it never writes layout or
  session JSON itself.
- Export `ORC_SESSION` and `ORC_PANE_ID` in every stage-launched pane so new
  `orc` runs carry origin attribution. Add end-to-end tests proving it.
- Implement raw passthrough with kitty keyboard protocol, bracketed paste, and
  content-relative mouse forwarding in Ghostty and kitty. Preserve the
  <16 ms input budget and 0.0%-class idle behavior; remeasure rather than
  copying Phase-1 numbers.
- `pi-orchestra attach` reconnects and replays. `orc top` routes into the new
  client's RUNS view shell; the full v3 RUNS port remains Phase 4, so label any
  Phase-2 placeholder honestly.
- Test HOME and STAGE with TestBackend in ember and phosphor, wide and exactly
  72×30. Capture/inspect VHS evidence for HOME, new-session flow, STAGE,
  detach/reattach, resize, and zoom. Also exercise real Ghostty + kitty; if
  macOS denies exact-app screenshots again, record process/socket evidence as
  process evidence, not image evidence.

### 2D. Conductor-down recovery

- Detect a dead brain without ending the session or workers. Preserve its last
  screen and show the words `CONDUCTOR DOWN` plus elapsed time.
- `R` respawns the configured brain with its harness `resume_args`, same cwd,
  `ORC_SESSION`, and `ORC_PANE_ID`. Never invent resume support for a harness
  whose registry entry has none; state the limitation plainly.
- Re-orientation state remains durable in session/registry/inbox files. Add
  tests for supported resume, unsupported resume, worker survival, last-screen
  retention, repeated crash, and daemon restart around a dead conductor.

### 2E. Delete Python and switch install/uninstall to Rust only

Only after 2A–2D and their parity gates are green:

1. Delete `orc_pkg/`, `.venv`, `pyproject.toml`, `requirements.txt`, Python
   pytest plumbing/tests, and Python-only demo/seed helpers. Do not delete the
   captured Rust golden fixtures or evidence docs.
2. Remove Python dependencies and stale fallback language from README, skills,
   scripts, and CI/config. There is one Rust implementation after this point.
3. Make `install.sh` Rust-only: locked release build, install/symlink `orc`,
   `orcd`, and `pi-orchestra` safely with backups/marked blocks as appropriate.
   Update `uninstall.sh` for all three binaries while preserving
   `~/.orchestra` data by default.
4. Test install/uninstall in an isolated temporary HOME. Do not change the
   user's live `~/.local/bin/orc` during development or tests.
5. Prove the Rust CLI against every golden fixture and ported fake-pi test after
   Python is gone. Search the repository for stale `python`, `.venv`, pytest,
   Textual, and Python-default claims; remove only genuinely stale references
   while preserving historical design/review evidence when clearly labeled.

## Phase-2 gate and handoff

Before claiming Phase 2 complete:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test` including golden, fake-pi, daemon protocol/restart, HOME/STAGE,
  recovery, and install tests
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`
- release build with `--locked`
- no `unwrap`/`expect` in orcd/orc-core outside tests
- protected-file checksums unchanged
- isolated-HOME Rust-only install/uninstall pass
- real Ghostty + kitty, ember + phosphor, wide + 72×30
- inspected VHS/screenshots and recorded latency, idle CPU, flood metrics, and
  the actual hours-long soak
- no Python runtime/package/test plumbing remains

Update `progress.md`, `task_plan.md`, README/guide as required, and write a
Phase-2 evidence note with exact commands/results. Commit small verified
slices, push `v4-bench`, and prove `origin/v4-bench` points to the final Phase-2
commit. End with shipped/cut/risks and the next blocker. **Stop there. Do not
implement or plan Phase 3 in code, and do not merge `v4-bench` to `main`.**
