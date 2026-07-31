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
| [#37](https://github.com/Legend101Zz/Agent-orchestra/issues/37) | V1-15 Persist the chosen theme + `<leader> t` switcher; unify the two config files | #13 (✅ merged 2026-07-29, PR #41) |
| [#38](https://github.com/Legend101Zz/Agent-orchestra/issues/38) | V1-16 STAGE as a live circuit: n-worker topology, fluid drag-resize, message-in-flight motion | #13 (✅ merged 2026-07-30, PR #42) |
| [#39](https://github.com/Legend101Zz/Agent-orchestra/issues/39) | V1-17 Visual identity carry-over: NO_COLOR trigger rainbow, recursive grep gate | #13 (✅ merged 2026-07-30, PR #47) |
| [#45](https://github.com/Legend101Zz/Agent-orchestra/issues/45) | V1-18 A conductor seated in the TUI dispatches to the panes it sits in, visibly (supersedes #43 + #44) | #38 (✅ merged 2026-07-31, PR #48 · 2 review findings still open) |
| [#49](https://github.com/Legend101Zz/Agent-orchestra/issues/49) | V1-19 A delegation you can watch: real-state-driven animation from brain to worker and back | #45 (phase 1 ✅ PR #50, phase 2 ✅ PR #56, both 2026-07-31 · **phase 3 is the last one, and it closes #49**) |
| [#51](https://github.com/Legend101Zz/Agent-orchestra/issues/51) | V1-20 Three places the board and the screen disagree: the 8-event window, orphaned supervisors, the reviewer's wire | #49 phase 1 (✅ merged 2026-07-31, PR #53 — review FIX (2) → both fixed → re-review ACCEPT) |
| [#14](https://github.com/Legend101Zz/Agent-orchestra/issues/14) | V1-12 README + positioning revamp for V1 launch | most of above |
| [#28](https://github.com/Legend101Zz/Agent-orchestra/issues/28) | V1-13 Dispatch pipe-buffer deadlock: drain the worker's output while it runs | #8 (✅ merged 2026-07-27, PR #29) |
| [#30](https://github.com/Legend101Zz/Agent-orchestra/issues/30) | V1-14 Background the worker: confirm delivery not completion; extract the answer | #28 (✅ merged 2026-07-28, PR #31) |
| [#33](https://github.com/Legend101Zz/Agent-orchestra/issues/33) | Side-fix (not part of the original epic): any known harness auto-registers, `pio harness add` for model profiles | — (✅ merged 2026-07-28, PR #34) |

**Order: #16, #17, #3, #4, #5, #9, #6, #7, #8, #10, #28, #30, #11, #12, #13,
#37, #38, #39, #45 and #49 phase 1 are merged. Every V1 feature is in; #14
(README) is the last original V1 item and all that stands between here and
launch.**

**#51 is merged (2026-07-31, PR #53) — all three defects, one branch.** Review
returned FIX (2); both were fixed in `4ae609b` and the re-review was ACCEPT.
Defect 1's cliff is closed at the seam rather than at
the client: `TaskSummary` gains `history_total`, the watermark becomes an
absolute index, and `orc_proto::TASK_HISTORY_WINDOW` is the named constant whose
doc says what depends on it — which is now *less* than before, because the client
is window-size-agnostic by construction. The daemon field was chosen over the
client-side anchor #49's review correctly identified, on merit and not on
impossibility: it is exact, survives any window size, and does not depend on
entry identity being unique within a second (`now_iso` is second-granularity, so
a retry inside one second can collide). Defect 2's fork was decided by
measurement — the issue's "any process that merely lists dispatches" is three
enumerable processes, all of which already write the board, and a TUI refresh is
not among them; option (ii) fails because `orc-mcp` has no daemon at all and
option (iii) is defeated by `seed_task_events`. The append lives in
`reconcile_record` behind its existing guards, best-effort, deduplicated inside
the board lock on the dispatch id. Defect 3's linkage was never unavailable —
`spec.confirmed_link` was in scope five lines from where the review branch threw
it away; `Task.reviewer_run` plus `circuit::Lane` now aim each message at its own
worker. Five gates green, 348 passed / 0 failed against `origin/main`'s 337, with
an interleaved three-round A/B showing 0 `orc-cli` quota failures on either tree.
Fifteen mutations, fifteen caught — two of which produced new tests because the
guarantee turned out to be held by nothing. **The review then found a sixteenth
the branch's own set had missed:** reverting the watermark assignment *alone*,
leaving the `skip` arithmetic correct, left the acceptance-check test green,
because a length and an absolute index agree until the *second* crossing of the
window. The combined revert was caught; only isolating the two lines exposed it.
Fixed in `4ae609b` by asserting the watermark directly at every board read and
driving the real eleven-entry reviewed lifecycle — plus two defects found while
fixing it, a daemon test that under-drove the lifecycle it claimed to measure
and an AC7 test that raced #50's board-before-record ordering one run in ten.
Re-review: 13/13 mutations caught, 348 passed across three clean runs, ACCEPT.
**The durable lesson, recorded in `findings.md`: when one fix changes two lines,
mutate them separately** — a combined revert only proves the pair is
load-bearing, never each line. **Carried out and reported rather
than fixed:** `tasks::lock_board` has no stale reclaim, so a process SIGKILLed
while holding `.board.lock` wedges that session's board for ever — the
self-referential case being a supervisor killed inside `append_execution`, whose
own orphan event then cannot land. `spawn_guard::lock_slots` already solved this;
porting it is ~30 lines but it is the core locking primitive for every board
writer and needs its own change. Also out of allowed paths: a
`pio dispatch reconcile` operator verb, and `orch::status`'s read-then-list
ordering. Evidence: `docs/notes/2026-07-31-issue-51-board-honesty.md`.

**Originally, before it was picked up: #51 is not optional sequencing.** #49 phase 1 merged with its
own review fully discharged, but it carried out three defects it was not
allowed to fix — and one of them is now *live on `main`*: the daemon truncates
`TaskSummary.history` to the last eight entries while `note_task_events`'
watermark counts into that window, so a task past eight events stops animating
for good. #50 added the ninth event to a full contracted lifecycle, which means
the `moved→done` that should fly the final confirmation home is precisely the
one that falls off. Plain delegations (seven events) still animate end to end;
contracted-and-reviewed ones do not. **Run #51 before #49 phase 2** — phase 2
builds more animation on top of a watermark that has already stopped moving.
*(Done: #51 merged as PR #53, so phase 2 is unblocked and the watermark moves.)*

**#49 phase 2 MERGED (2026-07-31, PR #56 as `2dc35db`) — a worker's partial
output is durable while it is still working.** Defect 3. Review **FIX (5)** →
all fixed in `881fb37` → re-review **ACCEPT**. The
supervisor's drain threads now mirror each stream verbatim to an append-only
per-attempt log beside the dispatch record, with a separate orchestrator-owned
counters journal; neither is ever fsynced, and neither touches the task board or
the dispatch record. Evidence:
`docs/notes/2026-07-31-issue-49-phase2-evidence.md`.

The measurement that decided it, and that the issue demanded: **one board write
costs a STAGE client a blocking `task_board` round-trip on the render thread —
221 us at 1 task, 4.27 ms at 64 — and the watcher's coalescing bounds nothing
(1.25–1.59 wakes per write at every cadence).** The board tolerates roughly two
durable writes per second; a dispatch lifetime is 9–11 in total. So progress
went nowhere near it, and 2000 lines of live output cost zero extra record
writes and zero extra board writes.

Two pre-existing defects found and reported rather than fixed: **#54**
(`tasks::lock_board` has no stale reclaim — filed *before* the work started, on
the expectation that phase 2 would raise the board write rate; in the event it
does not, so the window is not widened) and **#55** (`DispatchRecord.stdout` is
unbounded for any adapter with an extractor — measured at 25x the documented cap
with no truncation marker, and held by no test because the flood fixture's
adapter has no extractor).

**One follow-up merged unfixed, and it belongs to phase 3.** `ProgressLog`'s
`capped` latch is what stops a declined over-long line being followed by a
shorter one that fits — which would leave the log with a *hole*, the one thing
"byte N is byte N forever" forbids. It is correct in the code and **held by no
test**; removing it passes the whole suite. The test is written and verified
(passes on `b7f6954`, fails with the latch removed) and is named in the
re-review. Phase 3 is the natural place to land it, because phase 3 is the first
code that *reads* the log and therefore the first that a hole would lie to.

**The durable lesson from this review, third occurrence in this program:**
#50 shipped a test that could not fail, #51 one that under-drove the lifecycle it
claimed to measure, and #56 one that asserted against a helper standing in for
the path under test — `progress_paths(.., 1)` vs `(.., 2)` instead of a real
retry. Same error in three costumes: **testing the component instead of the
wiring.** Phase 3 is the most exposed yet, because a reveal is easy to assert
about a pure function and hard to assert about what a reader actually sees.

**Next: #49 phase 3** — the in-pane reveal, gated on Decision 1 (option (a),
answered by Mrigesh on 2026-07-31). Phase 3 is where the cost of *watching*
this state lands: `~/.orchestra/dispatches` is not in `file_watches()`, so
phase 2 raises no wakes at all, and phase 3 must decide deliberately what it
adds. **Phase 3 closes #49**, which unblocks #14 and V1. Copy-paste prompt lives
in
[`docs/prompts/2026-08-01-issue-49-phase3-next-session.md`](docs/prompts/2026-08-01-issue-49-phase3-next-session.md).

**#49 phase 1 pushed (2026-07-31, `issue-49-watchable-delegation`) — the board
now records that a worker *answered*, and the animation is driven from that.**
Defect 1 was the whole issue: `delivery_confirmed` is written by `mark_started`
immediately after `command.spawn()` and `persist_terminal`'s success arm
appended no history at all, so `circuit::message_for` was classifying "a
process started" as the return packet — the answer appeared to arrive
milliseconds after the brief left, on every delegation ever made, and nothing
durable ever said an answer had arrived. `orc_core::tasks` gains
`record_execution` / `record_review_execution`; `delivery_confirmed` is
reclassified `(Outbound, Confirmed)`, which is what it always meant. Measured
against a 1.5 s worker: delivery confirmed at 69 ms, `execution_succeeded` at
1.63 s. An adversarial review pass before pushing found and fixed seven real
defects (a stranded legend note with no repaint reason, an 80-column legend
overflow, the reduced-motion connector still `bold+dim`, an answer able to
overtake its own brief on the wire, the confirmed badge surviving a failed
execution, three untested new action words, and one tautological test, now
deleted). Also in phase 1: a `notify` watcher on `~/.orchestra/tasks` giving
board changes their own wake path (defect 4); `FLIGHT_MS_PER_CELL = 30`
replacing the two-cells-per-60 ms frame counter at identical speed (defect 5);
and the departure beat. **Decision 2 resolved in-branch: no trail** — the sheet
makes shape one of three separation legs and it is the only one that survives
on the monochrome tier, and the ASCII column has no unclaimed directional
character; argument in `findings.md`, sheet amended REV 2026.07c. Phases 2 and
3 not started; Decision 1 remains the product owner's. Carried out of scope and
reported on the issue: `orc-daemon`'s `task_board` truncates a task's history
to the last eight entries and `note_task_events`' watermark is a length into
that window, so a task past eight entries stops animating — the completion
event brings a full contracted lifecycle to nine, and the fix needs a field the
daemon populates.

#13 (PR #36) merged with its review verdict outstanding — FIX, 4 items — so
those findings were live on `main` and were re-homed rather than dropped: #37
(the `t` key still escapes the theme map), #38 (the baton freezes mid-sweep
instead of decaying), #39 (the trigger rainbow ignores `NO_COLOR`; the no-hex
gate doesn't recurse). #37 and #38 also carry the UX work Mrigesh asked for
after testing: a real theme switcher, per-worker connectors, and drag-resize
that doesn't round-trip the daemon every frame. **All four findings are now
discharged (2026-07-30).** #37 (PR #41): `t` no longer escapes the theme map,
the chord reaches all four screens, and the choice survives a relaunch through a
daemon round trip (`ClientRequest::SetTheme`) rather than a client-side file
write — `harnesses.json`'s `app.theme` is the authoritative record and
`config.json`'s copy is derived from it, see the 2026-07-29 entry in
`findings.md`. #38 (PR #42): the baton paints the frame it computed instead of
freezing mid-sweep, and the single floating dash became one routed connector per
worker. #39 (PR #47): the trigger gradient resolves through the colour tier, so
`NO_COLOR` really does drop it to bold, and the no-hex gate walks the whole
`src/` tree instead of only its top folder. One colour is deliberately left
outside that rule — a hosted pane's own SGR, replayed by `Theme::pane_color` at
every tier; see the 2026-07-30 entry in `findings.md`.
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

**#45 merged (2026-07-31, PR #48 as `490487e`) — reviewed FIX, merged with both
findings open.** The headline was independently verified against the real
`orch::delegate` (no second session; the only harness-matching seated pane
selected without `--pane`; the board and both animation events landing on it),
and five gates were green on a reviewer's own run — 319 tests, 0 failed. What
merged unfixed is honesty debt, not routing: `skills/orchestrate/SKILL.md` still
shows the contracted recipe with no git-worktree precondition, and STAGE's
`trigger_wired` is still one global `~/.claude` probe applied to every brain pane
regardless of harness, so a Pi/OpenCode brain shows a live `DELEGATE` badge for a
grammar `install.sh` reports as NOT wired. **Both need a follow-up issue before
#14 closes V1.** Detail in LOG.md under the #45 entry.

Originally, when pushed — a
conductor seated in a TUI pane now reads `ORC_SESSION`/`ORC_PANE_ID` and
dispatches into the workers already on screen instead of creating a second,
pane-less session. Two bugs, not one: the injected `session create` guidance,
*and* an independent defect in `note_task_events` that made STAGE ignore any
task it first saw already finished — which a delegation always is, since it is
created, sent and confirmed inside one synchronous call. Fixing the guidance
alone would have routed the work correctly and still animated nothing.

Carried out of scope, needing their own issues: **Pi and OpenCode trigger-grammar
wiring** (reported honestly by `install.sh` rather than guessed at — neither has
a skills directory on this machine to probe); **`uninstall.sh` unwiring** the
`settings.json` entry `install.sh --wire-claude-hook` can now add; **`README.md`**
lines 98-100 and 150, which now overstate "never edits protected config" and
understate what the pane environment is for; and **`ORC_DELEGATE_HINT`** in
`orc-daemon`, still handing every pane pre-rename `orc` vocabulary. All four sat
outside #45's allowed paths.

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

Two unfiled follow-ups from #37's review, both worth their own issue:

- **The bounded-probe flake family — four tests, one cause.** Alongside
  `background_dispatch.rs:195`'s sub-1s budget, `discovery.rs:35`'s
  `VERSION_PROBE_TIMEOUT = 2s` (`harness_cli::harness_list_is_additive…`) and
  `probe.rs:55`'s `HELP_PROBE_TIMEOUT = 5s` (`doctor_cli::doctor_probes…`) all
  lose to process spawn on an external-SSD checkout under load. Confirmed
  pre-existing, not caused by #37: interleaved A/B of the discovery one on the
  same volume gave branch 8/8, `origin/main` 7/8. **A fourth was identified
  during #38** — `orc-cli::quota_guard::cli_dispatch_at_cap_is_queued_then_drains…`,
  1 failure in 8, same A/B method giving branch 9/10 and `origin/main` 9/10 on
  the same volume (#38 touches only `orc-app`). The fix is one decision about
  how these budgets are set (scale, or make them env-tunable in tests), not
  four patches.
- **`handle_raw_event`'s dispatch layer has no test.** Deleting its
  `route_leader(...)` call — which kills `<leader>` `t`/`q`/`h`/`b`/`v`/`?` on
  HOME, SCORE and RUNS — leaves `orc-app` at 69 passed, 0 failed; so does
  dropping the STAGE `LeaderAction::Theme` arm. The tests drive `route_leader` /
  `route_runs_key` / `cycle_theme` directly, one level beneath the dispatch that
  decides whether they are reached. Both lines came in with `2112865`, and the
  altitude matches the rest of the crate, so this is a codebase-wide property
  rather than a #37 defect — it was deliberately not made a third fix round
  (ANTI-SLOP rule 4). One `handle_raw_event`-level test that presses real bytes
  would cover every leader command at once.

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
