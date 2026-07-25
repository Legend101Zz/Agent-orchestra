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
| [#13](https://github.com/Legend101Zz/Agent-orchestra/issues/13) | The new look: nocturne/ember/phosphor themes, glyphs, baton animation | ⬜ | — |
| [#6](https://github.com/Legend101Zz/Agent-orchestra/issues/6) | Any capable CLI can be a worker, not just pi/Hermes | ✅ | merged (PR #24) |
| [#7](https://github.com/Legend101Zz/Agent-orchestra/issues/7) | Never spawn so many workers that a subscription gets rate-limited | 🧪 | issue-7-quota-guard-v2 · [PR #25](https://github.com/Legend101Zz/Agent-orchestra/pull/25) |
| [#8](https://github.com/Legend101Zz/Agent-orchestra/issues/8) | The 7 `orch_*` commands + MCP server so any brain can drive pi-orchestra | ⬜ *unblocked* | — |
| [#11](https://github.com/Legend101Zz/Agent-orchestra/issues/11) | Each task runs in its own worktree, gets independently reviewed, produces a receipt | ⬜ *unblocked* | — |
| [#10](https://github.com/Legend101Zz/Agent-orchestra/issues/10) | Claude Code & Codex react to trigger words even outside pi-orchestra | ⬜ *needs #8* | — |
| [#12](https://github.com/Legend101Zz/Agent-orchestra/issues/12) | With only one CLI installed: still useful, honestly says so | ⬜ *unblocked* | — |
| [#14](https://github.com/Legend101Zz/Agent-orchestra/issues/14) | New README + screenshots for launch | ⬜ *last* | — |

**#6 merged (2026-07-24, PR #24) — universal probe-driven worker adapter
shipped: any harness the `pio doctor` probe shows can run non-interactively is
now a worker (claude/codex/opencode + hermes/pi), the adapter builds each
one's command line from the probe results, and an unprobed capability is
refused by name. Adversarial review went past the fixtures to real CLIs: found
and fixed a codex launch bug in-branch (`--skip-git-repo-check`, verified live),
and confirmed the probe itself is sound. Unblocks #12 (single-harness honest
mode). Remaining ready set: #7 (rate limiting), #8 (`orch_*` + MCP, reuses the
#5 schema), #11 (worktree isolation, builds on #5), #12, plus #13 anytime.
**Next: #7** (subscription rate-limit guard), then #8. Known follow-up seeded by
#5's review: the contract brief isn't yet wired into `dispatch send`
(`render_brief` is `pub`), and there's no headless `session create` — fold
both into #8. Start #13 before more TUI churn lands to avoid merge pain.**

## Prompts you run

### 0. Foundations research (Claude Code session, once, no code)

```
Work GitHub issue #16 of Legend101Zz/Agent-orchestra (clone or use
~/Agent-orchestra, branch issue-16-research). Read the issue and the V1 spec
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
You are the adversarial reviewer for pi-orchestra (~/Agent-orchestra), per
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
Re-review branch issue-<N>-* of ~/Agent-orchestra: verify ONLY the numbered
fix list from your previous review comment on issue #<N>, re-run the gates,
and confirm nothing new broke or crept in (git diff against the previously
reviewed commit). Verdict ACCEPT or FIX on the issue; update LOG.md status.
If this is already the second fix round and it still fails: STOP and
recommend re-scoping the issue instead (docs/ANTI-SLOP.md rule 4).
```

### 5. After YOUR local test passes → merge

```bash
cd ~/Agent-orchestra && git fetch origin
git checkout issue-<N>-<slug> && ./install.sh   # try the feature yourself
git checkout main && git merge --no-ff issue-<N>-<slug> && git push
```
Then tick the box on epic [#15](https://github.com/Legend101Zz/Agent-orchestra/issues/15), set ✅ here, close the issue.

## Ship log (newest on top — plain English, no jargon)

*code-puppy: after pushing a branch, add an entry here (in the same branch):
2-4 sentences — what can pi-orchestra do now that it couldn't before, what
you did NOT do, and what this unblocks. Claude reviewers append a one-line
verdict under the entry.*

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
