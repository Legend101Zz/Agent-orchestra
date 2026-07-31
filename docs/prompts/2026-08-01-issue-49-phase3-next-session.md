# NEXT SESSION — #49 phase 3 (the in-pane reveal)

One session. The block below the line is copy-paste ready.

State: `main` @ `2dc35db`. **#49 phase 2 merged as PR #56** (review FIX (5) →
all fixed in `881fb37` → re-review ACCEPT), #51 as PR #53, #50 (#49 phase 1) as
`32c5058`, #52 (SSD rule) as `e1b8e0a`. Sessions 1 (#51) and 2 (phase 2) have
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
the whole suite passes. The test exists and is verified (it passes on `b7f6954`
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

# #49 phase 3 — the in-pane reveal

```
Work phase 3 of GitHub issue #49 of Legend101Zz/Agent-orchestra - the in-pane reveal.
Branch: issue-49-phase3-brief-overlay, worktree slug issue-49-phase3.

[PASTE THE PREAMBLE HERE]

Read: AGENTS.md -> docs/WORKFLOW.md -> task_plan.md -> issue #49 IN FULL including the
Decision 1 comment -> PR #50 and docs/notes/2026-07-31-issue-49-watchable-delegation.md ->
docs/notes/2026-07-31-issue-49-phase2-evidence.md -> docs/design/visual-identity.md.

Phases 1 and 2 and #51 are all merged; main is at 2dc35db. Branch off fresh origin/main.

## What phase 2 actually built, since you are the first code to read it

Per attempt, beside the dispatch record, never fsynced ("durable" = flushed):

  ~/.orchestra/dispatches/<session>/<id>.a<N>.out.log       verbatim worker stdout
                                    <id>.a<N>.err.log       verbatim worker stderr
                                    <id>.a<N>.progress.jsonl  orchestrator counters

  dispatch::progress_paths(session, id, attempt) -> ProgressPaths   pure, always recomputable
  dispatch_progress::read_progress(&journal) -> Result<Option<ProgressView>>
  DispatchRecord.progress: Option<DispatchProgress>

RULES, all of which a reveal can violate and none of which are optional:

- Derive paths with progress_paths. record.progress is DISCOVERABILITY ONLY - write_dispatch
  takes no lock and reconcile_record rewrites the whole record, so it must never be the only
  way you find bytes that exist.
- ABSENT record.progress means "this supervisor is not streaming" -> render "progress
  unavailable". PRESENT with a zero-length log means "we looked and there was nothing yet".
  Different facts. Do not collapse them. (One known hole: a single line larger than the whole
  cap is declined, so a 300 KB worker can leave a zero-length log. The honest discriminator is
  kept < bytes from the journal, not the file's length.)
- extractable is TRUE ONLY FOR "pi". For codex, claude and hermes the entitlement is BYTES,
  never prose - extract_adapter_event returns (None, None) for all three. Never guess an
  extractor.
- The byte logs hold worker bytes and NOTHING else - no marker, no timestamp, no rendering.
  That is the whole honesty argument. Do not write into them.
- Nothing mid-flight is ever in record.stdout, and must not become so: report.rs's
  parse_review_verdicts brace-scans that field, so a folded partial can parse as a real verdict.
- read_progress never returns Err for a malformed journal, deliberately. Keep it off any path
  that would make a torn sidecar drop a dispatch from a listing.
- THE HONESTY FLOOR: drain_to_eof uses read_until(b'\n'), so whether anything is observable is
  the CHILD'S decision - a block-buffered Python worker delivered its first line at 2.187s,
  i.e. nothing until exit, while the same worker with -u delivered at 0.018s. The vocabulary is
  "no complete line observed since T". It is NEVER "the worker is quiet" and never "thinking".
- Phase 2 writes the board and the dispatch record ZERO extra times, and ~/.orchestra/dispatches
  is NOT in file_watches(). So nothing wakes the client on progress today. Whatever you add,
  add it deliberately and measure it: one board write costs a STAGE client a BLOCKING task_board
  round-trip on the render thread (221us at 1 task, 4.27ms at 64) against a 16ms animating tier,
  and spawn_change_watch delivers 1.25-1.59 wakes PER WRITE. The board tolerates ~2 durable
  writes/sec. A poll of the log files is not free either - say what it costs.

## Decision 1 is ANSWERED: option (a). This is binding.

The brief is NOT delivered into the seated pane. STAGE draws DispatchRecord.prompt over the
worker's card. No PTY write, no ClientRequest::Input, no second delivery, no orc-daemon. Do
not implement (b) or (c) and do not relitigate it - the reasoning is in the issue comment. If
you find something that genuinely bears on it, comment; do not act on it.

## Scope

1. The brief overlay on the worker card, sourced from DispatchRecord.prompt, which is durable
   at dispatch time. Phase 1's board watcher already wakes the client on that write, so no new
   plumbing should be needed - CONFIRM that rather than assuming it.
2. Whatever phase 2 made durable, revealed honestly - the rules above are the contract. The
   reveal must never show a character the worker has not actually produced, must never turn an
   empty log into "thinking", and must state which of the two "nothing here" facts it is
   showing. Land the `capped`-latch test named at the top of this file while you are in here.
   Note #55 is open and unfixed: for an adapter WITH an extractor, record.stdout is unbounded
   (measured 25x over MAX_CAPTURED_BYTES, no truncation marker) - if the reveal ever falls back
   to that field for "pi", it can be handed 400 KB.
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
(conductor_down, orc-app/src/lib.rs:4387); how the client would reach the progress artifacts at
all, given they cross no wire today; tachyonfx's actual API surface against our render path.
Judged design phase for the overlay's dismissal model. Implement, then adversarially verify -
attack "never shows a character the worker has not produced" hardest.

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
