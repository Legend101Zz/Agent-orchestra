---
name: orchestrate
description: Multi-worker orchestration of pi/MiniMax M3 delegations with quota guard and control-plane visibility. Use ONLY when the user's message explicitly contains the word "orchestrate" or "orchestrated". Never trigger for ordinary tasks, even heavy ones (use pi-delegate for those).
---

# Orchestrate (keyword-gated multi-worker mode)

The user said "orchestrate" — run the full orchestration flow. Otherwise this skill
must not activate.

## Flow

1. **Quota first**: run `pio quota` and report the numbers to the user. If exit code
   is 3 (block), stop and ask the user before any delegation.
2. **Decompose** the task into independent worker-sized chunks (each self-contained,
   with explicit file paths / scope). Read `max_parallel_workers` from
   `~/.orchestra/config.json` (default 3) and never exceed it.
3. **Launch** workers in the background, attributed to you and grouped as one
   session. Pick a session id once, export it, then launch every worker under it:

       export ORC_SESSION="orch-$(date +%Y%m%d-%H%M%S)-<slug>"
       pio run "chunk description" --cwd /path --brain <your-brain> --session "$ORC_SESSION" --bg

   Each prints a run id. The whole swarm shows up as a single expandable session
   in `pio top`. Tell the user they can watch live there.
4. **Monitor**: poll `pio list --json` every 30–60 seconds. Read finished output via
   `pio show <id> --tail 100`. Kill a stuck worker with `pio kill <id>` (stalled
   workers also self-terminate via the idle watchdog, exit code 124).
5. **Verify and synthesize**: workers are untrusted — check their outputs against
   the actual files before combining. Produce the final answer yourself.
6. **Report**: include per-worker status, exact `tokens.total` and `tokens.cost_usd`
   where present, `~tokens.estimated_total` only as fallback, the `pio stats` receipt,
   and the post-run `pio quota` numbers.

## Rules

- Relay every `ORC WARNING`/`ORC BLOCKED` line to the user verbatim.
- If two consecutive workers fail, stop the whole orchestration and report.
- Never edit files based on worker claims without spot-checking the claim.
- Do not invent pi or `pio` flags such as `--thinking`; tighten the prompt instead.
- On resume, run `pio task list --session "$ORC_SESSION"` plus `pio list` before acting;
  use `pio send`, `pio retry`, or `pio handoff` and preserve completed task context.
- If `ORC_PANE_ID` is present, retain it with `ORC_SESSION`; all board mutations
  still require explicit `--session` and `--actor brain|human` through `pio task`.
- `pi-orchestra` is an alias trigger for the same re-orientation and board
  maintenance workflow. Offer configured `default_workers`; never assume their
  acceptance or claim an adapter capability that has not been verified locally.
- Inside a Bench pane, read `ORC_WORKERS` and use the explicit `pio task add →
  assign → start → pio dispatch send` path. Pass `--session`, `--actor`, and the
  selected `--pane`; only durable `confirmed` delivery may be described as received.
