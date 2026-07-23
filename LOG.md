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
| [#17](https://github.com/Legend101Zz/Agent-orchestra/issues/17) | Rename the command `orc` → `pio` everywhere users see it | 🧪 *merge FIRST* | issue-17-rename-cli-pio |
| [#3](https://github.com/Legend101Zz/Agent-orchestra/issues/3) | Find every AI CLI installed on the machine and remember them | ⬜ | — |
| [#5](https://github.com/Legend101Zz/Agent-orchestra/issues/5) | Every delegated task carries a "contract": what to do, where allowed, how we check it worked | ⬜ | — |
| [#9](https://github.com/Legend101Zz/Agent-orchestra/issues/9) | When you type `delegate:` / `orchestrate:` / `deliberate:` inside a pane, it lights up like ultrathink | ⬜ | — |
| [#13](https://github.com/Legend101Zz/Agent-orchestra/issues/13) | The new look: nocturne/ember/phosphor themes, glyphs, baton animation | ⬜ | — |
| [#4](https://github.com/Legend101Zz/Agent-orchestra/issues/4) | Test what each installed CLI can actually do (`pio doctor`), never assume | ⬜ *needs #3* | — |
| [#6](https://github.com/Legend101Zz/Agent-orchestra/issues/6) | Any capable CLI can be a worker, not just pi/Hermes | ⬜ *needs #4* | — |
| [#7](https://github.com/Legend101Zz/Agent-orchestra/issues/7) | Never spawn so many workers that a subscription gets rate-limited | ⬜ *needs #4* | — |
| [#8](https://github.com/Legend101Zz/Agent-orchestra/issues/8) | The 7 `orch_*` commands + MCP server so any brain can drive pi-orchestra | ⬜ *needs #5* | — |
| [#11](https://github.com/Legend101Zz/Agent-orchestra/issues/11) | Each task runs in its own worktree, gets independently reviewed, produces a receipt | ⬜ *needs #5* | — |
| [#10](https://github.com/Legend101Zz/Agent-orchestra/issues/10) | Claude Code & Codex react to trigger words even outside pi-orchestra | ⬜ *needs #8* | — |
| [#12](https://github.com/Legend101Zz/Agent-orchestra/issues/12) | With only one CLI installed: still useful, honestly says so | ⬜ *needs #4, #6* | — |
| [#14](https://github.com/Legend101Zz/Agent-orchestra/issues/14) | New README + screenshots for launch | ⬜ *last* | — |

**Start now: #16 (Claude session, prompt 0) and #17 (puppy, prompt 1). Merge
#17 before anything else — it renames files everywhere and parallel branches
would all conflict. THEN the parallel set: #3, #5, #9, #13.**

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
