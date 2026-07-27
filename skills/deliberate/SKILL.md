---
name: deliberate
description: Honest handler for the "deliberate:" trigger — a parallel panel / Mixture-of-Agents (MoA) proposal. Use ONLY when the user's message contains the word "deliberate:" as a trigger. Parallel deliberation is a V2 feature and is NOT available in V1; this skill exists to route the trigger honestly and offer a real fallback rather than fake a panel.
---

# Deliberate (V2 — not yet available; degrade honestly)

The user cast the `deliberate:` spell — they want a **panel**: several agents
answer in parallel and a configurable judge synthesizes consensus, disagreement,
and blind spots. Per the V1 product spec, **Deliberate is a V2 mode and is not
built yet.** Do not fake it.

## What NOT to do

- Do not invent a "panel", "judge", "aggregator", or fake multiple independent
  opinions from one model and present them as cross-harness diversity.
- Do not invent `pio` flags (there is no `pio deliberate`).
- Do not silently downgrade to a single answer without telling the user.

## What to do

1. **Say so plainly**, e.g.:

   > Parallel cross-harness deliberation (a judged panel) is a V2 feature and
   > isn't available yet. Here's what I can do instead.

2. **Offer a real fallback** and ask which they want:
   - **Delegate** — one bounded hand-off to one worker (see the `pi-delegate`
     skill): `orch_delegate` / `pio orch delegate <harness> --session <id>
     --title "..." --objective "..." --check "..."`.
   - **Sequential self-review** — plan → implement → independent review using the
     normalized surface: `orch_plan` → `orch_delegate` → `orch_review` →
     `orch_finish` (or the `pio orch <verb>` CLI equivalents). This is the honest
     single-harness path: durable sessions, roles, retries, evidence — without
     claiming artificial diversity.

3. If **more than one capable harness** is installed you may run the same task on
   each and compare the results yourself, but label it clearly as a manual
   comparison, **not** the V2 judged panel.

## Guarantees to keep

- **Quota first:** run `pio quota` before spending tokens and tell the user its
  level and remaining percentages. `pio quota` reports the level (exit 0 ok /
  2 warn / 3 block, or `--json` for the numbers); the `ORC WARNING`/`ORC BLOCKED`
  markers themselves come from `pio run`/`pio orch delegate` — relay those
  verbatim when they appear. Never `--force` without the user's say-so.
- **Single-harness honesty:** if only one harness is available, say
  "One capable harness detected. Parallel cross-harness deliberation is
  unavailable. Running a sequential plan with self-review." and proceed.
- **Confirmed delivery only:** a worker received its brief only when the dispatch
  is `confirmed`; missing executables, absent capabilities, stopped panes,
  timeouts, and non-zero exits are unavailable/failed and reported as such.
