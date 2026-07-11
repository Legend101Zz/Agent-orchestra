# pi-orchestra

pi-orchestra is a Rust terminal workspace for one expensive conductor and a
user-editable pool of harness workers. `orcd` owns the PTYs and durable screen
state; `pi-orchestra` renders HOME and STAGE; `orc` remains the headless
delegation and registry CLI. Provider traffic goes directly between each
harness and its provider—there is no API proxy.

```text
pi-orchestra client  ← Unix socket →  orcd  → conductor PTY + worker PTYs
        HOME / STAGE                  │
                                      └→ ~/.orchestra plain JSON records
orc run / rpc / list / show / quota ────────────────────────────────┘
```

## Install and uninstall

```bash
./install.sh
./uninstall.sh
```

The installer performs a locked Rust release build in an isolated target under
your install HOME (or `ORC_INSTALL_CARGO_TARGET_DIR`) and safely links all three
binaries into `~/.local/bin`:

- `orc`
- `orcd`
- `pi-orchestra`

Existing commands are backed up before replacement. Marked shell/AGENTS blocks
are additive and idempotent. Uninstall removes the links and marked blocks but
preserves `~/.orchestra` data. The installer never edits
`~/.pi/agent/*`, `~/.claude/settings.json`, or `~/.codex/config.toml`.

Build without installing:

```bash
CARGO_TARGET_DIR=/tmp/pi-orchestra-build cargo build --manifest-path rust/Cargo.toml --release --locked
/tmp/pi-orchestra-build/release/pi-orchestra home
```

## First use

After installing, open a fresh terminal (or reload the helper functions), then
confirm that the installed commands resolve before opening the workspace:

```bash
source ~/.zshrc                 # only needed in an already-open zsh terminal
command -v orc orcd pi-orchestra
orc version
pi-orchestra home
```

In HOME, press `n` and choose a brain, review the preselected worker pool,
then choose a working directory. The daemon launches the session and STAGE
opens its panes. Close the client with `ctrl-g q` to detach safely; reconnect
later with:

```bash
pi-orchestra attach
```

Use the durable command line when you want to inspect, maintain, and delegate
from the board. A supported worker is marked received only after confirmed
non-interactive delivery:
outside the UI. Always pass the session shown by HOME and state who made a
mutation:

```bash
orc task list --session <session-id>
orc task add "small, reviewable task" --session <session-id> --actor human
orc task move T0001 review --session <session-id> --actor human
orc task assign T0001 hermes --run <worker-pane> --session <session-id> --actor brain
orc task start T0001 --session <session-id> --actor brain
orc dispatch send T0001 hermes "bounded brief" --pane <worker-pane> --session <session-id> --actor brain --json
orc list
orc quota
```

The configured default worker choices are offers, not assumptions: edit the
pool in HOME before launch. `orc run` requires a working local `pi` executable;
if it is unavailable, use HOME/STAGE or configure/install the worker first.

## Phase 3: durable tasks, worktrees, and SCORE

Tasks are plain additive JSON, mutated only through `orc task` with an explicit
session and actor. `--json` returns the complete task record or diff object.

```bash
orc task add "review parser" --isolate --session bench-1 --actor brain --json
orc task assign T0001 pi-m3 --session bench-1 --actor brain
orc task start T0001 --session bench-1 --actor brain
orc task review T0001 --session bench-1 --actor human
orc task diff T0001 --session bench-1 --json
orc task merge T0001 --session bench-1 --actor human --json
```

Isolation creates only an owned `orc/<session>/<task>` branch under the owned
worktree root. It refuses dirty, detached, non-Git, reused, symlinked, or
unprovable paths and never auto-resolves merge conflicts. `drop` preserves the
audit record and prunes only an owned clean worktree.

SCORE is the task board in an attached session: `j/k` selects, `h/l` requests
the adjacent valid lifecycle move, mouse drag requests a column move through
the daemon as `human`, `g` focuses the linked STAGE pane, and `ctrl-g b`
returns from STAGE to SCORE. SCORE is covered with ember/phosphor TestBackend
snapshots at wide and exactly 72x30 sizes; RUNS and baton animation remain
outside Phase 3.

## Bench client

```bash
pi-orchestra home
pi-orchestra attach                 # newest durable session
pi-orchestra attach <session-id>
orc top                             # opens the RUNS ledger
```

HOME shows durable sessions and a three-step launch flow:

1. choose a brain;
2. review/edit the worker pool (Hermes + pi/MiniMax-M3 are preselected);
3. choose the cwd.

STAGE renders daemon-owned terminal panes as floating ensemble cards with
rounded corners, half-block shadows, brass focus, keyboard swap, mouse drag,
resize, zoom-to-solo, and persisted per-session layout. Focused keyboard bytes
are forwarded raw, including kitty extended keys and bracketed paste. Mouse
coordinates are translated only when forwarding into pane content. `ctrl-g` is
the only leader; double-tap sends literal control-G.

Useful STAGE keys:

| Keys | Action |
|---|---|
| `ctrl-g n` / `ctrl-g p` | focus next / previous pane |
| `ctrl-g s` | swap focused pane with the next pane |
| `ctrl-g z` | zoom focused pane / restore ensemble |
| `ctrl-g +` / `ctrl-g -` | resize focused card |
| drag a card header | reposition and persist layout |
| `R` on a dead conductor | resume when the harness has `resume_args` |
| `ctrl-g h` | return HOME |
| `ctrl-g q` | detach; panes continue in `orcd` |
| `V` | cycle HOME / SCORE / RUNS |
| `?` | open or close first-use and recovery help |

When a brain exits, workers remain alive and its last screen becomes
`CONDUCTOR DOWN` with elapsed time. Recovery uses the configured command,
`resume_args`, cwd, `ORC_SESSION`, and `ORC_PANE_ID`. A harness without resume
support states `RESUME NOT SUPPORTED`; pi-orchestra never invents it.

The daemon starts on demand at `~/.orchestra/orcd.sock`. The parent is mode
`0700`, the socket is `0600`, attachment is bounded, and stale sockets are
removed only after type/owner/live-listener checks. Logs rotate under a bounded
retention policy at `~/.orchestra/orcd.log`. Closing a client is a normal detach,
not a warning.

Remote use needs no web server: SSH or mosh into the machine and run
`pi-orchestra attach`.

## Harness registry

`~/.orchestra/harnesses.json` is plain additive JSON and written atomically.
Unknown fields survive round trips. Defaults:

```json
{
  "harnesses": {
    "claude": {"command":"claude","args":[],"resume_args":["--continue"],"roles":["brain","worker"],"adapter":"claude"},
    "codex": {"command":"codex","args":[],"resume_args":["resume"],"roles":["brain","worker"],"adapter":"codex"},
    "hermes": {"command":"hermes","args":["--tui"],"resume_args":[],"roles":["brain","worker"],"adapter":"hermes","dispatch_args":["-z"]},
    "pi-m3": {"command":"pi","args":["--provider","minimax","--model","MiniMax-M3"],"resume_args":[],"roles":["brain","worker"],"adapter":"pi","dispatch_args":["-p","--no-session"]}
  },
  "default_workers": ["hermes", "pi-m3"],
  "max_parallel_workers": 3,
  "app": {"leader_key":"ctrl-g","reduced_motion":false,"theme":"ember"}
}
```

Only `ember` and `phosphor` are supported themes.

## Phase 5: verified adapters and availability

Check what this machine can honestly offer before creating work:

```bash
orc adapter list
orc adapter list --json
```

This command never contacts a provider. It reports the configured executable,
the declared non-interactive path, steering support, and exact-usage semantics.
An executable or a registry entry by itself is not delivery proof.

- **Hermes:** `hermes --help` verified `-z/--oneshot`, so a configured Hermes
  worker can receive a bounded brief. Hermes has no verified durable steering
  or exact-usage event, so both are displayed as unavailable.
- **pi / MiniMax M3:** `pi --help` verified `-p`, `--mode`, `--provider`, and
  `--model`; a real MiniMax M3 probe passed. pi supports one-shot delivery and
  `orc rpc` follow-ups. Exact usage is recorded only if the completed pi event
  contains usage—absence remains estimated, never fabricated.
- **Claude and Codex:** they remain interactive-pane choices only in this
  release. No headless delivery, steering, or exact-usage adapter is claimed.

Fresh registries include the `dispatch_args` shown above. Existing
`~/.orchestra/harnesses.json` files are intentionally not rewritten: add the
verified `dispatch_args` yourself after inspecting the local command help, or
leave the worker visibly unavailable. `orc adapter list` makes that degradation
explicit.

`orc run` is usable only when `pi --list-models minimax` lists `MiniMax-M3` and
a small local run succeeds. If either check fails, do not use `--force` or
claim the worker is available; use an available Bench worker or fix the local
pi installation first.

## Phase 4: confirmed delivery and polish

Every Bench pane starts with `ORC_SESSION`, `ORC_PANE_ID`, `ORC_WORKERS`, and
an `ORC_DELEGATE_HINT`. `ORC_WORKERS` is an offer: the brain selects a running
pane whose harness declares a demonstrated non-interactive `dispatch_args`
capability. Hermes uses its locally verified `-z/--oneshot` interface and a
fresh pi registry uses `-p --no-session`. Missing executables, missing
capability, stopped panes, timeouts, and non-zero exits are durable failures
and are never presented as receipt.

Dispatch records are bounded to 16 KiB prompt/output excerpts and a bounded
timeout. A confirmed record writes `delivery_confirmed` into the task history
and links the worker pane so SCORE and STAGE replay the state after reattach.
No terminal keystrokes are injected and provider traffic is never proxied.

HOME now includes a first-launch title and teaching empty state. Every view has
a compact key legend; `?` opens help. STAGE baton motion has bounded per-event
profiles and the configured reduced-motion path leaves a static filament.
RUNS embeds the v3 event ledger instead of the Phase 2 placeholder. A native
filesystem watcher refreshes it on registry changes without a polling loop.

Measured Phase 4 release behavior on the development M-series Mac:

- Unix-socket round trip, 5,000 samples: p50 **13 µs**, p95 **15 µs**,
  p99 **16 µs**, maximum **162 µs**.
- PTY input to visible replay, 100 samples: p50 **3.365 ms**,
  p95 **3.544 ms**, p99 **3.628 ms**, maximum **3.645 ms**.
- After the measurements settled, the isolated daemon measured **0.0% CPU**
  and **7,536 KiB RSS**.
- A bounded 20,000-line burst produced 268,890 bytes in 19,825 chunks; 20
  snapshots coalesced 19,819 intermediate generations. The sampled daemon was
  10.7% CPU and 14,112 KiB RSS during that short burst. Canonical PTY state was
  retained; this is a bounded stress sample, not a new two-hour soak claim.

## Headless CLI

| Goal | Command |
|---|---|
| Delegate once | `orc run "task" --brain codex` |
| Streaming RPC worker | `orc rpc "task" --brain codex` |
| List / inspect / kill | `orc list` / `orc show <id>` / `orc kill <id>` |
| Send an RPC follow-up | `orc send <id> "message"` |
| Retry / reviewed handoff | `orc retry <id>` / `orc handoff <id> "remaining work"` |
| Usage and savings | `orc stats --json` |
| MiniMax quota | `orc quota` (exit 0 ok / 2 warn / 3 block / 4 unknown) |
| Bound a stalled worker | `orc run "task" --idle-timeout 120` |

Shell helpers installed by the marked block:

```bash
deleg8 "task" /path/to/cwd
pi-rpc "task"
```

Quota transport failure is fail-open and printed as `ORC NOTE`. Warn and block
messages are `ORC WARNING` and `ORC BLOCKED`; callers must relay those lines
verbatim. Worker output is untrusted until the brain verifies it.

## Measured Phase 2 behavior

Release measurements on the development M-series Mac:

- Unix-socket round trip, 5,000 samples: p50 **13 µs**, p95 **17 µs**,
  p99 **42 µs**, max **72 µs**.
- PTY input to visible snapshot replay, 100 samples: p50 **3.770 ms**,
  p95 **4.000 ms**, p99 **4.363 ms**, max **4.601 ms**.
- Five idle samples: daemon **0.0% CPU**, client **0.0% CPU**; observed RSS
  about 6.7 MiB and 2.8 MiB respectively.

The required four-pane flood ran for 7,608 seconds (2 h 6 m 48 s), with each
producer writing 1,024 lines then pausing 50 ms. Daemon CPU was 21.2% at the
start, peaked at 36.5%, and ended at 22.8%; RSS was 31,168 KiB, 52,672 KiB,
and 33,520 KiB respectively. The captured in-run metrics recorded 33,392
coalesced updates and no dropped canonical PTY state. The exact interruption
caveat and raw evidence are recorded in the Phase 2 evidence note under
`docs/notes/`.

Visual evidence:

- `docs/v4-phase2-shell.gif` — HOME, launch flow, wide ember STAGE, zoom,
  swap, detach and reattach.
- `docs/v4-phase2-narrow-phosphor.gif` — new-session flow and STAGE at exactly
  72×30 in phosphor.

## Compatibility and verification

The former Python implementation was removed only after its behavior was
captured. The immutable corpus under
`rust/crates/orc-core/tests/fixtures/python-v3/` is now the compatibility
oracle for current, legacy, corrupt, exact-usage, killed, orphaned, RPC,
session-linked, retry, handoff, CJK, combining-mark, and wide-character data.
Rust tests compare meaningful JSON/exit structure and preserve unknown fields.

```bash
cd rust
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo build --release --locked
```

Historical design, benchmark, and review documents retain their original
language labels for auditability; they are not runtime fallback instructions.
