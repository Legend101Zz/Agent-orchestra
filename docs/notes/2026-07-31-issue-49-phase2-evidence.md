# Issue #49 phase 2 — partial output that is durable before the worker exits

Branch `issue-49-phase2-incremental-output`, off `origin/main` @ `e3a91a5`.
Defect 3 only. Phase 3 (the reveal) is untouched and stays scoped separately.

Everything below was run on this machine, not reasoned about. Where a number
decided a design question, the harness that produced it is named.

---

## 1. The defect, measured rather than asserted

`drain_to_eof` reads the child's pipes on their own threads — which is what
fixed #28's deadlock — but accumulated only into an in-memory `Captured`, and
`Drain::finish` is called from the wait branches, i.e. after the child exits.

A real worker emitting one line every 200 ms for six seconds, sampled every
750 ms through the reader path a user actually has:

```
  t (s)       exec   stdout  rec bytes    hist  board's last word
   0.07    running        0        777       4  delivery_confirmed
   0.83    running        0        777       4  delivery_confirmed
   1.58    running        0        777       4  delivery_confirmed
   2.34    running        0        777       4  delivery_confirmed
   3.09    running        0        777       4  delivery_confirmed
   3.84    running        0        777       4  delivery_confirmed
   4.60    running        0        777       4  delivery_confirmed
   5.36    running        0        777       4  delivery_confirmed
   6.11    running        0        777       4  delivery_confirmed
   6.87  succeeded      718       1582       5  execution_succeeded
```

Nine samples across 6.11 s of a worker that was steadily producing output.
`stdout` is `0` at every one, and the record on disk is **byte-for-byte
identical** — 777 B — for the whole run. Everything appears at once at 6.87 s.

---

## 2. The measurements that decided the design

All on the internal APFS volume, which is where `~/.orchestra` actually lives.
Debug build.

### Durable writes are fsync-bound, and the shape is irrelevant beside it

| operation | mean | p50 | p95 |
|---|---|---|---|
| `atomic_write_json`, 489 B record (what `write_dispatch` costs) | 4321 us | 4175 us | 5155 us |
| `atomic_write_json`, 16.9 KB record | 5860 us | 5978 us | 6919 us |
| append + flush, **no fsync**, reopened per write | 28.3 us | 28.5 us | 37.3 us |
| append + flush, **held open**, no fsync | **4.2 us** | **2.7 us** | 7.8 us |
| append + **fsync**, held open | 3827 us | 3980 us | 4933 us |

n=200 each. A 489 B record and a 16.9 KB record cost the same order, and a
held-open unsynced append is ~1000x cheaper than either. **The fsync is the
entire cost, not the write shape** — which is why this design writes often and
`fsync`s never, and why "durable" here means *flushed*, stated plainly.

### The board is the expensive place, and by how much

| tasks in session | `record_execution` (one board append) | `list_tasks` (daemon, per board read) |
|---|---|---|
| 1 | 5600 us | 330 us |
| 4 | 6170 us | 434 us |
| 16 | 7245 us | 814 us |
| 64 | 7950 us | 2236 us |

A board append takes `lock_board`, then `read_all_strict` — a strict re-parse of
**every** task in the session — then a whole-file fsynced rewrite.

### What one board write costs the STAGE client

A real `piod`, a real Unix socket, `BenchClient::task_board` exactly as
`read_board` calls it. This is blocking, **on the render thread**. n=200.

| tasks in session | mean | p50 | p95 |
|---|---|---|---|
| 1 | 221 us | 206 us | 342 us |
| 4 | 345 us | 320 us | 474 us |
| 16 | 1311 us | 1174 us | 1947 us |
| 64 | 4267 us | 4234 us | 4507 us |

For scale, the animating repaint tier is 16 ms and a whole STAGE frame is
1.83 ms debug / 0.23 ms release. At 64 tasks **one board read costs more than
two debug frames**.

### The watcher's coalescing bounds nothing

`spawn_change_watch` drains only what is already queued (`while
changes.try_recv().is_ok() {}`); there is no time-based debounce. Measured
against a byte-for-byte copy of that loop, writing task files the way
`write_task` does:

| write cadence | wakes/write | wakes/sec |
|---|---|---|
| as fast as possible | **1.59** | 7213 |
| 100/s | 1.40 | 111 |
| 10/s | 1.45 | 13.8 |
| 2/s | 1.25 | 2.5 |

Above 1 at every cadence, because a temp-create plus a rename raises several
`notify` events. The watcher faithfully amplifies the write rate; it does not
limit it.

**Combining the last two tables gives the ceiling that settled WHERE:** at 64
tasks, 10 board writes/sec would cost a client 5.9% of wall clock in blocking
round-trips, and 100/sec would cost 47%. The board tolerates roughly **2 durable
writes per second**. A whole dispatch lifetime is 9–11 board writes *in total*
today. Any per-tick board write was dead on this evidence, and this branch
writes the board zero extra times.

This answers the question #50's review left open — *"I expect this is fine, but
it is the exact pathology the issue quotes and phase 2 will make it hotter;
worth a measurement before that."* It was fine. It would not have been.

### A per-line mirror is free on the drain thread

The one place "cheap" had to be proven rather than assumed, because #28 was a
pipe-buffer deadlock and `drain_to_eof`'s reader thread exists to keep the pipe
empty. 20,000 lines (2.46 MB) through a real pipe, four ways, four repetitions:

| variant | run 1 | run 2 | run 3 | run 4 |
|---|---|---|---|---|
| read only (today) | 208.6 | 185.5 | 191.6 | 193.6 |
| + `write_all` + `flush` per line, uncapped | 191.9 | 201.0 | 194.1 | 192.6 |
| + per line, capped then counters | 201.2 | 193.7 | 196.0 | 201.4 |
| + `BufWriter`, flush every 8 KiB | 196.1 | 194.1 | 191.8 | 203.8 |

(ms; ~100k lines/sec throughout.) Every variant sits inside the run-to-run
variance of doing nothing and "read only" is **not** systematically fastest — it
lost two of four rounds. The drain is bounded by the child and the pipe, not by
our write. #28's hazard is not re-opened.

### The honesty floor nobody clears

`read_until(b'\n')` means nothing is observable until a newline arrives, and
whether one arrives is the **child's** decision. A worker printing every 50 ms
for two seconds:

| worker | first line reaches the reader | run |
|---|---|---|
| `/bin/sh` echo loop | 0.005 s | 2.378 s |
| `python3 script.py` | **2.187 s** | 2.197 s |
| `python3 -u script.py` | 0.018 s | 2.162 s |

A block-buffering child delivers **nothing** until it exits, and no persistence
design can change that — the bytes are in the child's buffer, not our pipe. So
the claim this branch makes is *"whatever the worker flushes, when it flushes
it"*, never *"you will see partial output"*. The journal reports **lines and
bytes observed**; a reader may say "no complete line observed since T" and must
not say "the worker is quiet".

---

## 3. What shipped

Two artifacts per attempt, with two different authorities, and keeping them
apart is the design rather than an implementation detail.

```
~/.orchestra/dispatches/<session_key>/
  D-….json                unchanged
  D-….supervisor.json     unchanged
  D-….a1.out.log          NEW  verbatim worker stdout
  D-….a1.err.log          NEW  verbatim worker stderr
  D-….a1.progress.jsonl   NEW  orchestrator counters
```

**The byte logs hold worker bytes and nothing else** — no marker, no counter, no
timestamp, no rendering, no `from_utf8_lossy`. Because an append-only file never
removes and never re-renders, it structurally cannot reproduce either lie the
in-memory capture contains: `Captured::raw()`'s `tail` is a `VecDeque` that pops
from the front, so a mid-flight render is not a prefix of the final one; and
`Captured::result()`'s `persisted` is `if answer.is_empty() { raw } else
{ answer }`, a kind-flip that swaps raw transport for prose the first time an
extractor fires. The log's claim is *"the worker wrote these bytes to this
stream, in this order, and byte N is byte N forever."* Because it carries no
orchestrator bytes, there is also nothing in it for a worker to forge.

**The journal holds orchestrator statements and nothing else**: exact cumulative
`bytes`/`lines`/`dropped`/`kept`, a one-shot capability declaration, and why the
attempt ended. It is written from the supervisor's **main** thread, so its
`close` is ordered against `persist_terminal` rather than racing it, and it
survives the two exit paths that return before the wait loop is entered.

**Cadence.** The logs are written per line with no rate limit at all, bounded
structurally by `PROGRESS_LOG_MAX_BYTES`. The journal has two gates, both
required: a **change** gate (a counter must have moved — no new bytes, no write,
ever) and a **floor** of `PROGRESS_NOTE_MIN_INTERVAL`, which is *referenced
from* `orch::DEFAULT_AWAIT_POLL_MS` rather than copied, because
`await_delegation` is the fastest durable reader in the tree and publishing
faster than any reader reads is amplification with no observer. The invariant a
mutation has to break: **every write is caused by a byte arriving; the floor
only ever removes a write and can never manufacture one.**

**Absent vs empty.** The logs are created empty at spawn, before any byte
exists. So *present and zero-length* means "the supervisor looked and there was
nothing yet", and *absent* means "this supervisor is not streaming". Two
different facts, kept apart for zero writes — which is why there is no idle
heartbeat.

**Retry.** Each attempt gets its own files. Attempt 1's bytes are neither
deleted nor spliced onto attempt 2's; which attempt produced which bytes is a
fact of the filesystem rather than a marker a reader has to notice.

**The hard rule.** Nothing mid-flight is ever written into `record.stdout` or
`record.stderr`, and `reconcile_record` is not touched. `report.rs`'s
`parse_review_verdicts` brace-scans `stdout` from the first `{` to the last `}`,
so an in-band "PARTIAL" banner would be stripped by construction and an orphaned
reviewer's half-finished thinking could parse as a real verdict.

---

## 4. Acceptance

### The definition of done: durable BEFORE it exits

`orc-core/tests/dispatch_progress.rs::partial_output_is_durable_before_the_worker_exits`
samples while a real worker runs and asserts, at the same instant, that the log
holds the worker's first two lines **and** that `execution_status == "running"`,
`ended_at.is_none()`, `record.stdout.is_empty()`, and the final answer is *not*
yet present.

### The I/O cost, measured

`making_progress_durable_adds_no_record_writes_and_no_board_writes` prints it,
in the style of `orc-app`'s existing `six_workers_…` benchmark:

```
issue #49 phase 2 — I/O shape over one 2.27s dispatch of 2000 worker lines:
  dispatch record writes (distinct mtimes) : 2
  task board writes      (distinct mtimes) : 3
  progress journal notes                   : 4
  progress log bytes                       : 134890
  journal bytes                            : 1103
  dispatch record bytes                    : 17847
```

2000 lines of durable partial output for **zero** extra record writes and
**zero** extra board writes. The four notes are 2.27 s against the 500 ms floor.
Counting distinct **mtimes** rather than distinct `updated_at` strings is
deliberate — `now_iso()` has one-second granularity, so a string count would
understate every implementation equally and could not fail.

### The repaint cost is zero, and it is zero by construction

`file_watches()` covers exactly `runs`, `reports` and the task board;
`~/.orchestra/dispatches` is not watched, and `grep '"dispatches"' orc-app/src`
is empty. No write in this design raises `BoardChanged`, so none costs a
blocking `task_board` round-trip, a daemon-side re-read of every task file, or a
`git diff` subprocess. Combined with "zero board writes" above, measured.

The honest reading, stated rather than buried: **phase 2 makes the state durable
and leaves the cost of watching it to phase 3.** What phase 3 inherits is
bounded by construction — a progress reader is a `metadata()` plus a bounded
read, not a board round-trip, and the note rate is capped at 2/sec per live
worker.

---

## 5. Mutation checks

Every guarantee was broken on purpose and the intended test observed to fail.
**Fifteen mutations, fifteen caught.**

Every mutation rebuilt `target/debug/pio` first. `dispatch_supervisor::execute`
runs in a separate process, and `cargo test -p orc-core` does not rebuild the
binary — the trap `findings.md` recorded on 2026-07-31, which this session hit
on its first test run and recognised from the note.

| # | Mutation | Caught by |
|---|---|---|
| 1 | `log.append` moved out of the drain loop into `Drain::finish` (i.e. `main`) | `partial_output_is_durable_before_the_worker_exits` + 3 others |
| 2 | log rewound and rewritten instead of appended | `the_progress_log_is_append_only_and_never_shrinks` + 3 |
| 3 | gate A removed (unconditional heartbeat) | `a_silent_worker_writes_nothing_at_all` |
| 4 | gate B removed (no rate floor) | `a_chatty_worker_writes_a_bounded_number_of_bytes` |
| 5 | the log cap removed | `a_chatty_worker_writes_a_bounded_number_of_bytes` |
| 6 | `has_extractor` claims an extractor it does not have | `the_record_declares_the_extractor_it_actually_has` |
| 7 | journal renamed `.json`, back inside `list_dispatches`' filter | `progress_sidecars_are_never_read_by_the_listing_path` |
| 8 | `attempt` dropped from `progress_paths` | `each_attempt_has_its_own_paths_and_they_never_collide` + 1 |
| 9 | `read_progress` errors on a torn trailing line | `a_torn_trailing_frame_does_not_lose_the_frames_before_it` |
| 10 | `skip_serializing_if` dropped from the record field | `the_progress_field_is_additive_in_both_directions` |
| 11 | note floor hardcoded away from `DEFAULT_AWAIT_POLL_MS` | `the_note_floor_is_the_fastest_durable_readers_poll_interval` + 1 |
| 12 | logs not created at spawn (absent and empty merge) | 4 tests |
| 13 | the log folded into `record.stdout` on reconcile | `a_killed_supervisor_leaves_its_partial_output_on_disk` |
| 14 | logs unlinked alongside `{id}.supervisor.json` | same |
| 15 | `progress` cleared by `reconcile_record` | same |

Mutations 13–15 are the ones worth naming: they are the design of the *rejected*
proposal. 13 in particular is the fold that would have let an orphaned reviewer's
verdict-shaped output parse as a real verdict.

---

## 6. What this does NOT do

- **No human reads it mid-flight.** This is the honest description of what
  shipped: the state now exists on disk, bounded and prefix-stable — not "the
  operator can watch it". `pio dispatch show` lives in `orc-cli`, outside this
  issue's allowed paths, and the TUI does not watch `~/.orchestra/dispatches`.
  In phase 2 the readers are `tail -f`, `dispatch_progress::read_progress`, and
  the tests. **If an operator-visible surface is wanted inside phase 2, that
  needs `orc-cli` unblocked and is raised on the issue rather than smuggled in.**
- **No `orc-daemon` and no `orc-proto` change.** Nothing mid-flight crosses the
  wire. A speculative field would be an unprobed capability.
- **No task-history word**, so no `circuit::message_for` arm, no `lane_for` arm,
  no `task_vocabulary.rs` row, and no animated packet. A board word per note
  would be catastrophic against `TASK_HISTORY_WINDOW = 8` versus an 11-entry
  lifecycle, and #54's missing stale-lock reclaim is not widened by one
  microsecond.
- **No fsync.** "Durable" means flushed: every byte has left this process, so a
  SIGKILLed supervisor loses nothing already written, but a power cut or kernel
  panic can lose the tail. This is the tier `runs/<id>/output.log` already uses.
  The defence is arithmetic — ~4 ms per fsync against any interesting rate —
  plus that precedent, not principle. A reviewer may reasonably disagree.
- **No signal handler**, so the blind window is narrowed to whatever the last
  `write(2)` missed rather than closed. Its own change.
- **No GC.** Worst case 5 attempts x 2 x 256 KiB plus journals; a typical
  single-attempt run is well under 200 KB. Stated, uncollected.
- **No prose for three of four adapters.** `extract_adapter_event` extracts only
  for `pi`, so `extractable` is durably `false` for codex, claude and hermes and
  the entitlement there is bytes. This design refuses to guess an extractor.
- **The byte log can trail the terminal record on the timeout path.** The
  timeout branch takes `Drain::snapshot` *without* joining — deliberately, so a
  surviving grandchild cannot wedge the supervisor — so a drain thread may
  append after `persist_terminal` has written. The record stays authoritative
  for "finished"; the log is authoritative for "what bytes existed". The
  journal's `close` is main-thread and therefore correctly ordered.

## 7. Defects found and reported, not fixed here

- **#55 — `DispatchRecord.stdout` is unbounded for any adapter with an
  extractor.** `Captured::answer` has no length check and `result()` prefers it
  over the bounded window, so `MAX_CAPTURED_BYTES` is bypassed whenever an
  extractor fires. Driven for real: **409,600 bytes, 25x the documented cap, with
  no truncation marker.** No test caught it because `dispatch_flood.rs`'s fixture
  declares `adapter = "flood-worker"`, which has no extractor — so the cap
  assertion has only ever exercised the bounded path. This branch routes around
  it (the logs have their own cap and never touch `stdout`) rather than
  inheriting it.
- **#54 — `tasks::lock_board` has no stale reclaim.** Filed before this work
  started, because phase 2 was expected to raise the board write frequency. In
  the end it does not — this branch writes the board zero extra times — so the
  window is not widened after all. Still real, still unfixed, still worth its own
  change.

---

## 8. Gates and the flake A/B

**360 passed, 0 failed** on a clean run (348 on `origin/main` + 12 new: ten in
`dispatch_progress.rs`, the orphan test in `dispatch.rs`, and the I/O
measurement).

Two tests failed at some point during the A/B. Neither is dismissed as a known
flake; each was chased to a controlled experiment.

### Interleaved full-workspace A/B, alternating trees on the same machine

| round | branch | `origin/main` |
|---|---|---|
| 1 | 360 / 0 | 348 / 0 |
| 2 | 359 / **1** — `the_message_vocabulary_is_snapshotted_against_the_ambient_pulse` | 347 / **1** — `a_real_dispatch…` |
| 3 | 360 / 0 | 348 / 0 |
| 4 | 359 / **1** — `a_real_dispatch…` | 348 / 0 |
| 5 | 360 / 0 | 348 / 0 |
| 6 | 359 / **1** — `a_real_dispatch…` | 348 / 0 |
| 7–12 | — | 5 clean, 1 × `a_real_dispatch…` |

Raw: branch **3 / 6**, main **2 / 12**. Taken alone that reads as a regression,
and it is the reading I set out to disprove or accept.

### `a_real_dispatch_writes_delivery_then_completion_and_the_gap_is_the_worker`

The assertion that fires is `delegate_returned < WORKER_SLEEP`:

```
panicked at crates/orc-app/tests/task_vocabulary.rs:699:5:
delegate confirms delivery, it does not wait for the answer (1.954582291s)
```

`orch::delegate` normally returns in ~70 ms against a 1.5 s budget. **There is a
real candidate mechanism in this branch**: `open_progress` creates three files
and writes one journal frame *before* `on_started`, i.e. on the delivery path
the budget covers. That had to be measured, not argued.

Three experiments:

| experiment | branch | `origin/main` |
|---|---|---|
| isolated binary, quiet machine, 12 runs | **0 / 12** | **0 / 12** |
| full workspace, uncontrolled | 3 / 6 | 2 / 12 |
| isolated binary, **identical synthetic load** (8 CPU burners + 4 IO writers), 10 runs | **3 / 10** | **5 / 10** |

Under controlled, identical load `origin/main` fails **more often**, and its
observed `delegate` latencies are worse — 3.83 s, 2.44 s, 2.39 s, 2.27 s against
the branch's 2.19 s, 1.65 s, 1.62 s. So the added file creates are not what
starves it; CPU and I/O contention is, and it starves `main` at least as hard.

I also ran the branch's full workspace six times with this branch's three
heaviest new tests skipped, to test whether their I/O was raising the load: the
rate was unchanged (3 / 6). It is not the new tests either.

**Conclusion: load-sensitive, reproduces on `origin/main`, not caused by this
branch.** The 3/6-vs-2/12 gap is uncontrolled-load noise across twelve samples;
the controlled experiment is what settles it and it points the other way.

### `the_message_vocabulary_is_snapshotted_against_the_ambient_pulse`

One occurrence, on the branch, in round 2. It is an `orc-app` time-anchored
golden, and **this branch changes no file in `orc-app`** —
`git diff origin/main --name-only` is `orc-core` and `findings.md` only. Under
identical CPU load, isolated: **0 / 10 on both trees.** #50's own evidence note
records this golden's family (a shared `now - 180ms` anchor across sequential
renders, tightened when phase 1 halved the packet quantum) and reports fixing it
per-case; a residual sensitivity to full-workspace interleaving remains.
Observed once, not reproduced, and not attributable to a branch that does not
touch the crate.

### Gates

```
cargo fmt --all -- --check                                 exit=0
cargo clippy --workspace --all-targets -- -D warnings      exit=0
cargo test --workspace                                     360 passed, 0 failed
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps  exit=0
cargo build --release --locked                             exit=0
```

---

## 9. Review round: FIX (5), six surviving mutations, all fixed

The reviewer ran eleven mutations of their own and **six survived**. Every one
is a real gap and all six are fixed on the branch. The mutation battery is now
**22/22**.

The pattern named in the review is the important part, and it is the third
occurrence in this program: **#50 shipped a test that could not fail, #51 one
that under-drove its own lifecycle, and this branch asserted against a helper
standing in for the path under test.** All three are the same error wearing
different clothes — testing the component instead of the wiring.

### 1. The headline retry guarantee was held by nothing

`each_attempt_has_its_own_paths_and_they_never_collide` compared
`progress_paths(…, 1)` against `progress_paths(…, 2)`. That tests the *naming
function*. **Nothing tested that the supervisor passes the real ordinal**, and
no test on the branch drove a retry at all — even though retry was one of the
four design questions the issue mandated.

The reviewer hardcoded the ordinal and the whole suite stayed green while
`ProgressLog::create`'s `.truncate(true)` destroyed the rate-limited attempt's
output: the exact outcome the design calls impossible.

**Fixed** by `a_rate_limited_retry_keeps_the_earlier_attempts_bytes`, which
drives a real retry through `dispatch_with_policy` against a worker that emits,
returns a 429 signal and exits non-zero on attempt 1, then succeeds on attempt 2.
It asserts attempt 1's bytes survive, attempt 2's are separate, neither log
contains the other's, and each journal carries its own `open`/`close` sequence.
Mutation 17 (constant ordinal) now fails **two** tests.

### 2. The progress-open warnings were built and thrown away

`dispatch_supervisor.rs` had `let _ = progress_warnings;`, while
`ProgressLog::create`'s doc promised "one durable warning on the record".
Deleting both `ORC WARNING:` strings passed the whole suite. Worse: when a log
could not be opened, `record.progress` still named files that do not exist —
a third state the doc never defined — and the artifact that would have explained
it was dropped.

**Fixed**: the warnings now flow into the `RefCell<Vec<String>>` that
`persist_terminal` assigns to `record.warnings`, where `pio dispatch list/show`
prints them. Threading them through that vec rather than writing them in
`mark_started` matters — `persist_terminal` *assigns* `record.warnings`, so
anything written earlier would have been overwritten.

`a_progress_log_that_cannot_be_opened_is_reported_on_the_record` plants a
**directory** at attempt 2's stdout-log path, which fails exactly that `open`
with `EISDIR` while leaving `write_dispatch` and the journal alone. Making the
whole dispatch directory read-only was the first attempt and was the wrong
instrument: it also breaks `atomic_write_json`, so the dispatch never reaches a
terminal state and the test hangs on a different failure. The backoff delay is
what makes the plant possible at all — attempt 1's `open` happens microseconds
after spawn and cannot be raced.

### 3. Two documented log properties were false

- **"A partial line is never written."** It was: the cap clipped mid-line, and
  the reviewer has a 262144-byte log ending on `.`. That ends the log on a byte
  boundary — for a JSON transport a truncated object nothing can parse, for
  UTF-8 a split code point — and makes the log's last line something the worker
  never emitted, which is the one claim the byte log exists to make.
  **Fixed in the code, not the doc**: a line that does not fit is declined in
  full and the log latches closed. `the_log_never_ends_mid_line` asserts the
  final byte is `\n`, that the bytes are valid UTF-8, and that every line is
  verbatim worker output.
- **`lines` counted reads, not complete lines.** `read_until` also returns the
  unterminated remainder at EOF, so `printf 'alpha\nbeta'` reported 2. That
  matters precisely because a block-buffered worker sitting mid-line is the case
  this journal must not overstate — the honesty floor in §2 is about exactly
  that. **Fixed**: counted on the terminator.
  `an_unterminated_final_chunk_is_not_counted_as_a_line` pins `bytes == 10,
  lines == 1`.

### 4. Three durable record fields were unheld

- `attempts` was structurally identical to `attempt` — the supervisor only ever
  knows the ordinal it is on, so no implementation could make them differ.
  **Removed** rather than tested; a field that cannot vary is not data.
- `extractable` and `log_max_bytes` survived being falsified. **Fixed** by
  `the_record_states_the_capability_and_cap_that_are_actually_in_force`, which
  asserts `extractable` against `runner::has_extractor` for the adapter the
  record itself names, and `log_max_bytes` against the constant the log
  enforces. Mutations 21 and 22 both die there.

### 5. Dead code

`read_progress`'s guarded arm was identical to its unguarded one — a distinction
the caller cannot observe and that could not fail — and `progress_lengths` was
public with zero callers. Both **deleted**.

### Mutation battery, now 22/22

| # | Mutation | Caught by |
|---|---|---|
| 17 | supervisor passes a constant attempt ordinal | `a_rate_limited_retry_keeps_the_earlier_attempts_bytes` + 1 |
| 18 | progress warnings dropped on the floor | `a_progress_log_that_cannot_be_opened_is_reported_on_the_record` |
| 19 | line clipped at the cap again | `the_log_never_ends_mid_line` |
| 20 | every read counted as a line again | `an_unterminated_final_chunk_is_not_counted_as_a_line` |
| 21 | `extractable` hardcoded `true` | `the_record_states_the_capability_and_cap_that_are_actually_in_force` |
| 22 | `log_max_bytes` zeroed | same |

(1–16 in §5 above.)
