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

## Gates

(recorded per commit below)
