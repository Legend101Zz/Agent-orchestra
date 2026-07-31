# NEXT SESSIONS — #51, then #49 phase 2, then #49 phase 3

Three sessions, run **in that order, one at a time, with a review between each**.
Each block below the line is copy-paste ready.

State when this was written: `main` @ `492ff6c`. #50 (#49 phase 1) merged as
`32c5058`, #52 (SSD rule) as `e1b8e0a`.

## Why this order, and why not in parallel

Not stylistic — each one is load-bearing for the next.

- **#51 first because its defect 1 is live on `main` right now.** `orc-daemon`
  truncates `TaskSummary.history` to the last eight entries while `orc-app`'s
  `note_task_events` watermark counts *into* that window, so a task past eight
  events stops animating permanently. #50 added the ninth event to a full
  contracted lifecycle, so the `moved→done` that should fly the final
  confirmation home is exactly the one that falls off. Start phase 2 first and
  you will spend it debugging phantom "why isn't this showing" problems that
  are #51's, not yours.
- **Phase 3 reveals what phase 2 makes durable.** Without phase 2 there is
  nothing to reveal and scope item 2 is empty.
- **Never in parallel.** All three touch `orc-app`'s event path, and #51 changes
  the very watermark phase 2's write frequency will hammer.

Session 1 and 2 are implementer sessions (code-puppy, or whatever is building);
after each push, a Claude session reviews adversarially per `docs/WORKFLOW.md`
prompt 2. Do not merge on the implementer's own say-so.

## Shared preamble — paste this at the top of all three

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

# Session 1 — issue #51 (do this first)

```
Work GitHub issue #51 of Legend101Zz/Agent-orchestra - three places the board and the
screen disagree. Branch: issue-51-board-honesty, worktree slug issue-51.

[PASTE THE SHARED PREAMBLE HERE]

Read in this order: AGENTS.md -> docs/WORKFLOW.md -> task_plan.md -> issue #51 INCLUDING
its comments -> PR #50 and its evidence note docs/notes/2026-07-31-issue-49-watchable-delegation.md.
#51 was carried out of #50; that note records what was already tried. The issue's task
contract is binding.

#50 is merged (32c5058); main is at 492ff6c. Branch off fresh origin/main.

## Scope

All three defects. They share a cause - the daemon's TaskSummary is too thin to carry what
the client needs - and the issue says not to split the branch.

Defect 1 is reproducible on main TODAY. Acceptance check 1 asks for a test that "fails if
the watermark goes back to being a length"; this probe shape is exactly it. Mimic
orcd::task_board's .rev().take(8).rev() against a contracted-then-reviewed task, drive it
PAST eight entries, and assert the next event still raises a flight. A test that stops AT
eight passes on the broken code.

Note on defect 1's framing: the issue says all three defects "need orc-daemon". True for 2
and 3, not for 1 - the watermark is orc-app's own StageState::seen_history and a
content-anchored watermark would fix it client-side. The daemon-side total-length field the
issue specifies is still the BETTER fix (it is exact, survives any window size, and does not
depend on entry identity being unique within a second - now_iso is second-granularity, so
(at, actor, action, to) can collide). Record that as "we chose the daemon field over the
client-side anchor, and here is why" in findings.md, not as "the client could not do it".

Defect 2 has a real design fork that is yours to resolve and record in findings.md:
reconcile_record runs inside read_dispatch/list_dispatches, so making it append task history
turns ANY process that lists dispatches into a board writer. Decide with evidence - measure
what actually calls those functions - not preference.

Acceptance check 5 is the one to be most careful with. #50's append_execution already had to
be best-effort-with-durable-warning because propagating a board-lock failure there would
abort execute() before drain_queued and strand every queued dispatch in the session. Option
(i) is the same hazard with a wider blast radius. Whatever you pick, state plainly which
processes may now write to the board.

## Use workflows

Orchestrate it. Fan out recon over: the daemon's task_board and everything consuming
TaskSummary; every call path reaching reconcile_record; the reviewer dispatch lifecycle
(orch::review -> dispatch_review -> record_review_delivery) and what it does and does not
link. Then a judged design phase for defect 2's fork. Then implement, then adversarially
verify - attack hardest the claim that two processes listing dispatches concurrently cannot
corrupt the board or deadlock.

## Definition of done

Additive JSON proven, not asserted: show an old reader parsing a new record AND a new reader
parsing an old one. Confirm deliberately whether PROTOCOL_VERSION moves (the hello handshake
compares BUILD_IDENTIFIER and refuses mixed builds - there is precedent in findings.md,
2026-07-29). The window size ends up a named constant with a comment saying what depends on it.
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

Confirm #51 is merged before starting. If it is not, check specifically whether its
history-window fix landed - without it a task past eight events does not animate at all and
you will chase ghosts that are not yours.

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
