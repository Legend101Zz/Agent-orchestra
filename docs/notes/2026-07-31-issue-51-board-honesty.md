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
walks the full contracted-and-reviewed lifecycle — **eleven** entries — through
`note_task_events` one board read at a time, serving each read through the
daemon's own `.rev().take(TASK_HISTORY_WINDOW).rev()` shape, and asserting at
*every* read that exactly the entries appended since the last one animate.

**Corrected after review (FIX 1).** The first version read 1…8 and then once at
9, and that is not enough. The two changed lines in `note_task_events` — the
watermark assignment and the `skip` offset — are independent, and reverting
**only** the assignment left the first version green. Below the window a length
watermark and an absolute one are the same number, and on the *first* crossing
they still agree on what to raise; they diverge from the **second** crossing on,
where a saturated length lags the sliding window and re-raises entries it has
already shown. The original mutation run reverted both lines together, which is
caught — isolating the assignment is what exposed it. A real reviewed lifecycle
is eleven entries, so the regression AC1 exists to catch was live for every
reviewed task: the mirror of the original defect, a silent double-animation
instead of a silent stall.

Three things changed to close it, and the third is the one that actually bites:

1. The fixture is the **real** eleven-entry lifecycle, pinned action-by-action
   against the live API by the daemon test below, so it crosses the window twice.
2. Both panes are seated, so the four `review_*` entries raise packets instead of
   correctly going to the off-stage legend — otherwise the per-read arithmetic
   would be measuring defect 3, not the watermark.
3. **The watermark itself is asserted**, and it has to be. The behavioural
   arithmetic alone is still insufficient on this lifecycle, measured rather than
   assumed: with only the assignment reverted, the entries the watermark re-raises
   happen to be `moved → review` and `report_persisted`, which animate nothing, so
   the packet count stays right while the watermark is wrong. The next durable
   event a real task appended would break that silence and double-animate.
   Asserting `seen_history` directly is what makes the revert fail here rather
   than in front of a user — and it is what the acceptance check names.

The issue's acceptance wording is *"fails if the watermark goes back to being a
length"*. That revert is mutations 1a and 1b below — together and in isolation —
and the test now dies on both at the same place, the second crossing:

```
=== 1a: both lines reverted ===
thread 'tests::a_task_past_the_history_window_still_animates_its_next_event' panicked at
  crates/orc-app/src/lib.rs:7492:13:
assertion `left == right` failed: board read 9: the watermark must be an absolute index
into the task's whole history. A length into the window saturates at 8 and then lags the
sliding window for ever
  left: Some(8)
 right: Some(9)

=== 1b: watermark assignment only, skip arithmetic left correct ===
thread 'tests::a_task_past_the_history_window_still_animates_its_next_event' panicked at
  crates/orc-app/src/lib.rs:7493:13:
assertion `left == right` failed: board read 9: the watermark must be an absolute index
into the task's whole history. A length into the window saturates at 8 and then lags the
sliding window for ever
  left: Some(8)
 right: Some(9)
```

The original defect — the silent stall — is caught by the same walk from the
other direction: mutation 1c drops the window offset from `skip` while leaving
the watermark absolute, and the packet count goes to zero.

The first version of this test reported `left: 0, right: 1` against `origin/main`
and was correct about the stall. It was the *replay* it could not see.

The daemon's half is pinned separately by
`orc-daemon/src/lib.rs::tests::a_task_board_card_carries_the_whole_historys_length_not_the_windows`,
which drives a real reviewed lifecycle through `orc_core::tasks` — in a real git
repo, so a contracted task really isolates and really writes `isolated` — and
asserts on the summary `ClientRequest::TaskBoard` really returns: `history.len()
== TASK_HISTORY_WINDOW`, `history_total` equal to the whole history, and the
window's first entry at absolute index `history_total - history.len()`.

It now also pins the lifecycle **action by action**:

```
created, isolated, assigned, moved→running, delivery_confirmed,
execution_succeeded, review_delivery_confirmed, review_execution_succeeded,
moved→review, report_persisted, moved→done
```

Eleven, measured from the live API rather than asserted, and the test requires
`>= TASK_HISTORY_WINDOW + 2` — i.e. two crossings — with the reason stated: the
client's fixture cannot catch a watermark that is a length without them. If the
real lifecycle ever shortens, this fails and the client fixture is told to
follow rather than silently going soft.

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

**One defect in this test, found by A/B after review and fixed (see
`findings.md`).** It first polled the *board* for `execution_succeeded` and then
called `review` — and `review` gates on the dispatch *record*. `append_execution`
writes the board **before** `write_dispatch`, so "the dispatch is terminal"
implies "the board has been told" but not the converse: there is a real window
where the board is ahead. The test raced it, 1 run in 10. It now waits with
`orch::await_delegation`, which is what a real conductor calls there and what
waits on the thing `review` actually reads. Branch and `origin/main` are both
0 failures in 12 on the isolated binary afterwards.

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
| 1a. **both** changed lines reverted together | `a_task_past_the_history_window_still_animates_its_next_event` |
| 1b. **only** the watermark assignment reverted, `skip` left correct | same — *survived the first round; found in review, see FIX 1* |
| 1c. **only** the `skip` offset dropped, watermark left absolute | same |
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

**Three of these were found by breaking things rather than by writing them.**
Mutations 3 and 14 survived the first round — the additive `.max()` guard and the
confirmed badge on an orphaned worker were both held by nothing — and produced
two new tests. Mutation **1b** survived the first round *and* the first fix, and
was found in review, not here: the original mutation reverted both changed lines
at once, which the test caught, so the independence of the two was never probed.
**The lesson is narrower than "mutate more" and worth stating: when one fix
changes two lines, mutate them separately.** A combined revert only proves the
pair is load-bearing, not that each one is.

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

## Flakes

Two failures appeared during the review round and **neither was dismissed as a
known flake**; separating them took an A/B per binary.

- A full-workspace run failed `a_real_dispatch_writes_delivery_then_completion_and_the_gap_is_the_worker`
  — a #50 test this branch does not modify — by 13 ms against its 1.5 s budget.
  In isolation it ran 6/6 clean.
- Running the `task_vocabulary` binary alone twelve times per tree is what
  actually resolved it: `origin/main` 0/10, this branch 1/10, and the named
  failure was **this branch's own AC7 test**, not the #50 one. That was a real
  defect of mine — polling the board for a precondition expressed on the
  dispatch record — and fixing it took both failures with it.

After the fix, on the same machine:

| experiment | branch | `origin/main` |
|---|---|---|
| `task_vocabulary` binary alone | 0 failures / 12 | 0 failures / 12 |
| interleaved full workspace | 0 failures / 4 (348 passed) | 0 failures / 4 (337 passed) |

The lesson worth keeping: **a load-sensitive failure in a test you did not touch
is not evidence that the cause is not yours.** The first reading here — "the
documented wall-clock flake family" — was wrong, and only the per-binary A/B
showed it.

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
