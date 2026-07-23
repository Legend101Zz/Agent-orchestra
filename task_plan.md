# Task Plan: V1 "Universal Delegation" program (2026-07-22)

v4 "Bench" is complete (Phases 0–6 evidenced, `main` @ 018d5a1; see git
history and `docs/notes/`). The product frame is now the V1 spec:
`docs/superpowers/specs/2026-07-22-v1-universal-delegation-design.md`.
Process: `docs/WORKFLOW.md` (issue → branch → review → merge; one issue at a
time). Implementer: code-puppy (Opus 4.8 long). Reviewer: Claude Code.

## Goal

Ship pi-orchestra V1: any installed harness can be conductor or worker,
capabilities are probed not assumed, delegation is contract-driven and
reviewed, triggers work inside hosted panes and standalone harnesses, and
the TUI wears the new visual identity.

## Issue map

The epic issue on GitHub tracks live status; this table is the plan of
record. (Issue numbers are filled in as issues are created.)

| Issue | Work item | Depends on |
|---|---|---|
| [#15](https://github.com/Legend101Zz/Agent-orchestra/issues/15) | EPIC: V1 Universal Delegation launch | — |
| [#16](https://github.com/Legend101Zz/Agent-orchestra/issues/16) | V1-0 Foundations research: crates + prior art (Claude session, no code) | — |
| [#17](https://github.com/Legend101Zz/Agent-orchestra/issues/17) | V1-0b Rename user-facing CLI `orc` → `pio` (`orcd` → `piod`) | — (✅ merged 2026-07-23, PR #19) |
| [#3](https://github.com/Legend101Zz/Agent-orchestra/issues/3) | V1-1 Harness auto-discovery → `~/.orchestra/harnesses.json` | — (✅ merged 2026-07-23, PR #20) |
| [#4](https://github.com/Legend101Zz/Agent-orchestra/issues/4) | V1-2 Capability probe suite + `pio doctor` honest report | — (✅ merged 2026-07-24, PR #21) |
| [#5](https://github.com/Legend101Zz/Agent-orchestra/issues/5) | V1-3 Task contract v2 (acceptance-driven schema + enforcement) | — |
| [#6](https://github.com/Legend101Zz/Agent-orchestra/issues/6) | V1-4 Universal worker adapter (any probed harness as worker) | #4 |
| [#7](https://github.com/Legend101Zz/Agent-orchestra/issues/7) | V1-5 Rate-limit-aware spawning (quota guard v2, concurrency caps) | #4 |
| [#8](https://github.com/Legend101Zz/Agent-orchestra/issues/8) | V1-6 `orch_*` control surface: normalized CLI verbs + MCP server | #5 |
| [#9](https://github.com/Legend101Zz/Agent-orchestra/issues/9) | V1-7 Trigger grammar in hosted panes (PTY detect + highlight) | — |
| [#10](https://github.com/Legend101Zz/Agent-orchestra/issues/10) | V1-8 Standalone integrations v2: Claude Code skill/hook + Codex block | #8 |
| [#11](https://github.com/Legend101Zz/Agent-orchestra/issues/11) | V1-9 Worktree isolation + independent review + final report | #5 |
| [#12](https://github.com/Legend101Zz/Agent-orchestra/issues/12) | V1-10 Single-harness mode (honest degradation + self-review) | #4, #6 |
| [#13](https://github.com/Legend101Zz/Agent-orchestra/issues/13) | V1-11 Visual identity v1: three themes + glyphs + baton | — |
| [#14](https://github.com/Legend101Zz/Agent-orchestra/issues/14) | V1-12 README + positioning revamp for V1 launch | most of above |

**Order: #16, #17, #3 and #4 are merged (#4 on 2026-07-24, PR #21). #4
landing unblocks #6/#7/#12. Next: #5 (task contracts) is the remaining
bottleneck (gates #8/#11); #6 (universal worker adapter) is the natural
follow-on to #4. Parallel-safe now: #5, #6, #9 — each from fresh `main`;
start #13 before more TUI churn lands to avoid merge conflicts.**

Naming decision (2026-07-22): user-facing CLI is `pio`, daemon `piod`; crate
names, `ORC_*` env vars and `~/.orchestra` unchanged (see #17).

## Phase status

- [x] V1 spec written and approved direction (2026-07-22)
- [x] Workflow, AGENTS.md, templates, design docs committed
- [x] Issues created on GitHub: epic #15, tasks #3–#14 (2026-07-22)
- [ ] Issues 1–11 implemented, reviewed, merged (tracked on the epic)
- [ ] V1 launch: README revamp, screenshots/gifs re-recorded in new identity

## Later (not V1 — do not start)

V1.5 DAG workflow engine · V2 deliberation/panel · V2.5 MBR memory ·
V3 federated collaboration. See the spec's roadmap section.
