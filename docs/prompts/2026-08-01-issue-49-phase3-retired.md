# NEXT SESSION — #49 phase 3 (the in-pane reveal)

One session. The block below the line is copy-paste ready.

State: `main` @ `cf1db85`. **#49 phase 2 merged as PR #56** (review FIX (5) →
all fixed in `3206006` → re-review ACCEPT), #51 as PR #53, #50 (#49 phase 1) as
`636afb6`, #52 (SSD rule) as `63b69db`. Sessions 1 (#51) and 2 (phase 2) have
been removed from this file per the maintenance rule at the bottom.

**Phase 3 is the last one, and it closes #49** — which unblocks #14 (README +
screenshots) and V1.

Phase 3 is an implementer session (code-puppy, or whatever is building); after
the push, a Claude session reviews adversarially per `docs/WORKFLOW.md` prompt 2.
Do not merge on the implementer's own say-so.

## Before you start — one follow-up merged unfixed with phase 2

`ProgressLog`'s `capped` latch (`dispatch_progress.rs`) is **held by no test**.
It is what stops a line declined for not fitting under `PROGRESS_LOG_MAX_BYTES`
being followed by a *shorter* line that does fit — which leaves the log with a
**hole**, the one thing "byte N is byte N forever" forbids, and with
variable-length lines that is the ordinary case at the cap. Remove the latch and
the whole suite passes. The test exists and is verified (it passes on `0aed94a`
and fails with the latch removed); it is pasted in the re-review comment on #49.
**Land it in phase 3**, because phase 3 is the first code that *reads* the log
and therefore the first that a hole would lie to.

Two smaller phase-2 leftovers, same comment: a single line larger than the whole
cap leaves the log at zero length for ever, and both stated reader rules
("present and zero-length means nothing yet"; "tell capped from quiet by `kept`
against `log_max_bytes`") then say *quiet* about a worker that wrote 300 KB —
the honest discriminator is `kept < bytes`. And `note` frames' `stderr` counters
are asserted by nothing.

## Preamble — paste this at the top of the block below

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

Baseline on main is 365 passed, 0 failed. Anything else is yours until you prove
otherwise.

Known LOAD-sensitive failures, measured on both trees, NOT volume-sensitive:
  - a_real_dispatch_writes_delivery_then_completion_and_the_gap_is_the_worker (orc-app)
    fires on a fixed 1.5s wall-clock bound, overshooting by ~140ms. Measured across 65
    runs: isolated on a quiet machine 0/12 on BOTH trees; isolated under identical CPU
    load 0/10 on BOTH trees; full workspace 2/7 on the phase-2 branch against 2/9 on
    origin/main. Same rate. It is the assertion, not any branch.
  - the orc-cli quota family (background_dispatch, cli_dispatch_at_cap_...,
    delegate_confirms_while_running_...), same character.

Re-run in isolation AND A/B against origin/main on the same machine before attributing
one to your branch. Do not use "known flake" as a blanket excuse - phase 2's session
chased one properly and was right to.

Mutation-check every new test: break the thing it protects on purpose, confirm THAT test
fails, restore. THREE times now this program has shipped a test that could not fail:
#50 one that no implementation could break, #51 one that under-drove the lifecycle it
claimed to measure, #56 one that asserted against a HELPER standing in for the path -
it compared progress_paths(.., 1) with progress_paths(.., 2) while nothing checked that
the supervisor passed the real ordinal, and hardcoding that ordinal destroyed a
rate-limited attempt's output with the whole suite still green.

Same error in three costumes: testing the component instead of the wiring. Phase 3 is
the most exposed yet - a reveal is easy to assert about a pure function and hard to
assert about what a reader actually SEES. Mutate the call site, not just the callee.

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

## Retired: #49 phase 3 — the in-pane reveal

Pushed 2026-08-01 on `issue-49-phase3-brief-overlay`; the block is removed per
the maintenance rule below, which exists because a prompt file that still says
"do this next" about finished work is the same class of hazard as the
internal-disk line this file was written to kill.

What that session found, for whoever writes the next prompt:

- **The prompt file told it to CONFIRM that phase 1's watcher already wakes the
  client on the write that makes the brief durable, "rather than assuming it".
  It confirmed, and the claim was false.** `DispatchRecord.prompt` was reachable
  from `orc-app` by no path — not over the wire, not off disk. The instruction
  to check rather than assume is what caught it, and it is worth keeping in
  future prompts verbatim.
- The two carried-over phase-2 follow-ups named at the top of this file are
  landed: the `capped`-latch test and the unheld `note`-frame `stderr`
  counters, plus both reader-rule docs corrected to `kept < bytes`.
- One pre-existing defect is open and **deliberately unfixed**: the
  `conductor_down` overlay is drawn before `render_pane`'s cell blit and
  overwritten by it in the same frame. It is covered by no test or golden. It
  needs its own issue or an explicit instruction to fold it in.
- Evidence: `docs/notes/2026-08-01-issue-49-phase3-evidence.md`. Durable
  decisions: `findings.md`, 2026-08-01.

---

## Maintenance

Delete a session's block once its issue merges, and say so in the `LOG.md`
ship-log entry. A stale prompt file that still says "do this next" is the same
class of hazard as the internal-disk line this file was written to kill.
