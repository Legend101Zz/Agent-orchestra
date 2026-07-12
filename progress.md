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

## Phase 1 — PTY / daemon / client spike
- Built new `orc-proto`, `orc-pty`, `orc-daemon`, and `orc-app` crates plus a
  standalone reproducible VT parser bake-off. New crates enable missing-docs
  warnings and use typed library errors / contextual binary errors.
- Verdict: GO with embedded PTYs; companion-mode fallback was not triggered.
  Real Claude Code 2.1.198 and Hermes 0.18.0 TUIs render with Unicode, color,
  alternate-screen state, resize/reflow, detach/reattach replay, and stable
  child PIDs. Full evidence is in
  `docs/notes/2026-07-11-tui-spike.md`.
- Parser results: vt100 165.0 MiB/s (selected, replayable), termwiz 233.1
  MiB/s (rejected, no screen state), alacritty-terminal 238.5 MiB/s
  (replayable fallback). Rejected-parser dependencies are isolated from the
  production workspace lockfile under `rust/spikes/vt-bakeoff`.
- The first wire shape failed latency: 356,840-byte blank snapshots and about
  540 ms visible echo. Compact default-cell serialization reduced the fixture
  to 8,217 bytes; 100-sample PTY-input-to-visible replay is now p50 6.149 ms,
  p99 6.676 ms, max 6.750 ms. Socket p99 is 48 microseconds.
- Event-driven idle measured 0.0% CPU for daemon and client. Four unbounded
  `yes` panes measured daemon 56.2-75.5% CPU and client 4.8-7.2%, with stable
  RSS during the sample. Pane grids, scrollback, requests, attached clients,
  panes, and UI event queues are explicitly bounded.
- Inspected VHS evidence: wide ember, exact 72x30 phosphor, and four-pane
  flood captures. Ghostty (`TERM=xterm-ghostty`) and signed kitty 0.47.4
  (`TERM=xterm-kitty`) both held live socket attachments. macOS denied GUI
  screen capture, so exact-app screenshot evidence is not claimed.
- Alacritty was attempted for the second-terminal gate, but Homebrew warned
  and Gatekeeper rejected the cask. No bypass was attempted; it was removed
  and signed kitty used instead.
- Final concurrency audit found and fixed a lost-wakeup window by acquiring
  the shared output epoch before sequence comparison; a regression test now
  proves output wakes the blocking client without polling.
- The capped Phase-1 `deleg8` review run
  (`20260711-131222-review-the-current-phase-03a7`) ignored the 150-second
  TERM cap, returned no report, and was killed (exit -15; fallback estimate
  1,990,554 tokens). It made no repository edits and was not used or retried.
- Phase gates: fmt clean; clippy `-D warnings` clean; all Rust tests and
  warning-free rustdoc pass; Python remains at 92 passing tests. Raw-byte
  passthrough for every kitty extended key remains a mandatory Phase-2 item;
  the spike currently re-encodes common decoded crossterm keys honestly.

## Session boundary after Phase 1
- User explicitly stopped this session before Phase 2 because context was
  becoming large. No Phase-2 implementation was started.
- Remote `origin/v4-bench` is verified at `91624a0`; the worktree is clean
  except for the pre-existing untracked `findings.md`, which must be preserved.
- Resume only with
  `docs/prompts/2026-07-11-v4-phase2-next-session.md`. Finish, gate, commit,
  and push Phase 2, then stop without beginning Phase 3.

## Session 4 — 2026-07-11 (v4 Phase 2)
- Captured the live Python compatibility oracle first and committed it at
  `89379a0`; the immutable Rust fixtures and fake-pi suite cover legacy,
  corrupt, Unicode, lifecycle, exact-usage, RPC, quota, retry, and handoff
  behavior. Python and Rust parity gates were green before deletion.
- Productionized the bounded per-user daemon/protocol at `0cac6bf`: private
  socket, safe stale handling, exact restart/reap identity, client caps,
  multi-size attachment, replay, atomic records, rotating tracing log,
  output coalescing, lost-wakeup/raw-input regressions, and soak tooling.
- Shipped HOME, STAGE, raw focused-pane input, daemon-owned layout/session
  mutation, launch attribution, and conductor-down recovery at `15d00d2`.
  TestBackend and inspected VHS evidence cover ember/phosphor, wide/72x30,
  launch, resize/zoom, detach, and reattach.
- Measured release behavior: socket p99 42 µs; visible-input p99 4.363 ms;
  daemon/client idle 0.0% CPU. The four-pane flood ran 7,608 seconds; daemon
  CPU start/peak/end was 21.2/36.5/22.8% and RSS was
  31,168/52,672/33,520 KiB. The user interrupted final snapshot collection,
  so the evidence note records the captured 33,392 in-run coalescing count and
  the missing post-run metrics total explicitly.
- Kitty 0.47.4 held an active isolated Phase-2 socket. Ghostty launched as a
  signed app process, but macOS did not spawn the helper and local UI control
  refused Ghostty access; only process evidence is claimed for Ghostty.
- Removed the Python runtime/test/package stack and switched install/uninstall
  to the three Rust binaries at `69a8a40`. Actual isolated-HOME install and
  uninstall passed while preserving `~/.orchestra`.
- Final isolated-target gates passed: fmt, clippy `-D warnings`, all tests,
  warning-free rustdoc, locked release build, no runtime Python plumbing, and
  no `unwrap`/`expect` in daemon/core production code. All four protected
  checksums are exact; `findings.md` remains the only unrelated untracked file.
- Phase 2 is complete. Phase 3 was not started and `v4-bench` was not merged
  to `main`.

## Session boundary after Phase 2
- Remote `origin/v4-bench` was proven equal to local HEAD at the Phase 2
  evidence commit `63a9b39841918bc0551edec0e847976ee3b53945`.
- The worktree was clean except for the preserved pre-existing untracked
  `findings.md`; all protected checksums matched their pre-phase values.
- Resume only with
  `docs/prompts/2026-07-11-v4-phase3-next-session.md`. Implement, gate,
  evidence, commit, and push Phase 3, then stop without beginning Phase 4.

## Session 5 — 2026-07-11 (v4 Phase 3)
- Audited the three existing Phase 3 commits rather than trusting their broad
  tests. The CLI exposed `task diff` and `task merge` but rejected both; they
  now exercise the core lifecycle and a real binary integration test proves
  additive JSON, diff statistics, explicit squash merge, and pruning.
- Hardened worktree ownership: a symlinked worktree root is refused before Git
  can write outside the owned root, and isolation history retains the actual
  human/brain actor.
- SCORE now renders review diff/token/history/dependency/error context,
  supports adjacent keyboard moves and SGR mouse drag moves via daemon/core as
  `human`, focuses the assigned STAGE pane with `g`, and returns with
  `ctrl-g b`. TestBackend covers ember/phosphor at wide and 72x30.
- Installer tests now prove idempotence, no duplicated owned blocks, user skill
  survival, and removal only of owned symlinks. Builds default to an isolated
  install target instead of the live repository target.
- Hermes local help was inspected. It did not demonstrate an AGENTS.md
  equivalent, so no Hermes instruction block was installed; this is documented
  in the source AGENTS block.

## Session 6 — 2026-07-11 (v4 Phase 4)
- Started a fresh `v4-phase4` branch from remote-verified `origin/main` at
  `a685133`; the older requested `69da971` was verified as its ancestor and the
  user authorized current remote main.
- Added bounded confirmed brain-to-worker delivery through core, daemon
  protocol, and `orc dispatch`; task history and pane linkage now distinguish
  confirmed receipt from durable failure across detach/reattach.
- Bench brains start with session, pane, worker-offer, and delegation-hint
  environment. Source skills, owned AGENTS block, shell helper, and installer
  propagation teach the explicit workflow.
- Shipped the first-launch title and teaching empty state, help page, active
  view legends, confirmed STAGE label, per-kind baton profiles, reduced-motion
  degradation, and the RUNS ledger port. TestBackend covers both themes at
  150x44 and exactly 72x30.
- Actual Claude produced a bounded brief and actual Hermes `-z` returned the
  requested dogfood sentinel. No adapter or exact usage claim was made.
- Phase 4 release measurements: socket p99 16 us; visible-input p99 3.628 ms;
  settled daemon 0.0% CPU; a 20,000-line burst coalesced 19,819 of 19,825
  output generations across 20 snapshots.

## Session 7 — 2026-07-12 (v4 Phase 5)
- Started `v4-phase5` from remote-verified `origin/main` at
  `43c0c5463d13b2e6a7ad4978a6e8ea6aa88e1313` while preserving the local Ghostty
  repair, updated next-session prompt, and untracked `findings.md`.
- Verified Hermes help before implementation: `-z/--oneshot` is a bounded
  non-interactive path. Verified pi help and `pi --list-models minimax`, then
  completed a real no-tools MiniMax M3 probe. The new typed adapter summary
  exposes only Hermes and pi capabilities: Hermes delivery only; pi delivery,
  RPC steering, and conditional exact usage. Claude/Codex remain explicitly
  best-effort interactive panes.
- Fresh registries now declare pi `-p --no-session` dispatch. Existing user
  registries are never rewritten; `orc adapter list` reports their unavailable
  delivery path until the user opts in with locally verified arguments.
- Ran an isolated real Bench dogfood: a clean worktree base allowed a
  worktree-isolated SCORE task, linked to a running Hermes pane. The durable
  dispatch returned exit 0 and `HERMES_DOGFOOD_OK`, then wrote
  `delivery_confirmed`. The original dirty working checkout correctly refused
  isolation. A raw `orchestrate` prompt reached the Hermes brain, but it spent
  the bounded trial inspecting the global orchestra home instead of performing
  the requested task; the human completed the explicit board path and the
  friction is recorded rather than attributed to the brain.
- A real release `orc run` completed with exact MiniMax usage (1,576 total
  tokens and $0.000440). The quota warning was relayed verbatim. Full commands,
  constraints, and gate results are in the dated Phase 5 evidence note.

## Session 8 — 2026-07-12 (polish + real-use pass)
- Committed pending prompt/tool fixes (1bd0de8) and pushed to main.
- Rebuilt HOME as an animated masthead: sparkle avatar frames, shimmer sweep
  over the title, rounded card, styled flow/shelf with brass selection;
  ambient 120 ms tick only on HOME, static under reduced_motion. All gates
  green; pushed as 22f7dda. Live tmux check confirmed the animation and
  0.4% client CPU while animating (daemon 0.0%).
- Reinstalled via ./install.sh (links now 12 Jul 10:16); removed the stale
  ~/.local/bin/orc.pi-orchestra.bak that pointed at the deleted Desktop copy.
- Dogfood: fresh ORC_HOME=/tmp/orc-bench-demo, temp git project
  /tmp/orc-demo-project with SPEC.md (stdlib todo API). Session
  orc-demo-project-1783831681-0000 launched through the real HOME flow with
  claude brain + hermes + pi-m3 workers; Claude trust prompt accepted through
  the focused pane. T0001→hermes (server.py), T0002→pi-m3 (test_server.py)
  dispatched concurrently. Quota at dispatch: 71% five-hour / 14% weekly,
  level warn (relayed).
- Both dispatches confirmed (exit 0): hermes wrote server.py, pi-m3 wrote
  test_server.py; `python3 test_server.py` printed PASS on first run. Tasks
  moved through review to done on the board.
- Captured real screenshots via VHS into docs/: home-welcome.{gif,png},
  home-flow.png, home-shelf.png, stage-workers.png, score-board.png.
- Bugs logged in findings.md: B1 flaky watcher test, B2 SCORE last-column
  clipping, B3 ctrl-g h dead on SCORE, B4 cwd editor UX, B5 $TMUX/TERM env
  leak into panes (pi warned about tmux extended-keys inside an orcd PTY).
- README rewritten (purpose, architecture, install, guide, verified
  capabilities, keys, performance, troubleshooting) with the real captures;
  pushed as f443ca1. Session pushed three commits to main:
  1bd0de8, 22f7dda, f443ca1. Demo daemons and tmux sessions torn down;
  stale phase-4/5 temp daemons killed.

## Session 9 — 2026-07-12 (input routing + live media)
- Root-caused the "Shift+V freeze": bare ?/V were intercepted as raw bytes
  before any view logic, so typing /Volumes/... in the launch flow jumped
  into the RUNS embed, which only answered literal V/q and had no legend.
  Fixed: view-aware ?/V (never in STAGE or during the flow), RUNS parses
  keys (V/h/Esc HOME, q quit) and shows a legend, SCORE gained leader
  handling (h HOME, v RUNS, ? help, q quit).
- Leader chord now configurable end-to-end: registry app.leader_key →
  daemon Home response (serde default ctrl-g) → client validation with a
  reserved-byte blocklist → RawRouter + dynamic legends/help. New leader
  actions: v (views) and ? (help). Gates green; pushed 911b9b8.
- Verified live in tmux: "/Volumes/Test?Vol" typed literally into the cwd
  field; RUNS legend renders; h exits RUNS to the animated HOME.
- Re-ran a real session (claude + hermes + pi-m3), dispatched a bounded
  hermes brief (DEMO_OK, confirmed), recorded docs/stage-live-dispatch.gif
  (typing + focus hops + baton pulses) and docs/stage-live.png showing
  HERMES · TASK CONFIRMED. Replaced stage-workers.png in the README.
