# NEXT SESSIONS — #49 phase 2, then #49 phase 3

Two sessions, run **in that order, one at a time, with a review between each**.
Each block below the line is copy-paste ready.

State: `main` @ `77b6c11`. **#51 merged as PR #53** (review FIX (2) → both fixed
in `4ae609b` → re-review ACCEPT), #50 (#49 phase 1) as `32c5058`, #52 (SSD rule)
as `e1b8e0a`. Session 1 (#51) has been removed from this file per the
maintenance rule at the bottom.

## Why this order, and why not in parallel

Not stylistic — phase 2 is load-bearing for phase 3.

- **Phase 3 reveals what phase 2 makes durable.** Without phase 2 there is
  nothing to reveal and scope item 2 is empty.
- **Never in parallel.** Both touch `orc-app`'s event path.
- **#51 is done, and that is what unblocks phase 2.** Its defect 1 — the
  eight-entry cliff that silently stopped a task animating for good — is fixed,
  so the watermark phase 2's write frequency will hammer now moves. Had you
  started phase 2 first you would have spent it chasing phantom "why isn't this
  showing" problems that were #51's.

Phase 2 is an implementer session (code-puppy, or whatever is building); after
the push, a Claude session reviews adversarially per `docs/WORKFLOW.md` prompt 2.
Do not merge on the implementer's own say-so.

## Before you start phase 2 — one open follow-up from #51

`tasks::lock_board` has **no stale reclaim**: a process SIGKILLed while holding
`.board.lock` wedges that session's board for ever. `spawn_guard::lock_slots`
already solved exactly this for `.slots.lock`. Found during #51 and deliberately
not fixed there (it is the core locking primitive for every board writer and
deserves its own change and its own test) — the fix is spelled out on issue #51
and in `findings.md`, 2026-07-31. **Phase 2 adds board writes at a much higher
frequency, which widens the window on this.** Worth its own issue first.

## Shared preamble — paste this at the top of both

Kept in one place deliberately. It was duplicated across three prompts once and
drifted: all three ended up telling the session to work on the internal disk,
which `AGENTS.md` prohibits. One copy cannot disagree with itself.

```
Repo: "/Volumes/Mrigesh SSD/pi-orchestra" — note the space, quote every path.

Work in a worktree on the SSD, never the shared checkout and never the system disk
(AGENTS.md: ~35 GB free there against the SSD's ~645 GB, and one worktree carrying a
debug plus a release build is 2-4 GB):

  [ -d "/Volumes/Mrigesh SSD/pi-orchestra/.git" ] || { echo "SSD not mounted - STOP"; exit 1; }
  git -C "/Volumes/Mrigesh SSD/pi-orchestra" fetch origin
  git -C "/Volumes/Mrigesh SSD/pi-orchestra" worktree add \
      "/Volumes/Mrigesh SSD/pi-orchestra-worktrees/<slug>" -b <branch> origin/main
  cd "/Volumes/Mrigesh SSD/pi-orchestra-worktrees/<slug>"

If the SSD is not mounted, STOP and report back. Do not fall back to $HOME, do not
clone "somewhere convenient", do not create the directory: an unmounted
/Volumes/Mrigesh SSD is an ordinary directory on the system disk, so anything written
there fills the wrong volume under a path that looks correct. Remove the worktree when
the issue merges - they never shrink on their own.

Give review subagents their OWN worktree under pi-orchestra-worktrees/, and never let
them run cargo test against yours. #50's session let them write probes into the working
tree and had to discard a gate run and an A/B that picked up their tests and CPU load.

The orc-cli quota flakes (background_dispatch, cli_dispatch_at_cap_..., 
delegate_confirms_while_running_...) are LOAD-sensitive, not volume-sensitive, and
reproduce on main - measured 2 failures in 6 full-workspace runs on origin/main. Re-run
in isolation AND A/B against origin/main on the same machine before attributing one to
your branch. Do not use "known flake" as a blanket excuse.

Mutation-check every new test: break the thing it protects on purpose, confirm THAT test
fails, restore. #50 shipped a test that could not fail for any implementation and only
caught it by doing this; #50's review then found two more guarantees held by nothing.

Stop and comment on the issue rather than improvising if: the task contract is wrong or
impossible, a fix needs a path outside the allowed list, or you find a defect that is not
yours. Scaling the work down is the product owner's call, not yours.

Five gates from rust/, all green before pushing:
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
  cargo build --release --locked

Finish by updating progress.md (dated entry), LOG.md (status + plain-English ship-log
entry), task_plan.md, an evidence note under docs/notes/, and findings.md for any durable
decision. Push, comment the evidence on the issue with pasted output per acceptance check,
open a PR. Say plainly what you did NOT do and why.
```

---

# Session 2 — #49 phase 2 (incremental progress persistence)

```
Work phase 2 of GitHub issue #49 of Legend101Zz/Agent-orchestra - incremental supervisor
output, the issue's defect 3. Branch: issue-49-phase2-incremental-output, worktree slug
issue-49-phase2.

[PASTE THE SHARED PREAMBLE HERE]

Read: AGENTS.md -> docs/WORKFLOW.md -> task_plan.md -> issue #49 IN FULL -> PR #50 and
docs/notes/2026-07-31-issue-49-watchable-delegation.md. Phase 1 is merged; do not rebuild it.

#51 is merged (PR #53); main is at 77b6c11. Branch off fresh origin/main. Read
docs/notes/2026-07-31-issue-51-board-honesty.md too: it changed the watermark your write
frequency will hammer (StageState::seen_history is now an absolute index located by
TaskSummary.history_total, not a length into the daemon's window), it added reviewer_run and
a per-message circuit::Lane, and it added execution_orphaned / review_execution_orphaned to
the vocabulary. If you add a task-history word, it needs a circuit::message_for arm, a
circuit::lane_for arm, and a row in orc-app/tests/task_vocabulary.rs.

Known open defect you may hit, NOT yours to fix: tasks::lock_board has no stale reclaim, so a
process SIGKILLed while holding .board.lock wedges that session's board until the file is
deleted by hand. Phase 2 writes the board far more often, so you are likelier to meet it than
anyone so far. If a test wedges, check for a stale .board.lock before assuming your own bug.
Report it, do not fix it here.

## The problem, precisely

drain_to_eof (dispatch_supervisor.rs) reads the child's pipes on their own threads - that is
what fixed #28's deadlock - but accumulates into an in-memory String. Drain::finish is only
called in the wait branches, i.e. AFTER the child exits. Between spawn and exit the durable
record says nothing, so there is no honest way to drive any reveal: there are no characters
until it is already over.

Phase 2 is ONLY: make partial progress durable, honestly. Do NOT build a reveal - that is
phase 3 and it is scoped separately.

## The design questions, which are the whole job

- WHAT is durable? Raw bytes, extracted adapter text, a token/byte counter, a heartbeat? Each
  has a different honesty cost. `Captured` (dispatch_supervisor.rs:199) already distinguishes
  the raw head+tail window from the adapter-extracted answer; say which one a reader is
  entitled to see mid-flight, and why.
- HOW OFTEN? Every line is write amplification on a chatty worker; every N seconds invents a
  cadence. Whatever you choose must be defensible as real state. Phase 1's board watcher wakes
  the client on EVERY write and read_board does a blocking task_board socket round-trip on the
  render thread - measure that cost, do not assume it.
- WHERE? write_dispatch rewrites the record wholesale. A growing field on a record rewritten
  every tick is a different I/O shape from an append-only sidecar.
- What happens to partial output when the worker is killed, times out, or is rate-limit
  retried? A retry re-runs invoke_harness - does attempt 2's partial output replace attempt
  1's, and is that honest?

Bring at least two independent proposals, judge them against each other, record the loser in
findings.md with why. "Durable JSON is additive; readers tolerate unknown fields" binds.

## Use workflows

Recon: the drain path end to end; every writer of the dispatch record and its frequency; what
the board watcher now costs per write; how orch await/status would surface progress. Then the
judged design phase. Then implement, then adversarially verify - attack write amplification
and the "real state only" guarantee specifically.

## Definition of done

Demonstrate with a real slow worker that partial output is durable BEFORE it exits, and that
the I/O and repaint cost is MEASURED, not asserted.
```

---

# Session 3 — #49 phase 3 (the in-pane reveal)

```
Work phase 3 of GitHub issue #49 of Legend101Zz/Agent-orchestra - the in-pane reveal.
Branch: issue-49-phase3-brief-overlay, worktree slug issue-49-phase3.

[PASTE THE SHARED PREAMBLE HERE]

Read: AGENTS.md -> docs/WORKFLOW.md -> task_plan.md -> issue #49 IN FULL including the
Decision 1 comment -> PR #50 and docs/notes/2026-07-31-issue-49-watchable-delegation.md ->
docs/design/visual-identity.md.

Confirm phases 1 and 2 and #51 are all on main before starting - do not assume it. Phase 3
reveals what phase 2 made durable, so if phase 2 has not landed, scope item 2 below is empty
and you should say so rather than invent something to reveal.

## Decision 1 is ANSWERED: option (a). This is binding.

The brief is NOT delivered into the seated pane. STAGE draws DispatchRecord.prompt over the
worker's card. No PTY write, no ClientRequest::Input, no second delivery, no orc-daemon. Do
not implement (b) or (c) and do not relitigate it - the reasoning is in the issue comment. If
you find something that genuinely bears on it, comment; do not act on it.

## Scope

1. The brief overlay on the worker card, sourced from DispatchRecord.prompt, which is durable
   at dispatch time. Phase 1's board watcher already wakes the client on that write, so no new
   plumbing should be needed - CONFIRM that rather than assuming it.
2. Whatever phase 2 made durable, revealed honestly. The reveal must never show a character
   the worker has not actually produced.
3. THEN, and only then, evaluate tachyonfx on its own merits. It is ALREADY a workspace
   dependency - rust/Cargo.toml:40, version 0.25.1 with features ["std-duration"], consumed by
   orc-app - so adopting it adds NO new dependency and no Cargo.lock change. (Issue #49's body
   still calls it "a new dependency"; that is stale.) The real cost is a second animation model
   beside circuit.rs. It CANNOT do cross-rect travel - effects are post-render and rect-confined
   - so it is irrelevant to the packet; its only candidate use is the in-rect reveal/fade. If
   you adopt it, its colour interpolation must be driven from the semantic slots or it trips the
   no-hex gate. "Not worth it" is a perfectly good answer - say so with evidence.

## The design constraints that will bite

- The overlay covers a hosted CLI's own output. When is it dismissed, and does it ever hide
  something the user needs? An overlay with no exit is worse than no overlay.
- Long briefs. The default is multi-line Markdown. Truncation must be visible, not silent.
- Every degradation tier: reduced motion, monochrome/NO_COLOR, the ASCII glyph column, 80x24,
  and zoom. Phase 1 left acceptance 9 (the packet vanishes when zoomed) untouched - decide
  whether the overlay survives zoom or STAGE says why, and pin it with a test.
- Decision 2 from phase 1 stands: the packet is ONE cell, no trail. findings.md, 2026-07-31,
  enforced by the_packet_is_one_cell_and_draws_no_trail.

## Use workflows

Recon: how render_pane composes its chrome and what an overlay would displace; where
DispatchRecord.prompt is reachable from the client; the existing overlay precedent
(conductor_down); tachyonfx's actual API surface against our render path. Judged design phase
for the overlay's dismissal model. Implement, then adversarially verify - attack "never shows a
character the worker has not produced" hardest.

## Definition of done

Goldens regenerated with the diff as the review. Acceptance checks 9 and 10 from #49 both
answered explicitly - that closes the issue. Note in the PR that #49 is fully discharged, so
#14 (README + screenshots) is unblocked and V1 can ship.
```

---

## Maintenance

Delete a session's block once its issue merges, and say so in the `LOG.md`
ship-log entry. A stale prompt file that still says "do this next" is the same
class of hazard as the internal-disk line this file was written to kill.
