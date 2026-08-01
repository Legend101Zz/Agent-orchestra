# Session prompt — #14: README + architecture docs for launch

*Written 2026-08-01 against `main` @ `4b9b915`. Copy the block below verbatim.*

> **Maintenance rule:** when this issue merges, retire this file the way
> `2026-08-01-issue-49-phase3-retired.md` was retired — rename, do not delete.
> Note that #14 itself may move this whole directory (Decision 1 on the issue);
> if it does, this file moves with it.

---

```
Work GitHub issue #14 of Legend101Zz/Agent-orchestra — the README and the
architecture docs for launch. Branch: issue-14-readme-architecture, worktree
slug issue-14-readme.

Repo: "/Volumes/Mrigesh SSD/pi-orchestra" — note the space, quote every path.

Work in a worktree on the SSD, never the shared checkout and never the system
disk (AGENTS.md: ~35 GB free there against the SSD's ~640 GB, and one worktree
carrying a debug plus a release build is 2-4 GB):

  [ -d "/Volumes/Mrigesh SSD/pi-orchestra/.git" ] || { echo "SSD not mounted - STOP"; exit 1; }
  git -C "/Volumes/Mrigesh SSD/pi-orchestra" fetch origin
  git -C "/Volumes/Mrigesh SSD/pi-orchestra" worktree add \
      "/Volumes/Mrigesh SSD/pi-orchestra-worktrees/issue-14-readme" -b issue-14-readme-architecture origin/main
  cd "/Volumes/Mrigesh SSD/pi-orchestra-worktrees/issue-14-readme"

If the SSD is not mounted, STOP and report back. Do not fall back to $HOME, do
not clone "somewhere convenient", do not create the directory: an unmounted
/Volumes/Mrigesh SSD is an ordinary directory on the system disk, so anything
written there fills the wrong volume under a path that looks correct. Remove
the worktree when the issue merges - they never shrink on their own.

NOTE: main's history was rewritten on 2026-08-01 (authorship cleanup). Branch
off a FRESH origin/main; do not reuse an old local checkout or worktree.

Baseline on main is 395 passed, 0 failed. This issue should not change that
number, and if it does you have touched code you were not asked to touch.

Five gates from rust/, all green before pushing — yes, all five, even though
this is a docs issue:
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
  cargo build --release --locked

`cargo doc` is the one that will actually bite you: rustdoc runs with
-D warnings and you will be editing doc comments to repoint doc paths.

Known LOAD-sensitive test failures, on main as well as on any branch — NOT
yours, and there is no reason this issue should meet them at all:
  - a_real_dispatch_writes_delivery_then_completion_and_the_gap_is_the_worker
    asserts a fixed 1.5s wall-clock bound. Filed as #60 with the full A/B.
  - the orc-cli quota family, same character.
If one fires, re-run in isolation and A/B against origin/main before saying a
word about it.

Stop and comment on the issue rather than improvising if: the task contract is
wrong or impossible, a fix needs a path outside the allowed list, or you find a
defect that is not yours. Scaling the work down is the product owner's call,
not yours.

Read: AGENTS.md -> docs/WORKFLOW.md -> issue #14 IN FULL -> LOG.md's ship log
(every entry — it is the best existing account of what this project does, it
was written for a human reader, and it is accurate) -> docs/ANTI-SLOP.md ->
docs/design/visual-identity.md -> README.md as it stands.

## Answer Decision 1 FIRST, on the issue, before writing anything

Archive or delete the internal doc directories. The issue lays out the cost:
95 citations across 66 files, five of them in Rust source. The recommendation
is docs/archive/ with a README inside it. Comment your decision and reasoning
on the issue, then proceed — do not silently pick.

BINDING regardless: docs/design/visual-identity.md and its visual-identity/
subtree do NOT move. Seven Rust files cite that path as the live source of
truth for the render layer. Confirm that yourself before you touch anything:

  grep -rn "docs/design/visual-identity" rust/crates/*/src/*.rs

## The three deliverables

1. README.md rewritten. Positioning, a CURRENT hero recording in Nocturne, an
   install guide that assumes nothing (prerequisites with version checks, what
   install.sh touches and refuses to touch, the no-install path, uninstall, and
   a verification step the reader runs BEFORE trusting it), a first session
   walked through to a delegation they can watch, and an honest capability
   table. Keep and extend Troubleshooting — it is the best section you have.

2. docs/architecture/ — the centrepiece. Mermaid diagrams (GitHub renders them
   natively, no toolchain). Split across files, not one wall. The sequence
   diagram for the dispatch lifecycle is the single most valuable thing in this
   issue: delegate -> supervisor -> per-attempt progress artifacts ->
   confirmation -> reconcile. Cover the crate map with real line counts, the
   daemon's PTY ownership and coalescing, the client's render path and
   degradation tiers, the ~/.orchestra data model, the capability model, and
   how this project is tested.

3. docs/ restructured per Decision 1, all 95 citations repointed.

LICENSE is already done (MIT, 1b93799) — it is off your list. Just state
the licence in the README.

## The trap this issue is actually about

Every stale line in the current README was true when it was written. It says
two themes; there are three and the default is Nocturne, which it never
mentions. Its keys table is missing ctrl-g t and ctrl-g i. Its screenshots are
from 2026-07-12 and show a UI that no longer exists.

So: VERIFY EVERY CLAIM AGAINST THE CODE, not against the old README and not
against LOG.md. When you write "three themes", have read ThemeName::ALL. When
you list a key, have found it in RawRouter::route. When you claim a harness can
dispatch, have run `pio adapter list`. The old README is a source of stale
claims, not a source of truth — treat it as a draft to be checked line by line.

For the three or four load-bearing claims in the architecture docs, name the
file and line in the PR so a reviewer can check them in one click.

## Recording

tools/*.tape holds the existing VHS recipes; they are all v4-era. Write new
ones. Hero in Nocturne, at least one Phosphor variant — the monochrome tier is
a real acceptance concern and showing it is the honest thing. State in the PR
which commit each capture was recorded at.

Do NOT try to capture the CONDUCTOR DOWN overlay. It has never been visible
(#59, open): it is drawn before the pane's cell blit and erased by it in the
same frame.

If a recording reveals a defect, FILE IT, do not fix it here. This issue
changes no behaviour.

## Definition of done

Every acceptance check on #14 answered explicitly, in the issue comment, with
pasted output where output exists — the link check especially, which must be a
command's output and not your eye. Say plainly which install steps you actually
executed and which you only read. Then update progress.md (dated entry), LOG.md
(status + plain-English ship-log entry), task_plan.md, and findings.md for any
durable decision — Decision 1 is one. Push, comment the evidence, open a PR.

Say plainly what you did NOT do and why.

#14 is the last original V1 item. When it merges, V1 ships.
```
