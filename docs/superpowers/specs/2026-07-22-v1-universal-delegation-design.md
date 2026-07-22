# pi-orchestra V1 "Universal Delegation" — product & architecture spec

Date: 2026-07-22 · Status: approved direction (Mrigesh) · Naming: user-facing CLI is **`pio`** (daemon `piod`), decided 2026-07-22; `orc`/`orcd` are the pre-rename names (issue #17) · Supersedes the
v4-Bench scope as the product frame; v4 code (daemon, TUI, pio CLI, quota,
tasks, dispatch) is the foundation, not a rewrite target.

## Positioning

> **pi-orchestra: turn the pile of AI subscriptions you already pay for into
> one orchestra — one brain, many hands, pooled quotas.**

People pay for multiple agent subscriptions (Claude Code, Codex, pi/MiniMax,
Hermes, OpenCode…) with separate quota pools. Subagents are intra-harness
only; no tool lets one harness spend another harness's quota. pi-orchestra
formalizes inter-harness delegation. Purpose: help the user **create and
deliver better and more, while spending fewer tokens** (cf. caveman.so for
the "spend less" framing).

Differentiator vs OpenRouter Fusion / MoA services: those burn one metered
credit pool; pi-orchestra spreads work across sunk-cost subscriptions the
user already owns. No API proxy, no key handling — provider traffic flows
directly between each harness and its provider (unchanged v4 principle).

## Core concepts

1. **Conductor** — the orchestrator brain. Claude Code, Codex, OpenCode,
   Hermes, or pi/MiniMax-M3 running inside pi-orchestra. Session-specific;
   the user can change it per session.
2. **Bench** — the pool of available harnesses on this machine (later:
   optional remote machines, see V3).
3. **Panel** — a transient MoA group: N agents answer in parallel, an
   aggregator synthesizes. Research before building (V2; see References).
4. **Spell / trigger** — a small explicit grammar that activates the
   conductor: `delegate:`, `orchestrate:`, `deliberate:` (plus launch-time
   flow: open pi-orchestra → pick brain, workers, count). pi-orchestra must
   be rate-limit-aware: never spawn more concurrent workers than a harness's
   quota/limits tolerate.
5. **Memory Root** — a project-local `.memory` directory managed by MBR
   (Memory-Build-Runtime). Deferred to V2.5 until MBR itself is ready to be
   an SDK.
6. **Workflows (later)** — n8n-style graphs where each node is a harness
   run controlled by pi-orchestra (V1.5 DAG engine is the substrate).

## Modes

### Orchestrate (default "serious coding" workflow)

The conductor decomposes work into dependency-aware tasks and assigns roles
(architect, implementer, test writer, reviewer, security critic), tracked on
the SCORE kanban board.

```
orchestrate: add OAuth login, tests, migration and documentation
```

### Delegate (single hand-off)

One bounded brief to one worker, confirmed delivery, verified result —
today's v4 flow, generalized to any capable harness.

### Deliberate (V2)

Parallel proposals from a panel, configurable judge, consensus /
disagreement / blind-spot report, cost-quality presets. Hermes's MoA
discussion shows why configurability matters: hard-coded frontier panels are
prohibitively expensive — models, temperatures, reasoning, minimum successes
and per-session activation need user control.

## Harness registry & capability probes

Executable presence is not sufficient. For each of `claude`, `codex`,
`hermes`, `pi`, `opencode` (extensible), pi-orchestra probes:

- non-interactive invocation;
- continuation / resume;
- tool access;
- model selection;
- structured output;
- usage reporting;
- timeout / cancellation;
- working-directory control.

Results persist in `~/.orchestra/harnesses.json`. Unavailable abilities
degrade honestly. Report format:

```
Claude Code   installed   conductor   interactive
Codex         installed   conductor   resumable
Hermes        installed   worker      headless dispatch
Pi/MiniMax    installed   worker      streaming + usage
OpenCode      unavailable
```

## Conductor-independent control protocol

Every harness receives the same small tool surface:

```
orch_plan  orch_delegate  orch_status  orch_await
orch_review  orch_cancel  orch_finish
```

Implementation layering (skills teach intent; CLI/MCP performs the
dependable operation — skills alone leave invocation inconsistent):

- local daemon (`piod`, today `orcd`) — exists;
- headless CLI (`pio`) for universal compatibility — exists, verbs to be
  normalized to the surface above;
- an MCP server where supported (new);
- thin harness-specific skills/plugins explaining when to invoke it.

## Acceptance-driven task contracts

Every delegated task includes: objective; allowed files/directories;
forbidden actions; dependencies; expected artifact; acceptance checks;
timeout and retry policy; reviewer; token or monetary budget. This turns
"ask another model" into an auditable engineering operation.

Hermes's roadmap distinction (issue #344): isolated children returning
summaries = delegation; true orchestration adds dependency-aware workflows,
cooperation, health monitoring, shared state. V1 ships contracts + review;
V1.5 ships the DAG engine.

## Single-harness mode

pi-orchestra still works with one harness, but never claims artificial
diversity. It provides durable sessions, task decomposition, sequential
implementer/reviewer roles, worktree isolation, retries, evidence and usage
records. If the sole harness has multiple models/accounts configured,
route among them; otherwise say:

```
One capable harness detected. Parallel cross-harness deliberation is
unavailable. Running a sequential plan with self-review.
```

## Trigger words & terminal highlighting

- **Hosted panes:** pi-orchestra owns the renderer — detect the trigger
  grammar in PTY output and highlight it (ultrathink-style).
- **Standalone harnesses:** skills/plugins for Claude Code, Codex, OpenCode,
  Hermes, pi detect the trigger word and call `pio` even outside
  pi-orchestra. Closed UIs (Claude Code, Codex) cannot be re-colored; use a
  hook/status acknowledgment instead. pi (extensible) can highlight.

## Collaborate (V3 — federated, ToS-clean)

Multiple people contribute **agent capacity**, not credentials, to one
project session. Each participant runs a local pi-orchestra node advertising
only: available capability, approved repository/project, concurrency limit,
budget, allowed task types, online/away status. Credentials never leave the
owner's machine. Work moves through scoped git branches, patches, or
artifact packets — never shared interactive shell access.

Required protections: explicit approval before joining; revocable peer
identity; encrypted transport; per-task permissions; signed receipts; no
simultaneous edits to the same worktree; clear attribution; local budget and
cancellation controls; audit log showing which person and harness produced
each artifact.

## Roadmap

**V1 — Universal delegation (launch scope)**
1. Harness auto-discovery → `~/.orchestra/harnesses.json`.
2. Capability probes with honest degradation (`pio doctor`).
3. `delegate:` / `orchestrate:` / `deliberate:` trigger grammar; PTY
   detection + highlight in hosted panes.
4. CLI + MCP `orch_*` tool surface.
5. Skills/integrations for Claude Code and Codex (standalone trigger).
6. Task contracts and acceptance criteria.
7. Worktree isolation.
8. Independent review and final report.
9. Single-harness mode.
10. Visual identity v1 (nocturne / ember / phosphor per
    `docs/design/visual-identity.md`).
11. Rate-limit-aware worker spawning (quota guard v2).

**V1.5 — Workflow engine:** dependency-aware DAGs; retry and replanning;
specialist roles; budget/quota routing; OpenCode, Hermes, pi integrations;
adapter SDK for community harnesses.

**V2 — Deliberation:** parallel proposals; configurable judge; consensus,
disagreement and blind-spot report; cost/quality presets; benchmark suite
proving which tasks actually improve.

**V2.5 — Memory:** MBR plugin; project decision records; failure/solution
retrieval; context packets with citations; inspection, redaction, forgetting.

**V3 — Federated collaboration:** peer discovery and invitation; secure
capability advertisement; remote task packets; branch/patch exchange; shared
audit report; per-user budgets and permissions.

## References

- MoA paper: https://arxiv.org/abs/2406.04692
- togethercomputer/moa: https://github.com/togethercomputer/moa
- Hermes multi-agent architecture: https://github.com/NousResearch/hermes-agent/issues/344
- Hermes configurable MoA: https://github.com/NousResearch/hermes-agent/issues/38952
- Token-economy framing: https://caveman.so/
- Prior specs: `docs/superpowers/specs/2026-07-11-pi-orchestra-v4-bench-design.md`
