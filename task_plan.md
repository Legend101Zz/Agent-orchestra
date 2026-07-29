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
| [#5](https://github.com/Legend101Zz/Agent-orchestra/issues/5) | V1-3 Task contract v2 (acceptance-driven schema + enforcement) | — (✅ merged 2026-07-24, PR #22) |
| [#6](https://github.com/Legend101Zz/Agent-orchestra/issues/6) | V1-4 Universal worker adapter (any probed harness as worker) | #4 (✅ merged 2026-07-24, PR #24) |
| [#7](https://github.com/Legend101Zz/Agent-orchestra/issues/7) | V1-5 Rate-limit-aware spawning (quota guard v2, concurrency caps) | #4 (✅ merged 2026-07-25, PR #25) |
| [#8](https://github.com/Legend101Zz/Agent-orchestra/issues/8) | V1-6 `orch_*` control surface: normalized CLI verbs + MCP server | #5 (✅ merged 2026-07-27, PR #26) |
| [#9](https://github.com/Legend101Zz/Agent-orchestra/issues/9) | V1-7 Trigger grammar in hosted panes (PTY detect + highlight) | — (✅ merged 2026-07-24, PR #23) |
| [#10](https://github.com/Legend101Zz/Agent-orchestra/issues/10) | V1-8 Standalone integrations v2: Claude Code skill/hook + Codex block | #8 (✅ merged 2026-07-27, PR #27) |
| [#11](https://github.com/Legend101Zz/Agent-orchestra/issues/11) | V1-9 Worktree isolation + independent review + final report | #5 (✅ merged 2026-07-28, PR #32) |
| [#12](https://github.com/Legend101Zz/Agent-orchestra/issues/12) | V1-10 Single-harness mode (honest degradation + self-review) | #4, #6 (✅ merged 2026-07-28, PR #35) |
| [#13](https://github.com/Legend101Zz/Agent-orchestra/issues/13) | V1-11 Visual identity v1: three themes + glyphs + baton | — (✅ merged 2026-07-29, PR #36) |
| [#37](https://github.com/Legend101Zz/Agent-orchestra/issues/37) | V1-15 Persist the chosen theme + `<leader> t` switcher; unify the two config files | #13 |
| [#38](https://github.com/Legend101Zz/Agent-orchestra/issues/38) | V1-16 STAGE as a live circuit: n-worker topology, fluid drag-resize, message-in-flight motion | #13 |
| [#39](https://github.com/Legend101Zz/Agent-orchestra/issues/39) | V1-17 Visual identity carry-over: NO_COLOR trigger rainbow, recursive grep gate | #13 |
| [#14](https://github.com/Legend101Zz/Agent-orchestra/issues/14) | V1-12 README + positioning revamp for V1 launch | most of above |
| [#28](https://github.com/Legend101Zz/Agent-orchestra/issues/28) | V1-13 Dispatch pipe-buffer deadlock: drain the worker's output while it runs | #8 (✅ merged 2026-07-27, PR #29) |
| [#30](https://github.com/Legend101Zz/Agent-orchestra/issues/30) | V1-14 Background the worker: confirm delivery not completion; extract the answer | #28 (✅ merged 2026-07-28, PR #31) |
| [#33](https://github.com/Legend101Zz/Agent-orchestra/issues/33) | Side-fix (not part of the original epic): any known harness auto-registers, `pio harness add` for model profiles | — (✅ merged 2026-07-28, PR #34) |

**Order: #16, #17, #3, #4, #5, #9, #6, #7, #8, #10, #28, #30, #11, #12 and #13
are merged. #14 (README) is the last original V1 item.**

#13 (PR #36) merged with its review verdict outstanding — FIX, 4 items — so
those findings are live on `main` and were re-homed rather than dropped: #37
(the `t` key still escapes the theme map), #38 (the baton freezes mid-sweep
instead of decaying), #39 (the trigger rainbow ignores `NO_COLOR`; the no-hex
gate doesn't recurse). #37 and #38 also carry the UX work Mrigesh asked for
after testing: a real theme switcher, per-worker connectors, and drag-resize
that doesn't round-trip the daemon every frame.
The delegation core is now sound. #28 (PR #29) fixed the pipe-buffer deadlock
that had made every non-trivial delegation fail since #8; #30 (PR #31) then
separated *delivery* from *execution* — `orch delegate` returns once the brief is
received, a detached supervisor holds the #7 slot lease for the worker's real
lifetime, a dead supervisor reconciles to `orphaned` instead of wedging the
board, and the record stores the extracted answer plus usage rather than raw
transport JSON. #11 (PR #32) then closed out V1's core delegation-safety story:
every contracted task now runs in its own Git worktree, completion requires an
independently reviewed verdict per acceptance check (or an honest
`self_review` with one harness), and `orch finish` refuses `done` while any
check is `fail`. Opened along the way, #33 (PR #34) fixed a real gap hit while
testing #11 locally — `opencode` was fully wired but unreachable through
`session create`, and `pi-m3` was a single hardcoded `pi` model with no way to
register another. #12 (PR #35) then completed the honest-degradation story: with
exactly one capable adapter family, launch prints the mandated sentence verbatim
and the full pipeline still runs sequentially; diversity is counted by adapter
family rather than registry key, so two model profiles of one CLI may alternate
implementer/reviewer roles but the report stays `self_review` — never
manufactured independence.

**Next: #37, then #38** — both are TUI work on top of #13, so land them before
#14's screenshots. #39 is small and independent; it can slot in anywhere.
**#14 goes last**, once the screens are what the launch photos should show.

Open and unfiled: the shipped screen *layouts* are thinner than the mockups
in `docs/design/visual-identity/`. The palette, glyph register and baton match
the sheet exactly, but `_fillScreens` draws HOME as three session cards with
health badges and sparklines plus a two-column bench grid with PATHs and an
`n / m on PATH` counter; what shipped is a flat list and a single-column
bench. Decide before #14 — that issue is the screenshots.

Still open by choice: `render_brief` is wired into `orch delegate` but not
`dispatch send` (`orch` is the canonical path). Small debts from #12, none
blocking: the `?` help screen is not single-harness aware; HOME calls
`single_harness::detect(.., None)` while `session create` passes `Some(cwd)`, so
the two can disagree on whether the mode is active; `alternate_profile` has no
test for the "executor profile has no `--provider/--model`, one sibling does"
edge; and `background_dispatch.rs:195`'s sub-1s wall-clock budget is
storage-dependent (fails on an external-SSD checkout, passes on internal disk).

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
