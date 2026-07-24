# Harness invocation-style fixtures (issue #6, V1-4)

Fake worker harnesses, one per **invocation style** the universal worker
adapter must drive. They let `orc-core`'s dispatch tests prove that the adapter
picks the right style from the capability probe results — without spawning a
real model provider.

| Fixture | Style | Real harnesses it stands in for |
|---|---|---|
| `flag-style.sh` | `<cmd> <flags…> "<brief>"` (brief = final arg) | `claude -p`, `hermes -z`, `pi -p` |
| `subcommand-style.sh` | `<cmd> <subcommand> <flags…> "<brief>"` | `codex exec`, `opencode run` |

Each script receives the brief as its final positional argument and returns a
**confirmed receipt** (`exit 0`) that echoes:

- the full `argv` it saw (so a test can assert the chosen style/flags),
- the `brief` (so a test can assert the prompt was delivered),
- the `cwd` (so a test can assert orchestrator-provided working-directory
  control).

Tests invoke them through `/bin/sh <script> …`, so the executable bit is not
required; the harness `command` is `/bin/sh` and the script path is its first
argument. See `rust/crates/orc-core/tests/invocation_dispatch.rs`.
