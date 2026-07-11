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

The installer performs a locked Rust release build and safely links all three
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
cargo build --manifest-path rust/Cargo.toml --release --locked
rust/target/release/pi-orchestra home
```

## Bench client

```bash
pi-orchestra home
pi-orchestra attach                 # newest durable session
pi-orchestra attach <session-id>
orc top                             # opens the honest RUNS shell placeholder
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
| `V` | cycle to/from the RUNS shell |

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
    "hermes": {"command":"hermes","args":["--tui"],"resume_args":[],"roles":["brain","worker"],"adapter":"hermes"},
    "pi-m3": {"command":"pi","args":["--provider","minimax","--model","MiniMax-M3"],"resume_args":[],"roles":["brain","worker"],"adapter":"pi"}
  },
  "default_workers": ["hermes", "pi-m3"],
  "max_parallel_workers": 3,
  "app": {"leader_key":"ctrl-g","reduced_motion":false,"theme":"ember"}
}
```

Only `ember` and `phosphor` are supported themes.

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

The required multi-hour flood result and exact producer/CPU/RSS/coalescing
numbers are recorded in the Phase 2 evidence note under `docs/notes/`.

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
