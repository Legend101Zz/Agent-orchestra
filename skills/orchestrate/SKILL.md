---
name: orchestrate
description: Multi-worker orchestration of pi/MiniMax M3 delegations with quota guard and control-plane visibility. Use ONLY when the user's message explicitly contains the word "orchestrate" or "orchestrated". Never trigger for ordinary tasks, even heavy ones (use pi-delegate for those).
---

# Orchestrate (keyword-gated multi-worker mode)

The user said "orchestrate" — run the full orchestration flow. Otherwise this skill
must not activate.

## Flow

1. **Quota first**: run `orc quota` and report the numbers to the user. If exit code
   is 3 (block), stop and ask the user before any delegation.
2. **Decompose** the task into independent worker-sized chunks (each self-contained,
   with explicit file paths / scope). Read `max_parallel_workers` from
   `~/.orchestra/config.json` (default 3) and never exceed it.
3. **Launch** workers in the background, attributed to you:

       orc run "chunk description" --cwd /path --brain claude --bg

   Each prints a run id. Tell the user they can watch live with `orc top`.
4. **Monitor**: poll `orc list --json` every 30–60 seconds. Read finished output via
   `orc show <id> --tail 100`. Kill a stuck worker with `orc kill <id>` (stalled
   workers also self-terminate via the idle watchdog, exit code 124).
5. **Verify and synthesize**: workers are untrusted — check their outputs against
   the actual files before combining. Produce the final answer yourself.
6. **Report**: include per-worker status, total estimated tokens (from `orc list
   --json` → `tokens.estimated_total`), and the post-run `orc quota` numbers.

## Rules

- Relay every `ORC WARNING`/`ORC BLOCKED` line to the user verbatim.
- If two consecutive workers fail, stop the whole orchestration and report.
- Never edit files based on worker claims without spot-checking the claim.
