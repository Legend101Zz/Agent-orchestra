# Issue #51 — three places the board and the screen disagree

Branch `issue-51-board-honesty`, off `origin/main` @ `e82d894`. All three
defects in one branch, as the issue requires: they share a cause — the daemon's
`TaskSummary` is too thin to carry what the client needs, and nothing owned
telling the board that a supervisor died.

Everything below was run, not reasoned about.

## What changed, in one place

| Defect | Cause | Fix |
|---|---|---|
| 1. the eight-event cliff | the client's watermark was a *length* into a truncated window, so it saturated at the window's width | `TaskSummary.history_total` carries the whole history's length; the watermark becomes an absolute index and the window's offset is `history_total - history.len()` |
| 2. a killed supervisor tells the board nothing | `reconcile_record` wrote the dispatch record and appended no task history | one `execution_orphaned` / `review_execution_orphaned` event, appended inside `reconcile_record`, best-effort, deduplicated inside the board lock |
| 3. the reviewer's answer flies down the executor's wire | `record_review_delivery` discarded the reviewer's link because there was nowhere to put it that was not the executor's | `Task.reviewer_run` / `TaskSummary.reviewer_run`, written by `record_review_delivery` exactly as `record_delivery` writes `assignee_run`; `circuit::Lane` picks per message which link a flight is aimed at |

`PROTOCOL_VERSION` stays at **1**, deliberately — see acceptance check 2.

---

## Acceptance checks

### 1. A task past the window still animates its next event

`orc-app/src/lib.rs::tests::a_task_past_the_history_window_still_animates_its_next_event`
drives the full contracted-and-reviewed lifecycle — nine entries — through
`note_task_events`, reading the board after every append and serving each read
through the daemon's own `.rev().take(TASK_HISTORY_WINDOW).rev()` shape. It
asserts *past* the window rather than at it, because a probe that stops at the
boundary passes against the broken code.

It fails on `origin/main`'s behaviour, which is what it is for:

```
thread 'tests::a_task_past_the_history_window_still_animates_its_next_event' panicked at
  crates/orc-app/src/lib.rs:7237:9:
assertion `left == right` failed: the reviewed task's `moved -> done` must still raise a
packet at entry 9 of a 8-entry window; a watermark that is a length into the window pins
itself at the window and raises nothing ever again
  left: 0
 right: 1
```

The issue's acceptance wording is *"fails if the watermark goes back to being a
length"*. Mutation 1 in the table below is exactly that revert, and the test
dies on it.

The daemon's half is pinned separately by
`orc-daemon/src/lib.rs::tests::a_task_board_card_carries_the_whole_historys_length_not_the_windows`,
which drives a real reviewed lifecycle through `orc_core::tasks` and asserts on
the summary `ClientRequest::TaskBoard` really returns: `history.len() ==
TASK_HISTORY_WINDOW`, `history_total == 9`, and the window's first entry is the
one at absolute index `history_total - history.len()`. Nine is measured from the
real API rather than asserted, and the test refuses to be meaningful if a
lifecycle ever stops outrunning the window.

### 2. Additive in both directions, and the protocol version does not move

`orc-proto/src/lib.rs::tests::the_new_task_summary_fields_are_additive_in_both_directions`
proves both directions rather than asserting additivity:

- **new reader ← old record**: a pre-#51 JSON literal with neither field parses,
  giving `history_total: 0` and `reviewer_run: None`.
- **old reader ← new record**: a locally declared `TaskSummaryBefore51` — this
  struct exactly as it stood on `origin/main` — parses a current serialization
  with both fields present. Nothing in this workspace uses
  `#[serde(deny_unknown_fields)]`, which is what makes this direction work; a
  grep for it returns one hit, in a design doc saying not to use it.
- an unreviewed task carries no `reviewer_run` key at all.

Before this branch **no test in the repo proved the forward direction by
construction** — every additive test covered old-JSON-into-current-struct only.

**`PROTOCOL_VERSION` does not move, and the reason is stated rather than
implied.** Its own doc reserves the version "for a change that alters the
meaning of an existing message". An added field nobody reads cannot: `history`
still means the newest window, `assignee_run` still means the executor's pane.
The handshake that actually protects mixed builds is `BUILD_IDENTIFIER`, which
refuses them outright — the same precedent recorded for `SetTheme` in
`findings.md` (2026-07-29). The test asserts `PROTOCOL_VERSION == 1` in the same
breath, so a future field cannot bump it by accident.

The one non-obvious consequence: `#[serde(default)]` means an absent
`history_total` parses as `0`, and taking that at face value would reset every
watermark and replay the window on every board read — with the board on a
`notify` watcher, a replay storm, which is *worse* than the cliff. The single
consumer therefore reads `history_total.max(history.len())`, degrading an absent
field to exactly the pre-#51 behaviour. Pinned by
`a_summary_with_no_total_degrades_to_the_old_behaviour_and_never_replays` and by
mutation 3.

### 3. The window is a named constant that says what depends on it

`orc_proto::TASK_HISTORY_WINDOW = 8`, with a doc comment naming its two
dependants — SCORE's history panel, and STAGE's animation, which now locates the
window with `history_total` and therefore no longer cares what the number is.
The comment records that it *did* care until #51, and that this is why the
number is named. The daemon and every test reference the constant, so the two
cannot drift.

The number is unchanged. The issue asks to "consider whether 8 is still the
right window now that something depends on it"; the answer is that *less*
depends on it than before — the client is now window-size-agnostic by
construction — so nothing argued for a different number, and widening it costs
one `TaskHistorySummary` per entry per task per board read for up to 256 tasks.

### 4. A killed supervisor produces a durable board event of its own

`orc-core/tests/dispatch.rs::a_killed_supervisor_leaves_a_durable_board_event_of_its_own`
kills a **real** detached supervisor with SIGKILL and lets an ordinary reader
reconcile it.

Two things about the test shape, both learned the hard way:

- **The dispatch goes through the real `pio` binary**, not in-process. The
  supervisor must be a *grandchild*: a SIGKILLed child of a still-running parent
  is a zombie, and `pid_alive`'s `kill(pid, 0)` correctly reads a zombie as
  alive. In production the dispatching `pio` exits immediately and the
  supervisor is reparented and reaped, so killing it really does make it
  disappear. Going through the binary reproduces that rather than fighting it.
- **It waits for `delivery_confirmed` to reach the board before killing.**
  `mark_started` writes the dispatch record *before* it appends the board event,
  so `pio dispatch send` can return in between; killing inside that window tests
  a supervisor that died before it delivered, which is a different thing. The
  first version of this test did exactly that and was flaky in a way that looked
  like a defect.

The assertions are the acceptance check's three requirements, separately:

- the board's last word is `execution_orphaned` — **distinguishable from "still
  running"**, whose signature is the absence of a completion event;
- neither `execution_failed` nor `execution_succeeded` appears —
  **distinguishable from "the worker failed"** and from a clean answer;
- the entry's detail names the dispatch and says the supervisor was lost.

On `origin/main`'s behaviour it fails with exactly the reported symptom:

```
assertion `left == right` failed: the board's last word must be the orphaning,
not `delivery_confirmed`: ["created", "assigned", "moved", "delivery_confirmed"]
  left: Some("delivery_confirmed")
 right: Some("execution_orphaned")
```

### 5. Two processes listing at once corrupt nothing and deadlock nothing

**Which processes may now write to the board, stated plainly** — the acceptance
check asks for this and it is the honest answer:

- **may write**: `pio` (already did, on every `task` and `orch` verb; newly on
  `dispatch list/show/drain` and `orch status/await/review/finish`), `pio-mcp`
  (already did via `orch_delegate/review/finish/cancel`; newly via `orch_status`
  and `orch_await`), and the detached `pio _dispatch_exec` supervisor (already
  did via `record_delivery` and `append_execution`; newly for a *sibling*
  dispatch, through the `drain_queued` at the tail of `execute`).
- **may not write, before or after**: `orc-app` (the TUI), `orc-tui`,
  `orc-pty`. The issue's framing — "the CLI, the MCP server, the daemon, a TUI
  refresh" — is wrong about the TUI: `orc-app` contains zero occurrences of
  `list_dispatches`, `read_dispatch`, `drain_queued` or `reconcile`, and
  `ClientRequest::DispatchBoard` has no production sender at all.

No process gains a capability it did not already have on this exact line: on
`main`, `pio dispatch list` already SIGTERMs a worker's process group, takes a
slot lock per harness directory and fsyncs a dispatch record. The only genuinely
new resource any of them touches is `.board.lock`. Full argument, including why
options (ii) and (iii) lose on measurement, in `findings.md`.

Two tests, and the split is deliberate:

- `orc-core/tests/dispatch.rs::concurrent_listers_write_one_orphan_event_and_never_wedge_the_board`
  — two dispatches, both supervisors SIGKILLed, then **four real `pio dispatch
  list` processes** spawned back to back. Every one exits 0 and returns *both*
  records (nothing silently dropped by `list_dispatches`' `.ok()`), the whole
  race completes well inside its deadline (no wedge), each task carries exactly
  one `execution_orphaned`, and every task file still parses through
  `list_tasks` (no torn write).
- `orc-core/tests/tasks.rs::the_orphan_event_is_written_once_per_dispatch_and_again_for_the_next_one`
  — the deterministic half. **A spawned race is corroboration, not proof**:
  nothing forces four children to overlap, so it can pass without ever
  exercising the contended path. This one calls `record_orphaned` twice with the
  same dispatch id (one entry), then with a different dispatch id on the same
  task (two entries — a re-dispatched task orphaned again is a second real
  event, and keying on the word alone would silence it), then the reviewer
  variant.

**Deadlock is unreachable by construction, not by test result.** `lock_board`
contains no blocking syscall: `create_new(true)` spun at most 100 × 5 ms, then
`TaskError::Busy`. The worst outcome of contention from any number of processes
is a ~500 ms refusal. No lock-order inversion is introduced — the slot lock is
released before the board lock is taken, and nothing in the workspace takes them
the other way round. No re-entrancy — `lock_board` is private to `tasks.rs`, and
`tasks.rs` never calls `dispatch::`.

**The append is best-effort, and here that is forced.** `list_dispatches` drops
a record whose reconcile returns `Err`, so propagating a busy board would make
the dispatch **vanish** from six commands: a silent board would become an
invisible dispatch. The refusal lands on `record.warnings` instead, which
`pio dispatch list` and `pio dispatch show` print. The known limit — a refused
append is permanently silent, because the record latches and the guards never
re-admit it — is written up in `findings.md` rather than left as a footnote.

### 6. The new words are classified deliberately and the table has its rows

`circuit::message_for` gains `execution_orphaned` and
`review_execution_orphaned` on the `(Inbound, Failed)` arm. It is a *failure on
the wire* without being `execution_failed` *on the board*: those mean different
things and the board keeps them apart, because an orphaned worker was killed
mid-flight and nothing knows what it had done. No fourth `Outcome` slot — one
would reorder three palettes and every golden's legend to say "failed,
differently".

`orc-app/tests/task_vocabulary.rs`'s table gains both rows, and the table itself
gains a fourth column: the **lane** (defect 3's). Every row now records what
STAGE does with a word *and* which of a reviewed task's two workers it is about.
`every_word_the_reviewer_branch_writes_is_in_the_reviewer_lane` pins all five
reviewer words and asserts the executor's are not swept in by a prefix rule.

One consequence outside the vocabulary: `confirmed_panes` matched
`delivery_confirmed | delivery_failed | execution_failed`, so without
`execution_orphaned` a pane whose supervisor was SIGKILLed would keep wearing
`✓ TASK CONFIRMED` for ever — the function's own doc already argues for it
("a worker that took the brief and then died still has a `delivery_confirmed`
behind it"). The reviewer's words stay out, matching the existing executor-only
rule. Mutation 14.

**Two words, not one, and it is a deliberate deviation from AC6's singular
wording.** `record_review_execution`'s doc says collapsing the reviewer and
executor vocabularies "would make a reviewer's verdict indistinguishable from
the executor's answer". In this branch it is stronger than style: with defect
3's lanes, one shared word would put an orphaned *reviewer* back on the
executor's wire — reintroducing defect 3 for exactly the case defect 2 exists to
report.

### 7. A review animates on the reviewer's connector, driven through the real `orch::review`

`orc-app/tests/task_vocabulary.rs::a_real_review_links_the_reviewers_pane_and_leaves_the_executors_alone`
builds a git-backed session with **two seated worker panes of different adapter
families**, delegates a contracted task to one, waits for the executor to finish,
and calls the real `orc_core::orch::review`. It asserts:

- the review dispatch selected the seated reviewer pane, unasked
  (`orch::review` passes `pane_id: None, run: None`);
- `assignee_run` is still the executor's pane — untouched by the review;
- `reviewer_run` is the reviewer's pane, and the two differ;
- every `review_*` entry in the *real* history is in `Lane::Reviewer` and
  nothing else is.

The value was never unavailable: `spec.confirmed_link` — the executor path's own
linkage, computed identically for a review dispatch — was in scope five lines
from where the review branch discarded it. What was missing was somewhere to put
it that was not `assignee_run`, which is what
`record_review_delivery`'s original "without replacing the executor's run
linkage" was protecting.

The aiming half is unit-tested next to the code that does it, because
`note_task_events` is private:
`a_reviewers_answer_crosses_the_reviewers_wire_and_not_the_executors` puts an
executor on `pane-1` and a reviewer on `pane-2` and asserts the reviewer's brief
and verdict both raise flights aimed at `pane-2` while the executor's connector
stays quiet. Against `origin/main`'s behaviour it reports
`left: ["pane-1", "pane-1"]` — the defect exactly as filed.

### 8. With no reviewer pane seated, the review is stated as off-stage

`review_traffic_with_no_reviewer_pane_seated_is_stated_rather_than_aimed_at_the_executor`.
When no seated pane matches the reviewer's harness, `dispatch::deliver` falls
back to the dispatch id, so `reviewer_run` is a `D-…` — not a pane on this
stage — and #50's off-stage legend takes it: one note per message, no flight
down the executor's wire, and the legend says so. A review with *no*
`reviewer_run` at all (a review whose delivery failed, or a record written before
the field existed) goes to the same legend rather than to the executor, which is
the one place it certainly did not happen.

---

## Mutation checks

Every new guarantee was broken on purpose and the intended test failed.
Fifteen deliberate mutations, fifteen caught.

| Mutation | Caught by |
|---|---|
| 1. the watermark goes back to being a length | `a_task_past_the_history_window_still_animates_its_next_event` |
| 2. the daemon reports the window's length as the total | `a_task_board_card_carries_the_whole_historys_length_not_the_windows` |
| 3. the `.max(history.len())` additive guard removed | `a_summary_with_no_total_degrades_to_the_old_behaviour_and_never_replays` |
| 4. `history_total` loses `#[serde(default)]` | `the_new_task_summary_fields_are_additive_in_both_directions` |
| 5. `reviewer_run` serialized even when absent | `the_new_task_summary_fields_are_additive_in_both_directions` |
| 6. review traffic aimed at the executor again | `a_reviewers_answer_crosses_the_reviewers_wire_…` + the off-stage test |
| 7. the reviewer lane collapses into the executor's | `every_word_the_reviewer_branch_writes_is_in_the_reviewer_lane` |
| 8. the daemon stops carrying `reviewer_run` | `a_task_board_card_carries_the_whole_historys_length_not_the_windows` |
| 9. `record_review_delivery`'s link discarded again | `a_real_review_links_the_reviewers_pane_and_leaves_the_executors_alone` |
| 10. reconciliation tells the board nothing (the shipped defect) | `a_killed_supervisor_leaves_a_durable_board_event_of_its_own` |
| 11. the orphan dedupe guard removed | `the_orphan_event_is_written_once_per_dispatch_and_again_for_the_next_one` |
| 12. dedupe keyed on the word alone, not the dispatch | same |
| 13. the orphan words vanish from `message_for` | `every_action_the_board_writes_has_a_decided_classification` |
| 14. the confirmed badge survives an orphaned worker | `a_delivery_receipt_does_not_take_the_confirmed_badge_off_when_the_worker_finishes` |
| 15. an orphaned reviewer is called an executor | `the_orphan_event_is_written_once_per_dispatch_and_again_for_the_next_one` |

**Two of these tests were added only because the first mutation round found the
guarantee was held by nothing** — mutations 3 and 14 both survived until
`a_summary_with_no_total_degrades_…` and the orphaned-badge case were written.
That is the same class of gap #50's review found twice, and the reason the
mutation round runs before the gates rather than after.

**A trap worth recording, because it briefly looked like a test that could not
fail.** Mutation 9 mutates `orc-core`, but the assertion runs through a real
dispatch — and `dispatch_supervisor::execute` runs in a *separate process*,
launched from `target/debug/pio`. `cargo test -p orc-app --test task_vocabulary`
rebuilds `orc-core` and the test target but **not** `pio`, so the first attempt
ran the unmutated supervisor and the test passed. `cargo build -p orc-cli --bin
pio` first and it dies immediately. The full-workspace gate builds everything,
so the gate was never wrong; only a narrowed mutation run is. Written up in
`findings.md`.

---

## What this branch does NOT do

- **`.board.lock` still has no stale reclaim.** A process SIGKILLed while
  holding it wedges that session's board permanently — every `record_*` and every
  `pio task` verb fails `Busy` until a human deletes the file. The case is
  self-referential and it is #51's own scenario: a supervisor killed inside
  `append_execution` wedges the board, and then the orphan event for that death
  cannot land either. `spawn_guard::lock_slots` already solved exactly this for
  `.slots.lock` with a recorded holder pid and `reclaim_if_stale`. **Found, not
  fixed**: it is a pre-existing defect in the core locking primitive of the whole
  task board, it affects every writer, and it deserves its own change with its
  own test. Reported on the issue with the fix spelled out.
- **No `pio dispatch reconcile` verb.** It came out of the design phase as a
  genuinely useful operator escape hatch for the "refused append is permanently
  silent" limit, and `orc-cli` is **not** in this issue's allowed paths. Not
  smuggled in; reported.
- **No retry for a refused orphan append.** Deliberate — see `findings.md`. The
  recorder is idempotent, so a repair verb is safe to add later.
- **`orch::status` still reads the task before it lists dispatches**, so the very
  invocation that performs a reconcile returns a board snapshot that predates its
  own append. Nothing here is wrong, but a caller wanting the event must re-read.
  Worth a one-line reorder in its own change; noted on the issue.
- **`TaskSummary.tokens` is still hardcoded `None`** in `task_board`. Noticed in
  recon, unrelated to all three defects, untouched.
- **#49 phases 2 and 3, and Decision 1** — explicitly out of scope on the issue.
