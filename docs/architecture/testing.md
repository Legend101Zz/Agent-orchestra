# How this project is tested

This is worth its own page, because the approach is unusual and it is the reason
the code is trustworthy at all. The short version: **a test that cannot fail is
treated as a bug, and every new test is broken on purpose to prove otherwise —
at its call site.**

395 tests, 0 failed, across 35 test files.

## The five gates

Every branch runs all five from `rust/` before it is pushed. Not four.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo build --release --locked
```

`--locked` matters: it fails rather than silently updating `Cargo.lock`, so a
dependency cannot drift in under cover of a feature branch. `-D warnings` on
both clippy and rustdoc means there is no warning backlog to hide a new problem
in.

## Mutation checking, at the call site

The rule that does the real work: **after writing a test, break the thing it
protects and confirm the test fails.** If it still passes, the test is asserting
against something other than the code under test, and it goes back.

The words "at the call site" are load-bearing, and this project learned them the
hard way — four consecutive branches shipped a test with the same shape:

> Not "a test was weak" — the line in `absorb_board` that fills the sidecars with
> data could be deleted outright, `⌃g i` would answer "has taken no brief in this
> session" on every pane in every session, and all 392 tests stayed green. Every
> test put the data in **by hand** and then checked it drew correctly. The code
> that actually goes and fetches it had no test touching it at all.

There is a detail in how that got caught which is the whole lesson. The reviewer
wrote the missing test, ran it, and then corrected themselves: it *still* did not
catch the worst mutation, because it called the fetching function **directly** —
the same one-level-short mistake they were reporting. The real fix was to make
`absorb_board` take the reader as a parameter, so a test could hand it a counting
stub and drive the whole path. One call then killed both "the line is gone" and
"the cost guard is gone".

So: mutate the **seam**, not the helper. A test that calls the function under
test directly proves the function works; it does not prove anything still calls
it.

A second rule, from [#51](https://github.com/Legend101Zz/Agent-orchestra/issues/51):
**when one fix changes two lines, mutate them separately.** A combined revert
only proves the pair is load-bearing. Reverting one line of that fix left the
suite green, because a length and an absolute index are the same number below the
history window and still agree on the first crossing.

And a third, from the same round: **a timing failure in a test you did not touch
is not proof the cause isn't yours.** A #50 timing test failed on a branch that
never touched it; A/B'ing twelve runs per tree showed the branch's own new test
was racing a real board-before-record gap about one run in ten.

Recent mutation results, from the ship log: 13/13 caught (#51), 22/22 (#49 phase
2), 11/11 plus two regression checks (#49 phase 3).

## Goldens: screenshots as text

29 committed text renders under `rust/crates/orc-app/tests/snapshots/`, and they
compare **colours, not just characters**:

```
home-nocturne.txt   home-ember.txt   home-phosphor.txt   home-ansi256.txt
home-no-color.txt   home-ascii.txt
stage-1-workers.txt stage-3-workers.txt stage-6-workers.txt stage-80x24.txt
stage-trigger-truecolor.txt  …-ansi256.txt  …-ansi16.txt  …-no-color.txt
score-*.txt   runs-*.txt   stage-message-{dispatch,output,return}.txt
```

The matrix is deliberate: every screen at every theme, plus each degradation
tier, plus the ASCII fallback, plus the 80×24 minimum viewport, plus one per
worker count where the circuit topology changes. This is how "monochrome must be
usable" stops being an aspiration — if the no-colour render stops carrying a
glyph, a golden moves and the build fails.

Two supporting gates guard the same property from the other side:

- `no_hex_literals_outside_the_theme_map` (`orc-app/src/theme.rs:1268`) fails the
  build if any raw colour appears outside `theme.rs`. It recurses subdirectories
  — it did not always, and a planted violation one folder down passed until
  [#39](https://github.com/Legend101Zz/Agent-orchestra/issues/39) fixed it — and
  it works out for itself how many files it *should* be scanning, so it cannot
  quietly stop looking again.
- The exemption compares the **whole relative path**, not the file name, so a
  future `src/widgets/theme.rs` cannot escape the scan by being called
  `theme.rs`.

## The inherited fixture corpus

`rust/crates/orc-core/tests/fixtures/python-v3/` is an immutable oracle captured
from the **live Python implementation before it was deleted**. The Python code is
gone; its observable behaviour is not.

It seeds current, legacy, exact-usage, killed, orphaned, RPC `agent_end`,
session-linked, retry, handoff, corrupt, truncated, CJK, combining-mark and
wide-character records, with recorded exit codes for `list --json`, `show`,
`stats --json` and cached `quota --json`.

What the Rust tests compare is chosen carefully:

> Tests compare parsed JSON and exit structure, not timestamps, whitespace, or
> temporary paths. Unknown top-level and token fields are invariants and must
> survive every Rust read/update/write round trip.

That last sentence is the corpus's real job. It is the enforcement mechanism for
[additive JSON](data-model.md#additive-json-and-why-unknown-fields-survive): a
record written by the old Python tool, read by Rust and written back, must come
out with every field it went in with.

The corpus's own README cites the Python capture helper that produced it, which
was deleted along with the implementation it captured — so that citation no
longer resolves. It is correct and deliberate: a record of how the oracle was
made is not a claim that the tool still exists. See
[the archive's note on historical citations](../archive/README.md).

## Where the tests live

| Crate | Test files | Notes |
|---|---:|---|
| `orc-core` | 18 | The domain. Dispatch, progress, contracts, quota, probe, worktrees. |
| `orc-cli` | 14 | 65% of the crate. CLI↔MCP parity is asserted, not assumed. |
| `orc-app` | 1 | Plus extensive in-module tests and the 29 goldens. |
| `orc-mcp` | 1 | Real JSON-RPC over stdio against the release binary. |
| `orc-proto` | 1 | Wire-format invariants. |

`orc-cli`'s parity test asserts **set equality** against `Verb::ALL`, so an
eighth CLI verb without an eighth MCP tool fails the build. An earlier version
compared one direction only and would have passed.

## Known load-sensitive tests

Two families assert wall-clock bounds and fail under load **on `main` as well as
on any branch**:

- `a_real_dispatch_writes_delivery_then_completion_and_the_gap_is_the_worker`
  asserts a fixed 1.5 s bound — filed as
  [#60](https://github.com/Legend101Zz/Agent-orchestra/issues/60).
- the `orc-cli` quota family, same character.

The house rule when one fires: **re-run it in isolation, then A/B against
`origin/main` before saying a word about it.** The precedent is in the ship log
more than once, including a case where the A/B proved the failure *was* the
branch's fault after it looked exactly like the documented flake.

Measured for this branch (issue #14, docs and comments only): the test failed
2 of 4 full-workspace runs on the branch and 2 of 5 on `origin/main`, 8 of 8 in
isolation on the branch, and both trees reach 395 passed / 0 failed on a clean
run.

## What a reviewer is expected to do

From `docs/../WORKFLOW.md` and `ANTI-SLOP.md`, and it is enforced socially rather
than by tooling:

- Re-run every acceptance check yourself; never trust the implementer's pasted
  output.
- Try to make each one fail.
- Check `git diff main --stat` for anything outside the issue's allowed paths.
- A verdict that reads like a compliment instead of an attack report is grounds
  for re-running the review.

*"A wrong ACCEPT costs more than a wrong FIX."*
