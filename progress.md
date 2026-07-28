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

## Session 10 — 2026-07-12/13 (Phase 6: stability + first-run UX)
- 6A (385dcbb): Welcome carries a serde-defaulted build identifier (crate
  version + compile-time git commit); a mismatched or pre-handshake daemon is
  refused with one actionable line. The old catch-all "invalid or oversized
  response" split into three honest messages; recoverable attach/resume
  failures land on the HOME/STAGE message line instead of exiting. New
  `orc daemon status` (exit 0/3/5) and `orc daemon restart` (refuses while
  live panes exist unless --force, lists what dies; pid discovery matches the
  daemon's --socket). install.sh probes the running daemon and prints restart
  guidance. Measured 3 fully-styled truecolor panes at 200x400 = 20.7 MB
  (62% of the 32 MiB cap): snapshots gained a session filter (the shell
  watch always uses it) and orcd replaces any over-cap response with an
  explicit bounded error. Focus reports (^[[I/^[[O) consumed outside STAGE.
  Reproduce-then-fix evidence against the actual pre-fix installed orcd.
- Small scope (dacd6e4): B5 pane env scrub (TMUX/TMUX_PANE/TERM_PROGRAM/
  TERM_PROGRAM_VERSION, regression-tested), B1 watcher-test deflake
  (rewrite-until-event, 10 s bound).
- 6B (c6afe21): RUNS embed keys now route into orc_tui::App (selection,
  expansion, session workspace, tabs, search, theme); documented exits
  reserved at the App dashboard only; view-aware honest legend fits 72
  cols; 500 ms ambient data-refresh tick fixes the frozen-screen symptom.
  TestBackend + live tmux smokes (captures in docs/notes/).
- 6C (e6bf1cd): wire gains harness available/dispatch_verified and session
  workers_live/workers_total/conductor (judged against hosted panes). HOME
  teaches brain/worker/detach with the configured leader chord and a BENCH
  AVAILABILITY strip; shelf cards show pane health with the R hint only
  where recovery applies. Cwd step (B4): tab completion, ctrl-u, tilde,
  live validation, refusal in place, brain/workers confirmation. SCORE (B2):
  ellipsis truncation inside a right gutter. README media re-captured via
  VHS from a real claude+hermes+pi-m3 session; README text updated.
- Session interrupted once by an external-SSD I/O failure (device stopped
  answering reads; fsck couldn't read the container superblock). All work
  up to 6A was already pushed; the user replugged the drive, git fsck clean,
  no data lost.
- Install refreshed via ./install.sh; the install-time check flagged the
  running old daemon; verified no live panes via ps (the old daemon predates
  pane reporting), then `orc daemon restart --force` brought the user's
  daemon onto the installed build (status exit 0, builds match).
- Observed, not caused by this session: ~/.codex/config.toml changed
  externally (mtime 2026-07-13 16:38; sha f0a989ad… → fdbc233c…). Recorded
  honestly per house rules; not restored. ~/.claude/settings.json and
  ~/.pi/agent/settings.json checksums unchanged.

## Session 9b — 2026-07-12 (triage + Phase 6 handoff)
- User reported: client exit with "daemon rejected request: invalid or
  oversized response" (with ^[[I noise), dead/frozen RUNS embed, and a
  ctrl-g system conflict. Diagnosis: the live ~/.orchestra orcd had been
  running 19.5 h on yesterday's binary (installs never restart it); the
  client error string also covers plain EOF; ^[[I is a FocusIn report;
  the RUNS embed never routes keys into orc_tui and never redraws between
  events. All three of the user's sessions were created with cwd=$HOME
  (bug B4) and had conductor_down brains.
- Actions now: set user registry app.leader_key to ctrl-b (verified live —
  STAGE legend shows ctrl-b and ctrl-b h returns HOME); stopped the stale
  daemon; fresh daemon on today's build verified healthy; wrote
  docs/prompts/2026-07-12-v4-phase6-ux-bugfix-next-session.md covering
  daemon build handshake + orc daemon status/restart, honest client errors,
  RUNS interactivity/redraw, first-run HOME redesign, cwd-step fix, SCORE
  clipping, env scrubbing, and gates.

## Session — 2026-07-22 (Claude Code / Fable): V1 program setup
- Pivoted plan of record from v4-Bench (complete) to V1 "Universal
  Delegation": spec at docs/superpowers/specs/2026-07-22-v1-universal-delegation-design.md.
- Imported visual identity spec (nocturne/ember/phosphor) into
  docs/design/visual-identity/ + distilled docs/design/visual-identity.md.
- Established multi-agent workflow (docs/WORKFLOW.md): code-puppy implements
  issues on branches, Claude reviews, Mrigesh tests + merges. Added root
  AGENTS.md, .agents/commands/work-issue.md, GitHub issue/PR templates.
- Rewrote task_plan.md (issue map) and findings.md (durable decisions).
- Next: create epic + 12 scoped issues on GitHub, fill issue numbers into
  task_plan.md.

## Session — 2026-07-22 (Claude Code): issue #16 foundations research
- Branch issue-16-research. Wrote the binding decision record
  docs/superpowers/specs/2026-07-22-v1-crate-and-prior-art-decisions.md:
  rmcp v2.2.0 (isolated tokio in new orc-mcp) for #8; probe-driven headless
  invocation templates (flags verified against local claude 2.1.217,
  codex 0.145.0, opencode 1.18.4, hermes 0.18.2, pi 0.80.7) for #4/#6;
  git-CLI worktree shell-out for #11; backon 1.6.0 for #7; schemars 1.2.1
  for #3/#5; insta 1.48.0 (dev-dep) for UI snapshots. Prior art mined:
  vibe-kanban executors (stream-json control protocol, codex app-server,
  opencode serve), claude-squad, hermes #344/#38952, togethercomputer/moa,
  OpenRouter Fusion (steal its consensus/blind-spot report format for V2).
- Six open questions listed in the doc (rate-limit signal strings, hermes
  one-shot resume, opencode serve-vs-run cost, etc.) — deliberately not
  researched further; timebox honored.
- Commented binding decisions on #3-#8 and #11; LOG.md status 👀 + ship-log
  entry. No code, no dependency changes.

## Session — 2026-07-23 (code-puppy): issue #17 rename orc→pio / orcd→piod
- Branch issue-17-rename-cli-pio off fresh main. Renamed the user-facing CLI
  to `pio` and the daemon binary to `piod`; the TUI stays `pi-orchestra`.
- Bin targets: orc-cli/Cargo.toml `orc`→`pio`, orc-daemon/Cargo.toml
  `orcd`→`piod` (the workspace rust/Cargo.toml has no bin targets, so the
  rename lives in the crate manifests — both inside allowed paths).
- Code: clap command names, `pio version` output, all help/hint/error/context
  strings in orc-cli/src/main.rs and orc-cli/src/daemon.rs. daemon.rs now
  spawns `piod` (with_file_name/PathBuf) and its pgrep discovery searches
  `piod` then `orcd` so `pio daemon restart` still finds a daemon started
  before the rename. orc-daemon/src/main.rs got the piod doc comment plus an
  explicit `#[command(name = "piod")]`.
- KEPT deliberately (compat, per the issue): crate names (orc-core…), ORC_*
  env vars, ORC WARNING/BLOCKED/NOTE markers, ~/.orchestra, and the
  socket/log filenames orcd.sock/orcd.log — renaming the socket would break
  the cross-version stale-daemon detection that install.sh's `pio daemon
  status` relies on, and the issue only allows piod.sock if that keeps working.
- install.sh: builds/links pio+piod+pi-orchestra, and a new retire_command
  backs up any prior orc/orcd once then drops a forwarding “renamed to pio”
  shim. uninstall.sh: removes the new links, removes our shims, restores the
  backup. Verified live in a throwaway HOME (backup + shim + forward + restore).
- Docs/integrations: README, codex/AGENTS-block.md, skills/*, shell helpers
  all say pio/piod now (KEPT tokens preserved). docs/guide.html left as-is:
  it is a dated historical artifact (“built 2026-07-10”, “Historical v3
  console reference”) and AGENTS.md says docs retain original labels for
  auditability; AC#2's gate scopes to help/README/skills only.
- Tests: retargeted CARGO_BIN_EXE_orc→_pio in 4 suites; extended install.rs
  for the shim/backup/restore migration; added tests/rename_gate.rs (AC#2
  grep gate over pio --help + README + skills, neutralizing kept tokens).
- Gates from rust/: fmt PASS, test PASS (89 passed / 0 failed / 32 suites),
  doc PASS, release build PASS. clippy: orc-cli/orc-daemon and all new tests
  are 100% clean; the only failures are 3 PRE-EXISTING lints in untouched
  files (orc-pty/src/lib.rs:159 while_let_loop, orc-core/src/dispatch.rs:499
  useless_borrows_in_formatting, orc-tui/src/app.rs:696 collapsible_match)
  that fire under the freshly-installed clippy 1.97.0 (repo MSRV is 1.91).
  Not my regressions and out of allowed paths, so left untouched and flagged
  on the issue.
- No Rust toolchain existed on this machine; installed via `brew install
  rust` (1.97.0) behind Walmart proxies to run the gates. Pushed the branch;
  human will open the PR.
- Follow-up (same PR, owner-approved scope expansion beyond #17's allowed
  paths): fixed the 3 pre-existing clippy 1.97 lints so the raw
  `cargo clippy --workspace --all-targets -- -D warnings` is green on modern
  toolchains. orc-pty/src/lib.rs loop -> `while let`; orc-core/src/dispatch.rs
  dropped a redundant `&` in a format arg; orc-tui/src/app.rs folded a
  bounds-check `if` into the match-arm guard. All behavior-preserving; full
  gate suite re-run with NO allow-flags: fmt/clippy/test(89-0)/doc/release all
  green. Cargo.lock unchanged. Kept as a separate commit from the rename.

## Session — 2026-07-23 (Claude, reviewer): issue #17 review → ACCEPT → merged
- Adversarial review of issue-17-rename-cli-pio (PR #19) per docs/WORKFLOW.md,
  run on the SSD checkout. All five gates re-run independently on MSRV Rust
  1.91.1: fmt / clippy (0 warnings, no allow-flags) / test 89-0 / doc /
  release build — all green (implementer had gated on 1.97, so both
  toolchain generations are now proven).
- Every acceptance check reproduced live, none trusted: real install.sh into
  a scratch HOME seeded with a pre-rename `orc` symlink (backup + shim +
  forward-with-nag verified), uninstall.sh restore verified by executing the
  restored binary, and an orc/orcd leak sweep across EVERY subcommand and
  sub-subcommand --help screen (broader than rename_gate.rs scans) — zero
  leaks. Cargo.lock/workspace root unchanged; the 3 out-of-path clippy fixes
  (b5025e9) read line-by-line and confirmed behavior-preserving +
  owner-approved. Verdict ACCEPT commented on #17; LOG.md 🧪 pushed (8cdf45c).
  Three non-blocking notes recorded on the issue (uninstall keeps a non-shim
  pre-rename orc link in one narrow upgrade path; backup-once policy;
  SUN_LEN probe quirk in absurdly long $HOME).
- Mrigesh tested locally and merged: PR #19 → main @ 846d74d; issue #17
  closed, epic #15 box ticked. Dashboards updated: LOG.md #17 → ✅ merged
  (PR #19) + "start the parallel set" note; task_plan.md order note updated.
- Next: parallel-safe set #3 / #5 / #9 / #13, one puppy terminal each,
  branching from fresh main (now includes the rename).

## Session — 2026-07-23 (code-puppy): issue #3 harness auto-discovery
- Branch `issue-3-harness-discovery` off fresh `main` (5a2ca74, post-#17-merge).
  Implemented V1-1: scan PATH for the extensible known set
  [claude, codex, hermes, pi, opencode] and persist an additive per-harness
  record (path, cheap version, first_seen, last_seen) in
  `~/.orchestra/harnesses.json`; surface it in `pio harness list` and the HOME
  availability strip.
- Design honored the fact that harnesses.json was ALREADY owned by
  bench.rs::HarnessRegistry. Extended it ADDITIVELY: new `DiscoveredHarness`
  struct + `#[serde(default)] discovered: BTreeMap<String,DiscoveredHarness>`
  field, both with `#[serde(flatten)] extra` so unknown fields at every layer
  round-trip untouched. Nothing existing was renamed or moved.
- New module orc-core/src/discovery.rs: `KNOWN_HARNESSES`, `discover(probe)`
  (scan + bounded `--version` probe via the now-`pub(crate)`
  quota::command_output_with_timeout, no duplicate timeout logic; additive
  upsert = first_seen set once, missing harnesses never deleted), and
  read-only `present_current()` for the strip. CLI got `pio harness list
  [--json]` mirroring `adapter list`.
- orc-app change kept to the availability-strip feature only: added a
  `discovered` field to HomeData, populated once on entry in
  BenchClient::home() via the READ-ONLY present_current() (respects the crate
  invariant "never write registry files" — discover() which writes lives only
  in the CLI), and rendered a "DISCOVERED ON PATH" block. No other screen or
  daemon/proto code touched (those are out of allowed paths).
- Tests (all 4 ACs): orc-cli/tests/harness_cli.rs — hermetic PATH with 3/5
  fake harness scripts proves all five are listed (found w/ paths, missing
  marked unavailable) [AC1] and a fixture-seeded registry proves additive
  round-trip (unknown fields at top/app/discovered layers survive, first_seen
  preserved, path/last_seen/version refreshed) [AC2]; orc-app HOME snapshot
  updated + new pure `availability_lines_render_discovered_section` unit test
  [AC3]; three orc-core discovery unit tests. tools/fixtures/
  discovered-harnesses.json added as the AC2 fixture.
- All five gates green on Rust 1.97 (brew) from rust/: fmt / clippy (0
  warnings, no allow-flags) / test 95-0 (was 89, +6) / doc / release build
  --locked. Cargo.lock unchanged. Live smoke test of the release `pio harness
  ` (+ --json) confirmed human output and that only found harnesses are
  persisted. One clarification noted on the issue: the orc-app edit touches
  HomeData + home() as the minimal plumbing to feed the strip (still
  "availability strip only" in spirit; no other UI/logic changed).
- Branch pushed; PR left for Mrigesh to open (per the #17 pattern).

## Session — 2026-07-23 (Claude reviewer): adversarial review of issue #3
- Reviewed `issue-3-harness-discovery` (PR #20) against the #3 contract on the
  SSD checkout. All 5 gates re-run green (95 tests, 0 failed); AC1/AC2/AC3
  reproduced live with the release `pio` against hermetic ORC_HOME/PATH
  fixtures; scope, deps (none), and additive JSON behavior all verified clean.
- Adversarial probes found one honesty bug: `probe_version` ignores exit
  status, so a failing `--version` persists its stderr error text as the
  recorded version (demonstrated twice, incl. a truncated sh error path).
  Verdict: **FIX** (2-item list on #3) — status back to 🔨, LOG.md verdict
  line pushed @ c428cc7.
- Next: code-puppy applies the fix + regression test (prompt 3), then
  re-review (prompt 4).

## Session — 2026-07-23 (code-puppy): issue #3 review fixes (round 2)
- Pulled the reviewer's commits (c428cc7 FIX verdict, 7625f2d progress) onto
  `issue-3-harness-discovery` before touching anything.
- Fix 1 (discovery.rs): `probe_version` now returns `None` unless
  `output.status.success()`, so a harness that rejects `--version` (non-zero
  exit) never has its stderr error text recorded as a "version". The existing
  `.or(stored_version)` fallback in `discover()` then keeps any prior version.
  Updated the fn doc to state the exit-status guarantee.
- Fix 2 (harness_cli.rs): added `failing_harness` helper (exit 1 + noisy
  stderr) and regression test
  `failed_version_probe_records_no_version_and_keeps_stored_fallback`: a
  fresh failing harness (claude) records NO version and shows "version
  unknown"; a failing harness with a seeded stored version (pi) keeps the
  stored fallback; and the registry file contains zero leaked error text.
- Verified live with the release binary: claude (fails --version, no stored)
  -> `version unknown` + `claude.version = None` persisted; pi (fails
  --version, stored `pi 0.0.1-preexisting`) -> stored fallback shown and
  kept; `grep -c "unrecognized option" harnesses.json` = 0.
- All 5 gates green from rust/: fmt / clippy (0 warnings) / test 96-0 (+1) /
  doc / release build --locked. Cargo.lock unchanged. No new features, no
  scope change — only the two numbered review items. Pushed; per-item
  evidence commented on #3; LOG.md #3 back to review + ship-log fix note.

## Session — 2026-07-23 (code-puppy): issue #4 — capability probe + `pio doctor`
- Started fresh from `main` (issue #3 / V1-1 confirmed merged, PR #20, so the
  `Depends on: V1-1` gate is satisfied); branch `issue-4-capability-probe`.
- New `orc-core/src/probe.rs`: probes each known harness by inspecting what its
  binary *advertises* — a bounded `--help` corpus scanned for the §2
  invocation-table flags (decision record binding: capabilities from probes,
  never version pins). Eight capabilities in spec order; seven are
  token-detected, `cancellation` is derived (orchestrator-provided: available
  iff the harness can be driven non-interactively). Case-sensitive word-set
  matching so short flags (`-p`) never collide with substrings (`--provider`).
- Cache keyed on `BinaryIdentity{path,mtime_ns,size}` stored under
  `discovered.<name>.probe` in `harnesses.json` with a `probed_at` stamp;
  identity mismatch (reinstall/upgrade) or `--refresh` forces a re-probe.
  Capabilities persist as string slugs so past/future pio tolerate unknown
  ones (`typed()` drops them) — same additive contract as the rest of the file.
- Downstream honesty API `probed_capabilities`/`has_capability` reads the
  persisted set only, so a harness whose probe failed (or was never probed)
  offers nothing (AC4). Discovery and doctor compose without clobbering:
  discovery refreshes version/last_seen, doctor refreshes the probe.
- `pio doctor` (+ `--json`, `--refresh`) in orc-cli renders the spec report
  table (display · installed/unavailable · role · summary) plus a
  glyph-paired capability matrix (color never load-bearing alone). Role is
  derived: conductor = resume + structured output; worker = non-interactive.
- Tests: 6 unit (`probe.rs`) + 3 orc-core integration (`tests/probe.rs`, AC4)
  + 4 orc-cli e2e (`tests/doctor_cli.rs`, AC1/AC2/AC3) against hermetic
  PATH/ORC_HOME fixtures whose fakes use only shell builtins. New fixture
  `tools/fixtures/probed-harnesses.json` carries unknown fields at every
  additive layer (incl. an unknown capability slug) to prove round-trip.
- All 5 gates green from rust/ on Rust 1.97 (brew): fmt / clippy (0 warnings,
  no allow-flags) / test (all pass, +13) / doc / release build --locked.
  Cargo.lock unchanged (no new deps). Live smoke with the release `pio
  doctor` confirmed the table, unavailable rows, and probe serialization.
- Stayed strictly inside the contract's allowed paths (orc-core/, orc-cli/,
  tools/fixtures/). Documented deviation: the report surfaces a harness's
  last-known probe for a harness that has since left PATH (marked
  unavailable), mirroring how #3 keeps last-known version/seen; exact
  per-harness signal strings remain the decision record's open Q1 (feeds #7).
- Next: Claude adversarial review of the branch (prompt 2).


## Session — 2026-07-23 (Claude reviewer): re-review of issue #3 → ACCEPT
- Verified only the 2-item fix list @ 8cdbc2d: exit-status guard in
  `probe_version` kills both original repros (no error text shown or
  persisted; stored version survives a failed probe; happy path intact),
  and the new end-to-end regression test covers exactly that failure mode.
- Gates 5/5 green (96 tests, 0 failed, +1). Creep check on
  `7625f2d..8cdbc2d`: only the 4 expected files, no scope drift.
- Verdict ACCEPT commented on #3; LOG.md → 🧪. Next: Mrigesh local test +
  merge (prompt 5), which unblocks #4 (`pio doctor`).

## Session — 2026-07-23 (Claude reviewer): #3 merged, dashboards to ✅
- Mrigesh tested locally and merged PR #20 → main @ 2a95b51; issue #3
  closed, epic #15 box ticked. LOG.md #3 → ✅, task_plan.md row + order
  note updated: #4 (`pio doctor`) unblocked and recommended next (+ #5 in
  parallel), #13 deferred until #4's strip changes land.

## Session — 2026-07-24 (Claude reviewer): #4 review → ACCEPT, then merged
- Adversarial review of `issue-4-capability-probe` (7fbe493) against the #4
  contract. Corrected a stale-local-`main` trap first: `git diff main` showed
  a bogus 36-file diff until re-based on `origin/main` (real change = 1 commit,
  11 files, allowed paths only).
- Re-ran all 5 gates green (MSRV 1.91.1). Reproduced AC1/AC2/AC4 live on
  hermetic fixtures + real harnesses; confirmed `discovered.<name>.probe`
  serialization, spec-shape table with `unavailable` rows + glyph/en-dash
  matrix, and that failed/never-probed/unknown harnesses offer nothing
  downstream. One non-blocking deviation: cache keys on path/mtime/**size**,
  not AC3's literal "hash" — demonstrated a contrived equal-size + exact-ns-mtime
  content swap that evades re-probe, but every real reinstall bumps ns-mtime;
  satisfies #16's "path + mtime/hash". Verdict ACCEPT commented on #4, LOG.md → 🧪,
  pushed 88ecd20.
- Mrigesh tested locally (./install.sh from branch) and merged PR #21 → main
  @ 6edd213; issue #4 closed. LOG.md #4 → ✅ (moved to merged cluster),
  #6/#7 unblocked, #12 dep trimmed to #6; task_plan.md row + order note
  updated. Next: #5 (task contracts) — the remaining bottleneck (gates
  #8/#11); #6 is the natural follow-on to #4.

## Session — 2026-07-24 (code-puppy implementer): issue #5 — task contract v2
- Branch `issue-5-task-contract-v2` from fresh `origin/main`. Implemented the
  acceptance-driven task contract v2 + its dispatch brief. Stayed inside the
  allowed paths: `orc-core/`, `orc-cli/` (orc-app left untouched — see below).
- New `orc-core/src/contract.rs`: `TaskContract` (objective, allowed_paths,
  forbidden, expected_artifact, acceptance_checks, reviewer) + nested
  `TaskLimits` (timeout_sec, max_retries) and `TaskBudget` (max_tokens,
  max_usd_cents). All derive serde + schemars 1.2.1 `JsonSchema` per the #16
  binding decision (one schema source, later powers #8's MCP tools). Every
  struct keeps a `#[serde(flatten)] extra` map so unknown future fields survive
  read->write. `render_brief(&Task)` renders the worker hand-off with all
  sections verbatim; unset sections read "(none specified)", never hidden.
  Dependencies are NOT duplicated in the contract — the brief reads the task's
  existing validated `depends_on` graph (DRY).
- `Task`/`NewTask` gain an additive `Option<TaskContract>` (skip_serializing_if
  none, so pre-v2 records and uncontracted tasks emit no `contract` key).
  `add_task` normalizes + validates the contract (an objective with no check is
  rejected) and drops a hollow contract to `None`.
- CLI: `pio task add` gains `--objective/--allowed/--forbidden/--check/
  --artifact/--reviewer/--timeout/--max-retries/--max-tokens/--max-usd-cents`
  (grouped into a `clap::Args` struct `ContractArgs`, boxed in the subcommand
  variant to satisfy `clippy::large_enum_variant` with no allow). `pio task
  show` renders the contract for humans; new `pio task brief` prints the
  dispatch brief (text or `--json`).
- Evidence: AC1 core test `tests/contract.rs` (pre-v2 record loads, no spurious
  contract key; contract + nested unknown fields survive read->write). AC2/AC3
  CLI test `tests/task_cli.rs::contract_flags_round_trip_through_add_show_and_brief`
  drives the real `pio` binary; plus 6 unit tests in `contract.rs`. Ran a live
  demo (add -> show -> brief) on a hand-seeded session. All 5 gates green
  (fmt, clippy -D warnings, test workspace, doc -D warnings, release --locked).
- Deviations: (1) SCORE card contract fields deferred — `TaskSummary` lives in
  `orc-proto` and is built in `orc-daemon`, both forbidden by this contract, so
  orc-app has nothing new to render without touching forbidden crates; noted as
  a clean follow-up. (2) The brief is not yet wired into `dispatch send` (still
  uses the caller's prompt) — out of scope here; `render_brief` is public and
  ready for that follow-up. Needed the Walmart sysproxy to fetch schemars 1.2.1
  (crates.io DNS blocked); it is now in Cargo.lock + cache.
- Next: Claude adversarial review of the branch (prompt 2). #5 landing unblocks
  #8 (orch_* control surface + MCP, reuses this schema) and #11 (worktree
  isolation + review).

## Session — 2026-07-24 (Claude reviewer): #5 review → ACCEPT, then merged
- Adversarial review of `issue-5-task-contract-v2` (a5630a1) against the #5
  contract. All 5 gates re-run green on MSRV 1.91.1, including the offline
  `cargo build --release --locked` — the key risk with the new `schemars 1.2.1`
  dep, which resolves from cache. Reproduced AC1/AC2/AC3 independently:
  pre-v2 records load with no spurious `contract` key + unknown fields survive
  at top/contract/nested-limits layers; `task add`→`show`→`brief` round-trips
  every field; brief reproduces each section verbatim and marks unset ones
  `(none specified)`.
- Two deviations judged justified: (1) `rust/Cargo.toml`/`Cargo.lock` for the
  schemars dep — mandated by #16 decision record §5 ("schemars v1.2.1 derives
  on contract v2"), MIT, exact version, workspace-inherited; (2) SCORE-card
  fields deferred because their data (`TaskSummary`) lives in the *forbidden*
  `orc-proto`/`orc-daemon`. Non-blocking note surfaced: brief not yet wired
  into `dispatch send` (`render_brief` is `pub`). Verdict ACCEPT on PR #22 +
  issue #5, LOG.md → 🧪, pushed c232f85.
- Mrigesh tested locally (fixed a wrong-branch install, then daemon build
  mismatch → `pio daemon restart`) and hand-verified all four ACs, including
  the honest-degradation edges (bare task → every brief header `(none
  specified)`, JSON has no `contract` key; objective-without-`--check` errors).
  Merged issue-5 → main @ 7aed68d; issue #5 closed. LOG.md #5 → ✅ (merged
  cluster), #8/#11 unblocked; task_plan.md row + order note updated. Both
  bottlenecks (#4, #5) cleared. Next: #6 (universal worker adapter) then #8.
  Two follow-ups seeded for #6/#8: wire the brief into `dispatch send`, and
  add a headless `session create`.

## Session — 2026-07-24 (code-puppy): #9 trigger grammar in hosted panes
- Worked issue #9 (V1-7) on `issue-9-trigger-grammar` from fresh `main`. The
  issue's `## Objective` is truncated at source (`... [truncated]`); rebuilt
  intent from the V1 spec's "Trigger words & terminal highlighting" section and
  the four acceptance checks, which are complete.
- Grammar as a single source of truth: new `orc-pty/src/trigger.rs` — `Trigger`
  enum (`Delegate`/`Orchestrate`/`Deliberate`), `TriggerMatch` (char offsets),
  and pure `scan_line()`. Line-anchored + case-sensitive: fires only when a
  keyword is the first non-whitespace token and is immediately followed by `:`.
  `redelegate:`, a colon-less `delegate`, mid-sentence `orchestrate:`, and
  `Delegate:` all stay quiet. Put here (not orc-app) so the daemon (already an
  `orc-pty` dependent) and #8's `orch_*` routing can reuse one definition.
- Added `orc_pty::cells_from_stream(rows, cols, bytes)` — runs the same vt100
  parser headlessly to turn a recorded fixture stream into the exact
  `TerminalCell` grid a snapshot carries, so AC1 tests real VT streams rather
  than hand-placed cells. Reuses the existing `terminal_cell` conversion (DRY).
- Highlight in the renderer: `orc-app` now depends on `orc-pty`. `render_pane`
  detects triggers in the conductor pane only (`role == "brain"` — the objective
  says conductor-pane output; a worker echoing the word must not light up),
  maps the grammar's char offsets to terminal columns (`scan_pane_row`), draws
  the token in `theme.focus` (brain accent) as a BOLD+REVERSED block, and adds
  a `◆ LABEL` title badge. BOLD+REVERSED+glyph+label = the affordance survives
  NO_COLOR/mono and is color-independent (AC3).
- Added `ThemeName::ALL` so snapshot coverage auto-tracks the palette set;
  today it is [Ember, Phosphor] and will pick up nocturne when #13 lands.
- Tests: 8 grammar unit tests + 2 `cells_from_stream` tests in orc-pty; in
  orc-app: `conductor_trigger_grammar_highlights_each_spell_in_every_theme`
  (AC1), `conductor_pane_does_not_highlight_non_triggers` (AC2),
  `worker_pane_never_highlights_a_trigger`, and
  `trigger_highlight_is_reduced_motion_and_color_safe` (AC3, asserts the two
  reduced-motion frames are byte-identical).
- All five gates green from `rust/`: fmt --check, clippy -D warnings, test
  --workspace (0 failed), rustdoc -D warnings, and `build --release --locked`
  (Cargo.lock gained only the orc-app→orc-pty edge; no new external crate).
- Deviations, all noted on the issue: (1) AC1 says "all three themes" but only
  ember+phosphor exist pre-#13 — tests iterate `ThemeName::ALL`, so nocturne is
  covered automatically on merge, no theme improvised into #13's scope; (2) no
  `orc-daemon` event plumbing — it is an allowed path but no acceptance check
  or consumer needs it yet (YAGNI); the grammar is left reusable for #8;
  (3) no `NO_COLOR` env detection exists anywhere yet (that's #13's degradation
  tiers) — AC3 is met by design (non-color affordances), not env plumbing.
- Next: Claude review of the branch against the #9 contract.

## Session — 2026-07-24 (code-puppy): #9 review FIX — prompt-glyph anchor bug
- Claude re-review reversed ACCEPT → FIX after a live test: in a real Claude
  Code brain pane, `U+276F delegate: some web research to the workers` did NOT
  highlight. Root cause: `scan_line` anchored to the first non-whitespace char,
  which in a hosted pane is the prompt glyph (U+276F / `>` / `$`), not the keyword.
  Every acceptance fixture fed a BARE stream (`"delegate: ...\r\n"`), so the
  suite was green but unrepresentative of any real pane. Fair catch; my testing
  gap. Pulled reviewer commits (0d8f503 ACCEPT, 317e621 FIX) first.
- Fix (orc-pty/src/trigger.rs): `scan_line` now tries the keyword at the first
  non-whitespace column AND, failing that, after one optional prompt marker.
  `skip_prompt_marker` accepts a bounded run (<= MAX_PROMPT_MARKER_RUN = 3) of
  non-alphanumeric, non-whitespace sigils followed by >=1 whitespace, then
  returns the next token index; `match_keyword_at` matches a bare keyword +
  colon at an exact index. `char_start` stays on the keyword, so only the
  keyword+colon highlights, never the prompt glyph. Renderer (orc-app) needed
  no change — it consumes char offsets, which are already correct.
- Policy decision (fix item 5, documented in the module): the marker is a
  SHAPE rule (skip a short sigil run), deliberately NOT a fixed glyph allowlist.
  Harm is asymmetric — a missed highlight behind an unlisted prompt glyph is the
  exact bug we just fixed, while a spurious highlight is cosmetic (nothing is
  dispatched; routing is #6/#8). Trade-off: a prompt with embedded alphanumerics
  (git-branch powerline, `[1]` job markers) is not tolerated.
- Tests: grammar now 18 (added prompt-prefix fires with char_start on keyword;
  indentation-before-marker; prefixed false positives; long-sigil banner is not
  a prompt; sigil-without-gap is not a prompt). orc-app fixtures rewritten to
  stream REAL prompts (bare + U+276F + `> `/`$ `/`% ` + oh-my-zsh); `highlighted_
  symbols` replaces the count helper so AC1/AC3 assert the span is EXACTLY
  `keyword:` (proves the glyph is excluded). New
  `recorded_claude_code_prompt_stream_lights_up_the_typed_trigger` replays the
  exact failing line as a recorded Claude-Code-shaped byte stream (ANSI color +
  U+276F) through the real vt100 parser + full render pipeline (fix item 4).
- Emoji-filter gotcha: the repo hook strips U+276F / U+279C (and status emoji)
  from file writes. Got the literal glyph into Rust source via `\u{276f}`
  escapes (string) and `\xe2\x9d\xaf` (byte string) — ASCII in source, real
  glyph at runtime. Restored the LOG.md status emoji via python codepoints.
- All 5 gates green from `rust/`. Live fully-interactive re-test in a real
  Claude Code pane is the merge-time human step (workflow 7); the recorded
  stream is the automated stand-in that would have caught the original miss.
- LOG.md status restored to needs-review + fix-applied note appended under the
  FIX verdict (history preserved). Next: Claude re-review.

## Session — 2026-07-24 (Claude review + owner-directed changes): #9 merged
- Adversarial-reviewed the pushed `issue-9-trigger-grammar` branch: ran all 5
  gates on a clean checkout and mutation-tested every acceptance check (neuter
  grammar / drop colon / drop role guard each fail the right test). First
  verdict ACCEPT — then Mrigesh live-tested and it did NOT highlight.
- Root-caused the miss: the grammar was line-anchored to the first
  non-whitespace char, but a real Claude Code pane renders input as
  `❯ delegate: …` — the prompt glyph is first, so it never matched. Every
  fixture used bare streams with no prompt prefix, so the green suite was
  unrepresentative. RETRACTED the ACCEPT → FIX with a numbered list; puppy
  applied the prompt-marker tolerance.
- Owner then directed three enhancements (implemented directly, all 5 gates
  green each time, no puppy round-trip):
  1. Ultrathink positioning: `scan_line` now returns ALL word-boundary
     `keyword:` matches (Vec) anywhere on the line, not just the first token.
     Deleted `skip_prompt_marker`/`MAX_PROMPT_MARKER_RUN` — the word-boundary
     rule subsumes prompt tolerance. Supersedes the original AC2 "mid-sentence
     does not trigger" line (owner wants mid-sentence to fire); colon +
     welded-word + case guards preserved.
  2. Rainbow highlight: per-character 7-stop `TRIGGER_RAINBOW` gradient, BOLD,
     replacing the reverse-video block. Colour not load-bearing — bold + the
     `◆ LABEL` badge carry meaning under NO_COLOR.
  3. Animated shimmer: `render_shell` derives a motion phase (gated on
     reduced_motion) threaded through `render_stage`→`render_pane`; colour
     index = `(offset + phase) % 7` slides one stop per ~120ms. Frozen static
     under reduced motion. Shell repaint loop keeps ticking while
     `StageState::has_live_trigger()`.
- Mrigesh tested and merged to `main` (PR #23, merge 2d64a42). Updated LOG.md
  (#9 → ✅, next-up → #6) and task_plan.md (#9 merged, order block). Next: #6
  (universal worker adapter).

## Session — 2026-07-24 (code-puppy): #6 universal worker adapter (pushed)
- Implemented issue #6 (V1-4) on `issue-6-universal-worker-adapter` from fresh
  `main`. Dependency V1-2 (#4, capability probes) confirmed CLOSED/merged
  (commit 7fbe493, PR #21) before starting.
- New `orc-core/src/invocation.rs`: probe-driven worker invocation. Two paths —
  (1) explicit override: a non-empty `dispatch_args` is trusted verbatim (keeps
  the pre-#6 hermes `-z` / pi `-p` defaults and every existing dispatch test
  passing = AC3); (2) probe-driven: a per-adapter `InvocationTemplate` (from the
  #16 §2 ground-truth table) is synthesized, adding structured-output flags iff
  `StructuredOutput` was probed and an explicit `--dir`/`-C` flag iff
  `WorkingDir` was probed. A required-but-unprobed capability refuses via
  `InvocationError`, message prefixed `CAPABILITY UNAVAILABLE:` naming the slug.
- Design call (documented in the module): cwd control is ORCHESTRATOR-provided —
  every worker is spawned with `Command::current_dir` = the task's effective cwd
  (worktree.path when isolated, else session.cwd), mirroring how probe.rs
  *derives* `cancellation`. So the single probe-GATED requirement is
  `NonInteractive`; that's why hermes/pi (no cwd flag) stay first-class workers.
- `dispatch.rs` rewired: `select_available_worker` → `select_worker` (role check
  only; deleted the dead `default_workers` fallback — dispatch always passes an
  explicit harness); `invoke_harness` now takes a resolved program + `Invocation`
  + cwd; 3 duplicated pre-invocation failure blocks consolidated into one
  `persist_failure`; additive `DispatchRecord.cwd` field records where the worker
  ran (evidence of cwd control). `probe::probed_from(&registry, adapter)` reads
  capabilities from the already-loaded registry (no second disk read).
- Fixtures: `tools/fixtures/harness-styles/{flag-style.sh,subcommand-style.sh,
  README.md}` — one fake worker per invocation style, each echoes argv+brief+cwd
  and exits 0.
- Tests: 7 unit tests in `invocation.rs`; new `tests/invocation_dispatch.rs`
  (AC1: flag + subcommand workers return confirmed receipts; probe toggles
  optional flags; AC2: unprobed-capability refusal names it); new CLI test in
  `orc-cli/tests/dispatch.rs` (AC2 at the CLI: exits 1, error names
  `non_interactive`).
- All 5 gates green from `rust/`: fmt --check, clippy -D warnings, test
  --workspace (0 failed), doc -D warnings (fixed one private intra-doc link),
  build --release --locked. Stayed strictly inside allowed paths (orc-core/,
  orc-cli/, tools/fixtures/). Out-of-scope respected: no rate limiting (#7), no
  MCP (#8), no session-id resume capture (deferred with #16 open-Q; resume
  dispatch isn't in this issue's ACs). Next: push + issue comment; then review.

## Session — 2026-07-24 (Claude review of #6: real-CLI test → FIX applied in-branch)
- Adversarial review of PR #24 / #6. All 5 gates re-run green; all 4 fixture-based
  ACs independently re-verified and mutation-tested (removing the capability gate
  or the structured-output probe-gate makes the tests fail, as they should).
- Went beyond the fixtures and dispatched to the **real** installed CLIs (isolated
  ORC_HOME, empty non-git sandbox cwd, trivial prompt). `claude` worked end-to-end
  (`claude -p --output-format stream-json --verbose …` → confirmed, returned PONG).
  **`codex` failed**: `codex exec --json -C <dir>` exits 1 with "Not inside a
  trusted directory and --skip-git-repo-check was not specified." A worker cwd is
  orchestrator-assigned and not guaranteed to be a git repo, so codex could never
  run as a probe-driven worker. Root cause confirmed by reproducing the exact
  invocation (with the flag → works and returns PONG; without → exit 1).
- **Fix (in-branch, orc-core only):** added a `fixed: &'static [&'static str]`
  field to `InvocationTemplate` for mandatory adapter-specific flags applied after
  `style`, independent of any probe. codex now carries `--skip-git-repo-check`
  (permissive only — NOT a sandbox/approval skip, per #16's rule against dangerous
  defaults). Other templates set `fixed: &[]`. Added two unit tests (exact codex
  argv incl. the flag even when optional probes are absent) written failing-first,
  plus an integration assertion that `--skip-git-repo-check` reaches `command_line`.
- Re-tested against real codex + claude: **both confirmed, exit 0, returned PONG**;
  codex ran in the non-git sandbox via `codex exec --skip-git-repo-check --json -C`.
  All 5 gates green again. opencode's template matches its 1.18.4 `run --format
  json --dir` interface; hermes/pi keep the override path.
- Probe follow-up — investigated and CLOSED here (no code fix needed): my first
  note that "`pio doctor` reports an identical capability set for all five" was
  wrong — a bug in my inspection script (sorted the `{cap: bool}` report map's
  keys instead of filtering by `value == true`). Re-probed correctly: the probe
  is honest and per-harness — Hermes probes `structured_output` and `working_dir`
  **false** (its profile has empty proof tokens), the full agent CLIs probe all
  eight true. Documented the real lesson (help-token probe proves *advertisement*
  not *runtime*; runtime quirks like codex's `--skip-git-repo-check` live in the
  invocation template, not the probe) in `findings.md` and the `probe/profiles.rs`
  module header. Unattended permission mapping for codex/claude on real coding
  tasks (approval prompts) is #16's open question, out of scope for #6's ACs.

## Session — 2026-07-24 (owner): #6 MERGED (PR #24)
- Mrigesh merged PR #24 into `main` (merge commit `9968839`); #6 closed.
- Dashboard bookkeeping on `main`: LOG.md #6 → ✅ merged, #12 → unblocked, new
  "#6 merged" headline + Next pointer → #7; task_plan.md #6 marked merged and the
  order/ready-set narrative advanced (Next: #7, then #8; #6 now unblocks #12).
- V1 status: #16, #17, #3, #4, #5, #9, #6 merged. Ready set: #7, #8, #11, #12,
  #13. Next up: **#7** (V1-5 rate-limit-aware spawning / quota guard v2 +
  concurrency caps; depends on #4, satisfied) — owner starting it next.

## Session — 2026-07-24 (code-puppy): #7 built — quota guard v2 (rate-limit-aware spawning)
- Branch `issue-7-quota-guard-v2` off `main`. Dependency #4 (V1-2) confirmed
  CLOSED/merged (probe.rs present). Allowed paths: orc-core/, orc-cli/.
- Binding #16 decision honored (issue comment + decision record §4): retry
  driver is `backon` v1.6.0, BLOCKING path (`BlockingRetryable` + default
  `StdSleeper`, no async runtime). Disabled backon default features (they pull
  `tokio` via `tokio-sleep`) → `default-features = false, features = ["std",
  "std-blocking-sleep"]`, so backon drags in only `fastrand` — truest to the
  decision's "avoid tokio in orc-core" intent. Blocking builder has no
  `.adjust()` (async-only), so a parsed retry-after is SURFACED in the warning
  but the sleep follows the exponential schedule (documented in ratelimit.rs).
- NEW `orc-core/src/spawn_guard.rs` — per-harness concurrency cap + durable slot
  leasing. `effective_cap` = user override (registry.concurrency[key]) else
  conservative per-adapter default (pi/hermes 3, claude/codex/opencode 2, unknown
  1), floored at 1. Slots are lease files under `~/.orchestra/slots/<harness>/`
  keyed by HARNESS not session → cap spans every session/process (AC3). RAII
  `SlotLease` deletes its file on drop/release; `acquire_slot` locks the harness
  dir (create_new .slots.lock, like tasks BoardLock), prunes leases whose pid is
  dead (registry::pid_alive) or whose TTL (f64 secs) elapsed, counts live, and
  only writes a new lease if under cap else returns Ok(None).
- NEW `orc-core/src/ratelimit.rs` — per-adapter rate-limit signal table (the #16
  "detection is ours, next to the adapter templates" mandate; common set + per
  adapter extras; case-insensitive substring), retry-after parser, `BackoffPolicy`
  {production 2s..60s x2 jitter 4 retries / fast ms-scale for tests}, and generic
  `run_with_backoff` wrapping backon's blocking exponential retry with when()+
  notify().
- `bench.rs`: additive `HarnessRegistry.concurrency: BTreeMap<String,usize>`
  (#[serde(default)], updated Default impl). Chosen over a HarnessConfig field
  because orc-daemon (OUT of allowed paths) has fully-explicit HarnessConfig
  literals that a new field would break; HarnessRegistry literals there use
  `..default()` spread, so the map is safe. Additive round-trip test added
  (`tests/bench.rs`): pre-#7 file with no concurrency key loads empty + unknown
  siblings survive read->write.
- `dispatch.rs`: refactored one-shot `dispatch()` into `deliver(request, policy,
  reuse)` with thin `dispatch()`/`dispatch_with_policy()` wrappers. Added
  `DeliveryStatus::Queued` (additive string), `DispatchFailureKind::RateLimited`,
  additive `DispatchRecord.warnings: Vec<String>` + `is_queued()`. `invoke_harness`
  now returns `Invoked{exit_code,stdout,stderr,success}` (captures output even on
  non-zero exit) so `invoke_with_backoff` can scan for rate-limit signals and
  retry. Slot gate sits after resolve+locate: no slot → `persist_queued` (visible
  record, task `delivery_queued` event, ORC WARNING, NO spawn = AC1); slot held
  across the backed-off invoke then released. Lease TTL = timeout*(retries+1) +
  max_delay*(retries+1), min DEFAULT_LEASE_TTL, so a long dispatch is never pruned
  mid-flight. `drain_queued[_with_policy]` re-delivers queued records oldest-first
  reusing id+created_at (one record, queue->run), per-record errors leave it
  queued. Consolidated the 3 pre-invocation failure params into `FailureSpec` to
  stay under clippy's 7-arg limit (no #[allow]).
- `tasks.rs`: `record_queued` appends a `delivery_queued` history event without
  changing status/assignee_run (no worker got the brief yet, but it's visible).
- `orc-cli/src/main.rs`: `dispatch send` prints `record.warnings` to stderr (ORC
  WARNING channel) and exits 0 confirmed / 75 (EX_TEMPFAIL) queued / 1 failed
  (`dispatch_exit`). NEW `dispatch drain` runs the queue. NEW `harness cap
  <harness> [max] [--clear] [--json]` sets/clears the per-harness override and
  prints the effective cap.
- Tests (all failing-first where behavioral): spawn_guard lib (2) + integration
  (3: cap-of-N queues the next, per-harness pools independent, abandoned lease
  pruned); ratelimit lib (5: signal detection, retry-after shapes, backoff
  recover/exhaust/non-retryable); orc-core `tests/quota_guard.rs` (4: AC1 queue+
  drain+no-spawn sentinel, AC2 recover-with-warning + retry-after, AC2 exhaust ->
  rate_limited, AC3 cross-session cap); orc-cli `tests/quota_guard.rs` (2: CLI
  queued exit 75 + ORC WARNING on stderr + drain runs same record; cap --clear ->
  adapter default); bench additive (1). +17 tests, all green.
- Gates (Rust 1.97, from rust/): fmt --check clean; clippy --workspace
  --all-targets -D warnings clean; test --workspace 0 failed; doc -D warnings
  clean; build --release --locked clean. crates.io DNS was down; fetched backon
  via HTTPS_PROXY=sysproxy.wal-mart.com:8080 (github.com direct).
- Deviations (flagged on the issue): (1) `rust/Cargo.toml` + `rust/Cargo.lock`
  touched (outside the two listed crate paths) — unavoidable + mandated: adding
  the #16-required backon dep necessarily edits the workspace manifest + lockfile
  (same situation #5 had with schemars, accepted then). (2) queued exit code 75
  (EX_TEMPFAIL) is a new convention, documented. (3) retry-after honored only in
  the warning, not the sleep (blocking backon has no adjust()).

## Session — 2026-07-24 (code-puppy): #7 review fixes (Claude verdict was FIX)
- Reviewer (Claude/Fable) on PR #25: all 4 ACs + 5 gates pass independently,
  but one confirmed blocker (Fix 1) + two disclosed minors (Fix 2, Fix 3).
- Fix 1 (BLOCKING): invoke_with_backoff (dispatch.rs) checked is_rate_limited
  BEFORE the success branch, so a worker that exits 0 whose output merely
  mentions a signal substring (429, "rate limit", "overloaded", "throttl",
  ...) was treated as rate-limited, retried the full budget, and recorded
  failed/rate_limited. Coding CLIs emit those words on SUCCESSFUL tasks (HTTP
  clients, retry logic, or literally summarizing this PR's own diff), so good
  work was silently failed and provider load multiplied 4x -- the exact
  opposite of the issue objective -- and AC2 hid it (all its fixtures exit
  non-zero). FIXED: a clean exit-0 run is confirmed regardless of output text;
  only a non-success (non-zero exit) invocation is scanned for a throttle
  signal (real provider limits exit non-zero, so AC2 stays green). Regression
  a_successful_worker_whose_output_mentions_a_rate_limit_is_confirmed_once:
  exit-0 worker printing "...rate limit backoff and 429 handling" -> confirmed,
  exactly 1 attempt, no warnings.
- Fix 2 (minor): spawn_guard.rs lock_slots had no stale-lock reclamation -- a
  dispatcher SIGKILLed while holding .slots.lock wedged that harness's cap
  (~1s spin then "busy" error) until manual cleanup. FIXED: the lock file now
  records the holder pid; on AlreadyExists, reclaim_if_stale reclaims it when
  the recorded pid is dead OR the lock is aged past STALE_LOCK (30s) via an
  ATOMIC rename-steal (only one racer's rename wins, so a concurrent reclaim
  can't delete a freshly-recreated lock; losers see the source gone and retry
  create_new). Unknown holder (empty/old-binary lock) reclaimed only when
  aged, so a live old-binary holder is never stolen. Regression
  a_lock_abandoned_by_a_dead_holder_is_reclaimed_not_wedged plants a dead
  (spawned+reaped) pid's lock and asserts acquire_slot reclaims it.
- Fix 3 (minor): ratelimit.rs digits_after -> seconds_after now honors
  second/minute/hour/millisecond unit words (defaulting to seconds), so
  "retry after 2 minutes" surfaces 120 not ~2; ms reported as whole seconds.
  Only shown in the operator warning (Deviation 3 unchanged) but no longer
  misleading. Unit assertions added to parses_retry_after_hints_in_several_shapes.
- Integrated the reviewer's verdict commit (4dd1805: LOG.md status -> hammer +
  verdict blockquote) by rebasing my fix commit on top; verdict preserved,
  LOG.md status set back to eyes (re-review requested) with a "Fixed" reply
  under the verdict.
- Gates from rust/ (Rust 1.97): fmt clean; clippy --workspace --all-targets
  -D warnings clean; test --workspace 0 failed (+2 tests: quota_guard 5,
  spawn_guard 4, ratelimit unit expanded); doc -D warnings clean; build
  --release --locked clean.

## Session — 2026-07-25 (Claude re-review + owner): #7 MERGED (PR #25)
- Re-review (Claude/Fable) of fix round 0aa7503: verified ONLY the numbered
  fix list and confirmed no regressions/creep vs the reviewed commit.
  - Fix 1 re-checked with a harsher independent probe than the shipped
    regression — an exit-0 worker whose ENTIRE output is a bare "HTTP 429" ->
    confirmed on attempt 1; an exit-1 "429" -> still failed/rate_limited after
    the full 4-attempt budget, so AC2 is intact (detection now gates on a
    non-zero exit).
  - Fix 2 rename-steal confirmed race-safe (create_new stays the mutex, one
    winner, a live-held lock is never stolen); dead-holder reclaim test passes.
  - Fix 3 unit-aware retry-after (warning-only) confirmed.
  - All 5 gates green on my own run; diff since the reviewed commit stays in
    orc-core/ (+ LOG.md/progress.md), Cargo.toml/Cargo.lock unchanged, no new
    deps. Verdict: ACCEPT (LOG.md status -> 🧪, verdict blockquote appended).
- Owner tested locally (pio dispatch drain / pio harness cap) and MERGED PR #25
  into main (merge commit 3e30dc9); issue #7 CLOSED, epic #15 #7 box ticked.
  Post-merge docs: LOG.md #7 row -> ✅ (PR #25) + status narrative advanced to
  "Next: #8"; task_plan.md #7 row -> merged + order line advanced. This
  completes the "quota guard" work (v1 token budget + v2 concurrency/backoff).
  Ready set now: #8 (orch_* + MCP), #11 (worktree isolation), #12
  (single-harness mode), #13 (visual identity).

## Session — 2026-07-25 (code-puppy): #8 implemented (orch_* control surface + MCP)
- Branch `issue-8-orch-control-surface` from fresh `main` (c1b1b9f). Dependency
  V1-3 (#5, task contract v2) confirmed merged, so #8 was unblocked.
- Design: one shared *synchronous* surface in orc-core (new `orch.rs`) with the
  seven operations (plan/delegate/status/await/review/cancel/finish) composed
  from the existing tasks/dispatch/control primitives. Both user surfaces are
  thin adapters over it, so they cannot drift (AC3): normalized `pio orch <verb>`
  verbs in orc-cli, and a new `orc-mcp` crate exposing the same seven as MCP
  (stdio) tools. `Verb::ALL` is the single source of truth for names/descriptions.
- MCP: `rmcp` v2.2.0 (features server+macros+transport-io) per the #16 decision
  record, in the new `orc-mcp` crate with its own `current_thread` tokio runtime;
  each tool runs the sync `orch` op via `spawn_blocking`, so tokio never leaks
  into orc-core/orc-daemon. Request structs derive schemars `JsonSchema` and
  reuse the #5 `TaskContract` schema directly.
- Deviation from #16's note ("translate MCP calls to the daemon protocol"): the
  daemon protocol has no task-creation verb, and the CLI already drives task/
  dispatch ops directly on orc-core, so both surfaces call orc-core directly.
  Same-operations parity is proven by test instead. Documented on the issue.
- Folded in the two follow-ups #5's review seeded: `delegate` defaults the worker
  prompt to `render_brief(task)`, and added headless `pio session create`/`list`.
- AC4: `pio mcp print-config [--format claude|codex]` emits a Claude `.mcp.json`
  object and a Codex `config.toml` block pointing at the sibling `pio-mcp`;
  prints only, never writes protected files. `install.sh` links `pio-mcp`
  (allowed path). `uninstall.sh` is outside the allowed paths, so unlinking
  `pio-mcp` on removal is a flagged follow-up.
- Tests: `orc-mcp/tests/tools.rs` — AC1 (`tools/list` over real stdio = exactly
  7 tools with input schemas), AC2 (delegate->await->status e2e over stdio
  against a fixture worker), router==`Verb::ALL` + description parity.
  `orc-cli/tests/orch_cli.rs` — CLI verbs==`Verb::ALL`, CLI delegate == core
  `orch::delegate` normalized (AC3 behavioral), full lifecycle, AC4 print-config
  validity + no protected writes, `session create/list`. orch.rs unit tests
  (verb/config/schema). Updated `install.rs` for the new link.
- Network: crates.io reachable only via the Walmart sysproxy; `rmcp` + transitive
  deps fetched through it. Gates from rust/ (Rust 1.97): fmt clean; clippy
  --workspace --all-targets -D warnings clean; test --workspace 0 failed
  (45 suites); doc -D warnings clean; build --release --locked clean.
- Pushing `issue-8-orch-control-surface` and commenting per-AC evidence on the
  issue; LOG.md #8 -> eyes + branch and a ship-log entry added in this branch.

## 2026-07-26 — Claude (Fable), issue #8 review fix round

Applied the fixes from my own adversarial review (verdict comment on #8) on the
same branch, since Mrigesh asked this session to close them out rather than
hand back to the implementer.

- **Fix 1 (blocker): `uninstall.sh` left a dangling `pio-mcp` symlink.** This
  branch links `pio-mcp` in `install.sh` but never unlinked it, so an
  install→uninstall cycle left a broken link in `~/.local/bin` pointing into a
  build directory. Added `remove_link pio-mcp`. The out-of-allowed-path edit is
  reviewer-authorized: it is the direct consequence of the allowed `install.sh`
  change. `orc-cli/tests/install.rs` now asserts `pio-mcp` in the *uninstall*
  loop too (it only had it in the install loop) and uses `symlink_metadata`, so
  a dangling link can no longer pass `Path::exists()`.
- **Fix 2: a failed or queued `orch_delegate` reported plain success over MCP.**
  The CLI signalled it with exit 1 / 75, but MCP has no exit code and
  `OrchOutcome.note` was left `None`, so a conductor reading the top-level
  result saw success. New `orch::delivery_note` fills `note` on any
  non-confirmed delivery: the failure kind and message for a failed delivery,
  or "call orch_await to wait for a free slot" for a queued one — deliberately
  not phrased as a failure, since queued work is still coming. A confirmed
  delivery stays quiet. The task is *not* rolled back out of `running`: that
  matches `dispatch send`, whose failed delivery is recorded as history rather
  than reverted, so the note names the task and its status instead.
- **Fix 3: the CLI half of the parity test only checked one direction.** It
  asserted every `Verb::ALL` entry is a real subcommand, so an eighth `pio orch`
  verb with no MCP twin would have passed. It now parses the `Commands:` block
  of `pio orch --help` and asserts set equality with `Verb::ALL`, matching the
  MCP side.
- **Fix 4: `orc-mcp/tests/tools.rs` overclaimed.** Its comment said the worker
  received the rendered brief but the assertion only checked stdout contained
  `fake-worker-stdout`, which is unconditionally true. It now asserts the
  delivered `prompt` contains the objective and the acceptance checks.
- **Fix 5 (doc): `orch_cancel`'s kill path.** Documented that termination only
  fires for a task linked to a real background run — a delegated task's
  `assignee_run` is the dispatch id, and a dispatch has already exited — which
  is why the verb promises "best-effort".
- **Left open on purpose:** the seeded follow-up "wire `render_brief` into
  `dispatch send`" is done for `orch delegate` only; `pio dispatch send` still
  takes an explicit prompt. `orch` is the canonical delegation path, so this is
  the right call — but the follow-up is not closed and should not be recorded
  as such.
- New tests: `orch::tests::delivery_note_speaks_only_when_the_delivery_did_not_confirm`,
  `orch_cli.rs::failed_delegation_is_announced_in_the_outcome`,
  `tools.rs::failed_delegation_over_stdio_carries_a_note`. Each fix was
  regression-proofed by reverting it and confirming the new test fails.
- Gates re-run from `rust/` on Rust **1.91.1**: fmt clean; clippy
  `--workspace --all-targets -D warnings` clean; `test --workspace` 45 suites /
  **188 passed / 0 failed** (was 185); `RUSTDOCFLAGS="-D warnings" cargo doc`
  clean; `build --release --locked` clean. Verified live with the release
  binaries: a scratch install→uninstall now leaves `~/.local/bin` empty with
  zero dangling links; a failed delegation carries the same note on the CLI and
  over raw JSON-RPC stdio; a queued delegation says "call orch_await" and doing
  so returns `confirmed`.
- Ready for Mrigesh to test locally and merge.

## 2026-07-27 — Claude (Opus 5), issue #8 merged, dashboards updated

- Mrigesh tested locally and merged **PR #26** into `main` (`0c9908a`), closing
  issue #8. `main` now carries the `orch_*` control surface: `orc_core::orch`
  defines the seven verbs once, `pio orch <verb>` and the `pio-mcp` MCP stdio
  server expose them twice, `pio mcp print-config` prints the registration
  snippets, and `pio session create` makes a session headlessly.
- Updated `LOG.md`: #8 → ✅ (merged, PR #26); the headline paragraph now
  summarizes #8 instead of #7 and names the new ready set; #10 dropped its
  *needs #8* marker; a merged blockquote was appended under the #8 ship-log
  entry noting that entry predates the fix round (its "uninstall.sh doesn't
  unlink pio-mcp yet" caveat is no longer true on `main`).
- Updated `task_plan.md`: #8 row marked merged, and the order paragraph rewritten
  — every dependency edge in the issue map is now cleared, so #11, #10, #12 and
  #13 are all startable in parallel from fresh `main`.
- **Still open, deliberately:** wiring `render_brief` into `pio dispatch send`.
  It is done for `orch delegate` only. Recorded in both dashboards so it is not
  mistaken for closed just because #8 merged.
- Next: **#11** (worktree isolation + independent review + receipt), then #10
  (standalone Claude Code/Codex integrations, which #8 just unblocked). Start
  #13 before more TUI churn lands.

## Session — 2026-07-26 (code-puppy): #10 standalone integrations v2 (Claude hook + Codex block v2)
- Worked issue #10 (V1-8) on `issue-10-standalone-integrations` from fresh `main`
  (HEAD `0c9908a`, the #8/PR-26 merge) — so the V1-6 dependency is satisfied and
  #10 was unblocked. Harness-side only; allowed paths `skills/ codex/ shell/
  install.sh uninstall.sh docs/`. No Rust source touched.
- New Claude Code `UserPromptSubmit` hook `shell/claude-userpromptsubmit-hook.py`
  (python3, stdlib only). Per the spec, closed UIs can't be re-colored, so the
  standalone answer is a hook/acknowledgment: on every prompt it detects the
  spell grammar, and on a hit it (1) runs a bounded `pio quota --json` and
  renders the reported level as the `ORC WARNING/BLOCKED/NOTE` advisory the
  skills promise, and (2) injects the exact `pio orch`/MCP invocation for the
  verb. Always exits 0 (never eats a prompt).
- Grammar is a faithful port of `orc_pty::trigger` (word-boundary + required
  colon, case-sensitive, mid-line, repeatable; `redelegate:`/`delegated:`/
  `Delegate:`/colon-less `delegate` stay quiet). The Rust crate can't be imported
  from a hook, so drift is guarded by a built-in `--selftest` (22 checks, all
  green) — flagged as the one intentional duplication.
- AC2 text: updated `skills/pi-delegate/SKILL.md` (standalone `delegate:` +
  `orch_delegate`/`pio orch delegate`), `skills/orchestrate/SKILL.md` (the seven
  `orch_*` verbs/tools), a new `skills/deliberate/SKILL.md` (honest “panel is
  V2, not available” fallback), and `codex/AGENTS-block.md` → v2 with a leading
  Trigger-grammar section covering all three verbs + the normalized surface +
  `pio mcp print-config`. Single-harness honesty and confirmed-delivery kept.
- AC1: `install.sh` links the hook into a pi-orchestra-OWNED dir
  (`~/.claude/pi-orchestra/`) and prints a manual `settings.json` snippet — it
  never edits protected config; adds `deliberate` to the skill loop.
  `uninstall.sh` removes the hook (own symlink only) + the dir if empty + the
  `deliberate` skill. Verified on an isolated HOME: two installs → every marker
  count == 1 and the three protected-config SHA-256s identical before==after;
  uninstall leaves protected config byte-identical and no dangling links. The
  Rust `install.rs` test (outside allowed paths) still passes unchanged.
- AC3: `docs/notes/2026-07-26-standalone-trigger-hook-manual-test.md` documents
  registration + a reproducible non-interactive simulation (real captured
  output; the relayed `MiniMax quota:` line proves the hook invoked `pio`) + the
  live Claude Code procedure + `--selftest`.
- AC4 gates from rust/ (Rust 1.97): fmt clean; clippy --workspace
  --all-targets -D warnings clean; test --workspace 45 suites / 0 failed; doc
  -D warnings clean; build --release --locked clean.
- Pushing the branch and commenting per-AC evidence; LOG.md #10 -> eyes + branch
  and a ship-log entry added in this branch.

## Session — 2026-07-27 (Claude): #10 review round + fix round

- Adversarial review of `issue-10-standalone-integrations` (see the verdict on
  issue #10). All five gates re-run green; AC1 install/uninstall reproduced in an
  isolated HOME; every documented `pio session`/`orch`/`mcp` invocation executed
  for real; and the Python↔Rust grammar port checked by building a harness over
  `orc_pty::trigger::scan_line` and differentially fuzzing **4,019 cases**
  (incl. Unicode boundary chars chosen to split Rust's `char::is_alphanumeric`
  from Python's `str.isalnum()`) — **zero mismatches**, keyword and offset.
- **Blocker found and fixed: the quota relay never fired.** The hook grepped
  `pio quota`'s human output for `ORC WARNING`/`ORC BLOCKED`/`ORC NOTE`, but
  those markers are produced by `quota::gate()` and only reach the user through
  `pio run`/`dispatch` — `Commands::Quota` prints a different report entirely.
  Measured: at level **warn** (exit 2) the conductor was told *"no quota advisory
  to relay"*, a confident false negative on the one guarantee AC2 names. Only
  the `unknown` branch worked, and that was the only branch the original
  evidence demonstrated. The relay now drives off `pio quota --json` (level is
  authoritative from `orc_core::quota`; the hook only renders it), carries the
  real 5-hour/weekly percentages, and falls back to the exit code (2→WARNING,
  3→BLOCKED, else→NOTE) when output is unparseable — never to silence.
- `--selftest` grew from 22 to 39 checks: one case per quota level through the
  pure renderer **and** end-to-end through a real subprocess against a stub
  `pio`, plus unparseable-output fallbacks. This is the regression guard.
- Corrected the three docs that asserted the impossible behavior (the manual-test
  note, `skills/deliberate/SKILL.md`, the LOG.md ship-log entry) and dropped the
  "read-only" claim for `pio quota` — it writes `quota.json` and appends
  `quota_history.jsonl`.
- Also fixed a **pre-existing** install bug surfaced by AC1's "run twice,
  identical result": `~/.codex/AGENTS.md` grew a blank line on every install
  (measured on `origin/main` too: 3036→3038→3040 bytes). The owned-block refresh
  now trims trailing blanks before re-appending; verified byte-identical across
  four consecutive installs.
- Rebased onto `main` (`a0fa88a`) to clear the LOG.md/progress.md conflicts;
  #8 stays ✅ merged and both progress entries survive.
- Next: Mrigesh tests locally (`./install.sh`, register the hook, type
  `delegate: …`) and merges; then #11.
## Session — 2026-07-27 (Claude): #28 dispatch pipe-buffer deadlock

- Found while reviewing #10: a live `delegate:` trigger routed a real delegation
  to `pi-m3`, which failed twice with `DISPATCH TIMEOUT` at exactly 120s.
- **Root cause: a pipe-buffer deadlock in `dispatch.rs::invoke_harness`.** It
  piped stdout/stderr, polled `try_wait()`, and only drained the pipes *after*
  the child exited. A worker that fills the ~64KB pipe blocks in `write()`, so it
  can never exit, so the parent never drains. Every non-trivial delegation was
  affected — the #8 control surface has never worked for a real task.
- Proven by A/B on the identical command: poll-then-drain hung with 0 bytes and
  was killed at 90s; draining concurrently exited 0 in 29.8s with 148KB of
  stdout. NOT the model — the same task run directly takes 6.3s on both
  MiniMax-M3 and MiniMax-M2.7-highspeed. `pi --mode json` emits ~4KB for a
  one-word answer.
- Fix: `drain_to_eof` spawns a reader thread per stream immediately after
  `spawn()`, consuming to EOF while retaining at most `MAX_CAPTURED_BYTES` and
  discarding the overflow. The old `bounded_capture` broke out of its read loop
  at the cap — deleted, since draining that stops early is the bug.
- A timed-out dispatch now records the partial output it did capture
  (`Drain::snapshot`, no join, so a surviving grandchild can't wedge the parent).
  The old empty stdout+stderr on timeout is exactly what hid this bug.
- Regression tests (`tests/dispatch_flood.rs`, written before the fix and
  confirmed failing on `main`): 2MB floods on stdout and on stderr separately,
  plus a bounded/truncation-marker check. 45.25s of timeouts → 0.30s green.
- AC5: `--timeout` is contract metadata and does NOT bound delivery; that
  ambiguity is what made a conductor pass `--timeout 300` and still get the 120s
  default. Clarified in the clap help for both flags and in `Verb::description()`
  (the MCP source of truth, mirrored in orc-mcp and pinned by a drift test).
- AC6 live: the delegation that timed out twice now returns `confirmed`, exit 0,
  in 24.0s, with the capture bounded at 16410 bytes and marked truncated.
- **NOT done in this branch: AC3 and AC4** (confirm-on-delivery + non-blocking
  conductor). `spawn_guard::SlotLease` releases on `Drop`, so returning early
  while the worker runs would free the concurrency slot immediately and silently
  defeat the #7 cap. Doing it right means moving lease ownership into a detached
  supervisor (the `pio _exec` pattern from `runner.rs:478`) — a real redesign
  that also collides with #11, which rewrites this file. Left open on #28.

### Follow-up the same day — head+tail capture (found by Mrigesh's live test)

- The #28 fix made delegation *complete*, but a live `delegate:` to `pi-m3`
  showed the conductor still could not *use* the result: the capture kept the
  first 16KB, and `pi --mode json` spends that on its session header and the
  model's thinking. The answer lives at the END of the stream and was discarded,
  so the brain redid the work itself — exactly the "no actual benefit of using
  pio" failure mode.
- `Captured` now keeps a head window (4KB) and a tail window (12KB) with the
  dropped byte count marked in between. Same bound, but the worker's conclusion
  survives.
- New test `the_conductor_receives_the_workers_answer_not_just_its_preamble`:
  the fixture emits 2MB and then its answer as the last line. Verified it FAILS
  on the head-only implementation (the version first pushed to PR #29) and
  passes now — not a vacuous test.
- Live re-run: `confirmed`, exit 0, 16402B captured, and the tail carries
  MiniMax-M3's actual finding plus its usage/cost block.
- Noted for #30: the record still holds the raw JSON firehose rather than the
  extracted answer. `orc_core::runner::extract_text` already parses pi's
  `text_delta` events for `pio run` — dispatch should reuse it per adapter.
- Correction to a claim made by the delegating session: `pi-m3` is NOT "marked
  dispatch=false". Its probe reports `non_interactive` + `structured_output`,
  the same capabilities as `codex`. There is no such flag.

## Session — 2026-07-27 (Claude): #10 and #28 merged, dashboards updated

- Mrigesh merged **PR #27** (#10, standalone triggers) and **PR #29** (#28,
  dispatch drain). `main` is `73729ac`.
- LOG.md: #10 and #28 → ✅; #30 added as *next*; the headline paragraph now
  covers both merges and names #30 → #11 as the order, with the reason (both
  rewrite `dispatch.rs`, and #11's per-check report depends on how a dispatch
  reaches terminal state). A merged note was appended under the #28 ship-log
  entry recording the head+tail follow-up.
- task_plan.md: #10 and #28 marked merged; #28 and #30 added to the issue map;
  the order paragraph rewritten around "#30 before #11".
- Renumbered the two new issues: #28 is **V1-13** and #30 is **V1-14**. My
  original titles said V1-11/V1-12, which already belong to #13 (visual
  identity) and #14 (README) in task_plan.md — corrected on GitHub before the
  collision could mislead anyone.
- #28 closed on GitHub referencing PR #29; epic #15 ticked for #10 and #28.
- **Next: #30.** Its contract now carries three things worth not re-deriving:
  the `SlotLease`-releases-on-`Drop` blocker, the `runner.rs:478`
  `spawn_background` precedent to follow rather than reinvent, and an
  acceptance check that the record must hold the worker's extracted answer
  (via `runner::extract_text`, already used by `pio run`) instead of raw
  transport JSON.

## Session — 2026-07-27 (code-puppy): #30 background dispatch

- Refreshed `main`, verified dependency #28 is closed/merged, and created
  `issue-30-background-dispatch`. The requested `gh issue view` command could
  not authenticate because this shell has no `GH_TOKEN`; the connected GitHub
  app supplied the full issue body and confirmed there are no contract-changing
  comments.
- Added a detached hidden `pio _dispatch_exec` supervisor. `orch delegate`
  transfers both the harness lease and a session-wide
  `max_parallel_workers` lease to it, returns after the worker receives the
  brief, and leaves completion/output to `orch status` and `orch await`.
- Dispatch records now keep delivery and execution as separate additive states,
  including supervisor/worker pids, start/end times, exact structured usage,
  and terminal output. Dead-supervisor reconciliation kills a surviving worker
  process group, marks the record `orphaned`, and releases only that dispatch's
  slots.
- Reused `runner::extract_text` / `extract_usage` through an adapter-aware event
  helper: pi JSON transport persists the readable assistant answer and exact
  usage, while unproven adapters retain the bounded raw head+tail fallback.
- Acceptance tests prove a two-second worker does not block delegate; cap 1
  queues until real exit and then auto-drains; cap 2 produces observed overlap;
  a different process awaits and later reads the durable result; a SIGKILLed
  supervisor reconciles and frees both slots; the 2 MB stdout/stderr flood
  tests remain green; and a real pre-#30 record with an unknown field still
  loads unchanged.
- Updated MCP tool descriptions plus the standalone Claude/Codex skills, AGENTS
  block, and hook guidance with the immediate-return → status/await flow and
  the `--dispatch-timeout` (real worker bound) versus contract `--timeout`
  (metadata) distinction. Hook self-test: 41/41 passing.
- Live release-binary proof against `pi-m3` with quota at 99% in both windows:
  delegate returned in 86 ms as `confirmed`/`running`; `orch await` then
  persisted `ISSUE30_LIVE_ANSWER_OK` as plain stdout with `succeeded` and exit
  0, rather than storing the JSON transport.
- Final issue gates are green: fmt, clippy with warnings denied, the complete
  workspace test suite, rustdoc with warnings denied, and locked release build.

### Fix round — failed execution must speak over MCP

- Preserved reviewer commit `066a7dc` and fixed its single blocker without
  changing the background supervisor or dispatch lifecycle. The newest
  terminal failed/orphaned execution now produces a top-level note for
  `orch_status` and `orch_await`; an older failed attempt cannot shadow a newer
  successful retry.
- Added real MCP stdio coverage with a worker that confirms receipt and then
  exceeds a one-second execution bound. Both observation tools return the same
  note naming `failed`, `timeout`, exit 124, and the task; the successful path
  remains pinned to `note: null`.
- Re-ran all five required Rust gates after the fix; all pass.

## Session — 2026-07-28 (Claude): #30 merged, dashboards updated

- Mrigesh merged **PR #31**; `main` is `77f069b` and issue #30 auto-closed.
- **Live acceptance by the owner, in a real Claude Code session:** a `delegate:`
  trigger confirmed delivery, reported the worker still running, then awaited its
  answer — exercising the new delegate → status → await flow end to end. Notably
  it ran on **`hermes`**, a third harness neither the implementer's evidence nor
  my review had exercised (they used `pi-m3`, `codex` and shell fixtures), so the
  detached-supervisor path is now demonstrated across three real harnesses. The
  answer (zero TODOs in `orc-core`) was independently re-verified with a local
  grep — matching my own earlier verification and the two harnesses before it.
- LOG.md: #30 → ✅; the headline paragraph now covers #30 and names #11 as next
  with the reason; a merged note was appended under the #30 ship-log entry
  recording the live hermes test.
- task_plan.md: #30 row marked merged; the order paragraph rewritten around
  "#11 next", recording that #11 was sequenced after #30 on purpose because both
  rewrite `dispatch.rs`.
- Earlier the same run (commit `fe0670d`, on `main`): recorded the checkout move
  to `/Volumes/Mrigesh SSD/Agent-orchestra` in docs/WORKFLOW.md + findings.md.
  The move had silently dangled every symlink `install.sh` creates — the three
  Claude skills and the `UserPromptSubmit` hook — so the trigger grammar had
  stopped firing with no visible error. Repaired by re-running `./install.sh`
  from the new location; verified all four links resolve and the hook responds.
- **Remaining for V1: #11, #12, #13, #14.** Next: #11.

