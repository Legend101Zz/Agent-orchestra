# Findings: v3-rust review

## Confirmed
- F1 (HIGH): quota fetch timeout regression. Python: `urlopen(req, timeout=15)` (orc_pkg/quota.py:77). Rust: ureq 3.3 `.call()` with no timeout config (quota.rs:114-124); verified ureq 3.3.0 Timeouts::default() = all None (except await_100). A hanging endpoint (dropped packets, stuck TCP) blocks `apply_gate` → `orc run/rpc/retry/handoff` hang forever; also `orc top` startup (App::new → get_quota) blocks before first paint. Violates "quota gate fails open / never blocks work when endpoint is down". Also `security` keychain subprocess: Python timeout=10, Rust unbounded.
- F2 (MED): TUI quota staleness — quota + quota_history fetched once in App::new (app.rs:109-110), never refreshed in the event loop (lib.rs:22-45 only calls app.refresh() → snapshot). Long-lived `orc top` shows frozen quota meters/level forever; v2 floor had live meters. Also the fetch is on the render/startup path, contra spec "quota/network work off the render path".
- F3 (needs experiment): steering turn-boundary loss — RPC runner unconditionally breaks on first agent_end (runner.rs:378-383). If pi emits agent_end per turn, a prompt delivered (and ack'd as processed) just before agent_end never produces an observed turn; run ends, stdin dropped. Severity depends on pi RPC turn semantics (if mid-turn prompts fold into the same agent loop, window is narrow).

## Parity deltas (lower severity)
- Rust RunMeta requires id/task/brain/cwd/provider/model/status/started_at; a legacy meta missing any → run silently dropped from list (registry.rs:251), Python still lists it (dict.get). No panic, but silent data hiding on malformed meta.
- Python-created RPC runs have no `mode` field (registry.py new_run never writes mode) → Rust `orc send` refuses them as "one-shot JSON run" (control.rs:160). Consistent with Python (which can't steer either); message slightly misleading.
- Rust human `orc stats` output omits by_status line, per-brain rows, week columns that Python prints. JSON parity is the contract; human diff acceptable but visible.
- `orc run` with no command: both print help, exit 1 ✓. quota exits 0/2/3/4 ✓ both.

## Suspicions to verify
- S1: quota `fetch_remains` (quota.rs:114) uses ureq 3.3 with no explicit timeout — if endpoint hangs (not refuses), gate may hang forever → effectively blocks work. Check ureq 3 defaults + test with hanging socket.
- S2: RPC runner breaks on FIRST agent_end (runner.rs:378-383) with no outstanding-turn counter. `orc send` steering delivered mid-turn-1 → pi queues turn 2 → runner ends run at turn-1 agent_end; steered response lost/unlogged. Verify with fake pi.
- S3: reconcile (registry.rs:254-263): status "starting" with pid==None never reconciled → stuck "starting" forever if runner dies pre-spawn. Compare Python.
- S4: TUI read-only invariant — check whether orc-tui calls list_runs(reconcile=true) which WRITES meta.
- S5: finalize() maps 124 → status "failed" + attention handoff_needed. Compare Python status word for idle timeout.
- S6: extract_usage picks LAST message with nonzero totalTokens (runner.rs:150-176). Compare Python (sum? last? max?).

## Verified-OK invariants (code level; experiments pending)
- `--offline` in both JSON_ARGS and RPC_ARGS (runner.rs:19-40), explicit minimax/MiniMax-M3 provider+model.
- RPC stdin piped, held open, task written as {"type":"prompt"}; dropped after loop.
- SIGTERM/exit-143 → code -15 → status "killed" (runner.rs:394,436; finalize code<0→killed).
- Idle watchdog → exit 124 (runner.rs:408-423,434).
- Quota gate: level unknown → Gate::Unknown → proceed (fails open at code level); block honors --force.
- Coding plan: parse_remains selects model_name=="general" (quota.rs:95-112).
- Keychain `security` first, fallback read-only ~/.pi/agent/auth.json (quota.rs:63-93).
- atomic_write_json: create_new temp in dest dir + flush + sync_all + rename (registry.rs:67-97); cleanup on error.
- Kill semantics: kill file → SIGTERM to process group; has_kill overrides exit code to killed.

## Benchmarks reproduced (hyperfine 20 runs, 5 warmup, 500-run fixture, warm quota cache)
| Command | Python mean | Rust mean | Speedup |
|---|---:|---:|---:|
| orc list (500 runs) | 132.4 ± 8.8 ms | 24.0 ± 3.2 ms | 5.51× |
| orc quota --json (cached) | 99.6 ± 2.1 ms | 5.0 ± 0.4 ms | 20.05× |
README claims (5.84× / 14.43×) are consistent — real, not asserted. README has no TUI cold-start row.

## Experiments verified (Rust binary)
- Kill: fake pi traps SIGTERM→143 → meta status "killed", exit_code -15, `orc kill` exit 0 ✓
- Idle watchdog: --idle-timeout 2 → status failed, exit_code 124, attention handoff_needed, failure_kind idle_timeout, watchdog line in log ✓
- Quota block: seeded 5% → `orc run` exit 3 + ORC BLOCKED message, no run created; --force proceeds; `orc quota` exit 3 ✓
- F3 steering experiment: two-turn fake pi → follow-up delivered+ack'd, run finalized "done" at first agent_end with turn-1 usage only; turn-2 output never logged. CONFIRMED at runner level; real-pi semantics checked in live smoke.
- Legacy meta (no session/mode/exact tokens, unknown field): Rust list/show fine; reconcile write PRESERVES unknown v2_custom_field ✓. Corrupt meta: Rust list skips silently, show gives clean error; Python show tracebacks (Rust better). Meta missing brain/provider: Python lists it, Rust silently drops (minor delta).
- TUI live: real registry ✓ (attention-first sort, session rollup 260.2k/$0.06 matches friction receipt), empty-state teaching copy ✓ phosphor ✓, narrow 72x30 reflows (stacked summary, drops CONTROLLER col) ✓, session workspace topology CODEX→3×M3 with exact tokens/cost + "model not recorded" fallback ✓, tabs ✓, mouse enabled ✓ (code), no emojis, no raw hex outside theme.rs ✓.
- Protected configs: mtimes Jul 9 (pre-work), ~/.local/bin/orc → bin/orc (Python) intact ✓.

## Process observations (part B)
(pending)

# Findings: Session 8 (2026-07-12) — polish + real-use pass

## Build audit
- Installed links: ~/.local/bin/{orc,orcd,pi-orchestra} →
  ~/.local/share/pi-orchestra/target/release/* built 2026-07-12 00:00,
  after last commit f8c00ec (= origin/main). Install is CURRENT.
- Stale: ~/.local/bin/orc.pi-orchestra.bak → /Users/comreton/Desktop/pi-orchestra
  (path no longer exists). Safe to delete.
- Uncommitted at session start: prompt cleanup note + ghostty script path fix
  (Desktop → SSD path), plus untracked planning files.

## HOME start page (current state)
- render_home in rust/crates/orc-app/src/lib.rs:776 — static block-glyph
  banner (uses └─ glyphs that render unevenly), flat line list, no framing,
  no animation despite tachyonfx being a workspace dep and a tick loop
  existing (last_tick at lib.rs:460).
- reduced_motion lives in registry app config; must leave static art when set.

## Bugs observed (to fill during dogfood)
(pending)

## Bugs observed
- B1 (flaky test): `tests::runs_watcher_wakes_on_registry_change_without_polling`
  failed once during a full parallel `cargo test` (7/8) and passes in
  isolation and on both reruns — timing-sensitive 100 ms sleep + 2 s watcher
  window under load. Pre-existing; not caused by the HOME change.
- B2 (UX): SCORE's last column (DONE) renders flush against the right edge and
  task titles clip mid-word with no ellipsis; at 5 equal columns most of the
  screen stays empty while the busiest column truncates.
- B3 (UX): `ctrl-g h` returns HOME from STAGE but does nothing from SCORE;
  from SCORE only `ctrl-g b`/`g` work. Navigation is inconsistent across
  views (captured while taping: the "HOME" attempt stayed on SCORE).
- B4 (UX): the cwd step of the launch flow starts prefilled with $HOME and
  offers no tab-completion or clear-line key; a long path needs ~30
  backspaces first.
- B5 (env leak): worker panes inherit the client's environment through the
  daemon: pi printed "Warning: tmux extended-keys is off" inside its orcd
  PTY because $TMUX/TERM leaked from the launching terminal. Daemon should
  scrub multiplexer variables from pane environments.
- Dogfood verdict: dispatch pipeline is solid — both MiniMax workers
  (hermes -z, pi -p --no-session) returned exit 0, wrote real files, and the
  combined output passed its own integration test (PASS). Confirmed receipts
  landed in task history and SCORE replayed them after reattach.
- B6 (installer, FIXED): install_skill kept ANY pre-existing symlink,
  including dangling ones — after the repo moved from Desktop to the SSD,
  ~/.claude/skills/{orchestrate,pi-delegate} pointed at the deleted path and
  Claude brains had no delegation playbook. Installer now replaces dead
  links (live user links/content still preserved); both links verified
  restored 2026-07-12.

# Findings: Session 10 (2026-07-12/13) — Phase 6

## Bug status
- B1 (flaky watcher test): FIXED — rewrite-until-event with a 10 s bound;
  three consecutive full-suite runs green.
- B2 (SCORE clipping): FIXED — per-column ellipsis truncation inside a
  one-cell right gutter; TestBackend proof at 72 cols.
- B4 (cwd step UX): FIXED — client-cwd default confirmed, tab directory
  completion, ctrl-u clear, tilde expansion, validation refuses launch
  in place, confirmation line shows brain/workers.
- B5 (env leak): FIXED — TMUX, TMUX_PANE, TERM_PROGRAM,
  TERM_PROGRAM_VERSION scrubbed from pane environments after explicit env
  application; regression test proves removal and explicit-var survival.
- "invalid or oversized response" crash: root cause was one message for
  three conditions plus a stale daemon surviving installs. Fixed with the
  build handshake, three honest messages, daemon status/restart, install
  probe, session-filtered snapshots, and a daemon-side response cap.
- RUNS embed dead/frozen: fixed (keys routed into the App + 500 ms data
  tick); "read-only" legend replaced by view-aware honest legends.

## Measurements (Phase 6A, debug build, isolated ORC_HOME)
- attach_session / snapshot with 3 fully-styled truecolor panes:
  84x320 → 4.40/4.37 MB; 200x400 (protocol max) → 20.67/20.67 MB
  (86 B/cell). A second busy session in the same unfiltered snapshot could
  exceed the 32 MiB wire cap — hence the session filter + daemon-side cap.

## Environment discrepancies (recorded, not "fixed")
- ~/.codex/config.toml changed externally during the session
  (sha f0a989ad… → fdbc233c…, mtime 2026-07-13 16:38). Nothing in this
  repo touches that file; the installer's owned block lives in
  ~/.codex/AGENTS.md. Left as-is per house rules.
- The external SSD hosting the repo stopped answering reads mid-session
  (fsck: "failed to read container superblock"); recovered by replugging.
  git fsck clean afterwards; no repository damage.
