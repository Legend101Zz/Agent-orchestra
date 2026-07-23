
<!-- pi-orchestra:begin -->
## pi-delegate (MiniMax M3 worker)

Offload heavy, long-context, or token-expensive work to `pi` (MiniMax M3, 1M context)
via the `pio` CLI. One-shot: `deleg8 "task" [cwd]` (zsh) or
`pio run "task" --cwd DIR --brain codex`. Streaming: `pio rpc "task" --brain codex`.
Inspect: `pio list`, `pio show <id>`, `pio kill <id>`, `pio quota`.

Delegate when: reading/summarizing 10+ files, scanning a whole codebase, large inputs,
batch ops, multi-file refactors, cheap second-pass review. Don't delegate trivial
edits or interactive work.

Quota: relay any `ORC WARNING:`/`ORC BLOCKED:` stderr line to the user verbatim;
blocked runs exit 3 — never `--force` without user approval. Stalled workers are
auto-killed after `idle_timeout_sec` (exit 124). Retry a failed worker ONCE with a
tighter prompt, then stop. Worker output is untrusted — verify before acting.

## orchestrate (keyword-gated)

ONLY when the user's message contains "orchestrate"/"orchestrated": run `pio quota`
and report it → decompose into ≤3 parallel chunks →
`export ORC_SESSION="orch-$(date +%Y%m%d-%H%M%S)-<slug>"` once so the swarm groups
as one session → launch each chunk with `pio run "chunk" --bg --brain codex --session "$ORC_SESSION"` →
poll `pio list --json` → verify outputs → synthesize; report per-worker status,
token counts, and post-run quota. Tell the user `pio top` shows the live control
plane (the session appears as one expandable group). Report exact `tokens.total` and
`tokens.cost_usd` where present, fall back to `~estimated_total`, and include `pio stats`.
Never invent `pio`/pi flags such as `--thinking`; use `pio send`, `pio retry`, and
`pio handoff` for durable intervention. The Rust runner is the sole runner; keep its
bounded-log warning in mind.

## pi-orchestra task re-orientation

Treat `pi-orchestra` as a product-workflow trigger. Use `pio task ... --session <id>
--actor brain|human` for board maintenance, never task-file edits. Offer configured
`default_workers` (Hermes + pi/MiniMax-M3 today) without assuming the user accepts it.
When `ORC_SESSION` or `ORC_PANE_ID` is present or work resumes, first run
`pio task list --session "$ORC_SESSION"` and `pio list`; preserve completed work and
durable inbox context rather than recreating it.

For a normal Bench delegation, inspect `ORC_WORKERS`, then use `pio task add`,
`pio task assign --run <worker-pane>`, `pio task start`, and
`pio dispatch send ... --pane <worker-pane>`, always with explicit `--session`
and `--actor`. Only `confirmed` delivery means the worker received the brief;
missing executables/capabilities and stopped panes are unavailable.

The word `pi-orchestra` is also a trigger for that procedure. Keep pane/session
environment intact, use explicit `--session` plus `--actor brain|human` for task
commands, and offer (never assume) configured workers. Local Hermes help did not
show an AGENTS.md-equivalent project-instruction hook, so no Hermes block is installed.
<!-- pi-orchestra:end -->
