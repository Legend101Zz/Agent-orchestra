# Issue #49 phase 3 — the in-pane reveal

Branch `issue-49-phase3-brief-overlay`, off `origin/main` @ `77b2f27`.
Phase 3 is the last of the three and it closes #49.

Everything below was run on this machine. Where a number decided a design
question, the harness that produced it is named. Where a claim in the issue or
the prompt file turned out to be false, it is corrected here rather than worked
around silently.

---

## 1. Baseline, before anything was touched

`cargo test --workspace` on a clean worktree at `77b2f27`:

```
passed=365 failed=0
```

The number the prompt file names, reproduced on this tree, so anything else
during this session is this branch's until proved otherwise.

---

## 2. The premise the issue is built on is false

Issue #49's Decision 1 comment (2026-07-31) and the phase-3 prompt file both
assert that the overlay needs no new plumbing:

> Phase 1 landed a `notify` watcher on `~/.orchestra/tasks`, so the client
> already wakes on the dispatch write that makes the prompt durable — the
> overlay needs no new plumbing, no protocol change, and nothing from
> `orc-daemon`.

The prompt file turns that into an instruction — *"CONFIRM that rather than
assuming it"*. Confirmed, and it does not hold. The wake is real. It does not
carry the prompt, and nothing else does either.

### The prompt becomes durable in a file nothing watches

`write_dispatch` (`orc-core/src/dispatch.rs:1115`) writes only
`~/.orchestra/dispatches/<session_key>/<D-id>.json`. The first write that makes
`prompt` durable is `dispatch.rs:911`, inside `deliver`, before the supervisor
is launched.

`file_watches()` (`orc-app/src/lib.rs:2913-2931`) is exhaustively three entries
— `runs`, `reports`, `home()/"tasks"` — and nothing else in `orc-app`
constructs a `notify` watcher. `~/.orchestra/dispatches` is watched by nothing,
which is what `docs/archive/notes/2026-07-31-issue-49-phase2-evidence.md:249-252`
already recorded for phase 2.

### The wake that does fire is one hop later, via a different file

| step | code | file written | wakes the client? |
|---|---|---|---|
| prompt built | `dispatch.rs:1032` `new_record` | — | — |
| **prompt first durable** | `dispatch.rs:911` `write_dispatch` | `dispatches/<sk>/<D>.json` | **no** |
| supervisor launched | `dispatch.rs:924-934` | `<D>.supervisor.json` | **no** |
| progress artifacts created empty | `dispatch_supervisor.rs:405` `open_progress` | `.a1.out.log`, `.a1.err.log`, `.a1.progress.jsonl` | **no** |
| `mark_started` re-writes the record | `dispatch_supervisor.rs:671` | `<D>.json` | **no** |
| **`record_delivery`** | `dispatch_supervisor.rs:689` → `tasks.rs:942` `write_task` | `tasks/<sk>/<T>.json` | **yes** |
| watcher → `BoardChanged` → `read_board` | `lib.rs:3028`, `:2704`, `:2753` | — | — |
| daemon projects | `orc-daemon/src/lib.rs:679-711` | — | **no prompt** |

So with every pane silent the client genuinely does wake — phase 1 works — but
what arrives is `(assignee_run \| reviewer_run, action word, to)`.

### And the prompt is on no message in either direction

- `TaskSummary` (`orc-proto/src/lib.rs:207-260`) carries `id, title, status,
  assignee, assignee_run, reviewer_run, isolated, isolation, blocked, tokens,
  diff, history, history_total`. No prompt, no dispatch id. Even the on-disk
  `TaskHistory.detail` string (`orc-core/src/tasks.rs:398-406`) is dropped by
  `TaskHistorySummary` (`orc-proto/src/lib.rs:263-273`).
- `DispatchSummary` (daemon → client, `orc-proto/src/lib.rs:300-333`) has no
  `prompt` field. Verified against the projection itself,
  `dispatch_summary` at `orc-daemon/src/lib.rs:900-916`, which drops it.
- `prompt` appears on the wire only in `DispatchCommand`
  (`orc-proto/src/lib.rs:292-293`), which is **client → daemon**, and no
  production client sends it: `pio orch delegate` (`orc-cli/src/main.rs:1576`)
  and the MCP tool (`orc-mcp/src/lib.rs:110-114`) both call
  `orc_core::orch::delegate` in-process.
- `orc-app` imports nothing from `orc_core::dispatch`. `lib.rs:27-28` import
  `discovery` and `single_harness`; the other `orc_core` reads are
  `report::list_reports`, `bench::read_harness_registry`, `trigger_grammar`,
  and `registry::home()` for the watch roots.

**`DispatchRecord.prompt` is reachable from `orc-app` by no path today.**

### Why the wire is not the fix

Both projections that would have to carry it live in `orc-daemon`
(`dispatch_summary` at `:900-916`, the `TaskSummary` one at `:679-711`), and
`orc-daemon` is outside this issue's allowed paths with an instruction to
comment first. Note the handler for `ClientRequest::DispatchBoard` already
exists in production (`orc-daemon/src/lib.rs:853`) — so the *verb* is not the
missing piece; the *field* is, and adding it is a daemon change.

Reported on the issue before implementation started rather than after the diff.

---

## 3. Three traps that shape any correct implementation

### T1 — `read_dispatch` and `list_dispatches` are writes

Both are `pub` and both run `reconcile_record` (`dispatch.rs:1210-1231`), which
on a lost supervisor **terminates the worker's process group**
(`runner::terminate_pid`, `:1215`), releases spawn-guard slots,
**appends to the task board** (`append_orphan`, `:1221`), and rewrites the
record with `write_dispatch` (`:1228`).

A reveal that "just reads the record" from the render thread would make drawing
a frame kill a worker and mutate the board — and that board write raises
`BoardChanged`, which re-reads the board. The genuine reader,
`read_dispatch_unreconciled` (`dispatch.rs:1172`), is `pub(crate)` and not
reachable from `orc-app`.

This is a correctness constraint, not a performance preference, and it is why
a new reader is required rather than a call to an existing one.

### T5 — the prompt is *usually* `render_brief(task)`, which makes reconstruction look correct and be wrong

`orch.rs:459-463`:

```rust
let prompt = match request.prompt {
    Some(prompt) if !prompt.trim().is_empty() => prompt,
    _ => render_brief(&task),
};
```

`render_brief` (`orc-core/src/contract.rs`) is a pure function of the `Task`, so
a reveal that rendered the brief from board data would match the delivered
prompt most of the time and **differ silently whenever an explicit prompt was
supplied**. It could not even reproduce the default: `TaskSummary` carries
`title` and none of `description`, `contract.objective`, `acceptance_checks`,
`limits` or `budget`.

The overlay must show the persisted `prompt` or show nothing.

### T4 — `record.progress == None` does not mean "no bytes exist"

`DispatchProgress` is written only by `mark_started`
(`dispatch_supervisor.rs:665,671`), which is on the delivery-handshake path.
The handshake-failed arm (`dispatch_supervisor.rs:499-508`) closes the journal
and returns **before any record write**, while `open_progress` (`:405`) has
already created all three artifacts. So files routinely exist on disk with the
record's `progress` field absent.

`dispatch.rs`'s own struct doc already says absent means "this supervisor is not
streaming" and must render as *progress unavailable*, never as "the worker is
silent". This is the code path that makes that a live case rather than a
theoretical one.

---

## 4. A pre-existing defect, found and reported, not fixed here

**The `conductor_down` overlay is painted and then erased in the same frame.**

`render_pane` (`orc-app/src/lib.rs:4323-4482`) draws the overlay `Paragraph` at
`:4399-4412` and then runs the cell blit at `:4414-4481`:

```rust
let rows = inner.height.min(pane.rows);
let cols = inner.width.min(pane.cols);
…
target.set_symbol(if source.text.is_empty() { " " } else { &source.text });
target.set_style(style);
```

The blit is unconditional and writes `" "` for an empty source cell.
`resize_to_cards` (`lib.rs:2823-2839`) asks the daemon for exactly
`rows = area.height - 2`, `cols = area.width - 2` — which is exactly `inner` —
so on a live-sized pane the grid covers the whole overlay rect and overwrites
it. It would survive only transiently, between a resize request and the next
snapshot.

Nothing catches it: the shared STAGE fixture `stage_panes()` sets `state: None`
(`snapshot.rs:273`), and `rg 'state: Some\('` over `orc-app` `src/` and `tests/`
returns nothing — **no golden and no unit test ever renders a `conductor_down`
pane.** The `CONDUCTOR DOWN` assertion at `lib.rs:6410` is HOME's shelf line
from `session_health` (`lib.rs:1793-1800`), a different code path.

Not fixed on this branch: it is a different feature from a different issue, and
both "one issue = one branch" and "a defect that is not yours → comment, don't
improvise" point the same way. It is reported on #49 for the product owner to
route.

Two consequences for phase 3 regardless of that routing:

- **The ordering constraint is now known.** Anything drawn inside `inner` must
  be rendered *after* the blit, or the blit must be told to skip its cells.
  Following the cited precedent literally would have produced an invisible
  overlay and a golden that looked correct.
- **`state: None` in the fixture means no golden shows the production title
  either.** The daemon always sets `state`
  (`orc-daemon/src/lib.rs:312-319`: `running` / `stopped` / `conductor_down`),
  so every committed STAGE golden shows the fallback word `LIVE`
  (`lib.rs:4351`).

---

## 5. The two carried-over phase-2 tests, landed and mutation-checked

Both were named in the prompt file as merged-unfixed follow-ups. Each mutation
rebuilt `target/debug/pio` first via `cargo build --workspace`, per the
`findings.md` trap at `:596-607` — `dispatch_supervisor::execute` runs in a
separate process and a narrowed `cargo test -p orc-core` would otherwise run the
*unmutated* supervisor.

### `the_log_is_a_contiguous_prefix_even_at_the_cap`

The `capped` latch in `ProgressLog::append` was held by no test. It is what stops
a line declined for not fitting under `PROGRESS_LOG_MAX_BYTES` being followed by
a *shorter* line that does fit, which leaves the log with a hole — the one thing
"byte N is byte N forever" forbids, and with variable-length lines the ordinary
case at the cap.

The worker alternates a 4000-byte line with a ~9-byte one for 100 iterations, so
the cap (262144) is reached mid-alternation by construction: the long line at
that point does not fit and the short one after it does. The log is compared
against a stream rebuilt independently from the script's own definition.

**Mutation N7 — drop `self.capped` from `append`, both the guard read and the
assignment:**

```
test the_log_is_a_contiguous_prefix_even_at_the_cap ... FAILED

the log must be a contiguous prefix of what the worker wrote. Without the
`capped` latch a declined long line is followed by a shorter one that fits, and
the log gains a hole — the one thing "byte N is byte N forever" forbids.
log ends: "RT-93\nSHORT-94\nSHORT-95\nSHORT-96\nSHORT-97\nSHORT-98\nSHORT-99\n"

test result: FAILED. 16 passed; 1 failed
```

Seven consecutive `SHORT-` lines with the 4000-byte lines missing between them —
the hole, in the log itself. **Exactly one test fails and it is this one**, which
confirms the review's "remove the latch and the whole suite passes" and that this
test is now what holds it.

Two deliberate differences from the sketch pasted in the re-review comment. Its
sole assertion is `stream.starts_with(&log)`, and `starts_with` is **true of an
empty log** and true of any log that never reached the cap — i.e. it would pass
against an implementation that wrote nothing, and against one where the cap was
never exercised, which is the only place the latch does anything. The landed
version adds the two guards that make it non-vacuous (`log.len()` bounded above
by the cap and within one long line of it) and a direct "no two consecutive
`SHORT-` lines" scan, because `starts_with` reports only *that* something
diverged and not *what*.

### `a_note_frame_carries_the_stderr_counters_too`

The third phase-2 leftover: `note` frames' `stderr` counters were asserted by
nothing. A worker writes to both streams for ~3 s, crossing the 500 ms note
floor several times.

**Mutation — `stderr: None` in `ProgressJournal::note`:**

```
test a_note_frame_carries_the_stderr_counters_too ... FAILED
every note frame must carry stderr counters, not just stdout; seq 1 did not

test result: FAILED. 17 passed; 1 failed
```

Again exactly one test, confirming the review's report that setting every note
frame's `stderr` to `None` passed the whole suite. The test guards against its
own vacuity: it asserts the note set is non-empty before iterating it, requires
`stderr.bytes > 0` so `Some(default)` cannot satisfy it, and pins the close
frame's `kept` against the stderr log's actual length so the counters must be
that stream's rather than a copy of stdout's.

### Both reader-rule docs corrected to `kept < bytes`

The two stated discriminators both mislead at the boundary they exist for: a
single line larger than the whole cap is declined in full, so a worker that
emitted 300 KB as one line leaves `kept == 0`, which is nowhere near
`log_max_bytes` and reads as *quiet*. Corrected in
`DispatchProgress`'s struct doc (`dispatch.rs`) and `ProgressLog::append`'s doc
(`dispatch_progress.rs`); `kept < bytes` says "the supervisor observed more than
this log holds" in every capped case, including that one.

---

## 6. Measurements taken this session

### The golden style budget is not a constraint

`snapshot.rs:29`'s `KEYS` alphabet is 74 characters and `:85-90` hard-`assert!`s
above it. Measured on the committed goldens:

| golden | distinct styles | budget |
|---|---|---|
| `stage-nocturne.txt` | 12 | 74 |
| `stage-6-workers.txt` | 13 | 74 |
| `stage-80x24.txt` | 10 | 74 |
| `stage-no-color.txt` | 3 | 74 |
| `stage-message-dispatch.txt` | 11 | 74 |

Ample headroom, so the overlay's palette is free to name whatever slots it
needs without risking a golden panic that reports a palette problem instead of
an overlay one.

---

## 7. tachyonfx — evaluated on its own merits

Already a workspace dependency (`rust/Cargo.toml:40`, `0.25.1`, features
`["std-duration"]`), listed by exactly one crate (`orc-app/Cargo.toml:24`), so
adopting it adds no dependency and no `Cargo.lock` change. Issue #49's body still
calls it "a new dependency"; that is stale, and the prompt file already says so.

It is also **already used** — but only as a stopwatch. One import site in the
whole workspace, `orc-app/src/lib.rs:41`
(`use tachyonfx::{EffectTimer, Interpolation};`), driving a 400 ms decay
countdown (`lib.rs:578, 593-596, 789-796`). `Effect`, `EffectRenderer`,
`EffectManager` and `fx::` have zero occurrences anywhere in the workspace. Even
`EffectTimer::alpha()` is never called; only `from_ms`, `reset`, `done` and
`process` are.

*(Verdict and its argument in §8 once the reveal is implemented.)*

**Verdict: no. Zero new surface, and the argument is its timing model, not taste.**

1. **The band has no motion to render.** Its only candidate beat is a fade-in,
   and a fade-in is a duration chosen for looks — which acceptance check 6
   forbids. The band's appearance is anchored to a keypress, which is a real
   event.
2. **The timing model is a per-frame delta accumulator, and there is no
   absolute-`Instant` API anywhere in the crate.** `rg -n "Instant" src/` over
   the vendored 0.25.1 returns two hits, both inside a doc comment.
   `EffectTimer::process(&mut self, duration)` subtracts a delta from
   `remaining`, and `alpha()` reads that mutated state. Single-shot effects are
   exact under `std-duration`; **looping ones are not** — `fx::repeat(Forever)`
   and `ping_pong` compute the overflow that crossed the cycle boundary and
   throw it away, so the period is a function of the poll cadence. Ours is
   deliberately variable (8/16/30/120/500/30000 ms). That is a direct violation
   of acceptance check 3.
3. **`ColorSpace::lerp_rgb` always returns `Color::Rgb`**, so any faded cell
   becomes truecolor regardless of `ColorTier` — routing around
   `theme.rs`'s no-hex gate at runtime and breaking what `stage-no-color` and
   `stage-trigger-ansi16` promise.
4. **tachyonfx maps `Color::Reset` to `(0,0,0)`** (`color_ext.rs`), and our
   Monochrome tier resolves **every** slot to `Color::Reset`. A fade on the one
   tier whose entire design answer is "no colour" would be the tier that got a
   colour ramp, interpolating toward black.
5. Goldens require two renders of unchanged facts to be byte-identical. A
   mutable per-frame accumulator makes that impossible by construction.

`EffectTimer` and `Interpolation` keep their existing 400 ms decay use.
`Effect`, `EffectRenderer`, `EffectManager` and `fx::` stay at **zero** uses in
the workspace.

For the record, since issue #49's body is stale on this: tachyonfx is **already**
a workspace dependency — `rust/Cargo.toml:40`, `0.25.1`, features
`["std-duration"]`, listed by `orc-app` alone. Adopting it would have added no
dependency and no `Cargo.lock` change. It is declined on its merits, not on its
cost. One incidental finding worth a follow-up: `default-features` is left on,
so the `dsl` feature and its `anpa` parser (~20 files) are compiled and nothing
in this tree calls them.

---

## 9. Two deliberate deviations from the design of record

Both are recorded because the design is in `findings.md` and a later reader
should not have to guess why the code differs from it.

### The blit does **not** skip the band's rows

The design had `render_pane`'s cell blit start at `band` as well as drawing the
sidecar after it, arguing that the skip makes the guarantee "structural rather
than a matter of draw order".

It was implemented, and then removed, because it is not observable.
`render_reveal` opens with `Clear` over the whole band rect followed by a styled
fill, so with the skip and without it every cell in the band is identical — no
test and no user can tell the two apart. Verified directly: with `Clear` in
place, deleting the skip leaves the entire suite green.

A mechanism nothing can distinguish is exactly the shape of defect this program
has paid for three times (#50, #51, #56). One mechanism, one test that holds it:
`the_hosted_grid_is_never_drawn_under_the_reveal` fails the moment
`render_reveal` moves above the blit.

**The `Clear` is load-bearing and was found by that test**, not by design. Before
it, `Block::new().style(..)` was the only fill — and `Block::style` sets a cell's
style while leaving its **symbol**, so a worker glyph showed through wherever a
composed line was shorter than the pane was wide. The test caught a `#` at row 0
column 34.

### `resolve_reveals` takes the reader as a parameter

`absorb_board` passes `orc_core::dispatch::read_briefs` in. That is what lets a
test count reads and assert the invalidation guard holds — the "1 then 0"
assertion that turns the cost claim into a value rather than a paragraph.

---

## 10. What this branch does NOT do

- **The `conductor_down` blit-order defect is not fixed.** §4 characterises it.
  It is a different feature from a different issue, and folding it into this
  diff would make the reveal unreviewable. Reported on #49 for routing.
- **`clip_ellipsis` (`orc-app/src/lib.rs`) still pushes a hard `…` on the ASCII
  tier.** A real pre-existing defect, found while registering `Glyph::Elide`.
  Reveal code does not call it — it uses the register — so phase 3 routes around
  it rather than inheriting it. Reported, not fixed.
- **#55 is not fixed**, and phase 3 measured what it costs a *reader* that
  already refuses it (§6). `DispatchBrief` has no `stdout` field, so the reveal
  structurally cannot be handed 400 KB; serde still has to scan the string to
  skip it.
- **No dispatch-directory retention policy.** `read_briefs` is O(records in the
  session) and nothing prunes `~/.orchestra/dispatches`. The invalidation guard
  makes the read rare and §6 bounds 64 records, but the accumulation of a real
  long-lived session has not been measured. Named here rather than papered over.

---

## 11. Review round: FIX (6), eleven surviving mutations, all fixed

The reviewer ran seventeen call-site mutations and **eleven survived**. Not one
was a wrong behaviour — the happy path and the wiring were correct. Every one
was a guarantee this branch states in prose and nothing held.

**The pattern, and it is the fourth consecutive occurrence.** #50 shipped a test
no implementation could fail, #51 one that under-drove its own lifecycle, #56
one comparing two filenames instead of driving a retry — and this branch
populated **by hand** the map the feature exists to populate. Every render test
did `state.reveals.insert(...)` directly, so `resolve_reveals` and
`reveal::compose` had zero test callers between them.

The sharpest one: **delete `stage.resolve_reveals(..)` from `absorb_board` and
all 392 tests still passed.** The map is never populated, `⌃g i` answers "has
taken no brief in this session" on every pane in every session, and nothing
notices. `absorb_board`'s own docstring diagnosed this exactly — *"a test needs
something to hold… leaves the suite green unless something pins the composition
itself"* — and nothing was built on it.

### The reviewer's correction to their own test, and why it mattered

They wrote the missing test, verified it on `4c1d913`, and it killed 5 of 7 —
but **not** mutation 1, because it called `resolve_reveals` directly, which is
the same one-level-short mistake being reported. `absorb_board` hardcoded
`orc_core::dispatch::read_briefs`, so it could not be driven at all.

Fixed by threading the reader through as a parameter: `read_board` passes
`orc_core::dispatch::read_briefs`, a test passes a counting closure. One call to
`absorb_board` now kills mutations 1 and 2 together. That is the whole reason the
parameter exists and the doc comment says so.

### The battery, re-run against the fix — 11/11 caught

| # | mutation (all at the call site) | now caught by |
|---|---|---|
| 1 | delete `resolve_reveals` from `absorb_board` | `the_board_populates_the_sidecars_once_per_watermark_move` |
| 2 | delete the `if !stale { return 0 }` watermark guard | same |
| 3 | `Lane::Reviewer` → `Lane::Executor` for `reviewer_run` | same |
| 4 | drop the `.skip(1)` keeping the conductor out | same |
| 5 | collapse `Undeclared` into `NotStreaming` in `compose` | `compose_tells_absent_progress_from_absent_artifacts` |
| 6 | default `attempt` to `Some(1)` in `compose` | `the_board_populates_the_sidecars…` |
| 7 | drop `sanitise` from the **worker-bytes** path | `a_forged_badge_in_the_workers_own_bytes_cannot_reach_the_screen` |
| 8 | drop `sanitise` from `hold_prompt` | `the_board_populates_the_sidecars…` |
| 9 | show the **oldest** tail line instead of the newest | `a_forged_badge…` |
| 10 | never `remove` a reveal whose brief is gone | `the_board_populates_the_sidecars…` |
| 11 | delete the themed `Block` beside `Clear` | `the_hosted_grid_is_never_drawn_under_the_reveal` |

Each was re-applied to the fixed tree and observed to fail: **142 passed / 1
failed** in every case, and the tree diffed clean against a backup after each.

Mutation 7 is the one that mattered most in kind. `sanitise` was well tested —
*as a pure function*. The path carrying **worker** bytes to it runs inside
`compose` and had no test caller, so replacing the call with a plain clone left
392 green. The new test writes `\x1b[2J\x1b[H FORGED ✓ TASK CONFIRMED` into a
real byte log beside a real journal, composes, and asserts no `\x1b` and no
control character survives — while the text `FORGED` *does*, because this
replaces rather than deletes and the reader should see what the worker tried.

Mutation 11 was the reviewer's other structural point: the call site argued
`Clear` and the themed `Block` were *both* needed with a distinct reason each,
and only `Clear` was held. The `Block`'s stated consequence — an unthemed hole
punched in the card, because `Clear` resets to the **terminal's** default rather
than to `overlay` — is now asserted directly on a band cell's background.

### Three smaller findings, all real

- **A test docstring described a mechanism that is not in the code.**
  `the_hosted_grid_is_never_drawn_under_the_reveal` claimed "the blit's row
  range starts at `band`" and named dropping the row-skip as a required-failing
  mutation. There is no row-skip — it was removed deliberately (§9) and the PR
  body says so. So the doc contradicted both the code and the PR, and named a
  no-op mutation as binding. Corrected: the ordering is the whole mechanism,
  worker cells *are* written into those rows and then painted over, and the
  second named mutation is now deleting `Clear`.
- **`DispatchBrief` claimed unknown fields were "preserved in the usual way"**
  while carrying no `#[serde(flatten)] extra` and still deriving `Serialize` —
  a serde contract stated and not kept, in a file where both neighbouring types
  carry that map. The field is added rather than the claim softened.
- **`clock` panicked on a non-ASCII durable stamp**, on the render thread.
  `&stamp[11..19]` is guarded by `len() >= 19`, which is **bytes**. Reproduced:

  ```
  byte index 11 is not a char boundary; it is inside 'あ' (bytes 9..12)
  of `あああああああ+00:00`
  ```

  Only reachable from a well-formed journal with a corrupt `t`, so it is remote
  — but `read_progress` is deliberately infallible precisely so a torn sidecar
  cannot blank the screen, and this would have undone that. `stamp.get(11..19)`
  with a fallback to the whole string, pinned by the multi-byte case.

### Gates after the fix

```
cargo fmt --all -- --check                                   exit 0
cargo clippy --workspace --all-targets -- -D warnings        exit 0
cargo test --workspace                                       395 passed, 0 failed
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps    exit 0
cargo build --release --locked                               exit 0, Cargo.lock unchanged
```

392 → 395: three new tests, and the `absorb_board` signature change.
