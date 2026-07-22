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

| # | Work item | Depends on |
|---|---|---|
| E | EPIC: V1 Universal Delegation launch | — |
| 1 | Harness auto-discovery → `~/.orchestra/harnesses.json` | — |
| 2 | Capability probe suite + `orc doctor` honest report | 1 |
| 3 | Task contract v2 (acceptance-driven schema + enforcement) | — |
| 4 | Universal worker adapter (any probed harness as worker) | 2 |
| 5 | Rate-limit-aware spawning (quota guard v2, per-harness concurrency) | 2 |
| 6 | `orch_*` control surface: normalized CLI verbs + MCP server | 3 |
| 7 | Trigger grammar in hosted panes (PTY detect + renderer highlight) | — |
| 8 | Standalone integrations v2: Claude Code skill/hook + Codex block | 6 |
| 9 | Worktree isolation hardening + independent review + final report | 3 |
| 10 | Single-harness mode (honest degradation + sequential self-review) | 2, 4 |
| 11 | Visual identity v1: three themes + glyph register + baton spec | — |
| 12 | README + positioning revamp for V1 launch | most of 1–11 |

## Phase status

- [x] V1 spec written and approved direction (2026-07-22)
- [x] Workflow, AGENTS.md, templates, design docs committed
- [ ] Issues created on GitHub (epic + 12)
- [ ] Issues 1–11 implemented, reviewed, merged (tracked on the epic)
- [ ] V1 launch: README revamp, screenshots/gifs re-recorded in new identity

## Later (not V1 — do not start)

V1.5 DAG workflow engine · V2 deliberation/panel · V2.5 MBR memory ·
V3 federated collaboration. See the spec's roadmap section.
