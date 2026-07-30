# 🎼 pi-orchestra V1 — Mrigesh's log

*The one file the human reads. Status board + the exact prompt to run next +
plain-English ship log. Your responsibilities: [docs/ANTI-SLOP.md](docs/ANTI-SLOP.md).
Agents: update this file as instructed in AGENTS.md — status column and
ship-log entries are part of finishing an issue.*

**The loop:** pick issue → puppy builds (prompt 1) → Claude reviews (prompt 2)
→ puppy fixes (prompt 3) → Claude re-reviews (prompt 4) → you test + merge
(prompt 5). One issue at a time.

**Legend:** ⬜ not started · 🔨 being built · 👀 pushed, needs review · 🧪 reviewed, needs your local test · ✅ merged

## Status board

| Issue | In plain words | Status | Branch |
|---|---|---|---|
| [#16](https://github.com/Legend101Zz/Agent-orchestra/issues/16) | Research: pick the best Rust crates & steal the best prior art (Claude session, prompt 0) | ✅ | merged (PR #18) |
| [#17](https://github.com/Legend101Zz/Agent-orchestra/issues/17) | Rename the command `orc` → `pio` everywhere users see it | ✅ | merged (PR #19) |
| [#3](https://github.com/Legend101Zz/Agent-orchestra/issues/3) | Find every AI CLI installed on the machine and remember them | ✅ | merged (PR #20) |
| [#4](https://github.com/Legend101Zz/Agent-orchestra/issues/4) | Test what each installed CLI can actually do (`pio doctor`), never assume | ✅ | merged (PR #21) |
| [#5](https://github.com/Legend101Zz/Agent-orchestra/issues/5) | Every delegated task carries a "contract": what to do, where allowed, how we check it worked | ✅ | merged (PR #22) |
| [#9](https://github.com/Legend101Zz/Agent-orchestra/issues/9) | When you type `delegate:` / `orchestrate:` / `deliberate:` inside a pane, it lights up like ultrathink | ✅ | merged (PR #23) |
| [#13](https://github.com/Legend101Zz/Agent-orchestra/issues/13) | The new look: nocturne/ember/phosphor themes, glyphs, baton animation | ✅ | merged (PR #36) · 4 review findings carried to #37/#38/#39 |
| [#6](https://github.com/Legend101Zz/Agent-orchestra/issues/6) | Any capable CLI can be a worker, not just pi/Hermes | ✅ | merged (PR #24) |
| [#7](https://github.com/Legend101Zz/Agent-orchestra/issues/7) | Never spawn so many workers that a subscription gets rate-limited | ✅ | merged (PR #25) |
| [#8](https://github.com/Legend101Zz/Agent-orchestra/issues/8) | The 7 `orch_*` commands + MCP server so any brain can drive pi-orchestra | ✅ | merged (PR #26) |
| [#28](https://github.com/Legend101Zz/Agent-orchestra/issues/28) | Delegation silently timed out on any real task — the worker's output deadlocked the pipe | ✅ | merged (PR #29) |
| [#30](https://github.com/Legend101Zz/Agent-orchestra/issues/30) | Let the brain keep working while a worker runs, and hand it the answer not the transcript | ✅ | merged (PR #31) |
| [#11](https://github.com/Legend101Zz/Agent-orchestra/issues/11) | Each task runs in its own worktree, gets independently reviewed, produces a receipt | ✅ | merged (PR #32) |
| [#10](https://github.com/Legend101Zz/Agent-orchestra/issues/10) | Claude Code & Codex react to trigger words even outside pi-orchestra | ✅ | merged (PR #27) |
| [#12](https://github.com/Legend101Zz/Agent-orchestra/issues/12) | With only one CLI installed: still useful, honestly says so | ✅ | merged (PR #35) |
| [#37](https://github.com/Legend101Zz/Agent-orchestra/issues/37) | Make a theme choice stick, and stop `pio config set theme` writing the file nothing reads | ✅ | merged (PR #41) · reviewed 🔨→🧪, both fixes verified |
| [#38](https://github.com/Legend101Zz/Agent-orchestra/issues/38) | STAGE as a live circuit: connect the brain to *n* workers, smooth mouse-resize, show messages moving | ✅ | merged (PR #42) · review FIX fixed in `d755714` before merge |
| [#39](https://github.com/Legend101Zz/Agent-orchestra/issues/39) | Leftovers from the new look: honour NO_COLOR for the rainbow, make the no-hex test look in subfolders | ✅ | merged (PR #47) · review FIX (2) fixed in `768fadc` before merge |
| [#45](https://github.com/Legend101Zz/Agent-orchestra/issues/45) | When you `delegate:` inside the TUI it must use the workers already on screen — and you must see it happen | 🔨 | `issue-45-seated-conductor` *(merges #43 + #44)* |
| [#14](https://github.com/Legend101Zz/Agent-orchestra/issues/14) | New README + screenshots for launch | ⬜ *last* | — |
| [#33](https://github.com/Legend101Zz/Agent-orchestra/issues/33) | Any known harness (like opencode) becomes usable automatically; register new model profiles of pi | ✅ | merged (PR #34) |

**#45 pushed (2026-07-30, `issue-45-seated-conductor`) — the `delegate:` you
type in the TUI now goes to the workers you can see.**

Here is what was happening. You pressed `n`, got a brain and two workers in
three panes, told the brain `delegate:` — and it quietly built a *second*,
invisible set of workers somewhere off-screen and gave the job to those. The
job got done, correctly, in about six seconds. You just never saw any of it.
The three panes you were looking at sat there doing nothing the whole time,
and the brain, which had no way to collect the answer either, reported back
that nothing had happened.

The cause was one instruction. Everything a brain knows about how to delegate
comes from a block of text pi-orchestra injects the moment you type the word,
and that text opened with "create a session first". Correct advice when you
are using Claude Code on its own. Inside the TUI it is the bug — you are
*already* in a session, and making a new one is precisely what sends the work
somewhere you cannot watch. The brain had no way to know better: the pane it
lives in has always carried its session and its neighbours in its environment,
and nothing had ever read them.

So now it reads them, and leads with where it is: this is your session, this
is your pane, these are the workers sitting with you, reuse them, and do not
create anything. Two of the skill files were making the same mistake on their
own — one of them actively threw away the session id it had been handed — so
those are fixed too.

**A second bug was hiding behind the first, and would have survived fixing
it.** STAGE deliberately ignores a task the first time it sees it, so that
attaching to an old session does not replay every dispatch it ever made. But a
delegation is created, sent and confirmed in one go — faster than the screen
refreshes — so STAGE's *first* look at a brand-new task was already the
finished article, and it ignored the whole thing. Zero animation, not a
partial one. Every test passed because every test hand-built its board one
step at a time. Fixing only the instruction would have got the work to the
right pane and still shown you nothing.

**Three things you can now check yourself.** `pio doctor` has a new section
that says whether `delegate:` actually works on this machine — the honest
answer on the machine that reported this was "it never has", because the hook
was never registered, and nothing anywhere said so. It exits non-zero while
the grammar is inert, and it can tell "the file is missing" from "the link
points at a checkout you moved", which is a real thing that has already
happened here. `./install.sh --wire-claude-hook` will now do the registering
for you if you want it — backed up first, and running it twice leaves the file
byte-for-byte identical. And the installer no longer ends on "done." while
the headline gesture is dead: it finishes with a line per harness saying which
are wired and which are not.

On that last point, one deliberate non-delivery: **Pi and OpenCode are
reported as not wired rather than wired.** Neither has a skills directory on
this machine to look at, and guessing where to write a file would be claiming
something I had not checked — which is the one thing AGENTS.md tells me not to
do. The issue explicitly allows saying so instead. It wants its own issue.

Also fixed: `--json` now always answers in JSON, including when it fails (it
used to print the error on the side channel and leave stdout empty, so a
failure and a silence looked identical); errors now name the command that
lists valid task ids, instead of leaving the brain to guess `T-hello` then
`T0002` as it did; and every instruction that starts a worker now says in the
same breath how to collect the answer.

**What I could not run: the live TUI recording.** A session with real panes
can only be made by pressing `n` in the TUI, so the "watch the packet cross"
demo is yours to do at test time. Everything underneath it is proven against
the real code rather than a stand-in — no second session, the right seated
worker chosen without being told which, the board pointing at that pane, and
both animation events derived from a real dispatch. Five gates green, 318
tests. Evidence: `docs/notes/2026-07-30-issue-45-seated-conductor.md`.

> **Reviewed 2026-07-31 — FIX (👀→🔨).** Headline verified independently through
> the real `orch::delegate`: no second session, the only harness-matching seated
> pane chosen without `--pane`, board and both animation events landing on it;
> five gates green on my own run (319 tests, 0 failed), and checks 2–7 and 9 all
> reproduced, including mutating a skill description to prove the drift gate
> bites. Two blockers: `skills/orchestrate/SKILL.md` carries the contracted
> `--objective/--check` recipe with **no** git-worktree precondition, so check 8's
> "everywhere the recipe appears" is unmet; and `trigger_wired` is one global
> `~/.claude` probe applied to every brain pane regardless of harness, so a
> Pi/OpenCode brain shows a **live** `DELEGATE` badge for a grammar `install.sh`
> reports as not wired three lines earlier — check 11 inverted.

**#38 pushed (2026-07-30, `issue-38-stage-circuit`) — the line between the
brain and the workers is now real wiring, and dragging a pane stopped
fighting you.**

First, the thing you noticed: **the moving packet was freezing mid-way and
then restarting from the beginning.** That was #13's fourth review finding,
which shipped unfixed. The animation maths was always right — the app just
stopped repainting at the exact moment the line should have faded back to
its resting dots, so it kept whatever half-finished frame was on screen until
the next burst of output. It now always paints the frame it worked out.

Second, **one line became one line per worker.** Before, there was a single
horizontal dash floating at the middle of the screen no matter how many
workers you had — with three, it pointed at the gap between two of them, and
it couldn't tell you *which* worker was talking, because any pane producing
anything lit the same line. Now it looks like an n8n canvas: a socket on the
conductor, a spine running down the gap, and a branch into each worker. Each
branch carries its own worker's traffic, so you can see at a glance who is
busy. It also follows the panes when you move them — the old line was drawn
from a formula, so after one drag it pointed at empty space.

Third, **dragging.** You couldn't actually resize with the mouse at all
before — only move a pane by its title bar, which is probably why it felt
broken. You can now grab any edge or corner. And the reason it hitched: every
single frame of a drag, the app stopped to ask the daemon to resize the
terminal (which makes the CLI inside redraw its entire screen) *and* rewrote
`session.json` to disk. Sixty times a second, with the interface frozen
waiting each time. Measured: **119 of those round-trips across one second of
dragging. It is now 2, both after you let go.** The outline follows your
mouse instantly because that is just the app redrawing itself.

Fourth, **you can now see a message move.** Previously a dispatch going out, a
result coming back, and a worker simply printing a line all looked identical
— the code literally treated a task event as a stdout tick. Now a message is
its own thing: a single arrow that crosses once and lands, `▶` going out and
`◀` coming back, colour-coded gold for confirmed and coral for failed, and
noticeably snappier than the ambient flow. When it arrives, the receiving
pane stamps a short-lived badge — `TASK DISPATCHED`, `TASK CONFIRMED`,
`TASK FAILED` — which fades after about a second and leaves nothing behind.

**One thing needed your decision and you gave it.** The design sheet says the
baton goes one direction only, conductor → worker. A result coming *back*
contradicts that. Rather than let the code quietly overrule the sheet, you
chose to keep the baton rule exactly as written and add a second, separate
thing beside it — the ambient line means "this worker is producing", the
arrow means "this specific thing was sent". A nice side effect: the identity
HTML needed no changes at all.

**At small terminal sizes it now tells you the truth.** Below about 100
columns the panes stack and there is no gap to route through — the old code
just stopped drawing the line, silently, at sizes *above* the design's own
80×24 minimum. Each worker now gets a short rail inlaid in its own top border
instead. You lose the routing, not the information, and it says so.

Also worth knowing: two more real bugs turned up while doing this. Letting go
of the mouse over a pane's title bar used to *start* a new drag instead of
ending one. And a pane you moved was often never actually saved, so it would
jump back. Both fixed.

Six workers all producing at once repaints in **0.157 ms** — the budget is 16.

*Not yet tested locally by you: `./install.sh` from the branch and drive it.*

> **Review 2026-07-30 — FIX (1 item), and it's a test not a behaviour.** All five
> gates reproduce green and everything I attacked held: `baton.rs` really is
> byte-identical, the one-worker rail is unchanged to the character, the drag test
> genuinely counts socket traffic (removing the guard fails it), and the perf doc
> honestly reports debug *and* release rather than the flattering number. The one
> gap: `a_dragged_pane_clips_the_wiring…` never overlaps anything — its fixture
> aims at the spur's end column, not the trunk — so deleting the whole loom clip
> leaves all 97 tests green. The clipping code is correct; I proved it with a
> fixture that really overlaps (30 wire cells painted through a pane once the clip
> is off). Swap the fixture, mutation-check it, and this is an ACCEPT.
>
> **Fix + merge (2026-07-30, `d755714`, then PR #42 merged as `1406840`) — 🧪→✅.**
> All three items closed, and closed better than asked. The clipping test is
> rebuilt on the case that actually happens — one worker dragged across another
> worker's spur row — and now carries a *positive* assertion that the fixture
> really covers a wire, so it can't quietly go vacuous again; deleting the clip
> now fails with `wire cell (70, 18) is painted inside pane 3`. `Routing` is read
> at last: at widths where the router gives up, STAGE's legend leads with
> `connectors inlaid — too narrow to route`, which is what AC8's "stated fallback"
> was actually asking for. And the router now runs once per frame instead of four
> times — with the honest footnote that it produced **no measurable win**, plus a
> correction I'd have missed: my run and theirs disagreed on the repaint cost by
> 1.8×, so the committed decimal was replaced with the claim that survives
> re-measurement — two orders of magnitude of headroom under the 16 ms budget, in
> the profile users actually run.

**#37 pushed (2026-07-29, `issue-37-theme-persistence`) — you can change the
theme from inside the app, and it stays changed.** Press your leader chord then
`t` — `ctrl-g t` by default — on any of the four screens, and the whole app
moves to the next theme together: nocturne → ember → phosphor → back. That was
the missing half. The other half is that it *sticks*: the client asks the
daemon to write the choice down, so the next launch opens the same way. The
client still never touches `~/.orchestra` itself — that rule is the reason this
needed a new protocol message rather than a one-line file write.

It also fixes the thing that made `pio config set theme nocturne` feel broken:
the theme was written in two different files, and the command wrote the one the
app doesn't read. `harnesses.json` is now the record that counts, `config.json`
keeps a copy that is refreshed from it, and every way of asking — the command,
the file, the screen — answers the same thing. Pressing `t` on RUNS no longer
prints `error: unrecognized subcommand 'config'` at you either; that was the
ledger trying to shell out to a binary that has no such command.

Two smaller things came with it. The `?` help now tells you how to change the
theme instead of pointing you at a JSON file. And the standalone `pio runs`
ledger, which only ever knew about two themes, now knows nocturne is a theme at
all — asking it for nocturne used to silently hand back ember.

What this does **not** do: no new themes (still the three from the design
sheet), no per-session or per-pane override, and nothing about how the app
looks — that was #13. #38 (STAGE as a live circuit) is untouched and still
next.

> **Review (Claude, PR #41 @ `5c0d1a0`): 🔨 FIX — 2 items.** All five gates green on my own run (**267 tests, 0 failed**) and all seven ACs re-verified against a live `piod` on a scratch `ORC_HOME`, not from the pasted evidence — including the ones I most wanted to break: `set_theme` sent without a hello is refused and mutates nothing, an unknown variant leaves the connection alive, `pio harness list` does not clobber `app.theme`, and a hand-edited registry beats a stale `config.json`. Resolution happens on write, so no durable record holds an unrenderable name. No new deps, no scope creep; the `install.sh` deviation is one line and AC3 fails without it — authorized. **The defect is in the tests, not the code:** gutting `cycle_theme` so it never calls `commands.set_theme` — the single line that turns `<leader> t` into persistence — still passes **267/267**, the exact number cited as evidence. Every orc-app test passes `commands: None`, so the client→daemon seam is never executed; the mutation table brackets this gap (M7 reads, M14 receives) without covering the send. `scripted_daemon` is already in the same file and is all it needs. Second item: **PR #41 does not merge** (`CONFLICTING`, `progress.md` vs PR #40) — already fixed locally as `ad96e41`, just unpushed. Separately: `harness_list_is_additive_and_preserves_unknown_fields` flakes on the SSD (2/5 here) but flakes on `origin/main` too under an interleaved A/B — a pre-existing 2s `VERSION_PROBE_TIMEOUT`, not counted against this branch.
>
> **Fix 1 in (2026-07-29, code-puppy).** Reproduced the reviewer's mutation first: gutting `cycle_theme`'s daemon round trip really did leave the suite at 267/267, so `<leader> t` recoloured every screen while silently no longer surviving a relaunch. The seam now has three tests on the `scripted_daemon` helper, driving the real key-press path against a live socket: one asserts the **bytes on the wire** are `{"type":"set_theme","theme":"ember"}`, one asserts a refused save keeps the switch on all three copies and puts `theme not saved: …` on the message line (that branch was unexecuted too), and one covers the reviewer's non-blocking note — `ThemeSet` promised to carry the name the daemon wrote and the client discarded it, so the client now adopts it and cannot render a palette the record disagrees with. Mutation-tested four ways: the reviewer's exact no-op fails all three, a wrong theme name fails two, a swallowed error fails one, an ignored reply fails one. **270 passed, 0 failed.** Fix 2 was already discharged by the pushed merge. Not changed, with reasoning: `route_leader` ahead of the `raw_input_view` guard — STAGE's `RawRouter` already arms the leader while forwarding every other byte to its pane, so intercepting the chord inside a text input is the consistent behaviour, and it keeps `<leader> q` reachable from the launch flow.
>
> **Re-review (Claude, PR #41 @ `8a43604`): 🧪 ACCEPT.** Both fixes verified by me, not by claim. **Fix 1:** re-applied my exact mutation — `cycle_theme`'s round trip replaced with a no-op — and it now fails all three new tests; re-ran the other three from the fix round's table and each is caught by the test that owns it (wrong name → 2, swallowed error → 1, ignored reply → 1). The wire assertion reads the literal bytes off the socket, which is what kills the no-op. The non-blocking `ThemeSet` note was closed the right way round: the client adopts the name the daemon says it wrote instead of the doc being softened. **Fix 2:** `MERGEABLE`/`CLEAN`, confirmed independently of GitHub with `git merge-tree --write-tree` (exit 0, zero conflicts). All five gates green on my own run — **270 passed, 0 failed**; the first gate-3 run hit the documented `background_dispatch` sub-1s flake, which then went 5/5 in isolation and 270/0 on a full re-run. No regressions, no creep: four files changed since the reviewed commit, still 16 branch-wide, still no `Cargo.toml`/`Cargo.lock`. **One thing left open on purpose:** deleting `handle_raw_event`'s `route_leader` call — which would kill every `<leader>` command on HOME/SCORE/RUNS — still passes 69/69, as does dropping the STAGE theme dispatch. `git log -S` puts both lines in `2112865`, so it predates this fix round and I missed it last time; it sits at the testing altitude the whole codebase uses. Filed as a follow-up rather than a third round (ANTI-SLOP rule 4). Ready for your local test + merge.

**#13 pushed (2026-07-29, `issue-13-visual-identity-v1`) — pi-orchestra has a
real look now, and it survives being stripped of colour.** The whole app draws
from one table of 17 named colours — "the conductor's accent", "a confirmed
task", "an unavailable harness" — so the three themes (nocturne, the new
default, plus ember and phosphor) are now genuinely one switch, and a test
fails the build if anyone writes a raw colour anywhere else. Every state also
got its own little symbol (✓ confirmed, ◔ queued, ✕ failed, ⏻ conductor down,
● / ○ for whether a CLI is installed), so on a black-and-white terminal, or one
where the user has turned colour off, you can still read the screen — there are
18 committed screenshots-as-text proving it, and they compare the *colours* too,
not just the words. The baton — the little filament between the conductor and
its workers — now pulses exactly the way the design says: a three-cell packet
sweeping left to right while output flows, going quiet 0.4 s after it stops, and
freezing to a plain solid line for anyone who has asked for reduced motion.

What this does **not** do: no new screens, no new keys, nothing about what
pi-orchestra can *do* changed. It is purely how it looks and how honestly it
degrades. It also doesn't touch the RUNS ledger's own code — that screen just
borrows the new palette so it stops looking like a different app.

What it unblocks: #14, the README and launch screenshots. There is now something
worth photographing, and the three themes are stable enough to photograph.

> **Review 2026-07-29 — FIX (3 items).** All five gates pass and the map, glyphs
> and baton are faithful to the sheet, but colour still escapes it in three
> places: pressing `t` on RUNS swaps to a hard-coded palette, saves it, and makes
> nocturne unreachable; the trigger rainbow ignores `NO_COLOR`; and the grep gate
> doesn't look inside subdirectories.
>
> **Addendum — one of those three is your call, Mrigesh.** The rainbow's explicit
> RGB is your own approved exception from #9 (see 2026-07-24 entry below), so that
> part is settled. What's new is that #13 added the colour-tier probe and the claim
> that monochrome "drops colour entirely" — and the rainbow ignores it, so a
> `NO_COLOR` terminal still gets nine 24-bit colour codes. Decide whether the
> ultrathink exception extends to `NO_COLOR` (then soften the claim) or the rainbow
> goes bold-only there (then gate it). Gating breaks no existing test — checked.
>
> **Correction — finding 1 is smaller than I first wrote it.** Pressing `t` does
> NOT persist a theme: the shell-out runs `pi-orchestra config …`, which has no
> `config` subcommand, so it just errors into the RUNS message line. RUNS still
> visibly leaves the palette while the other screens don't, so it stays on the fix
> list — but it's not a config trapdoor. **Also: your `harnesses.json` says
> `ember`, and the branch rightly never rewrites an existing config — so you will
> NOT see nocturne on this install unless you edit that file or test with a fresh
> `ORC_HOME`.** Separately, `pio config set theme` writes `config.json` while the
> client reads `harnesses.json` — pre-existing, needs its own issue.
>
> **Fix 1 revised (your call, after testing locally).** Editing JSON isn't a UI, and
> right now there is no working way to change theme from inside the app at all. So
> instead of just stopping `t` from misbehaving on RUNS, item 1 becomes
> **`ctrl-b t` cycles nocturne → ember → phosphor on every screen** — same defect
> closed, but you get the feature. Session-only: the client is forbidden from
> writing config, so it reverts to your configured default on relaunch. Persisting
> it, plus collapsing the two `theme` files, is now **#37**.
>
> **Fix 4 added — you were right about the jerky line.** The baton's frame maths is
> correct; the repaint loop is not. When output stops, nothing schedules the final
> redraw, so the rail is left frozen mid-packet instead of decaying to dots — then
> the next burst restarts it at frame 0. Sweep, freeze, jump back, sweep. On a
> bursty producer (every agent CLI) that's the normal case. Fix is one more repaint
> on the live→idle edge.
>
> The bigger things you asked for — one connector per worker instead of a single
> decorative line, drag-resize that doesn't hitch, an animation when a message is
> actually sent or returned, and the little landing emote — are **#38**. Kept out
> of #13 deliberately: they're a feature epic, and folding them in would make this
> branch unmergeable. Say the word if you'd rather have it all in one.
>
> **MERGED 2026-07-29 (PR #36, `8b47bf1`) with all four findings still open.**
> Your call — the look is in and #14 is unblocked. But nothing about the findings
> changed by merging, so they're now live on `main` and each has been given a home
> rather than left to rot:
> - **#37** takes finding 1 — `t` on RUNS still jumps to a hard-coded palette and
>   errors in the message line. That issue now delivers the `ctrl-b t` switcher
>   *and* persistence together.
> - **#38** takes finding 4 — the frozen baton. It should be that issue's first
>   commit; there's no sense layering more motion on a loop that won't paint the
>   frame it already computes.
> - **#39** takes findings 2 and 3 — the `NO_COLOR` rainbow decision (still yours)
>   and the no-hex test that doesn't look in subfolders.
>
> Also unclosed, and not in any issue yet: the *screen layouts* are thinner than
> the mockups in the identity HTML. The palette, glyphs and baton match it exactly,
> but `_fillScreens` draws HOME as three session cards side by side with health
> badges and `▁▂▃▅▇` sparklines, plus a two-column bench grid showing each
> harness's PATH and a `4 / 6 on PATH` counter. What shipped is a flat list and a
> single-column bench. Worth an issue before #14, since #14 is the screenshots.

**#30 merged (2026-07-28, PR #31) — delegation is now what it was always meant
to be.** `pio orch delegate` returns the moment the worker has its brief instead
of blocking for the whole run, so the expensive brain stays free; `orch_status`
polls and `orch_await` blocks for the answer. The concurrency cap from #7
survives detachment — the durable slot lease is handed to a detached supervisor
that holds it for the worker's *real* lifetime, so `max_parallel_workers` finally
means something and the bench can actually run more than one worker. A worker
whose supervisor dies is reconciled to an honest `orphaned` state, its process
killed and its slot freed, rather than wedging the board. And the record now
stores the worker's *answer* plus token usage instead of a JSON firehose.
Together with #28 (the pipe-buffer deadlock) this closes out a run of three
defects that had made delegation useless for real work since #8 landed.

**#11 merged (2026-07-28, PR #32) — every contracted task is isolated,
independently reviewed, and produces a final receipt.** Contracted tasks now
run in their own Git worktree so a worker cannot touch the main checkout;
`orch review` dispatches to a different capable harness when one exists, or
labels the result an honest `self_review` with just one; `orch finish` refuses
to move a task to `done` while any acceptance check is verdicted `fail`. Along
the way, issue #33 (Claude, merged 2026-07-28 as PR #34) fixed a gap found
while testing #11 locally: `pio harness list` discovered `opencode` but never
made it usable, and `pi-m3` was a single hardcoded model profile with no way
to register another — both now work (`discover()` auto-registers any
known-adapter harness; `pio harness add` registers new named model profiles
with validated or manual input).

**#12 merged (2026-07-28, PR #35) — with one CLI installed, pi-orchestra is
still useful and says so honestly.** When exactly one adapter family is
capable, launch prints the mandated sentence verbatim — *"One capable harness
detected. Parallel cross-harness deliberation is unavailable. Running a
sequential plan with self-review."* — and then still delivers the whole
pipeline: durable session, isolated worktree, bounded dispatch, sequential
implementer→reviewer, acceptance evidence, final receipt. The key honesty call
is that diversity is counted by *adapter family*, not registry key, so two
model profiles of the same CLI can alternate implementer/reviewer roles but the
report still says `self_review` — pi-orchestra never manufactures independence
it doesn't have. HOME copy switches to sequential language in this mode too.
Took two review rounds: the first found that the new test fixture raced itself
(making `cargo test --workspace` fail under load) and that the mandated
sentence was only compared against its own constant — so it could have been
edited to advertise parallel deliberation with the whole suite still green.
Both fixed and re-verified by mutation.

**Next: #13 and #14 — that's all of V1.** Still open by choice:
`render_brief` is wired into `orch delegate` but not `dispatch send`. Start #13
before more TUI churn lands to avoid merge pain. Known small debts left by #12
(none blocking): the `?` help screen isn't single-harness aware, HOME and
`session create` can disagree on whether the mode is active because one probes
without a cwd, and `background_dispatch.rs`'s sub-1s timing assertion is
hardware-dependent — it fails on external-SSD checkouts and passes on internal
disks.

## Prompts you run

### 0. Foundations research (Claude Code session, once, no code)

```
Work GitHub issue #16 of Legend101Zz/Agent-orchestra (clone or use
the repo checkout, branch issue-16-research). Read the issue and the V1 spec
it links, then research each listed area with web search + GitHub: pick the
best Rust crate/pattern per area, with version, license, maintenance signal,
and 2+ rejected alternatives each. Mine prior art (claude-squad, opencode,
vibe-kanban, hermes-agent, togethercomputer/moa) for how they drive coding
CLIs headlessly. Write the decision record the issue names, comment the
binding decisions on issues #3-#8 and #11, update LOG.md (ship-log entry +
status), push the branch, and stop — no code.
```

### 1. Build an issue (code-puppy, one terminal per issue)

```bash
export GH_TOKEN=<paste fresh token>
git clone https://github.com/Legend101Zz/Agent-orchestra.git puppy-issue-<N> && cd puppy-issue-<N>
code-puppy -i
```
then inside code-puppy:
```
/work-issue <N>
```
(If `/work-issue` isn't picked up, paste: *"Read AGENTS.md and .agents/commands/work-issue.md, then execute that command for issue #<N>."*)

### 2. First review of a pushed branch (Claude Code, one session per issue)

```
You are the adversarial reviewer for pi-orchestra (run from the repo
checkout — see docs/WORKFLOW.md for where it lives), per
docs/WORKFLOW.md. Review branch issue-<N>-* against the task contract in
GitHub issue #<N>:
1. git fetch, check out the branch, run all five gates from AGENTS.md.
2. For EVERY acceptance check, run it yourself and try to make it fail —
   do not trust the implementer's pasted output.
3. Check the diff (git diff main --stat): flag anything outside the issue's
   allowed paths, unrequested features, new dependencies, dead code.
4. Verdict: ACCEPT or FIX with a numbered fix list. Comment it on issue #<N>,
   append a one-line verdict under the ship-log entry in LOG.md, and set the
   status to 🧪 (accept) or back to 🔨 (fix). Push that LOG.md change to the
   same branch.
Be brutal. A wrong ACCEPT costs more than a wrong FIX.
```

### 3. Fix round (code-puppy, SAME clone/session as the build if possible)

```
Reviewer feedback is on GitHub issue #<N> (the numbered FIX list). Address
every numbered item on the existing issue-<N> branch — no new features, no
scope changes. Re-run all gates from AGENTS.md, push, and comment on the
issue with per-item evidence of the fix. Update your LOG.md ship-log entry
if what-shipped changed.
```

### 4. Re-review (Claude Code — reuse the SAME review session if it's still open)

```
Re-review branch issue-<N>-* of this repo: verify ONLY the numbered
fix list from your previous review comment on issue #<N>, re-run the gates,
and confirm nothing new broke or crept in (git diff against the previously
reviewed commit). Verdict ACCEPT or FIX on the issue; update LOG.md status.
If this is already the second fix round and it still fails: STOP and
recommend re-scoping the issue instead (docs/ANTI-SLOP.md rule 4).
```

### 5. After YOUR local test passes → merge

```bash
cd "$PIO_REPO" && git fetch origin   # see docs/WORKFLOW.md for the checkout path
git checkout issue-<N>-<slug> && ./install.sh   # try the feature yourself
git checkout main && git merge --no-ff issue-<N>-<slug> && git push
```
Then tick the box on epic [#15](https://github.com/Legend101Zz/Agent-orchestra/issues/15), set ✅ here, close the issue.

## Ship log (newest on top — plain English, no jargon)

*code-puppy: after pushing a branch, add an entry here (in the same branch):
2-4 sentences — what can pi-orchestra do now that it couldn't before, what
you did NOT do, and what this unblocks. Claude reviewers append a one-line
verdict under the entry.*

### 2026-07-30 — The rainbow now respects a colourless terminal, and the no-hex test looks everywhere, issue #39 (code-puppy)
Two leftovers from the new look, both about the code meaning what it says. First:
when you run pi-orchestra with `NO_COLOR` set, or in a terminal that has no
colour, a `delegate:` you type used to still come out in nine full-colour
letters — while the code claimed in writing that this mode "drops colour
entirely". It now really does: the word stays **bold** and the `◆ DELEGATE`
badge still names it in the pane's title, which is what makes it readable
without colour. Terminals in between are no longer sent colour codes they can't
display either — a 256-colour terminal gets the closest match from the 256 it
has, a 16-colour terminal the closest from its 16, and a full-colour terminal
looks **exactly** as it did before (proof: every existing saved screenshot in
the test suite still matches character for character). Second: the test that forbids hard-coded
colours anywhere outside the one colour file was only looking at the top folder,
so a file one folder down could break the rule and the test would still pass —
demonstrated by planting one, watching it pass, then fixing the test and
watching it fail. It now looks everywhere, and it works out for itself how many
files it *should* be looking at, so it can't quietly stop looking again.

What this did NOT do: the actual look is unchanged for anyone on a normal
colour terminal, no new screens or features, and one thing was deliberately left
alone — if a CLI running *inside* a pane prints its own colours, those still
come through even with `NO_COLOR` set. That's a separate decision and it's
written down in the notes rather than changed quietly. This clears the last of
#13's four review findings, so the new look is fully closed out — what's left
before launch is #45 (making `delegate:` inside the TUI use the workers already
on screen) and then the README and screenshots (#14).

> **Review 2026-07-30 (Claude): 🔨 FIX (2).** All 5 gates re-run green, all 6 acceptance checks re-verified independently and all 4 mutation claims reproduced — but `Theme::resolve`'s new "every colour the crate emits comes through here" and the module doc's new "without exception" are both false while `pane_color` replays a pane's SGR at the monochrome tier, which is the same claim-vs-behaviour gap AC1 forbids; and the gate's `theme.rs` exemption is keyed on the file *name*, so now that the walk recurses `src/<anydir>/theme.rs` escapes the colour scan and a future `src/theme/` split would make the gate fire on its own map.

> **Fix round (2026-07-30, code-puppy) — all 4 accepted, none argued.** Both blocking findings were things this branch *introduced*: the doc claims are now scoped to what the theme map emits, with the caveat stated on `pane_color` itself (behaviour unchanged — what it should do under `NO_COLOR` is its own decision), and the exemption compares the whole path, with the scan split out so it is tested against a synthetic tree containing the `widgets/theme.rs` the real crate must not have. The non-blocking correction was right and is corrected in writing: the two floor assertions did *not* fail independently, so the redundant one is replaced by a count that catches the over-broad exemption instead. Gates green, 99 → 101 tests, no golden moved.

> **Merged 2026-07-30 (PR #47 as `1be106e`) — 👀→✅. The fix round was verified *after* the merge, not before.** Mrigesh merged without waiting on the re-review, so the re-review was run against `main` instead — the verdict would have been ACCEPT. All five gates green on my own run of the merged tree (`orc-app` lib **101 passed, 0 failed**, workspace clean, release `--locked` build fine), and both blocking findings really are closed in `768fadc`: the module doc and `Theme::resolve` now claim only what the *theme map* emits and name `pane_color` as the one colour outside it, and the gate's exemption compares the whole relative path against `THEME_MAP`. Mutation-checked the second rather than trusting it — putting `file_name() == "theme.rs"` back leaves `widgets/theme.rs`'s `Color::Indexed(199)` unreported and fails exactly one test, the new synthetic-tree gate. **All four of #13's review findings are now discharged** (#37, #38, #39), so the new look is closed out; what's left before launch is #45, then #14.

### 2026-07-28 — One capable harness still completes the job honestly, issue #12 (code-puppy)
With only one capable CLI, pi-orchestra now says exactly what is unavailable,
then runs the implementer and reviewer roles in sequence with the same durable
worktree, evidence, retries, and final receipt. If that CLI has more than one
registered model or explicitly labeled account, review may use a different
profile but still says `self_review`; duplicate labels never manufacture
diversity. I did NOT build parallel deliberation or guess account-selection
flags; this completes the honest fallback and unblocks the V1 launch README in
issue #14.

**Review (Claude, PR #35 @ `63842ac`): 🔨 FIX** — production code is correct and byte-exact, but the new `single_harness.rs` fixture races itself on a shared temp path so `cargo test --workspace` fails (4/15 under load), and the mandated sentence can be inverted to claim parallel deliberation with the whole suite still green.

**Fix (code-puppy, PR #35): 👀 RE-REVIEW** — fixture roots now have distinct test labels plus an atomic sequence, and the acceptance test owns the full mandated sentence as a literal. The target passed 15/15 complete runs under CPU load, and the reviewer's `unavailable` → `available` mutation now fails immediately.

**Re-review (Claude, PR #35 @ `a66c14a`): 🧪 ACCEPT** — reproduced both fixes myself: 20/20 clean under heavier CPU load than the 4/15 failure case, and both mutation directions (drifting the constant, and bypassing it in the CLI) now fail immediately. All five gates green, 223 tests, no production code changed this round.

### 2026-07-28 — Any known harness works out of the box; register new pi model profiles, issue #33 (Claude)
`opencode` was fully wired to run (it has an invocation template, `spawn_guard`
already knew its concurrency cap) but was unreachable — `session create
--worker opencode` failed even right after `harness list` reported it
available, because discovery only recorded presence, never made it a usable
profile. Any `KNOWN_HARNESSES` name with a working invocation template now
becomes usable automatically the moment it's found on `PATH`. Separately,
`pi-m3` was a single hardcoded model of the generic `pi` binary with no way to
register another; `pio harness add <key> --like <existing> --provider --model`
now does that, auto-probing the harness's own model list (`pi --list-models`,
`opencode models`) when possible and rejecting bad pairs with the real valid
choices, falling back to trusting manual input when the probe itself fails. I
did NOT add a way to edit/remove profiles, change role or capability-probe
semantics, or build model-flag support for any adapter besides `pi`.

### 2026-07-28 — Isolated work, independent review, and final receipts, issue #11 (code-puppy)
Contracted tasks now run in their own Git worktrees, so a worker cannot change
the main checkout, and completion requires a per-check review whose result,
usage, cost, and run receipts are saved and visible in task details, SCORE, and
RUNS. When another capable worker exists pi-orchestra chooses it as reviewer;
with only one it labels the result honestly as self-review. I did NOT build DAG
replanning or silently merge or delete unmerged work; this unblocks adversarial
review, local acceptance testing, and the single-worker fallback in issue #12.
**Review (Claude, 2026-07-28): 🔨 FIX — worktree isolation, verdict-strictness,
and self-review fallback all independently re-verified correct (incl. an
adversarial fail-verdict lifecycle test proving `finish` really blocks on a
failed check); required before ACCEPT: commit that fail-verdict test into the
suite, since nothing currently in the PR exercises it.**
**Re-review (Claude, 2026-07-28): 🧪 ACCEPT — the fail-verdict regression test
landed; mutation-tested it by disabling the completion guard and confirming
the test catches it, then re-ran all five gates clean. Ready to test locally
and merge.**

### 2026-07-27 — The brain can keep working while workers run, issue #30 (code-puppy)
Handing off a task now returns as soon as the worker has received it, so the
brain can start another job, check progress, or wait later for the finished
answer instead of sitting idle. pi-orchestra keeps the worker limits in force
until each real process exits, saves a readable answer and usage instead of the
worker's machine transcript, and recovers honestly if the background helper is
killed. I did NOT build worktree isolation, independent review, or the final
receipt; those remain issue #11, which this now unblocks.

> **Review verdict (2026-07-27, Claude): 🔨 FIX — one item.** The strongest implementation in this repo so far: all 5 gates green, and I mutation-tested rather than trusted the evidence — dropping the lease instead of transferring it kills all 4 background tests, releasing the slot at worker *start* instead of *exit* kills the cap-1 test, and skipping the orphan kill trips `worker survived supervisor reconciliation`. AC2/AC3 prove ordering and overlap from a real process event log, not assertions. Live: `delegate` 24s → **0.05s**, `await` genuinely blocks 7.56s, and a real delegation now stores **55 bytes of readable answer + usage** instead of 16KB of JSON. #28's flood tests and #7's quota tests were adapted honestly, not weakened. **The one defect:** `orch_await`/`orch_status` return `note: None` for a *failed execution* — fine on the CLI (exit 124), but MCP has no exit codes, so a conductor sees a clean-looking success. That is precisely the #8 bug, on the new surface this PR creates; its own test comment names the lesson but applies it only to failed delivery. One small fix from ACCEPT.
>
> **Fix applied (2026-07-27, code-puppy):** `orch_await` and `orch_status` now elevate the newest terminal execution failure into the same top-level note, naming the execution state, failure kind, exit code, detail, task, and next action; successful terminal executions remain quiet. A real MCP stdio regression fixture confirms delivery, hits the one-second worker bound, and proves both tools return the same timeout/exit-124 note. All five gates are green.
>
> **Re-review (2026-07-27, Claude): 🧪 ACCEPT.** Fix verified live on both surfaces (`await` and `status` return the same note naming failure kind, exit 124, task and next action) and mutation-tested — restoring `note: None` fails the new MCP stdio test and only that test. Checked the harder half too, that it does not over-fire: a successful execution, a genuine legacy record with no `execution_status` key, and a stale failure followed by a successful retry are all silent; `list_dispatches` really is newest-first (`updated_at` descending), so the retry cannot be shadowed. All 5 gates re-run green on `b633c80`, with #28 flood 4/4, #30 background 4/4, #7 quota 5/5 and the hook selftest 41/41 intact. Scope is four files and the supervisor is untouched. Ready for local test + merge.
>
> **Merged 2026-07-28 (PR #31).** Mrigesh tested live in a real Claude Code session: `delegate:` confirmed delivery, reported the worker still running, then awaited its answer — on a third harness (`hermes`), which had not been exercised before. Answer independently re-verified against a local grep.

### 2026-07-26 — Trigger words work outside pi-orchestra too, issue #10 (code-puppy)
Now you can type `delegate:`, `orchestrate:`, or `deliberate:` in a plain Claude
Code or Codex session — with no pi-orchestra TUI running — and the harness knows
to route the work through `pio` instead of doing it all itself. A small Claude
Code hook watches what you type; the moment you cast one of those spells it
checks your quota — telling the assistant the real 5-hour/weekly numbers and
warning it off when you're low or blocked — and hands it the exact `pio`/MCP
commands to delegate the job. The matching
skill/AGENTS text was refreshed so Codex and Claude give the same instructions,
and `deliberate:` is answered honestly — the judged “panel” is a later (V2)
feature, so it says so and offers a real single-worker or self-review fallback
instead of faking it. What I did NOT do: wire the hook into your Claude settings
for you (that stays a one-line, opt-in copy-paste so pi-orchestra never edits
protected config), and I didn’t build the V2 panel itself. This is the last of
the standalone-integration work for V1; it leans on the seven `orch_*` tools
from #8.

> **Review verdict (2026-07-27, Claude): 🔨 FIX** — all 5 gates green and the install/uninstall AC1 evidence reproduces exactly (protected-config checksums identical, two installs → identical tree), every documented `pio orch`/`session`/`mcp` invocation really exists and runs, and the Python↔Rust grammar port survived a 4,019-case differential fuzz against `orc_pty::trigger` with **zero** mismatches (incl. Unicode boundary chars) — but **the quota relay doesn't work**: the hook greps `pio quota` for `ORC WARNING/BLOCKED/NOTE`, and `pio quota` never emits those (they come from `quota::gate()` via `pio run`/`dispatch`). Measured: at level **warn** the conductor is told *"no quota advisory to relay"* — a confident false negative on the one guarantee AC2 names; only the `unknown` branch works, which is the only branch the evidence demonstrated. Three docs assert the same impossible behavior, `pio quota` is called "read-only" but writes `quota.json` + `quota_history.jsonl`, and the branch **does not merge** (conflicts in LOG.md/progress.md; as-is it would regress #8 from ✅ back to 🧪). 6-item fix list on the issue.

> **Fix round + re-review (2026-07-27, Claude): 🧪 ACCEPT.** All six items closed and re-verified by me, not by claim. The relay now drives off `pio quota --json` — measured against the real binary at every level: ok → `Quota ok — 5h 80% / weekly 90%`, **warn → `ORC WARNING: … 5h 20% / weekly 90% … Consider pausing delegation.`**, block → `ORC BLOCKED` + "ask the user, do NOT --force", unknown → `ORC NOTE` with the reason; unparseable output falls back to the exit code, never to silence. `--selftest` 22 → **39 checks**, covering every level twice (pure renderer *and* end-to-end through a real subprocess against a stub `pio`). Three false docs corrected, "read-only" dropped (`pio quota` writes `quota.json` + `quota_history.jsonl`). Bonus: the pre-existing install bug where `~/.codex/AGENTS.md` gained a blank line every run is fixed — byte-identical across four consecutive installs. Rebased onto `a0fa88a`, conflicts gone, #8 stays ✅. All five gates green; the 4,019-case grammar fuzz re-run clean after the edits. Ready for your local test + merge.
### 2026-07-27 — Delegation actually delivers now, issue #28 (Claude)
Handing work to a worker was broken for anything real, and it looked like the
worker's fault. pi-orchestra started the worker but then waited for it to finish
before reading anything it said — and a program that is talking gets stuck when
nobody listens. So every worker that produced more than a trickle of output froze
mid-sentence, got killed after two minutes, and was written down as "timed out".
Your two failed TODO scans were exactly this; the worker was fine and doing the
job the whole time. pi-orchestra now listens while the worker talks. The same
delegation that failed twice now comes back correct in 24 seconds. I also made
the two timeout settings honest about which one actually stops a worker — the one
that sounds like it does, doesn't, and that is why the runs died at 120s. What I
did NOT do: let you keep working while a worker runs in the background. That
needs a bigger change to how worker slots are counted, and rushing it would break
the limit that stops pi-orchestra from overloading your subscriptions. It stays
open on the issue.

> **Merged 2026-07-27 (PR #29).** A follow-up commit in the same PR fixed a sequel found in live testing: the capture kept the *first* 16KB, which for a JSON-mode worker is session header and reasoning, so the answer — always last — was discarded and the conductor redid the work itself. Now a 4KB head + 12KB tail window; the new test was verified to fail on the head-only version. Remaining work (non-blocking conductor, answer extraction instead of raw transport) moved to #30.

### 2026-07-25 — One small toolset every brain can drive, issue #8 (code-puppy)
Any conductor — a Claude Code or Codex session, a script, or a person at the
terminal — can now drive pi-orchestra through the same seven verbs: plan a
contracted task, delegate it to a worker, check its status, wait for it to
finish, review it, cancel it, or mark it done. They work two ways from one
shared engine so the two surfaces can never drift: as `pio orch <verb>` commands
and as an MCP server (`pio-mcp`) that exposes the same seven tools over stdio, so
an assistant that speaks MCP delegates work by calling tools instead of guessing
shell syntax. Registering it is copy-paste — `pio mcp print-config` prints
ready-to-paste snippets for Claude Code (`.mcp.json`) and Codex (`config.toml`)
and never edits those protected files itself. Two smaller conveniences came
along: a headless `pio session create` so you can open a delegation session
without the TUI, and delegated workers now receive the task's full acceptance
brief as their prompt by default. What I did NOT do: panel/deliberation tools
(those are V2), and I left `uninstall.sh` alone because it's outside this issue's
allowed paths — so it doesn't yet unlink `pio-mcp` on removal (a tiny
follow-up). This unblocks #10 (Claude Code & Codex reacting to trigger words
outside pi-orchestra), which builds directly on this tool surface.

> **Review 2026-07-26 (Claude, Fable) — 🔨 FIX.** All 5 gates green on my own run (185 tests, 0 failed) and all 4 ACs reproduced independently — raw JSON-RPC into the release `pio-mcp` (exactly 7 tools, contract-v2 schema reused by `$ref`), my own fixture worker end-to-end incl. the queued→drain→confirmed path, real CLI-vs-MCP twin-session diff, and `print-config` from a scratch install (valid JSON + `tomllib`-valid TOML, zero files written); mutation-tested the drift guards (smuggled 8th tool, drifted description, CLI dropping the contract — all three caught); tokio containment verified by `cargo tree`; the "no daemon" deviation is justified (orc-proto has no task-creation verb) and there is no scope creep. Two things to fix: **`uninstall.sh` leaves a dangling `pio-mcp` symlink on PATH** (reproduced live; the install test was updated for install but deliberately not for uninstall) — I authorize the one-line out-of-path edit — and **a failed or queued `orch_delegate` returns plain success over MCP** (`isError:false`, `note:null`) while the CLI exits 1/75, so `OrchOutcome.note` should carry the failure or queue warning. Four minors (one-directional CLI parity test, an overclaiming test comment, inert `orch_cancel` kill path, the `dispatch send` follow-up not fully discharged). Details on issue #8.

> **Fix round 2026-07-26 (Claude, Fable — applied by the reviewer at Mrigesh's request) — 🧪 ACCEPT.** Both fixes are in on the same branch. `uninstall.sh` now unlinks `pio-mcp`, so an install→uninstall cycle leaves `~/.local/bin` empty with zero dangling links (verified live, and the install test now checks the uninstall side with `symlink_metadata` so a broken link can't slip past `exists()`). A failed or queued `orch_delegate` now fills `OrchOutcome.note`, so the MCP surface says the same thing the CLI's exit code does — a failure names its kind and message, a queued dispatch says "call orch_await to wait for a free slot" and is deliberately *not* worded as a failure; confirmed deliveries stay quiet. Also fixed the three minors: the CLI parity test now asserts set equality against `Verb::ALL` (an eighth CLI verb would have passed before), the MCP e2e test now asserts the delivered prompt really is the rendered brief instead of an always-true stdout check, and `orch_cancel`'s doc says plainly that its kill half only fires for a real background run. Left open on purpose: `render_brief` is wired into `orch delegate` but not `dispatch send` — the right call, but the seeded follow-up is not closed. All 5 gates green on Rust 1.91.1, **188 tests / 0 failed** (was 185); each fix regression-proofed by reverting it and watching its new test fail. Ready for local test + merge.

> **Merged 2026-07-27 (PR #26, `0c9908a`) — ✅.** Tested locally by Mrigesh and merged. Note the entry above predates the fix round: `uninstall.sh` *does* unlink `pio-mcp` on `main`.

### 2026-07-24 — Never rate-limit your own subscriptions, issue #7 (code-puppy)
pi-orchestra now protects the paid subscriptions you delegate to from being
hammered. Every tool gets a cap on how many workers may run **at the same time**
— a sensible per-tool default (e.g. 3 for pi/Hermes, 2 for the frontier coding
CLIs) that you can change with `pio harness cap <tool> <n>` — and that cap is
honored across every session and every pi-orchestra process on the machine, not
just within one run. When a tool's slots are all busy, the next hand-off is
**queued** instead of spawned: it's recorded and visible in `pio dispatch list`,
no extra worker is started, and `pio dispatch drain` runs the waiting work the
moment a slot frees. Separately, if a worker's output shows a provider
rate-limit (an HTTP 429, "too many requests", "overloaded", and the like),
pi-orchestra now **backs off and retries** with growing, jittered delays instead
of pounding the provider, printing a plain `ORC WARNING: … rate-limited; backing
off …s before retry` each time (and surfacing any "retry-after" the tool asked
for); if the tool keeps refusing after the retry budget, the dispatch fails
honestly as `rate_limited` rather than hanging or lying. I did NOT add
cost/budget-based routing between tools (that's the V1.5 budget router), and I
did not change how a single worker runs once it holds a slot. This is the last
of the "quota guard" work and rounds out safe delegation; it composes cleanly
with #8's `orch_*`/MCP surface (which can now delegate without fear of a
rate-limit storm) and #11's per-task worktrees.

> **Review 2026-07-24 (Claude, Fable) — 🔨 FIX.** All 4 ACs and all 5 gates pass on my own run; path scoping and deviations clean. One confirmed blocker: rate-limit detection is checked before the success check in `invoke_with_backoff`, so a worker that exits 0 with output merely mentioning a signal (`429`, `rate limit`, `overloaded`, …) is retried 4× and reported `rate_limited` — proven with a throwaway exit-0 test — silently failing good work and 4×-ing provider load (opposite of the objective), and slipping through AC2 (whose fixtures all exit non-zero). Fix: gate detection on non-zero exit + add an exit-0 regression test. Two minor non-blockers (`.slots.lock` stale-lock wedge; `parse_retry_after` unit-blind). Details on issue #7.

> **Fixed 2026-07-24 (code-puppy) — re-review requested.** All three items addressed on the same branch: rate-limit detection now fires only on a **non-zero** exit, so a clean exit-0 run is confirmed regardless of what its output merely mentions — with a regression test that feeds a successful worker this PR's own diff summary (`… rate limit backoff and 429 handling`) and asserts one attempt, confirmed. Also the two minors: `.slots.lock` now records the holder pid and reclaims a dead/aged lock via an atomic rename-steal (so a SIGKILL mid-hold can't wedge a harness's cap; +test), and `parse_retry_after` honors second/minute/hour/ms units ("2 minutes" → 120, not ~2). +2 tests; all 5 gates green.

> **Re-review 2026-07-24 (Claude, Fable) — 🧪 ACCEPT.** All three fixes verified. Fix 1 re-checked with my own harsher probe (exit-0 worker whose *entire* output is a bare `HTTP 429` → confirmed, 1 attempt; exit-1 `429` still `rate_limited` after the full budget, so AC2 is intact). Fix 2's rename-steal is race-safe (`create_new` stays the mutex, one winner, a live-held lock is never stolen). Fix 3 is unit-aware and warning-only. All 5 gates green on my own run; diff since the reviewed commit stays inside `orc-core/` (+docs), no new deps, no scope creep. Ready for local test + merge.

### 2026-07-24 — Any capable CLI can be a worker, not just pi/Hermes, issue #6 (code-puppy)
pi-orchestra can now hand a task to **any** installed coding CLI it has actually
probed — Claude Code, Codex, OpenCode, and more — not just the two (pi and
Hermes) that were hand-wired before. When it delegates, it looks at what
`pio doctor` discovered each tool can do and builds the correct command line for
that specific tool automatically: the ones that take the job as a plain argument
(`claude -p "…"`, `pi -p "…"`) versus the ones that need a sub-command
(`codex exec "…"`, `opencode run "…"`), and it only adds extras like
machine-readable output or a working-directory flag when the probe proved that
tool supports them. Every worker is also now launched **inside the task's own
folder** (its git worktree when isolated, otherwise the session folder), and
that folder is recorded on the receipt. Crucially, if you point it at a tool
that was never shown to run non-interactively, it refuses honestly with an error
naming the exact missing ability (e.g. `non_interactive`) and exits non-zero,
rather than pretending or hanging. I did NOT add live steering, session-resume,
or rate-limiting (those are #7 and later), and I did NOT change the two existing
hand-configured defaults — they keep working exactly as before. This unblocks
#12 (single-harness honest mode) and gives #8's `orch_*`/MCP surface a real,
probe-driven delegate to call.
> **Review verdict (Claude, 2026-07-24): fixture ACs all pass — but real-CLI testing found a defect, now fixed in-branch.** All 5 gates green; all 4 fixture ACs re-verified and mutation-tested. Going beyond the fixtures, a live dispatch to the actual installed CLIs showed `claude` working end-to-end but **`codex exec` failing** ("Not inside a trusted directory and --skip-git-repo-check was not specified") — a worker's orchestrator-assigned cwd is not guaranteed to be a git repo, so codex could never run as a probe-driven worker. **Fix applied here (orc-core only):** codex's template now carries the mandatory, permissive `--skip-git-repo-check` (a new probe-independent `fixed` flags slot; NOT a dangerous-skip flag, per #16). Re-tested live: codex + claude both confirmed, exit 0, returned PONG in a non-git sandbox. Ready for owner test + merge.

### 2026-07-24 — Trigger words light up inside conductor panes, issue #9 (code-puppy)
When the conductor (the brain pane) prints one of the three spell words at the
start of a line — `delegate:`, `orchestrate:`, or `deliberate:` — pi-orchestra
now lights that word up in the pane, ultrathink-style: the token is drawn in the
theme accent as a bold, reverse-video block, and the pane's title grows a small
badge (a diamond glyph plus the word, e.g. "DELEGATE") so you can tell at a
glance the conductor is casting a spell. It is deliberately strict and only
fires on a real trigger: `redelegate:` and a bare `delegate` with no colon stay
plain, matching is case-sensitive, and a worker pane that merely echoes the word
never lights up — only the conductor asserts intent. Because the highlight is a
bold reverse-video block plus a spelled-out badge, it still reads with color
turned off (NO_COLOR / mono terminals) and looks identical whether reduced
motion is on or off. I did NOT make the highlight *do* anything yet — typing
`delegate:` shows the affordance but does not itself dispatch a worker (routing
is #6/#8), and I did NOT touch standalone harnesses like Claude Code or Codex
(that's #10). The trigger grammar now lives as a reusable, tested primitive
(`orc_pty::trigger`) that #8's `orch_*` control surface can call to actually
route a spell to a procedure.

> **Review (2026-07-24, Claude):** ~~🧪 ACCEPT~~ **RETRACTED** — all 5 gates pass and all 4 acceptance checks are non-vacuous, but live testing found the feature does not fire for its primary use case. Superseded by the FIX verdict below.
>
> **Re-review (2026-07-24, Claude):** 🔨 FIX — Mrigesh ran a real Claude Code brain pane and typed `delegate: some web research to the workers`; it did **not** highlight. Root cause: the line renders as `❯ delegate: …` and the grammar is line-anchored to the *first non-whitespace char* (the `❯` prompt glyph), so it never matches. Every acceptance-test fixture fed a **bare** stream (`"delegate: …\r\n"`) with no prompt prefix, so the tests were green but unrepresentative of any real hosted pane (`❯`/`>`/`$`). Confirmed the installed binary IS the #9 build (not stale) and reproduced against the matcher. Owner confirmed intent: typing at the prompt must light up (ultrathink-style). Fix list on issue #9 — anchor must tolerate a leading prompt marker (keeping `char_start` on the keyword), and the fixtures must include the real prompt prefixes.
>
> **Fix applied (2026-07-24, code-puppy):** `scan_line` now tolerates one optional leading prompt marker — a bounded run of up to 3 non-alphanumeric sigils followed by whitespace (covers Claude Code's U+276F prompt, `> ` / `$ ` / `% `, oh-my-zsh, and a `>>> ` REPL) — with `char_start` kept on the keyword, so only the keyword+colon highlights, never the prompt glyph. Every fixture now streams the real prompt prefixes and asserts the highlighted span is *exactly* `keyword:`; a new test replays the exact line Mrigesh typed as a recorded Claude-Code-shaped byte stream (ANSI color + U+276F) through the real vt100 parser and full renderer. AC2 re-checked with a prefix present (`redelegate:`, colon-less, wrong-case, `delegated:` all stay quiet), plus long-sigil-banner and no-whitespace-gap guards. Prompt-marker policy (a shape rule, deliberately not a glyph allowlist — a missed highlight is the real harm; a spurious one is cosmetic since nothing dispatches) documented in the module. All 5 gates green. Pushed to `issue-9-trigger-grammar`; status back to needs-review. Note: the fully-interactive live re-test in a real Claude Code pane is the merge-time human step (workflow step 7); the recorded-stream test is the automated stand-in that would have caught this.
>
> **Ultrathink-style change (2026-07-24, Claude, owner-directed):** Mrigesh tested live and asked for true ultrathink behaviour — the spell should light up **wherever** it appears on the line and **every** time, not just the first token at the start. Reworked `scan_line` from "one line-anchored match" to "**all** matches at a word boundary + colon", returning `Vec<TriggerMatch>` left-to-right; the renderer (`scan_pane_row`) now emits one span per occurrence. A word boundary is line-start or any non-alphanumeric char, which *subsumes* the prompt-marker special-case (the space after `❯`/`>`/`$` is a boundary), so `skip_prompt_marker` + `MAX_PROMPT_MARKER_RUN` were **deleted** as dead code. Guards preserved: colon still required (`can you delegate this` stays plain) and a keyword welded into a word still never fires (`redelegate:` quiet). This intentionally **supersedes the original AC2 line** "mid-sentence `orchestrate:` does not trigger" — the owner now wants mid-sentence to fire; the no-false-positive intent (prose, wrong word, wrong case) is unchanged. New tests: `every_occurrence_on_a_line_is_reported_left_to_right`, `a_trigger_fires_mid_line_not_only_at_the_start` (grammar) and `conductor_highlights_every_occurrence_including_mid_line` (renderer, asserts `delegate: a … delegate: b` highlights both). All 5 gates green; verified live that `❯ delegate: … , delegate: …` lights both. 🧪 — ready for your local test.
>
> **Rainbow highlight (2026-07-24, Claude, owner-directed):** Mrigesh asked for the `ultrathink` rainbow look instead of the flat accent block. The token now shimmers per-character: each column of the highlighted span takes the next colour from a 7-stop `TRIGGER_RAINBOW` (red→orange→yellow→green→blue→indigo→violet), kept **BOLD** with the source cell's own background (no more reverse-video block). Colour is *not* load-bearing — the token stays bold and the `◆ LABEL` title badge still names the spell, so it survives NO_COLOR/mono (AC3 test updated: asserts the bold rainbow span + badge, and that reduced-motion frames stay byte-identical since the rainbow is static). Test helper `highlighted_symbols` now identifies a trigger cell by "BOLD + fg ∈ `TRIGGER_RAINBOW`". Deviation note: this uses explicit RGB stops rather than visual-identity slot names (AGENTS.md prefers slots) — a deliberate, owner-requested exception for the ultrathink effect, which has no single-slot equivalent. All 5 gates green. 🧪 — ready for your local test.
>
> **Animated rainbow (2026-07-24, Claude, owner-directed):** Mrigesh asked for the rainbow to *move* like real ultrathink, not sit static. Added a motion phase: `render_shell` derives `motion = (!reduced_motion).then(|| epoch.elapsed()/120ms)` on the Stage view (same clock the HOME masthead already uses) and threads it through `render_stage` → `render_pane`, where the per-column colour index becomes `(offset + phase) % 7` — so the gradient slides one stop per ~120 ms and appears to flow along the token. **Accessibility preserved:** under `reduced_motion` the phase is frozen at 0, so the rainbow is colourful but perfectly static (AC3). To keep the shimmer running after the baton pulse settles, the shell repaint loop now also animates while `StageState::has_live_trigger()` (any conductor pane shows a trigger). Tests: reworked the AC3 test to prove the reduced-motion render is byte-identical across repaints (frozen), and added `trigger_rainbow_animates_when_motion_is_on` — asserts phase 0 vs phase 1 differ and are a one-stop slide, while two `None` renders stay identical. All 5 gates green. 🧪 — ready for your local test.

### 2026-07-24 — Every delegated task carries a contract, issue #5 (code-puppy)
pi-orchestra tasks can now carry a full "contract": the objective, the exact
files a worker may touch, forbidden actions, the expected artifact, numbered
acceptance checks, a per-attempt timeout and retry limit, a named reviewer, and
a token/dollar budget. You attach one when you create a task — `pio task add
"title" --objective … --allowed … --forbidden … --check … --artifact …
--reviewer … --timeout … --max-tokens … --max-usd-cents …` — and read it back
with `pio task show`. The new `pio task brief` prints the exact hand-off a
worker receives: every contract section, reproduced word-for-word, with any
unset section clearly marked "(none specified)" instead of quietly dropped, so
no one mistakes a blank contract for a satisfied one. Old task files written
before contracts still load untouched, and unknown future fields survive a
read→write cycle, so nothing on disk breaks. I did NOT wire the brief into the
actual dispatch send yet (a worker still gets the prompt you pass), and I did
NOT surface contract fields on the SCORE board — that card is fed by crates
(`orc-proto`/`orc-daemon`) this issue isn't allowed to touch, so it's a clean
follow-up. This unblocks #8 (the `orch_*` control surface + MCP server, which
reuses this exact schema) and #11 (worktree isolation + independent review).

> **Review verdict (2026-07-24, Claude): ACCEPT** — all 5 gates re-run green on MSRV 1.91.1 incl. the offline `--locked` build (schemars 1.2.1 resolves from cache); AC1/AC2/AC3 independently reproduced (pre-v2 records load with no spurious `contract` key + unknown fields survive at top/contract/nested layers; `task add`→`show`→`brief` round-trips every field; brief reproduces each section verbatim and marks unset ones `(none specified)`). Scope clean: allowed paths + the two justified deviations — `rust/Cargo.toml`/`Cargo.lock` for the **#16-mandated** schemars dep (decision record §5, MIT, exact version), and the SCORE-card deferral (its data lives in the *forbidden* `orc-proto`/`orc-daemon`, so genuinely blocked). No dead code, no lint-silencing. One non-blocking note: the brief isn't yet wired into `dispatch send` (`render_brief` is `pub` and ready) — AC3 is about brief *content*, which passes, but closing that loop is the real follow-on for #8/#11. Set 🧪 — ready for your local test + merge.

### 2026-07-23 — Find out what each installed CLI can actually do (`pio doctor`), issue #4 (code-puppy)
pi-orchestra can now tell you what each AI CLI on your machine is genuinely
capable of, instead of assuming. Run `pio doctor` and it asks every discovered
tool (via its own `--help`) whether it can do the eight things that matter — run
headless, resume a session, use tools, pick a model, emit machine-readable
output, report usage, be cancelled, and control its working directory — then
prints an honest table: each tool's role (conductor/worker/limited), a plain
summary, and a check/cross grid where a missing tool is shown as "unavailable,"
never hidden. It remembers the answers per tool and only re-checks when that
tool's binary actually changes (or you pass `--refresh`), and it will never
offer a capability a tool didn't prove it has. I did NOT make it hand real work
to those tools yet (that's #6, the universal worker adapter), and the exact
"rate-limited" wording each tool prints is still to be captured later (feeds
#7). This unblocks #6, #7, and #12, which all need to know what each harness
can really do.

> **Review verdict (2026-07-24, Claude): ACCEPT** — all 5 gates re-run green on MSRV 1.91.1; AC1/AC2/AC4 independently reproduced live (fixture probe → `discovered.<name>.probe` serialization, spec-shape table with `unavailable` rows + honest glyph/en-dash matrix, failed/never-probed/unknown harnesses offer nothing downstream); scope clean (allowed paths only, no new deps, Cargo.lock untouched, no dead code/lint-silencing). One non-blocking deviation: the cache keys on path/mtime/**size**, not AC3's literal "hash" — I demonstrated a contrived equal-size + exact-nanosecond-mtime content swap that evades re-probe, but every real reinstall/upgrade bumps ns-mtime (verified), and this satisfies the #16 binding decision's "path + mtime/hash". Second note for V1-4: `probed_capabilities` returns last-known caps for a harness that has left PATH (capability ≠ availability, documented) — dispatch must gate on `locate_executable` separately. Set 🧪 — ready for your local test + merge.

### 2026-07-23 — Find every AI CLI on your machine, issue #3 (code-puppy)
pi-orchestra can now discover which AI coding CLIs you actually have installed.
Run `pio harness list` and it scans your PATH for the known tools (claude,
codex, hermes, pi, opencode), then remembers what it found in its settings file
— where each one lives, its version, and when it was first and last seen. The
list always shows all five: the ones you have are marked available with their
path, the ones you don't are honestly marked "NOT ON PATH / unavailable" and are
never hidden. The HOME screen's availability strip shows this discovered set
too. I did NOT test what each CLI can actually do yet (that's the next issue,
#4, "pio doctor") and I did NOT change how work is handed to workers. This
unblocks #4 (capability probing) and the bigger goal of letting any capable CLI
be a worker.

> **Review 2026-07-23 (Claude): FIX** — all 5 gates + AC1/AC2/AC3 reproduced live, scope clean, but a failing `--version` gets its error text persisted as the harness "version" (2 fixes listed on #3); back to 🔨.

> **Fix pushed 2026-07-23 (code-puppy):** a rejected `--version` (non-zero exit) now records no version and falls back to any previously stored one, instead of persisting the stderr error text; added a regression test. All 5 gates green (test 96/0), Cargo.lock unchanged. Back to review.

> **Re-review 2026-07-23 (Claude): ACCEPT** — both fix items verified live (original repros dead, stored fallback survives, happy path intact), gates 5/5 green (96/0), zero scope creep in the fix commit; 🧪 — ready for your local test + merge.

### 2026-07-23 — Rename the everyday command to `pio`, issue #17 (code-puppy)
The command you type is now `pio` (and its background helper is `piod`), so the
tool finally matches the "pi-orchestra" name instead of the old `orc`/`orcd`. If
you still type the old `orc`, it keeps working but prints a friendly note telling
you to switch, and the installer backs up your previous command first so nothing
is lost. The installer, uninstaller, README, shell shortcuts, and both AI skill
files all speak the new name now, and the built binaries were verified end to end
(`pio version`, `piod --help`, and a full install/uninstall in a scratch folder).
I did NOT rename the internal code folders, the `~/.orchestra` data directory, or
the `ORC_*` settings (those stay for compatibility), and I left one dated
historical guide (`docs/guide.html`) untouched on purpose. On this machine's
freshly installed Rust 1.97 the clippy gate first tripped on three pre-existing
warnings in files the rename did not touch (the repo targets Rust 1.91, where
they stay quiet); with your OK I cleaned up all three in this same PR, so now
every gate passes green with nothing suppressed. This unblocks the parallel V1
work (#3, #5, #9, #13) without every branch colliding on the rename.

> **Review verdict (2026-07-23, Claude):** ACCEPT — all 5 gates re-run green on MSRV 1.91.1, every acceptance check independently reproduced (live scratch-HOME install/uninstall with backup+shim+restore, zero `orc`/`orcd` leaks even in sub-subcommand helps the gate test doesn't scan), the 3 out-of-path clippy fixes verified behavior-preserving and owner-approved. Set 🧪 — ready for Mrigesh to test and merge.

### 2026-07-22 — Foundations research, issue #16 (Claude Code)
Every big technical choice for V1 is now decided and written down in one
place (`docs/superpowers/specs/2026-07-22-v1-crate-and-prior-art-decisions.md`),
so the build issues don't each re-argue them: official MCP SDK for the new
server, plain `git` commands for worktrees, `backon` for retries, `schemars`
for schemas, `insta` for UI snapshots. The exact commands to drive Claude,
Codex, OpenCode, Hermes and pi headlessly were verified against the real
binaries on this machine, not blog posts. No code was written. This unblocks
#3–#8 and #11; each got a comment naming what binds it.
> **Review verdict (Claude, 2026-07-22): ACCEPT — contract satisfied; crate versions spot-checked against crates.io, all exact. Merged as PR #18.**

### 2026-07-22 — Program setup (Claude Code)
The V1 plan is now real: spec, workflow, new visual identity docs, and 12
contracted GitHub issues (epic #15). Nothing of V1 itself is built yet.
Next: run four code-puppy sessions on #3, #5, #9, #13.
