# Issue #45 — a conductor seated in the TUI dispatches to its own panes: evidence

Branch `issue-45-seated-conductor`, from `main` @ `17f0942`.
Twelve acceptance checks, five gates. Everything below was run, not reasoned.

---

## The bug the issue did not know it had

The issue's root-cause section says the path from "brain delegates" to "you
watch it happen" already exists end to end, and that only the hook's injected
`session create` wires the two halves apart. That is true of three of the four
links. The fourth was broken independently, and fixing the hook alone would
have left check 1 failing.

`note_task_events` skipped every task whose watermark was unset:

```rust
// A first sighting is history, not news: attaching to a session
// with a finished board must not replay every dispatch it ever made.
if seen == 0 { continue; }
```

That reasoning holds for the board that already existed at attach — and
`attach_stage` *already* seeds the watermark for exactly that board, so the
guard was doing the job twice. The second time it was wrong. A task created
**after** attach also has no watermark, and `pio orch delegate` creates,
assigns and confirms one inside a single synchronous call. STAGE's first
sighting of it is the finished article, so the guard swallowed the whole
delegation.

Measured before the fix, against the real render path:

```
running 1 test
test tests::probe_a_task_created_after_attach_animates_its_whole_delegation ... FAILED

assertion `left == right` failed: the outbound dispatch and the inbound confirm both animate
  left: 0
 right: 2
```

**Zero flights.** Not a degraded animation — none at all.

Every existing test passed because each hand-fed a `created`-only board first,
sharing the assumption that a task is always seen before it is dispatched.
This is the same class of bug `circuit::message_for`'s own doc comment already
warns about in that file: *"Every test around it passed, because they all
built their own history and shared the same wrong assumption."*

Fix: split seeding from noting. `attach_stage` calls `seed_task_events`, which
is now the only thing permitted to treat history as old news; the polling path
treats a new task as the news it is.

---

## Check 1 — the delegation reaches the pane on screen

Pinned by `orc-app/tests/task_vocabulary.rs::a_delegation_into_a_seated_session_lands_on_the_pane_on_screen`,
driven through the **real** `orch::delegate` the CLI calls — the same integration
target that exists because "only the real API can catch that class of bug".

Two workers are seated, only one matching the delegated harness, so "it picked
the right one" is a real claim and not a coin toss. The test asserts, in order:

| claim | assertion |
|---|---|
| no second session | `list_sessions().len()` unchanged across the delegation |
| the seated pane was chosen, unnamed | `dispatch.pane_id == "…-worker-2"` with `pane: None` in the request |
| the board aims STAGE at that pane | `task.assignee_run == "…-worker-2"` |
| it animates | real history → `message_for` → `(Outbound, Dispatched)` **and** `(Inbound, Confirmed)` |

```
test a_delegation_into_a_seated_session_lands_on_the_pane_on_screen ... ok
test result: ok. 5 passed; 0 failed
```

Stable 5/5 on repeat. The first version of this test was itself racy — it read
the outcome `delegate` returned, and the confirmation is written by the
detached supervisor. That is not a defect (STAGE re-reads the board on every
snapshot and picks it up on the next one) but a test that read once was racing
the supervisor, and did. It now polls the durable board, which is what STAGE
actually does.

**One code change fell out of writing it.** `record.pane_id` was populated from
`request.pane_id` — the pane the caller *asked* for — so when dispatch selected
a seated worker on its own, the durable receipt stayed empty. "Which pane got
this brief?" could only be answered when you already knew. It now records the
pane that was chosen.

### Not demonstrated: a live TUI recording

The issue asks for the animation observed on STAGE with panes seated by `n`.
A paned session can only be created by the daemon's `launch_session`, which is
driven by the interactive TUI; there is no CLI path to one. What is proven here
is every link in the chain — pane selection, board linkage, the vocabulary, and
the animation — each against the real API rather than a fixture. **The live
`n` → `delegate:` → watch-it-move run is left for the local test step**, and it
is the one thing in this branch I could not run myself.

---

## Check 2 — the injected context names the seat

Run with the vars a real pane carries, against the real `pio` binary and the
actual session from the issue's reproduction (`bin-1785416854-0000`, still in
`~/.orchestra`):

```
$ ORC_SESSION=bin-1785416854-0000 \
  ORC_PANE_ID=bin-1785416854-0000-brain \
  ORC_WORKERS='…-worker-1=hermes,…-worker-2=pi-m3' \
  python3 shell/claude-userpromptsubmit-hook.py <<< '{"prompt":"delegate: ask hermes to …"}'

YOU ARE ALREADY INSIDE A pi-orchestra SESSION. The panes on screen are the
bench — dispatch into them, and the user watches it happen.
  session:  bin-1785416854-0000   <- REUSE THIS. Do NOT run `pio session create`.
  your pane: bin-1785416854-0000-brain   role=brain   harness=claude
  cwd:      /Users/comreton/.local/bin
  workers seated with you:
    hermes     running   bin-1785416854-0000-worker-1
    pi-m3      running   bin-1785416854-0000-worker-2
Creating a second session is the one thing that breaks this: a new session has
no panes, so dispatch falls back to a headless worker, the board of the session
you are sitting in never changes, and STAGE never moves. It would still 'work'
— invisibly, to a worker the user cannot see.
```

followed by the seated recipe, which names a seated harness rather than a
placeholder:

```
    pio orch delegate hermes --session "$ORC_SESSION" \
      --title "<what>" --objective "<done-when>" --check "<acceptance check>" --json
```

Session, pane, role, cwd and seated workers, with the seat stated **before**
the routing detail — pinned by `the seat is stated before the recipe`.

### Why the record and not `ORC_WORKERS`

`ORC_WORKERS` is built before any pane spawns and encodes only
`pane_id=harness`. When a worker dies the daemon writes `state: "stopped"` into
the session record, but the env var is frozen. A hook trusting it would offer a
dead worker. The seat prefers the record and falls back to the environment,
saying which it used:

```
  ok   record seat learns worker state
  ok   a stopped worker is not offered
  ok   an unreadable record still yields a seat
  ok   and says the seat is the environment's account
```

`pio session show [<id>] --json` is the one read-only call that answers it,
defaulting to `$ORC_SESSION` so a pane need not know its own id. `pio session
list --json` would have worked without new Rust, but it parses every session on
the machine (43 here, 42 KB) and has no unknown-session semantics.

`pio session create` was **not** an option for the hook to call: it invokes
`load_harness_registry()`, which writes a default `harnesses.json` when absent.
The hook stays on the read-only side.

---

## Check 3 — standalone is unchanged, and the two are told apart by the environment

```
delegate: — one bounded hand-off to one worker.
  You are NOT inside a pi-orchestra session (no $ORC_SESSION), so
  create one first — this is the standalone path only.
  …
    pio session create --brain claude --worker <harness>   # once; note the id
```

Both paths covered by `--selftest` (50 checks, all passing), including the one
that matters most:

```
  ok   standalone still creates a session
  ok   seated context never teaches session create
  ok   a worker pane reports the worker role
```

The last is there because **both roles get the same `ORC_SESSION`** — worker
panes carry it too — so "`ORC_SESSION` is set" does not mean "I am the
conductor". The role comes from the record, never from an assumption.

### Two more places carried the same bug independently of the hook

- `skills/orchestrate/SKILL.md` **exported a new `ORC_SESSION` over the one it
  was given** (`export ORC_SESSION="orch-$(date …)"`), unconditionally.
- `skills/pi-delegate/SKILL.md` and `codex/AGENTS-block.md` taught
  `pio session create` with no environment guard.

Fixing only the hook would have left a seated Codex brain, or a Claude brain
that reached the skill by another route, doing exactly what the issue reports.

---

## Check 6 — every description names its own trigger token

Skill selection reads the `description:` field and nothing else. `delegate:`
appeared only under a body heading, so `pi-delegate` competed on task *shape*
against `superpowers:dispatching-parallel-agents` ("2+ independent tasks") and
lost — spawning four Claude Code subagents instead of delegating.

The recon corrected the issue's table on one point: `orchestrate` named its
keyword **without** the colon; only `deliberate` carried the full token. All
three now carry keyword + colon, the same thing `orc_pty::trigger` matches.

`orc-cli/tests/trigger_grammar.rs` reads only the YAML frontmatter — matching
anywhere in the file is what made this invisible, since every skill mentions
its verb in prose. The colon is load-bearing: without it the test would pass on
the directory name `pi-delegate` and gate nothing, which the second test pins.

---

## Checks 4, 5, 10 — an install that says whether the grammar works

`install.sh` linked the hook, printed a snippet, and ended with `done.` — while
`delegate:` did nothing, because registering it means editing
`~/.claude/settings.json`. Not editing the user's file is defensible. Ending on
`done.` while the headline gesture is inert is not.

### `pio doctor` reports it, and the exit code carries the verdict

```
TRIGGER GRAMMAR (✓ wired · ✗ inert — `delegate:` needs all three)
✗ FAIL  hook installed    …/.claude/pi-orchestra/claude-userpromptsubmit-hook.py does not exist
           fix: run ./install.sh to link it
✗ FAIL  hook registered   …/.claude/settings.json does not exist
           fix: add this to …/.claude/settings.json — pi-orchestra never edits it
                for you, or run ./install.sh --wire-claude-hook …
✗ FAIL  skills installed  …/.claude/skills: missing pi-delegate, orchestrate, deliberate
           fix: run ./install.sh; it replaces dead symlinks and never overwrites your own files

`delegate:` / `orchestrate:` / `deliberate:` will NOT fire until the failures above are fixed.
exit=1
```

and on this machine, which is wired:

```
✓ ok    hook installed    /Users/comreton/.claude/pi-orchestra/claude-userpromptsubmit-hook.py
✓ ok    hook registered   /Users/comreton/.claude/settings.json calls it on UserPromptSubmit
✓ ok    skills installed  /Users/comreton/.claude/skills has all pi-delegate, orchestrate, deliberate
exit=0
```

A **dangling** symlink is reported apart from a missing file, because they need
opposite advice and the dangling case is the one that actually bit: everything
`install.sh` links is an absolute symlink into the checkout, so moving the
checkout silently stops the grammar firing (`findings.md`, and `progress.md`'s
own 2026-07-30 entry records the skills links still pointing at the stale
checkout right now).

Adding this made the existing doctor tests non-hermetic — they read the
developer's real `~/.claude` and their exit code would depend on whether *they*
had registered the hook. `run()` now fakes `$HOME`.

### `--wire-claude-hook` — opt-in, backed up, idempotent

Against a settings.json holding the user's own UserPromptSubmit hook and an
unrelated `Stop` hook and a `theme` key:

```
    registered in …/settings.json (backup: …/settings.json.pi-orchestra.bak)
    Claude Code   WIRED — skills + live hook
```

Both survived; ours was appended. Second run:

```
    already registered in …/settings.json
checksum before=78fbfc566615e13a2c60f920c6c735e3ff33a5023f896814d6132534fe7916b5
checksum after =78fbfc566615e13a2c60f920c6c735e3ff33a5023f896814d6132534fe7916b5
IDEMPOTENT: byte-identical
```

Then `pio doctor` → exit 0. The loop closes.

### Check 10 — reported, not guessed

`pio doctor` marks four harnesses conductor-capable; only two have an
integration surface we can write to. The installer now ends with:

```
==> trigger grammar (delegate: / orchestrate: / deliberate:)
    Claude Code   skills linked, hook NOT registered
                    `delegate:` will NOT fire until it is. Re-run: ./install.sh --wire-claude-hook
    [the exact snippet]
    Codex         WIRED — static block in ~/.codex/AGENTS.md
                    static text only: no live quota relay and no session context
    Pi/MiniMax    NOT wired
                    pi has no integration pi-orchestra installs. Paste skills/pi-delegate/SKILL.md …
    OpenCode      NOT wired
                    opencode has no integration pi-orchestra installs. …
    Hermes        worker only — needs no trigger grammar

    Verify any time with: pio doctor   (exit 1 while the grammar is inert)
```

**Pi and OpenCode are reported rather than wired, deliberately.** Neither
`~/.agents/skills` nor `~/.config/opencode/skill` exists on this machine to
probe, and writing into a config path whose convention I could not verify is
exactly the claim AGENTS.md forbids: *never claim a capability that wasn't
probed*. Check 10 explicitly permits this branch. Wiring them properly wants
its own issue, with each harness's documented instruction path confirmed first.

---

## Check 7 — `--json` answers in JSON on every outcome

Before: `ISOLATION REQUIRED` on stderr, exit 1, **stdout empty**. A caller
promised JSON got nothing to parse and no way to tell failure from silence —
which is how a six-second success came to be reported as silence.

```json
{
  "verb": "orch_delegate",
  "ok": false,
  "error": {
    "reason": "isolation_unavailable",
    "message": "ISOLATION REQUIRED: contracted task T0001 worktree is unavailable (ISOLATION UNAVAILABLE: session cwd is not a Git work tree). A contracted task takes an isolated worktree, so it needs a Git work tree; delegate from inside a repository, or use an uncontracted task (no --objective/--check), which needs none."
  }
}
```

The reason words are the ones the system already speaks: `isolation_unavailable`
is a durable board history action, and `WORKER UNAVAILABLE` / `UNKNOWN HARNESS`
are dispatch's own refusals. One vocabulary, not a second that only the envelope
knows.

Two tests, because a failure has two genuinely different shapes:

- an **error**, which aborts before an outcome exists → the envelope above;
- a **recorded failure**, where dispatch persists a failed record and reports it
  inside a normal outcome (unknown harness) → still JSON, still non-zero.

Only the first was broken. A test pinning only the first would let the second
silently regress into a bare stderr line. Writing that test corrected my own
assumption: I first asserted an unknown harness produced the envelope, and it
does not — the board keeps the receipt, which is better behaviour than I had
assumed, so the test now pins what is actually true.

---

## Check 8 — the isolation precondition

The issue offers "the recipe succeeds from a non-git cwd" **or** "the
precondition is stated everywhere the recipe appears and a trivial no-file-change
task has a documented path needing no worktree". **The second**, because the
first means either an isolation opt-out flag — new delegation capability, which
the issue puts out of scope — or making contracted tasks skip isolation, which
is the guarantee `#11` exists to provide.

The failure was also opaque. `materialize_worktree` swallows its cause into a
task state, so the refusal arrives at dispatch time as "worktree is
unavailable" with the actual reason — `session cwd is not a Git work tree` —
stored but never surfaced. It is now included, along with the way out.

Stated in all five places the recipe appears: both hook blocks (seated and
standalone), `skills/pi-delegate/SKILL.md`, `codex/AGENTS-block.md`, and the
error itself. The no-worktree path is `pio task add` → `assign` → `start` →
`pio dispatch send` — uncontracted, so `requires_isolation` is false.

---

## Check 9 — every instruction names how to collect the result

`orch delegate` returns after *delivery* by design (#30). The hook said so but
never named `orch_await` in the same breath, so a six-second success was
reported as silence. Every block that starts work now ends with how to finish
it, in the same paragraph.

And the id problem — the issue records a conductor guessing `T-hello`, then
`T0002`:

```
$ pio dispatch send T9999 fake-worker "say hello" --session <S>
missing dispatch task T9999; list this session's tasks with `pio task list --session <S>`
```

The session id is interpolated so the suggestion is copy-pasteable. Pinned for
both a plausible id and a malformed one.

---

## Check 11 — the highlight no longer implies a capability that is not wired

The in-pane highlight is pure text analysis. Its entire input is `pane.role` and
`pane.cells`; there is no capability field anywhere in `PaneSnapshot`. So it lit
up `delegate:` whether or not anything was listening — and on the reporting
machine nothing was.

Taking the first branch (reflect real state). `attach_stage` probes the same
`trigger_grammar` doctor reports, and the badge becomes:

```
· ○ DELEGATE INERT
```

`○` is the register's existing `Glyph::Unavailable`, resolved through the
tier-aware accessor so ASCII terminals get the ASCII form. The shimmer — the
part that reads as *live* — does not run. The token keeps its bold and stays on
screen: the conductor did type the word, and unavailable is shown, never hidden.

Pinned by `an_unwired_grammar_is_badged_inert_instead_of_looking_live`, which
renders both states and asserts they are two genuinely different frames, that
the inert one never wears the live marker, and that the spell is still painted
and still emphasised.

---

## Check 12 — all five gates

```
cargo fmt --all -- --check                                    OK
cargo clippy --workspace --all-targets -- -D warnings         0 warnings
cargo test --workspace                                        318 passed, 0 failed
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps     0 warnings
cargo build --release --locked                                Finished
```

Baseline on `main` @ `17f0942` was green before any change.

**One flake seen and attributed, not waved away.** An early full-workspace run
hit `background_dispatch.rs::delegate_confirms_while_running_and_cap_one_queues_until_real_exit`
at its `started.elapsed() < 1s` assertion. It went 3/3 in isolation, and LOG.md
already documents it twice — *"the documented `background_dispatch` sub-1s
flake, which then went 5/5 in isolation"* (line 160) and again at line 298. It
is a wall-clock bound on a subprocess spawn under parallel load; nothing on this
branch touches that timing path. Subsequent full runs: clean.

A second early failure was **mine**, in the new check-1 test, and is fixed
rather than excused — see check 1.

---

## Deviations from the contract

- **`uninstall.sh` is not in the allowed paths.** Now that `install.sh` can wire
  `settings.json`, `uninstall.sh` should be able to unwire it — removing only
  the `UserPromptSubmit` entry whose command is ours, backed up, leaving every
  other hook alone. I did not improvise outside the contract. **Follow-up
  issue.**
- **`README.md` is not in the allowed paths.** Its line 98-100 promise about
  never editing protected config, and its line 150 pane-env paragraph, both want
  a sentence updating. Same call. **Follow-up issue.**
- **`orc-daemon` is not in the allowed paths**, and does not need to be. The
  fix needs no new env var: session and pane id come from the environment the
  daemon already sets, and role/harness/state/cwd come from the record. Worth
  recording that `ORC_DELEGATE_HINT` there still says `orc task` / `orc dispatch
  send` — pre-rename vocabulary handed to every pane. **Follow-up.**
