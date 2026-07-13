# Phase 6 evidence — daemon lifecycle, RUNS embed, first-run UX (2026-07-12)

Baseline: `main` = `origin/main` = `ls-remote` = `57d837c`. Protected-path
checksums captured before work (scratchpad `protected-checksums-before.txt`)
and reproduced after the final gate (see end of this note).

## 6A — build handshake, honest errors, daemon lifecycle

### Response-size measurements (debug build, isolated ORC_HOME, real orcd)

Three daemon-hosted panes running a continuous truecolor "noise" script
(every cell styled: 24-bit fg + bg, bold/italic/underline), resized by
protocol request, measured over the real Unix socket with a raw JSON client:

| Pane grid (rows×cols) | Cells (3 panes) | `attach_session` | `snapshot` (all panes) |
|---|---|---|---|
| 84×320 ("big terminal") | 80,640 | 4,397,308 B | 4,368,101 B (54.2 B/cell) |
| 200×400 (protocol max) | 240,000 | 20,674,010 B | 20,670,921 B (86.1 B/cell) |

Conclusion: a single 3-pane fully-styled session at the protocol maximum is
already at ~62% of the 32 MiB client cap; a 4-pane session extrapolates to
~27.5 MB, and the previously unfiltered `snapshot` request summed **every**
session hosted by the daemon, so two busy sessions could exceed the cap and
kill the client's watch thread with the old opaque error. Bounded
daemon-side instead of raising the client cap:

- `snapshot` now accepts an optional `session_id` filter; the shell's
  screen-watch thread always filters to the attached session.
- `orcd` never emits a response over 32 MiB; it replaces it with an explicit
  error naming the byte count and remedy (regression-tested).

An earlier measurement run gave misleading numbers (3.45 MB at 200×400)
because the noise script had finished before the resize, leaving most cells
unstyled; the table above is from the corrected continuous-output run.

### Reproduce-then-fix: new client against a deliberately old daemon

Old daemon = the installed release `orcd` built from `57d837c` **before**
Phase 6 (no build field, no status protocol), run with an isolated
`--home/--socket`. New client/CLI = Phase 6 debug builds.

```
$ pi-orchestra --socket .../orcd.sock home       # new client, old daemon
Error: the running daemon predates this client (client build 0.4.0+57d837c0e90f) — detach other clients, then run `orc daemon restart`

$ orc daemon status                              # old daemon → exit 5
orcd: running
  pid: unknown (daemon predates the status protocol)
  build: unknown (older than client 0.4.0+57d837c0e90f)
  ...
  BUILD MISMATCH — detach clients, then run `orc daemon restart` (live panes die with the daemon)

$ orc daemon restart                             # refuses: cannot verify panes
refusing to restart: this daemon predates pane reporting, so live panes cannot be verified and would die silently
re-run with --force to restart anyway            # exit 1

$ orc daemon restart --force                     # old → new build, exit 0
stopping orcd (pid 63767)
starting orcd on build 0.4.0+57d837c0e90f
orcd: running ... build: 0.4.0+57d837c0e90f ... live panes: 0 (of 0 hosted)
```

With a live pane on a current-build daemon, plain `restart` refuses and
prints exactly what would be lost:

```
refusing to restart: 1 live pane(s) die with the daemon (PTYs are daemon-owned):
  cwd-1783854740-0000-brain · session cwd-1783854740-0000 · noise brain
re-run with --force to kill them and restart     # exit 1
```

The user's real daemon (`--home ~/.orchestra`) was running throughout and
was never touched; pid discovery matches the `--socket` argument, so
multiple per-home daemons are disambiguated (verified: with two daemons
running, restart stopped only the isolated one).

### Known limitation

The build identifier is crate version + git commit of HEAD at compile time.
Two dirty-worktree builds from the same commit are indistinguishable; the
mechanism targets the real failure class (installs from commits, daemons
outliving installs), and `install.sh` now probes the running daemon and
prints restart guidance when it predates the installed build.

## 6B — embedded RUNS view: honest and alive

Root causes fixed: the embed called `orc_tui::draw` but never routed keys
into `orc_tui::App` (the v3 legend advertised dead functionality), and
nothing scheduled a redraw between events, so channel-delivered quota and
registry updates were not painted until the next keypress.

Changes:
- Keys in `ShellView::Runs` now route into `App::handle_key` via
  `route_runs_key`. Documented exits stay reserved at the App's top-level
  dashboard only (`V`/`h`/`Esc` → HOME, `q` → quit); deeper views and
  active text inputs receive every key, so Esc cancels a search or returns
  from the session workspace instead of leaving the ledger, and literal `V`
  types into search. An App-initiated quit leaves only the shell view.
- The embed legend is view-aware (dashboard / session / settings) and lists
  only interactions that actually route; "read-only" removed. Legends fit
  72 columns.
- A 500 ms ambient tick repaints the RUNS view (kept under reduced_motion —
  it is data refresh, not animation); `App::refresh` stays internally
  rate-limited to 500 ms, so no busy loop.
- `raw_home_keys` now decodes Tab, BackTab, PageUp/PageDown for the embed.

Tests: TestBackend renders of the embedded view (both themes, 150x44 and
exactly 72x30, asserting the legend line), plus routing tests covering
selection, expansion, search-input capture of `V`/`Esc`, reserved exits,
session-view Esc, and tab cycling.

Live tmux smoke (120x36, release build, isolated ORC_HOME):
captures under `docs/notes/2026-07-13-phase6b-captures/`.
- `runs-dashboard.txt` — embed renders the control plane with the new legend;
- `runs-live-redraw-no-keypress.txt` — a registry run written while idle
  appeared with zero keypresses (the "frozen unless V pressed" symptom);
- `runs-session-detail.txt` — enter opened the session workspace, tab
  cycled tabs, the legend switched to session keys, Esc returned to the
  dashboard; `h` then exited HOME, `V` re-entered RUNS, `q` quit cleanly
  (exit 0, prompt restored).

## 6C — first-run HOME, launch flow, shelf health, SCORE polish

Wire (all serde-defaulted for compatibility): `HarnessSummary` gains
`available` (executable resolves on PATH) and `dispatch_verified` (bounded
non-interactive dispatch locally verified); `SessionSummary` gains
`workers_live`, `workers_total`, and `conductor` (`live`/`down`/`dead`).
The daemon computes availability from the existing adapter summary (no
provider contact) and pane health from what it actually hosts, so a session
record claiming `running` panes that died with a previous daemon is
reported honestly as dead.

HOME: the empty state teaches brain/worker/detach in plain language, lists
the three first keys (`n`, `enter`, `?`), shows the configured leader chord
from `HomeData.leader_key` (tests assert no hardcoded ctrl-g), and renders
a BENCH AVAILABILITY strip. The shelf shows per-card pane health with the
`R` hint only where recovery genuinely applies (`down`, i.e. hosted but
exited); `dead` sessions read `ALL PANES DEAD · daemon restarted`.

Launch flow (bug B4): the cwd step keeps its client-cwd default, gains
`tab` directory completion (single match completes with a slash, several
extend to the longest common prefix, files never complete), `ctrl-u`
clear-line, a live validity line, a launch-refusing validation with the
reason in place, and a confirmation line naming the chosen brain and
workers. `~` expands via `$HOME`.

SCORE (bug B2): every column now truncates its lines to the column width
with an ellipsis and keeps a one-cell right gutter; a TestBackend test
proves the rightmost column of a 72-column board keeps its gutter clear and
shows the ellipsis.

Tests: HOME empty/shelf snapshots at both themes, 150x44 and 72x30, with
teaching/leader/availability/health assertions; cwd-step render test at
both sizes; pure unit tests for tilde expansion, directory completion on a
real temp tree, and ellipsis clipping; daemon tests assert `down` + 1/1
live workers on the live daemon and `dead` + 0 live after a restart, plus
adapter-derived availability.

Live tmux smokes (captures in `docs/notes/2026-07-13-phase6c-captures/`):
- `home-first-run.txt` — teaching HOME with ctrl-b leader (mirrors the
  user's registry) and an honest availability strip including a
  deliberately missing executable (`NOT ON PATH · unavailable`);
- `cwd-step-completion-validation.txt` — default `/private/tmp` (client
  cwd), `tab` completing `de` → `deep-subdir/`, `ctrl-u`, and
  `/nope/nothing` flagged NOT A DIRECTORY with enter refused in place;
- `shelf-conductor-down.txt` — real fixture session: `2/2 workers ·
  CONDUCTOR DOWN · R recovers`;
- `shelf-daemon-restarted.txt` — after killing and restarting orcd the
  same card reads `ALL PANES DEAD · daemon restarted`;
- `score-72col-ellipsis.txt` — long titles in BACKLOG and DONE ellipsized
  at 72 columns with the right gutter intact.

README media re-captured with VHS (`tools/v4-phase6-home.tape`) against a
real daemon and a real claude + hermes + pi-m3 session on this machine:
`docs/home-welcome.{gif,png}` (teaching HOME + availability),
`docs/home-flow.png` (new cwd step), `docs/home-shelf.png` (shelf card
showing `2/2 workers live · READY` for the real session).
`docs/score-board.png` is unchanged from Session 8: the SCORE change only
affects titles that overflow their column, which that capture does not.
README text updated (first session, cwd keys, shelf health, daemon
lifecycle rows, build-mismatch troubleshooting).

## Gates and shipping (final, 2026-07-13)

Every commit was gated before push with, in the isolated
`CARGO_TARGET_DIR=/tmp/pi-orchestra-phase6`:
`cargo fmt --check` (clean); `cargo clippy --all-targets -- -D warnings`
(0 errors); full test suite (final count 86 passed / 0 failed, including
the new TestBackend renders at both themes, wide and exactly 72x30);
`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` (clean);
`cargo build --locked --release` (clean); `git diff --check` (clean).

Real tmux smokes of every user-visible fix, captures under `docs/notes/`:
- stale-daemon actionable message + status exit 5:
  `2026-07-13-phase6-final-captures/stale-daemon-actionable-message.txt`;
- RUNS interaction and no-keypress redraw: `2026-07-13-phase6b-captures/`;
- new HOME, cwd step, shelf health, SCORE gutter:
  `2026-07-13-phase6c-captures/`.

Install refresh: `./install.sh` rebuilt and relinked; its new daemon probe
flagged the user's running pre-Phase-6 orcd. That daemon predates pane
reporting, so liveness was verified externally (`pgrep -P` showed no
children) before `orc daemon restart --force`; afterwards
`orc daemon status` exits 0 with daemon and client both on
`0.4.0+e6bf1cd57f33`
(`2026-07-13-phase6-final-captures/install-refresh-daemon-status.txt`).

Protected paths: `~/.claude/settings.json` and every file under
`~/.pi/agent/` reproduce their pre-work checksums exactly.
`~/.codex/config.toml` changed externally during the session
(`f0a989ad…` → `fdbc233c…`, mtime 2026-07-13 16:38); nothing in this
repository touches that file (the installer's owned block is in
`~/.codex/AGENTS.md`), so the discrepancy is recorded, not "fixed".

Session interruption: the external SSD hosting the repo stopped answering
reads mid-session (fsck could not read the container superblock). All
completed work had already been pushed; after a replug, `git fsck` was
clean and `main` matched `origin/main`.

Commits on `main`: `385dcbb` (6A), `dacd6e4` (B5+B1), `c6afe21` (6B),
`e6bf1cd` (6C), plus the final docs/evidence commit. Phase 7 was not
started.
