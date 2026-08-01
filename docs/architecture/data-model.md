# The data model

Everything durable lives under `~/.orchestra` as plain JSON. No database, no
schema migrations, no binary formats. The CLI, the TUI, the daemon and your own
scripts all read the same files.

Override the root with `ORC_HOME` — which is also how you try this project
without touching your real state.

## Layout

```
~/.orchestra/                      (0700)
├── orcd.sock                      (0600) the daemon socket
├── harnesses.json                 the registry — harnesses, profiles, app config
├── config.json                    operator-console config (derived copy of app.*)
├── quota_history.jsonl            append-only quota observations
├── sessions/                      one JSON per durable session: panes, layout, cwd
├── tasks/<session>/               the task board, one JSON per task
├── dispatches/<session>/
│   ├── <id>.json                  the dispatch record
│   ├── <id>.supervisor.json       supervisor spec; deleted on reconcile
│   ├── <id>.aN.out.log            worker stdout bytes, append-only
│   ├── <id>.aN.err.log            worker stderr bytes, append-only
│   └── <id>.aN.progress.jsonl     orchestrator counters, append-only
├── runs/<id>/
│   ├── meta.json                  atomically rewritten
│   ├── output.log                 streamed worker output
│   └── inbox/                     follow-up messages for a live RPC run
├── reports/                       final receipts: usage, cost, verdicts
├── slots/                         durable concurrency leases (+ .slots.lock)
└── worktrees/                     isolated per-task git worktrees
```

Every one of those paths is constructed in `orc-core/src/registry.rs` and
`dispatch.rs`; none of them is assembled anywhere else.

## Additive JSON, and why unknown fields survive

**Records are additive. A reader must tolerate fields it has never heard of, and
a writer must not drop them.**

Every durable struct carries a catch-all:

```rust
#[serde(flatten)]
pub extra: BTreeMap<String, Value>,
```

So a record written by a newer build, read by an older one, and written back
again comes out with the newer fields intact. That is what lets this project add
`execution_status` (issue #30) or a progress journal (#49 phase 2) without a
migration and without breaking anybody's existing `~/.orchestra`.

The rule has a sharp edge, and the project has already cut itself on it. In #49
phase 3 an `#[serde(flatten)] extra` map was added to `DispatchBrief` to satisfy
a review finding about preserving unknown fields — eleven lines below a docstring
promising that a struct with no `stdout` field made a 400 KB leak *"structural,
not a rule someone has to remember"*. The map put `extra["stdout"]` back within
reach. `DispatchBrief` is never serialized, so the map preserved nothing and
cost the guarantee. Filed as
[#58](https://github.com/Legend101Zz/Agent-orchestra/issues/58).

The lesson worth carrying: **a preservation map belongs on types that are
actually round-tripped.** On a read-only projection it is not preservation, it is
just a hole.

## Atomic writes

Anything rewritten in place goes through `atomic_write_json`
(`orc-core/src/registry.rs`): write a temp file beside the target, then rename.
A rename is atomic on both macOS and Linux, so a reader either sees the whole
old record or the whole new one — never a half-written one, and never an empty
file because a process died mid-write.

The append-only artifacts are the deliberate exception. A byte log is never
rewritten, so it needs no atomicity: it only ever grows, and byte *N* is byte
*N* forever. That property is the whole reason it can be trusted as a
contiguous prefix of what the worker produced.

**"Durable" means flushed, not fsynced.** Measured: `sync_all` costs ~4 ms
against ~4 µs for a held-open unsynced append. A SIGKILLed process loses nothing
already written; a power cut or kernel panic can lose the tail. That is the same
tier `runs/<id>/output.log` has always used.

## Board locking

The task board is written by more than one process, so it takes a lock. There
are two lock families and **their orders are kept disjoint on purpose**: nothing
in the workspace takes the board lock and then a slot lock
(`orc-core/src/dispatch.rs:1363-1366`). Two lock families that are never nested
in both orders cannot deadlock, and that is cheaper to guarantee than to detect.

Both locks are held briefly and scoped to one operation.

⚠️ **Known defect, not fixed here.** `.board.lock` has **no stale reclaim**: a
process killed while holding it wedges that session's board until someone deletes
the file by hand. The sibling `.slots.lock` already solved exactly this with an
atomic rename-steal that records the holder pid and reclaims a dead or aged lock,
so the fix is about thirty lines of already-reviewed code. It is filed as
[#54](https://github.com/Legend101Zz/Agent-orchestra/issues/54).

## Durable concurrency leases

`slots/` is what makes `max_parallel_workers` mean something. A lease is a file,
not a process-local counter, so a cap is honoured:

- across every `pio` process on the machine,
- across sessions,
- and across detachment — the lease is **transferred** to the detached
  supervisor rather than released when `delegate` returns.

Without that transfer, non-blocking delegation would have silently removed the
cap: you could start unbounded workers simply by returning fast enough. See
[the dispatch lifecycle](dispatch-lifecycle.md#why-the-caller-returns-early).

Caps are per-harness (`pio harness cap <tool> <n>`) with sensible defaults, plus
a session-wide `max_parallel_workers`. When a harness is at its cap the next
hand-off is recorded as `queued` and **no worker is spawned**; `pio dispatch
drain` runs the waiting work when a slot frees.

## The history window

A task's history is a durable append-only event list. But the summary the daemon
ships to the client carries only the newest eight:

```rust
pub const TASK_HISTORY_WINDOW: usize = 8;
```
— `orc-proto/src/lib.rs:204`

This constant has caused two bugs, both worth knowing because they are the same
mistake from opposite ends.

**The 8-event cliff.** The client tracked its place in the list by *counting how
many entries it had seen*. That works right up until the list stops growing —
past eight, the count sticks, and *nothing about that task ever animates again*.
No error, no fallback, no warning. It went live on `main` when #50 added a ninth
event to a fully reviewed job, so the very last step — the answer coming home —
was exactly the one that fell off. Fixed in
[#51](https://github.com/Legend101Zz/Agent-orchestra/issues/51) by having the
daemon also report the *total* history length, so the client knows where its
visible window sits in the job's life. The eight is still eight; it is just no
longer load-bearing.

**The mirror image.** The fix changed two lines, and reverting only one of them
left the whole suite green — because a length and an absolute index are the same
number below the window, and still agree on the *first* crossing. They diverge
from the second, where a length lags the sliding window and replays entries
already shown. A real reviewed lifecycle is eleven entries, so this was live for
every reviewed task.

The durable lesson from that round is recorded in `findings.md` and is worth
repeating here because it generalises well beyond this constant: **when one fix
changes two lines, mutate them separately.** A combined revert only proves the
pair is load-bearing.

## Reading it yourself

Nothing here is private to the binaries:

```bash
ORC_HOME=/tmp/scratch pio harness list          # try it without touching your real state
jq . ~/.orchestra/harnesses.json
jq . ~/.orchestra/tasks/<session>/T0001.json
pio task list --session <id> --json
pio dispatch list --session <id>
```

If a durable record and the screen ever disagree, the record is the truth and the
screen has a bug. That has been the case three times so far and each is written
up in the ship log.
