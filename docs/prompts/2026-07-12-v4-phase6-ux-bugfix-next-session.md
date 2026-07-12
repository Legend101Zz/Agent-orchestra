# v4 Bench — Phase 6 stability and first-run UX execution prompt

Work only in `/Volumes/Mrigesh SSD/pi-orchestra`.

Start from the current verified remote `main`. First run `git fetch origin`,
prove local `main`, `origin/main`, and `git ls-remote origin refs/heads/main`
agree, then work in small conventional commits pushed to `main` after each
gate passes. Do not force-push or merge unrelated history.

You are authorized to complete **Phase 6 only**: the daemon/client stability
work, the RUNS embed repair, and the first-run HOME/UX redesign described
below. Do not start a web surface, a provider proxy, or any unapproved
feature. Never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
`~/.codex/config.toml`. Use an isolated `CARGO_TARGET_DIR` for the final
gates. The live install may be refreshed only through `./install.sh`.

## Required reading and preflight

Before editing, read in full: `README.md`, `progress.md`, `task_plan.md`,
`findings.md` (Sessions 8–9 record verified bugs B1–B6 with locations),
`docs/prompts/2026-07-11-v4-phase4-phase5-next-session.md` (house rules),
the orc-app/orc-daemon/orc-proto/orc-tui sources, `install.sh`, and this
prompt. Capture protected-path checksums before work and reproduce them
after the final gate.

## Context already established (do not re-fix, do not regress)

- 2026-07-12 session 9 (`911b9b8`) made `?`/`V` view-aware, gave the RUNS
  embed exit keys (`V`/`h`/`Esc`/`q`) and a legend, added SCORE leader
  handling, and made the leader chord configurable end-to-end via registry
  `app.leader_key` → daemon Home response → client validation.
- The user's live registry is set to `"leader_key": "ctrl-b"` — preserve it.
- `0900fa0` made the installer replace dangling skill symlinks; the Claude
  skills now resolve to this checkout.
- The user's long-running stale `orcd` (19.5 h old, predating the protocol
  field) was manually stopped on 2026-07-12; a fresh daemon on the current
  build verified healthy. That stale daemon is the leading suspect for the
  user-reported crash below — the class of failure must now be made
  impossible to hit silently.

## Phase 6A — daemon lifecycle and the "invalid or oversized response" crash

User report: while using a session, the client exits with
`Error: daemon rejected request: invalid or oversized response`, sometimes
with a leading `^[[I` on the terminal.

Verified facts to build on:

1. `BenchClient::request` (orc-app `lib.rs` ~404–418) returns that single
   message for three unrelated conditions: EOF (`read == 0`, i.e. the daemon
   dropped the connection), a response over `MAX_RESPONSE_BYTES` (32 MiB),
   and a missing trailing newline. A daemon that merely closed the socket is
   reported as "oversized".
2. `orcd` persists across installs. Nothing restarts it, nothing compares
   builds, and `PROTOCOL_VERSION` alone cannot catch a same-version daemon
   running older code.
3. `^[[I`/`^[[O` are FocusIn/FocusOut reports (the client enables
   `EnableFocusChange`); they flow through the raw stdin path and, outside
   STAGE, `raw_home_keys` decodes them as junk characters.

Required work, test-first where a regression test is expressible:

- Add a build-identity handshake: the daemon's `Welcome` gains a build
  identifier (crate version plus compile-time git hash or equivalent,
  serde-defaulted for wire compatibility). On mismatch the client must show
  one clear, actionable line (e.g. "daemon build X is older than client Y —
  detach, then run `orc daemon restart`") instead of failing obscurely later.
- Add `orc daemon status` (running/pid/build/socket, live pane count) and
  `orc daemon restart`. Restart must refuse while live panes exist unless
  `--force`, because daemon-owned PTYs die with it; print exactly what will
  be lost. `install.sh` must detect a running daemon on an older build and
  print the restart guidance.
- Split the client error into three honest messages (connection closed /
  response too large with the observed size / malformed response), and stop
  exiting the whole client on a recoverable command failure where a message
  in place (HOME/SCORE message line) is sufficient.
- Measure real response sizes for `attach_session` and snapshots with three
  large busy panes at a big terminal size; record the numbers in the
  evidence note. If any plausible session approaches the cap, bound or
  chunk the response daemon-side rather than raising the client cap blindly.
- Consume focus-report sequences in the client raw path (never type `[I`
  into a flow field, never forward them from non-STAGE views).
- Reproduce-then-fix evidence: demonstrate a new client against a
  deliberately old daemon build produces the new actionable message.

## Phase 6B — make the embedded RUNS view honest and alive

User report: Shift+V lands on the "orc top control plane" (this is the
intended third view of the `V` cycle — HOME → SCORE → RUNS), but none of the
commands its own legend advertises work, and the screen feels frozen unless
`V` is pressed repeatedly.

Verified facts: the embed calls `orc_tui::draw` but never routes keys into
`orc_tui::App`, so the v3 control-plane legend (tabs, theme, expansion keys)
advertises dead functionality; and in the Runs view nothing schedules a
redraw between events, so quota/history updates arriving on the App's
internal channel are not painted until the next keypress — the "must press
V to sustain it" symptom.

Required work:

- Route keys in `ShellView::Runs` into the orc-tui App's existing input
  handling so the advertised interactions (selection, expansion, tabs,
  theme, whatever the App genuinely supports embedded) actually work; keep
  `V`/`h`/`Esc`/`q` as documented exits, quitting only the shell view, and
  keep the one-line embed legend consistent with reality.
- Give the Runs view a modest periodic redraw (reuse the ambient-tick
  pattern from HOME; do not busy-loop; respect reduced_motion by dropping
  animation, not data refresh).
- TestBackend coverage for the embedded RUNS view at both themes, wide and
  72x30, including the legend line.

## Phase 6C — first-run HOME and launch-flow UX

The user wants a start screen a new user can understand without reading the
repo. Keep the ember/phosphor identity, the masthead avatar, and honest
claims. Design first, then implement:

- HOME should teach at a glance: what a brain is, what workers are, that
  sessions survive detach, and the three first keys (`n`, `enter`, `?`).
  Show the configured leader chord (it is in `HomeData.leader_key` — the
  user runs `ctrl-b`) rather than hardcoding `ctrl-g` anywhere.
- Add an availability strip: which configured harnesses actually resolve on
  PATH and which dispatch capability is verified (the daemon/core already
  know this via the adapter summary; never contact a provider from HOME).
- Session shelf cards must show pane health (live workers / conductor down /
  all dead after a daemon restart) so a dead session is not a surprise on
  attach; keep the `R` recovery hint visible where it applies.
- Launch flow cwd step (recorded bug B4): default to the directory the
  client was started from instead of `$HOME`, support tab-completion or at
  minimum a clear-line key and path validation before launch, and show the
  chosen brain/workers in the confirmation line. All three of the user's
  stranded test sessions were created with cwd `$HOME` because of this.
- SCORE polish (recorded bug B2): stop clipping the last column at the
  right edge; truncate titles with an ellipsis and use the freed width.
- Update README screenshots/GIFs afterwards with real captures (VHS tapes
  exist under `tools/`; follow the Session 8 pattern) and keep the docs'
  key tables in sync with the leader-aware legends.

## Also in scope, small

- B5: scrub `$TMUX`, `$TMUX_PANE`, and stale `$TERM_PROGRAM` from
  daemon-spawned pane environments (worker TUIs printed tmux warnings
  inside orcd PTYs).
- B1: deflake `runs_watcher_wakes_on_registry_change_without_polling`
  (single 100 ms sleep under parallel load).
- `orc version` and `pi-orchestra --version` should print the build
  identifier introduced in 6A.

## Gates (every phase, before each push)

`cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; all tests;
`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`; locked release build in an
isolated target; TestBackend snapshots for changed views at both themes,
wide and exactly 72x30; a real tmux smoke of every user-visible fix
(stale-daemon message, RUNS interaction, new HOME, cwd step) with captures
saved under `docs/`; `./install.sh` refresh and `orc daemon status` proof
that the running daemon matches the installed build. Update `progress.md`,
`task_plan.md`, and `findings.md` honestly, including anything that did not
work. Then stop — do not begin Phase 7.
