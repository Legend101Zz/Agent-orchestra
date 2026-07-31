# Issue #49 phase 1 — a delegation you can watch

Branch `issue-49-watchable-delegation`, off `origin/main` @ `47a4b33`.
Everything below was run, not reasoned about. Phase 1 only: defects 1, 4 and 5
plus the departure beat. Phases 2 and 3 are deliberately untouched — see
"What this does not do".

## The shape of the fix

The issue's corrected framing is the right one: **the story had to become true
before it could be made pretty.** `delivery_confirmed` is written by
`mark_started` from the `on_started` callback, immediately after
`command.spawn()`, and `circuit::message_for` classified it as the *inbound*
packet. So on every delegation ever made, the answer appeared to come back
milliseconds after the brief went out — a blip, not a hand-off — and nothing
durable ever said the answer had actually arrived.

Four changes, in the order they had to happen:

1. **A completion event exists.** `persist_terminal` now appends
   `execution_succeeded` / `execution_failed` to the task board (and
   `review_execution_*` for a reviewer dispatch), written at the moment the
   child has exited and both its pipes are drained to EOF.
2. **The vocabulary tells the two apart.** `delivery_confirmed` is
   reclassified from `(Inbound, Confirmed)` to `(Outbound, Confirmed)` — it is
   the outbound journey *landing*, which is what it always was — and
   `execution_succeeded` becomes the genuine return.
3. **The board is watched, not overheard.** A `notify` watcher on
   `~/.orchestra/tasks` raises `UiEvent::BoardChanged`.
4. **The packet's position is continuous**, and the brain shows the hand-off
   leaving.

## Acceptance checks

### 1. A completion event exists, distinct from "spawned"

`orc-core/tests/dispatch.rs::dispatch_through_a_fake_worker_is_confirmed_records_actor_and_pane_linkage`
now pins the ordered pair on a real dispatch driven to completion:

```rust
assert_eq!(
    words.iter().rev().take(2).rev().copied().collect::<Vec<_>>(),
    vec!["delivery_confirmed", "execution_succeeded"],
);
```

It is deterministic rather than a race because `append_execution` runs
**before** `write_dispatch`, so "the dispatch is terminal" implies "the board
has been told".

### 2. The return packet means the answer

`orc-app/tests/task_vocabulary.rs::a_real_dispatch_writes_delivery_then_completion_and_the_gap_is_the_worker`
drives the real `orch::delegate` against a worker that sleeps 1.5 s:

```
delegate returned at 68.814792ms; delivery_confirmed seen at 68.8785ms; \
execution_succeeded seen at 1.62922425s
```

Before this branch both events landed in the same millisecond, by
construction. Five consecutive runs, all green, 1.60–1.69 s each.

### 3. Smooth — position is a continuous function of elapsed time, and the answer never overtakes the brief

`FLIGHT_FRAME_MS = 60` / `FLIGHT_STEP = 2` are replaced by
`FLIGHT_MS_PER_CELL = 30`. Identical speed (33⅓ cells/s), one cell at a time
instead of two-cell jumps at ~16 fps. `circuit::travel_time(12)` is still
360 ms, and the committed goldens catch the packet in the same cell at 180 ms.

Three tests in `circuit.rs`:

- `the_packet_crosses_one_cell_at_a_time_and_skips_none_of_them` — no
  consecutive draw jumps more than one cell, and every cell of a 40-cell route
  is somewhere the packet is actually drawn.
- `the_sheets_travel_speed_is_unchanged`.
- `an_answer_never_overtakes_the_brief_it_is_answering` — see below.

**Half of AC3 needs no test, and the first version of this branch wrote one
anyway.** "Halving the frame interval must not change where the packet is at
time *t*" is structural: `flight` takes a `Duration` and nothing else, so it
cannot depend on a poll cadence for *any* implementation, including the one
this issue replaced. A test that sampled the same pure function on a 30 ms grid
and a 15 ms grid and found they agreed proved only that one grid is a subset of
the other — it could not fail. It has been deleted rather than left as
reassurance, and the reasoning is recorded in the surviving test's comment.

**The answer overtaking the brief.** Travel time is a function of the wire, so
on a 30-cell connector (the default three-worker stage at 120x40) a worker that
answers in 200 ms answers while its own brief is still drawn at cell 6 — and
then a `▶` and a `◀` cross in opposite directions on one wire, which is the
picture the issue opens with. The times are real; the brief in transit is not,
because an answer coming back is *proof* that the brief arrived. Raising an
inbound flight now advances any outbound flight still travelling on that wire
to its landing — advanced, not deleted, so the worker's card still shows what
reached it.

`RepaintReasons` gains a `travelling` term, distinct from `in_flight`, and it
is what leads the wait chain at `FLIGHT_MS_PER_CELL / 2` (15 ms). The two had
to be split: `in_flight` stays true for the whole 1.2 s `EMOTE_HOLD`, and under
reduced motion `flight` is `Landed` from frame 0 with the flash suppressed, so
running the shell — every hosted pane included — at the travel cadence through
that redraws ~80 times with nothing able to change. A landed emote keeps the
30 ms tier the packet used before this branch. The 16 ms `animating` tier is
the **baton pulse**, and the source comment claiming otherwise is corrected in
place.

### 4. The board is watched, not overheard

`the_board_watcher_wakes_the_shell_with_every_pane_silent` — a task file is
written, `UiEvent::BoardChanged` arrives, no PTY anywhere in the test.
`the_board_watcher_watches_where_the_board_is_written` holds the watched root
against `orc_core::tasks::task_path`, so watching the wrong tree cannot pass.

### 5. A departure beat on the brain pane

Visible in the regenerated golden:

```
- ╭ ◆ CODEX  LIVE ──────────────────────────────────╮
+ ╭ ◆ CODEX  LIVE · ▶ HANDING OFF ──────────────────╮
```

and its mirror in `stage-message-return.txt`:

```
- ╭ ● PI-M3-0  LIVE ──────────╮
+ ╭ ● PI-M3-0  LIVE · ◀ ANSWER╮
```

(clipped by that pane's width, as every title in that column is).

### 6. Real state only

No boundary in this branch has a duration chosen for looks:

- Travel is `route length × FLIGHT_MS_PER_CELL`. A two-cell connector lands in
  60 ms; a forty-cell one takes 1.2 s. `a_worker_that_finishes_fast_is_not_shown_a_slow_animation`.
- The two events that bound a delegation are `assign_task` returning and
  `child.try_wait()` yielding a successful exit. A 200 ms worker is shown
  200 ms of gap; a 5-minute worker is not shown as finished until it is.
- An answer can no longer overtake the brief it is answering (see AC3).

**Two qualifications, because the unqualified claim would be false.**

- **The departure beat's length is geometry, not event duration.** Its doc
  first said it had "no hold of its own". It is bounded by
  `min(DEPART_CELLS, route) × FLIGHT_MS_PER_CELL` and so can never outlive its
  packet — but every route the router actually plans is longer than twelve
  cells, so in practice it is a constant 360 ms. What "real state only" buys
  here is narrower than the first wording implied: the beat *starts* at a real
  event and is bounded by real geometry rather than by a number picked to look
  right. STAGE does not claim to show how long a hand-off took.
- **Travel time is the wire's length, not the work's.** A message takes
  `route × 30 ms` to cross whatever connector it is on — 480 ms on a one-worker
  stage, 900 ms on a three-worker one — and then holds its emote for 1.2 s.
  That is the same for every message and is not derived from the worker. What
  the branch guarantees is the thing the issue actually asks for: no *phase
  boundary* is invented, nothing is padded to represent a duration, and the
  gap between the outbound and inbound beats is the worker's own runtime.

### 7. A flight aimed at no pane is not silently dropped

Two halves. **By construction:** `note_task_events` no longer raises a flight
for a run link that is not a worker pane on this stage — which is the case
#45's fallback creates whenever no seated pane matches the harness.
**Stated:** the legend says so instead — one note per message, not per task —
for exactly as long as the packet it replaced would have been on screen, and
with its own `RepaintReasons` term so the frame that *removes* it is actually
asked for. Without that the note was painted once and then stranded until the
next unrelated event, which on a quiet stage means the loop's 30 s timeout. The
run is named only when the line has room: a fallback link is a
`D-{cwd}-{epoch}-{slug}-{nonce}` dispatch id, and at 80 columns — where the
inlaid-connector prefix has already claimed half the legend — naming it pushed
every control key off the end.
`traffic_aimed_at_a_run_that_is_not_a_pane_is_stated_rather_than_dropped` and
`a_message_with_no_wire_asks_for_the_frame_that_takes_its_note_away`.

### 8. Reduced motion, colour tiers, no-hex gate

All green in the five gates. Reduced motion keeps its shape: `circuit::flight`
returns `Landed` from frame 0, so there is no travel, no packet and — by the
same mechanism — no departure beat; the arrival emote carries the whole event,
which is the sheet's own reduced-motion rule.

One thing there *did* change. The reduced-motion connector was being painted
`bold+dim` for exactly the reason the travelling packet was: `paint_cell` merges
modifiers into whatever the cell already carries, and an idle rail underneath is
`Slot::Faint`. Fixing only the packet would have left the smudge on the path
where the connector **is** the message. It is fixed in both, and
`reduced_motion_lands_the_message_without_ever_travelling` now asserts on the
painted cells' modifiers rather than only on their symbols. (My first attempt at
that assertion was itself wrong and passed against the broken code: a fresh
`StageState` has a live pulse, so the rail was `Steady`/`Slot::Glow`, already
bold. It needs a decayed pane to bite.)

### 11. Five gates

```
### GATE 1: cargo fmt --all -- --check                                  exit=0
### GATE 2: cargo clippy --workspace --all-targets -- -D warnings       exit=0
### GATE 3: cargo test --workspace                                      exit=0
### GATE 4: RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps    exit=0
### GATE 5: cargo build --release --locked                              exit=0
```

**335 passed, 0 failed** — 319 on `main` plus the 16 tests this branch adds.

`orc-cli::background_dispatch` (the documented sub-1 s wall-clock flake) ran
6/6 green in isolation on the branch and 3/3 on `origin/main`, and an
interleaved A/B of three full `cargo test --workspace` runs on each tree gave
0 failures on both. It *did* fail once, in a gate run taken while five review
subagents were each running `cargo test` against this same worktree —
`orch await` timing out at its 10 s budget under that load. That number is
discarded rather than explained away: the clean runs above are the evidence.
This worktree is also on the internal disk rather than the external SSD, which
is the variable that flake is known to turn on.

## Mutation checks

Every new guarantee was broken on purpose and the intended test failed. Nothing
here passed by luck.

| Mutation | Test that caught it |
|---|---|
| `flight` back to two-cell jumps per 60 ms frame | `the_packet_crosses_one_cell_at_a_time_and_skips_none_of_them` — *"the packet jumped from cell 0 to cell 2 between 45 ms and 60 ms"* |
| `persist_terminal` stops appending the completion event | both end-to-end tests |
| watermark advanced before the aim check (the shipped order) | `the_hand_off_survives_a_board_read_taken_before_the_pane_link_lands` |
| `confirmed_panes` back to `history.last()` | `a_delivery_receipt_does_not_take_the_confirmed_badge_off_when_the_worker_finishes` |
| off-stage traffic raised as a flight again (the silent drop) | `traffic_aimed_at_a_run_that_is_not_a_pane_is_stated_rather_than_dropped` |
| departure beat removed | `the_brain_shows_the_hand_off_leaving_and_the_worker_shows_the_answer_leaving` + the golden |
| a two-cell trail added behind the packet head | `the_packet_is_one_cell_and_draws_no_trail` |
| the off-stage repaint reason removed | `a_message_with_no_wire_asks_for_the_frame_that_takes_its_note_away` |
| `travelling` folded back into `in_flight` in `wait()` | `a_landed_emote_does_not_hold_the_shell_at_the_travel_cadence` |
| the reduced-motion connector inheriting the rail's DIM again | `reduced_motion_lands_the_message_without_ever_travelling` |
| `confirmed_panes` ignoring `execution_failed` again | `a_delivery_receipt_does_not_take_the_confirmed_badge_off_when_the_worker_finishes` |
| an inbound flight allowed to overtake its outbound again | `an_answer_never_overtakes_the_brief_it_is_answering` |

## Two things the issue got slightly wrong, found by reading the code

**The `lib.rs:6667` test would *not* have failed a trail.** The issue says a
trail drawn from `▓▒░·─━` would fail the shipped test at that line.
`the_message_vocabulary_survives_no_color_and_the_ascii_column` asserts only on
`circuit::packet()`'s return value — it never counts painted cells, so a trail
of any glyph would have slipped past it silently. That gap is now closed by
`the_packet_is_one_cell_and_draws_no_trail`, which counts packet cells on the
route in the rendered buffer. The issue's *conclusion* about the glyph register
is right; the mechanism it named would not have enforced it.

**Silencing `delivery_confirmed` would have lost the hand-off on two live
paths.** The first design considered making it silent, on the reasoning that
`assigned` always precedes it. It does not: `orch::review` dispatches a
reviewer without ever calling `assign_task`, and `orch::delegate` skips
`assign_task` for a task already in `running` — a retry against the same
worker. On both paths `delivery_confirmed` is the only record that a brief left
the conductor.

## Two defects found on the way, both fixed here because this branch causes them

1. **`confirmed_panes` read `history.last()`.** It only ever worked because
   `delivery_confirmed` happened to be the last durable word a dispatch wrote.
   Appending a completion event after it would have quietly taken the
   `✓ TASK CONFIRMED` badge off every pane on the stage, and no test would have
   failed.
2. **The packet was drawn `bold+dim`.** `paint_cell` merges modifiers into
   whatever the cell already carries, and the rail underneath is `Slot::Faint`
   (DIM). It is in the committed golden's own legend:
   `j fg=#5ad1c8 bg=#0a0c11 mod=bold+dim` → now `mod=bold`. A packet that has
   to out-contrast the wire it crosses cannot inherit the wire's dim, and this
   is the "intensity" half of Decision 2 (see `findings.md`).

## What this does not do

- **Phase 2 (incremental supervisor output, defect 3)** — not started.
  `drain_to_eof` still accumulates into memory and `Drain::finish` is still
  only called after the child exits, so there is still nothing partial to
  animate against. That is the honest foundation for a reveal and it is its own
  phase.
- **Phase 3 (the in-pane reveal)** — not started. It is gated on Decision 1,
  which is the product owner's call, and nothing here implements option (b) or
  (c). What phase 1 learned that bears on it is commented on the issue.
- **Acceptance check 9 (zoom)** — unchanged and still true: `render_circuit` is
  skipped when zoomed, so the packet is not drawn. The landing emote *is* still
  stamped on the focused pane's title. Making the packet survive zoom (or
  saying why it cannot) is a rendering decision that touches none of phase 1's
  defects, and inventing an answer for it here would have been scope I was not
  given.
- **Acceptance check 10** — Decision 1's, not mine.
- **The daemon's 8-entry history window.** `orc-daemon`'s `task_board`
  truncates `TaskSummary.history` to the last eight entries, and
  `note_task_events`'s watermark is a length into that window. A task with more
  than eight durable entries can therefore stop animating entirely. This branch
  adds one entry per dispatch and so brings a full contracted lifecycle
  (`created, isolated, assigned, moved, delivery_confirmed,
  execution_succeeded, moved→review, report_persisted, moved→done`) to the
  edge of it. **Correction (review):** this first said the fix "has to populate
  a total-length field from the daemon", and that is wrong. The watermark is
  `StageState::seen_history` — `orc-app`, inside this issue's allowed paths —
  and a content-anchored watermark (the last-seen entry's identity, located in
  the current window) fixes it with no daemon change at all. A daemon-side
  total-length field would be cleaner; it is not a precondition. Deferred to
  its own issue on scope, not on impossibility. Reported on the issue as a
  follow-up.
- **Orphan reconciliation still tells the board nothing.** If the detached
  supervisor is killed — SIGKILL, OOM, reboot — `dispatch::reconcile_record`
  marks the dispatch `orphaned` and appends no task history, so the board's
  last word stays `delivery_confirmed` and the task reads as running forever.
  Pre-existing, and this branch does not make it worse; but it is now
  *conspicuous*, because a completion vocabulary exists and its absence means
  something. Fixing it turns `reconcile_record` — which runs inside
  `read_dispatch`/`list_dispatches`, i.e. in any process that merely *lists*
  dispatches — into a task-board writer. That is a real semantic change and
  deserves its own issue rather than a rider on this one.
- **A reviewer's completion flies down the executor's wire.**
  `record_review_execution` sets no linkage (deliberately, like
  `record_review_delivery`), and `note_task_events` aims every flight at
  `task.assignee_run`, which is the executor's pane. Pre-existing for
  `review_delivery_confirmed`; the new word inherits it. The fix needs a
  reviewer linkage that `TaskSummary` does not carry, which means `orc-daemon`.
- **`append_execution`'s failure is durable but not surfaced.** A refused board
  append lands on `record.warnings`, which `pio dispatch status` shows and no
  TUI surface does. That is the deliberate trade — see the function's own doc —
  but "durable" is not the same as "visible", and it is worth stating plainly.

## A process note, because it cost real time

Five review subagents were told they could write throwaway probes into this
worktree. They did, and they cleaned up afterwards — but while they were
running, a gate run and a branch-vs-`main` A/B both picked up their `probe_*`
tests and their CPU load, and reported failures that were not mine. Those
numbers are discarded; every figure in this note comes from a quiet tree. Next
time: give review agents their own worktree.
