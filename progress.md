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


## Session — 2026-07-23 (Claude reviewer): re-review of issue #3 → ACCEPT
- Verified only the 2-item fix list @ 8cdbc2d: exit-status guard in
  `probe_version` kills both original repros (no error text shown or
  persisted; stored version survives a failed probe; happy path intact),
  and the new end-to-end regression test covers exactly that failure mode.
- Gates 5/5 green (96 tests, 0 failed, +1). Creep check on
  `7625f2d..8cdbc2d`: only the 4 expected files, no scope drift.
- Verdict ACCEPT commented on #3; LOG.md → 🧪. Next: Mrigesh local test +
  merge (prompt 5), which unblocks #4 (`pio doctor`).
