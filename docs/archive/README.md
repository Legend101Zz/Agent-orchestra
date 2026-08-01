# Archive — working artifacts, kept for the evidence trail

**Nothing in this directory is maintained, and nothing in it should be read as
current.** It is a record of how pi-orchestra was built, kept because the
project's ship log, review threads and pull requests cite it constantly: almost
every claim in `LOG.md` names the note that proves it, and a citation you cannot
follow is a citation nobody follows.

If you are trying to understand what pi-orchestra *is*, you want
[`docs/architecture/`](../architecture/) and the [README](../../README.md).
Come here only when a ship-log entry, a PR body or a review comment sends you.

## Where things moved (issue #14, 2026-08-01)

These paths were cited from merged PR bodies and review threads that cannot be
edited, so if you arrived from one of those, this table is what you need:

| Cited as | Now at |
|---|---|
| `docs/notes/…` | [`docs/archive/notes/…`](notes/) |
| `docs/prompts/…` | [`docs/archive/prompts/…`](prompts/) |
| `docs/reviews/…` | [`docs/archive/reviews/…`](reviews/) |
| `docs/superpowers/…` | [`docs/archive/superpowers/…`](superpowers/) |
| `docs/guide.html` | [`docs/archive/guide.html`](guide.html) |
| `docs/*.gif` · `*.png` · `*.svg` | [`docs/archive/media/`](media/) |
| `docs/WORKFLOW.md` | [`WORKFLOW.md`](../../WORKFLOW.md) — still live |
| `docs/ANTI-SLOP.md` | [`ANTI-SLOP.md`](../../ANTI-SLOP.md) — still live |

The basenames are unchanged, so a path from an old citation still finds its file
by search (`git ls-files | grep <basename>`, or GitHub's `t` file finder).

**One thing did not move.** [`docs/design/visual-identity.md`](../design/visual-identity.md)
and its `visual-identity/` subtree are *live* — they are the source of truth for
the render layer: **seven Rust source files cite that path**, over eight lines
(`orc-app/src/theme.rs` carries two). They are not archived and must not be.

*A ninth mention exists and is deliberately not counted: `orc-pty/src/trigger.rs:52`
names the identity register in prose without citing the path, so a move cannot
break it. The number that matters here is path citations, not mentions.*

## What is in here

| Directory | What it is |
|---|---|
| `notes/` | Per-issue evidence notes. The measurements behind the ship log — latencies, mutation tables, soak results, the A/B runs that settled whether a flake was ours. |
| `superpowers/specs/` | Design specs, including the V1 product spec `2026-07-22-v1-universal-delegation-design.md` and the crate/prior-art decision record `2026-07-22-v1-crate-and-prior-art-decisions.md`. |
| `superpowers/plans/` | Implementation plans that preceded the specs. |
| `prompts/` | The session prompts handed to agents, one per phase or issue. Useful mainly as a record of what each session was actually asked to do. |
| `reviews/` | One 2026-07-11 review record from the v3 Rust rewrite. |
| `media/` | Screen recordings and screenshots captured as evidence, 2026-07-10 to 2026-07-13. |
| `guide.html` | A dated standalone guide from the v3 era. |

## Two caveats worth stating plainly

**The media here is stale by design.** Every recording in `media/` predates the
visual identity (#13), the STAGE circuit (#38), the seated conductor (#45) and
all three phases of watchable delegation (#49). They show a UI that no longer
exists. They are kept because the notes that cite them are evidence, not because
they show you anything current. Current recordings live in
[`docs/media/`](../media/).

**A few citations in here dangle, and that is correct.** Some of these documents
cite tooling that has since been deleted — the Python capture scripts, for
instance, removed with the Python implementation. Those are historical statements
about what existed when they were written, and rewriting them would falsify the
record. `tools/check-doc-links.sh` classifies them separately for exactly this
reason: it reports them and does not fail on them, while still requiring that
every citation *within* the archive resolves.
