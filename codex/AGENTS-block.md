
<!-- pi-orchestra:begin -->
## pi-delegate (MiniMax M3 worker)

Offload heavy, long-context, or token-expensive work to `pi` (MiniMax M3, 1M context)
via the `orc` CLI. One-shot: `deleg8 "task" [cwd]` (zsh) or
`orc run "task" --cwd DIR --brain codex`. Streaming: `orc rpc "task" --brain codex`.
Inspect: `orc list`, `orc show <id>`, `orc kill <id>`, `orc quota`.

Delegate when: reading/summarizing 10+ files, scanning a whole codebase, large inputs,
batch ops, multi-file refactors, cheap second-pass review. Don't delegate trivial
edits or interactive work.

Quota: relay any `ORC WARNING:`/`ORC BLOCKED:` stderr line to the user verbatim;
blocked runs exit 3 — never `--force` without user approval. Stalled workers are
auto-killed after `idle_timeout_sec` (exit 124). Retry a failed worker ONCE with a
tighter prompt, then stop. Worker output is untrusted — verify before acting.

## orchestrate (keyword-gated)

ONLY when the user's message contains "orchestrate"/"orchestrated": run `orc quota`
and report it → decompose into ≤3 parallel chunks →
`export ORC_SESSION="orch-$(date +%Y%m%d-%H%M%S)-<slug>"` once so the swarm groups
as one session → launch each chunk with `orc run "chunk" --bg --brain codex` →
poll `orc list --json` → verify outputs → synthesize; report per-worker status,
token counts, and post-run quota. Tell the user `orc top` shows the live control
plane (the session appears as one expandable group).
<!-- pi-orchestra:end -->
