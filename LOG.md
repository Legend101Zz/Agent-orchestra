# 🎼 pi-orchestra V1 — Mrigesh's log

*The one file the human reads. Status board + the exact prompt to run next +
plain-English ship log. Agents: update this file as instructed in AGENTS.md —
status column and ship-log entries are part of finishing an issue.*

**Legend:** ⬜ not started · 🔨 being built · 👀 pushed, needs review · 🧪 reviewed, needs your local test · ✅ merged

## Status board

| Issue | In plain words | Status | Branch |
|---|---|---|---|
| [#3](https://github.com/Legend101Zz/Agent-orchestra/issues/3) | Find every AI CLI installed on the machine and remember them | ⬜ | — |
| [#5](https://github.com/Legend101Zz/Agent-orchestra/issues/5) | Every delegated task carries a "contract": what to do, where allowed, how we check it worked | ⬜ | — |
| [#9](https://github.com/Legend101Zz/Agent-orchestra/issues/9) | When you type `delegate:` / `orchestrate:` / `deliberate:` inside a pane, it lights up like ultrathink | ⬜ | — |
| [#13](https://github.com/Legend101Zz/Agent-orchestra/issues/13) | The new look: nocturne/ember/phosphor themes, glyphs, baton animation | ⬜ | — |
| [#4](https://github.com/Legend101Zz/Agent-orchestra/issues/4) | Test what each installed CLI can actually do (`orc doctor`), never assume | ⬜ *needs #3* | — |
| [#6](https://github.com/Legend101Zz/Agent-orchestra/issues/6) | Any capable CLI can be a worker, not just pi/Hermes | ⬜ *needs #4* | — |
| [#7](https://github.com/Legend101Zz/Agent-orchestra/issues/7) | Never spawn so many workers that a subscription gets rate-limited | ⬜ *needs #4* | — |
| [#8](https://github.com/Legend101Zz/Agent-orchestra/issues/8) | The 7 `orch_*` commands + MCP server so any brain can drive pi-orchestra | ⬜ *needs #5* | — |
| [#11](https://github.com/Legend101Zz/Agent-orchestra/issues/11) | Each task runs in its own worktree, gets independently reviewed, produces a receipt | ⬜ *needs #5* | — |
| [#10](https://github.com/Legend101Zz/Agent-orchestra/issues/10) | Claude Code & Codex react to trigger words even outside pi-orchestra | ⬜ *needs #8* | — |
| [#12](https://github.com/Legend101Zz/Agent-orchestra/issues/12) | With only one CLI installed: still useful, honestly says so | ⬜ *needs #4, #6* | — |
| [#14](https://github.com/Legend101Zz/Agent-orchestra/issues/14) | New README + screenshots for launch | ⬜ *last* | — |

**Start now, in parallel (no conflicts): #3, #5, #9, #13.**

## Prompts you run

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

### 2. Review a pushed branch (Claude Code, one session per issue)

```
Review branch issue-<N>-* of ~/Agent-orchestra against the task contract in
GitHub issue #<N>, per docs/WORKFLOW.md. Be adversarial: run the gates and
try to make each acceptance check fail. Comment findings on the issue and
update LOG.md (status + ship log note if verdict changes).
```

### 3. After YOUR local test passes → merge

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

### 2026-07-22 — Program setup (Claude Code)
The V1 plan is now real: spec, workflow, new visual identity docs, and 12
contracted GitHub issues (epic #15). Nothing of V1 itself is built yet.
Next: run four code-puppy sessions on #3, #5, #9, #13.
