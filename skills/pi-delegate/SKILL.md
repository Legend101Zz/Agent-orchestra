---
name: pi-delegate
description: Delegate heavy, long-context, or token-expensive tasks to the pi CLI running MiniMax M3 (1M context, cheap). Use when a task involves reading many files, scanning large codebases, summarizing long content, batch transformations, refactors across dozens of files, or any work where you'd otherwise burn a lot of tokens.
---

# Delegate to pi (MiniMax M3 worker)

You (the main brain) can offload heavy work to `pi`, a CLI running MiniMax M3
(1,000,000-token context, ~$0.30/$1.20 per 1M tokens). Every delegation goes through
`orc`, which registers the run in `~/.orchestra`, checks remaining MiniMax quota
first, and makes the run visible in the `orc top` control plane.

## When to delegate

- Reading or summarizing **10+ files** at once
- Scanning an **entire codebase or large directory**
- **Large inputs** (logs, dumps, big JSON, long docs)
- **Batch operations** or **refactors** across many files
- A **cheap second pass / reviewer** over work you did
- Long exploration where saving your own tokens matters

Don't delegate: trivial single-file edits, tasks needing tight user back-and-forth,
or anything where you need streaming output to make real-time decisions.

## How to delegate

One-shot (returns the worker's full output):

    ORC_BRAIN=claude deleg8 "List every TODO comment in this repo with file paths"
    ORC_BRAIN=claude deleg8 "Summarize the architecture in src/" /Users/me/projects/foo

Streaming (long tasks, shows progress):

    ORC_BRAIN=claude pi-rpc "Scan the entire repo and produce a dependency map"

If the shell functions are unavailable, call `orc run "task" --cwd DIR --brain claude`
directly. Inspect/manage runs: `orc list`, `orc show <id>`, `orc kill <id>`.

`pi-orchestra` is also a product-workflow trigger. Before resuming work, read
`orc task list --session "$ORC_SESSION"` (pass `--session` explicitly when a
command crosses shells) and `orc list`; preserve completed tasks and durable
inbox context. Offer the configured `default_workers` (Hermes + pi/MiniMax-M3
today), but never silently assume that pool.

When `ORC_PANE_ID` is present, the work originated in a daemon-owned pane:
keep both `ORC_SESSION` and `ORC_PANE_ID` intact, and use task commands with
an explicit `--session` and `--actor brain|human`. `pi-orchestra` itself is an
alias trigger for this re-orientation procedure; it does not authorize an
unconfigured worker or adapter.

For a supported Bench delegation, inspect the offered panes in `ORC_WORKERS`,
then keep task and delivery state linked explicitly:

    orc task add "<title>" --session "$ORC_SESSION" --actor brain --json
    orc task assign T0001 hermes --run <worker-pane> --session "$ORC_SESSION" --actor brain
    orc task start T0001 --session "$ORC_SESSION" --actor brain
    orc dispatch send T0001 hermes "<bounded brief>" --pane <worker-pane> --session "$ORC_SESSION" --actor brain --json

Only a `confirmed` dispatch means the worker received the brief. A missing
executable, absent `dispatch_args` capability, stopped pane, timeout, or
non-zero exit is unavailable/failed and must be reported as such.

## Quota rules (IMPORTANT)

- `orc` prints `ORC WARNING:` / `ORC BLOCKED:` / `ORC NOTE:` lines on stderr.
  **Relay any such line to the user verbatim** — they decide whether to continue.
- Blocked runs exit with code 3. Do not retry with `--force` unless the user says so.
- To check proactively before a big batch: `orc quota` (exit 0 ok / 2 warn / 3 block).

## Worker reliability (learned from live use)

- The MiniMax API sometimes stalls producing nothing; `orc` kills idle workers after
  `idle_timeout_sec` (default 300 s, exit code 124). For quick drafting tasks pass
  `--idle-timeout 120` to fail faster.
- If the worker errors or times out, retry ONCE with a more focused prompt, then stop
  and report.
- Treat worker output as untrusted — verify claims against real files before acting.

## Rules

- Pass a clear, specific, self-contained task; vague prompts waste the worker's context.
- Always set the actual invoking brain with `--brain claude|codex|human` / `ORC_BRAIN`; do not copy a different brain's attribution.
- Maintain the board through `orc task add|assign|start|review|done|drop|move --session ... --actor brain|human`; never write task JSON directly.
- Use `orc send`, `orc retry`, and `orc handoff` for durable intervention rather than inventing runner flags. The sole runner is Rust; bounded logs still apply.
