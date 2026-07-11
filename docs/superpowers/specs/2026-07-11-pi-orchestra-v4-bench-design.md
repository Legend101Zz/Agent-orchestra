# pi-orchestra v4 — the Bench: multi-harness orchestration workspace

**Date:** 2026-07-11 (rev 3 — TUI-only flagship, client-server daemon, Python
deleted, worktree-per-task in scope; supersedes rev 2's Tauri pivot per user
decision)
**Status:** Proposed — awaiting explicit user approval
**Branch after approval:** `v4-bench`, created from `main` (`v3-rust` was
merged via PR #1 *before* the fix-first items landed, so they open Phase 0)
**Prereq:** the three ordered fixes in `docs/reviews/2026-07-11-v3-rust-review.md`
(quota timeout, steering turn boundary, TUI quota refresh) — now live on
`main`, to be fixed as the first `v4-bench` commits

## Product position

pi-orchestra today is a delegation registry with a control-plane TUI: brains
(Claude Code / Codex) shell out to `orc`, workers run headless, and `orc top`
observes. The operator still lives in N disconnected terminal windows and
reconstructs the swarm in their head.

v4 changes the product contract to:

> Run every harness of a working session — one brain, N workers — inside a
> single pi-orchestra terminal window; watch instructions flow between them as
> living connections and tasks burn down a board; survive crashes of the UI
> *and* of the brain; and never get between a harness and its own I/O.

The origin idea is the [advisor tool pattern](https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool)
inverted: an expensive brain dispatches cheap executors, and pi-orchestra is
the *inter-harness* coordination fabric no single harness provides.

## Presentation decision (rev 3): a TUI that earns "how is this a terminal?"

The user's call: **terminal only** — no Tauri, no browser — but the UI must
feel fast, stable, and genuinely delightful, not AI-slop and not stock
ratatui. So the question becomes: what is the best Rust base, and what
actually makes a TUI feel amazing?

### Framework verdict — keep ratatui, replace the application architecture

| Candidate | Verdict | Why |
|---|---|---|
| **ratatui** + ecosystem | **Rendering engine** | Cell-diff rendering, mature widgets, [tachyonfx](https://github.com/ratatui/tachyonfx) (ratatui-org effects library: 50+ shader-like cell effects, easing timers, spatial patterns), braille Canvas for sub-cell curves, tui-term precedent for PTY panes. The v3 console proves we can make it look non-stock. |
| cursive | Rejected | Curses-era model, weaker styling/animation ecosystem. |
| notcurses (C) bindings | Rejected | Famous for TUI graphics, but Rust bindings are poorly maintained; unacceptable foundation risk. |
| iocraft / rooibos etc. | Rejected | Young, small ecosystems; nothing they add that we need. |
| Building on/inside zellij | Rejected as host; **adopted as architecture** | We don't want zellij's chrome or plugin sandbox — but its client-server model is exactly right (below). |

`orc top` in its current form is retired as the flagship: it is a synchronous
poll-every-2s instrument panel, structurally incapable of animation, embedded
terminals, or crash-survival. Its reviewed visual language (themes, glyph kit,
attention-first dashboard) is *ported into* the new client as the RUNS view;
the `orc top` command becomes an alias that opens the new client on that view.

### The architecture that makes it stable: client-server (zellij model)

```
┌ pi-orchestra (client, ratatui) ──────────────┐      ┌ orcd (daemon, per user) ─────────────┐
│ render @ adaptive fps · input · themes       │◄────►│ owns PTYs (portable-pty)             │
│ HOME · STAGE · SCORE · RUNS                  │ unix │ vt state per pane (screen replay)    │
│ crash/quit = detach, nothing dies            │ sock │ session state · event bus            │
└──────────────────────────────────────────────┘      │ fs watcher over ~/.orchestra         │
   orc CLI (headless brains, skills) ────────────────►│ tails registry runs (bounded)        │
                                                      └──────────────┬───────────────────────┘
                                                                     │ spawns/observes
                                                       brain pty · worker ptys · orc runs
```

- **`orcd`** (new crate) owns every interactive PTY and the canonical vt
  screen state per pane, exactly like zellij's server: the client renders
  from daemon state over a Unix domain socket. Detach (`ctrl-\` or just
  closing the terminal) leaves everything running; `pi-orchestra attach`
  reconnects and replays screen state. Client crash ≠ session death — this
  is the stability guarantee, structural, not aspirational.
- The daemon starts on demand (first `pi-orchestra` invocation), one per
  user, socket at `~/.orchestra/orcd.sock`, logs via `tracing` to
  `~/.orchestra/orcd.log`. It does no rendering and no policy — PTY hosting,
  vt parsing, event fan-out, bounded log tailing. `orcd` failure is designed
  for too: panes are process-group children it can reap on restart via pid
  records, and registered `orc run` workers never depended on it anyway.
- Multiple clients may attach (a laptop screen and an external monitor see
  the same session at different sizes, zellij-style).

### Graceful brain death (user Q1 — now a core feature)

Two distinct failures, both handled:

1. **UI dies / user closes the window** → nothing happens to the session.
   Panes live in `orcd`; reattach and continue. (This was rev 2's punted
   "detach/reattach"; the daemon makes it foundational instead.)
2. **The brain process itself exits** (crash, OOM, accidental `exit`) → the
   session does not end. Workers keep running (they never depended on the
   brain process). The session enters a `CONDUCTOR DOWN` attention state:
   the stage shows the dead pane with its last screen, elapsed-since-death,
   and one-key recovery — `R` respawns the brain using the harness's
   configured `resume_args` (`claude --resume`/`--continue`, codex/hermes
   equivalents recorded in the harness registry), landing the operator back
   in the same brain conversation where the harness supports it. The
   registry, board, and inbox are durable, so the resumed brain re-orients
   from `orc task list` + `orc list` — the skills gain a "re-orientation on
   resume" note. No orchestration state ever lives only in a process.

### What "amazing" means in cells (and how it stays fast)

The terminal emulator (Ghostty, kitty, Alacritty, WezTerm) is our GPU; the
job is to feed it frames worth rendering:

- **Sub-cell geometry.** Baton lines are cubic Béziers plotted at braille
  resolution (2×4 dots per cell — ratatui's Canvas braille marker), giving
  visibly smooth curves, not staircases. Card elevation uses layered
  half-block/quadrant shading; corners use arc glyphs (`╭╮╰╯`); focus uses an
  edge-light gradient, not a color flip.
- **Real motion.** tachyonfx drives pulses of light traveling the baton
  curves on registry events (dispatch, steer, complete, handoff — each with
  its own pulse shape/tempo), staged easing when a session opens (cards
  settle in), and dissolve/sweep transitions between views. Motion budget is
  concentrated there; everything else animates only on state change. A
  reduced-motion config collapses pulses to state flashes.
- **Tear-free.** Every frame is wrapped in synchronized-output
  (`BeginSynchronizedUpdate`/`EndSynchronizedUpdate`, DEC 2026) so grids of
  live terminals never shear mid-redraw.
- **Adaptive frame clock.** Event-driven redraw at rest (0 fps idle — a
  quiet session costs ~0 CPU); the animation clock runs at 60 fps only while
  an effect or PTY frame is in flight. Input is echoed to the focused pane on
  its own path with a sub-16 ms budget, never queued behind rendering.
- **Honest budgets, measured.** Frame time, input latency, and daemon
  memory are benchmarked (criterion + scripted measurements) and recorded in
  the README the same way v3 recorded CLI benchmarks — measured, not asserted.
- **Kitty keyboard protocol + bracketed paste + mouse passthrough** are
  negotiated and forwarded verbatim to the focused pane so harness TUIs keep
  their full input fidelity (spike verifies against all four harnesses).

## Research: prior art and what we take from each

| Source | What it proves | Take | Reject |
|---|---|---|---|
| [Advisor tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool) | Two-tier model pairing is an endorsed pattern | Brain/worker framing, honest cost receipts | Server-side coupling |
| [zellij](https://zellij.dev/features/) | Rust client-server terminal workspace: detach, multi-client, floating panes, session resurrection | The architecture wholesale; serialized-layout resurrection idea | Its chrome, keybinding culture, WASM plugin surface |
| [BridgeSpace](https://www.bridgemind.ai/products/bridgespace) | 1–16 terminals + kanban beside them is a shippable product | Terminals-beside-board, workspace-per-project | Being a GUI |
| [Vibe Kanban](https://github.com/BloopAI/vibe-kanban) | Kanban as the agent command center; worktree-per-task isolation | Board-centric UX; **the worktree model (user Q4: in scope)** | Web UI, Node server |
| [claude-squad](https://github.com/smtg-ai/claude-squad) | Harness-agnostic launcher via configurable commands + worktrees | Configurable harness commands | tmux as engine |
| Claude Code [agent teams](https://code.claude.com/docs/en/agent-teams) | Shared task list + mailbox + lead/teammate | File-based task semantics (claim, depends_on, locking) | Single-harness lock-in |
| [fulsomenko/kanban](https://github.com/fulsomenko/kanban) | Serious ratatui task domain | Card anatomy, T-prefix ids, atomic JSON + watching | Sprints, SQLite, undo |
| [hermes-agent](https://github.com/nousresearch/hermes-agent) | Multi-provider agent CLI, subagents, active project | First-class harness, **default worker** with pi | Gateway/messaging surface |
| [tachyonfx](https://github.com/ratatui/tachyonfx) | Shader-like effects are practical in ratatui at scale | The entire motion layer | Using effects decoratively |
| [tui-term](https://github.com/a-kenji/tui-term)/portable-pty | PTY panes in ratatui work | portable-pty; vt-parser bake-off (`vt100`, `termwiz`, `alacritty_terminal`) | Its experimental controller |

## Non-negotiable constraints

- All implementation commits land on `v4-bench`; merge to `main` only at
  explicit gates; never force-push.
- Never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`. Installer appends only marked blocks with backups.
- Registry/sessions/tasks stay plain JSON/text with atomic
  temp+fsync+rename writes and additive-field tolerance. (Python is deleted —
  user Q2 — but the *format* contract survives via golden fixtures; external
  tools and old data keep working.)
- Quota gate fails open on transport problems; every network/subprocess call
  is bounded by a timeout.
- **Single-writer rule:** the client never writes registry/task files; every
  mutation (launch, kill, steer, task move, merge) goes through orc-core
  command paths, recorded with its actor (`brain` | `human`).
- **Prime directive:** never hinder a harness. Focused panes get verbatim
  input; chrome never overlays pane cells; no API-traffic proxying;
  coordination is filesystem-only.
- No emojis anywhere; every color from theme tokens; state as words; the two
  themes are **ember and phosphor only** (user Q5) — no third theme, ever, in
  v4.

## Engineering standards (user-mandated: good code, docs, tests)

- **Docs:** every public item and every module carries rustdoc explaining
  purpose and invariants (`#![warn(missing_docs)]` on all crates);
  `cargo doc --no-deps` builds warning-free as a CI gate; each crate has a
  README-level module doc stating what it owns and what it must never do.
- **Tests:** TDD for core logic (failing test first). Unit tests per module;
  integration tests drive the real binaries (fake-pi and fake-hermes become
  small Rust test helpers); golden fixtures captured from today's Python
  output *before deletion* pin the registry/CLI JSON contract forever;
  daemon protocol gets round-trip tests; UI snapshot tests via ratatui's
  TestBackend for every view in both themes at wide and 72×30 sizes.
- **Quality gates per phase:** `cargo fmt --check`, `cargo clippy
  --all-targets -- -D warnings`, `cargo test`, `cargo doc` warning-free,
  benchmarks re-run when a hot path changes. No `unwrap`/`expect` in orcd or
  orc-core outside tests (enforced by clippy lint config); errors are typed
  (`thiserror`) in libraries, contextual (`anyhow`) in binaries; `tracing`
  spans in the daemon.
- **Stability:** bounded memory everywhere (scrollback caps, tail windows),
  no busy loops, adaptive frame clock verified idle-quiet, daemon soak test
  (hours-long session with flooding pane) before Phase 2 closes.

## Architecture (system view)

### Control plane vs data plane

Unchanged from rev 2 in substance: the **data plane** is each harness talking
to its own provider — orc never sits in that path. The **control plane** is
the filesystem (`~/.orchestra/{runs,sessions,config}`): brains call `orc`
(via skills / AGENTS blocks), which writes runs, inbox messages, and tasks;
`orcd` watches and fans out events; the client renders. Panes launched from
the stage export `ORC_SESSION` and `ORC_PANE_ID`, so runs created inside them
carry their origin — baton lines are derived from registry linkage, never
sniffed. Headless use (a brain in a plain terminal saying "orchestrate")
keeps working with no client at all.

### Crate layout

```
rust/crates/
├── orc-core/      registry, quota, runner + adapter seam, tasks, sessions,
│                  worktrees, metrics, search, notifications  (exists, grows)
├── orc-cli/       `orc` binary: run/rpc/send/retry/handoff/task/session/…  (exists, grows)
├── orc-daemon/    new: `orcd` — PTY host, vt state, event bus, socket protocol
├── orc-proto/     new: client↔daemon message types (serde), versioned
├── orc-pty/       new: portable-pty wrapper + vt parser choice + input encoder
└── orc-app/       new: `pi-orchestra` client — views, compositor, motion layer,
                   themes (ports v3 orc-tui's theme tokens, glyph kit, dashboard)
```

Dependency floor: existing core deps + `portable-pty`, one vt parser (bake-off
in the spike), `tachyonfx`, a Unix-socket IPC layer (std or `interprocess`),
`notify` for fs watching, `tracing`. Still no tokio unless the daemon's
concurrency genuinely demands it over threads — the spike decides and records
why.

### Sessions, tasks, worktrees

Session and task models carry over from rev 2 (file-per-task, statuses
backlog/assigned/running/review/done/dropped, `depends_on`, `assignee_run`,
actor-attributed `history`), plus **worktree-per-task (user Q4)**:

- `orc task add --isolate` (or session default `"isolation": "worktree"`)
  creates a git worktree at `~/.orchestra/worktrees/<session>/<task-id>` on
  branch `orc/<session-slug>/<task-id>`; the assigned worker's cwd is the
  worktree, so parallel workers physically cannot trample each other.
- REVIEW column = the brain (or human) reviews the worktree diff
  (`orc task diff <id>`); `orc task merge <id>` squash-merges to the session's
  base branch and prunes the worktree; `drop` prunes without merging. All
  through the CLI, actor recorded; the client only invokes.
- Non-git cwds degrade gracefully: isolation unavailable, stated plainly.

### Harness registry and worker defaults

As rev 2, with `resume_args` added for conductor-down recovery:

```json
{
  "harnesses": {
    "claude": {"cmd": "claude", "args": [], "resume_args": ["--continue"], "roles": ["brain","worker"], "adapter": "claude"},
    "codex":  {"cmd": "codex",  "args": [], "resume_args": ["resume"], "roles": ["brain","worker"], "adapter": "codex"},
    "hermes": {"cmd": "hermes", "args": [], "resume_args": [], "roles": ["brain","worker"], "adapter": "hermes"},
    "pi-m3":  {"cmd": "pi", "args": ["--provider","minimax","--model","MiniMax-M3"], "roles": ["brain","worker"], "adapter": "pi"}
  },
  "default_workers": ["hermes", "pi-m3"],
  "max_parallel_workers": 3,
  "app": {"leader_key": "ctrl-g", "reduced_motion": false, "theme": "ember"}
}
```

**The default worker pool is hermes + pi (MiniMax-M3); the choice is always
the user's** — claude, codex, or any configured command works. Worker
adapters (capability flags `steerable`, `exact_usage`; honest degradation)
as rev 2: pi is the reference; hermes is the first new adapter after local
shape verification (`hermes --help`); claude (`claude -p --output-format
stream-json`) and codex (`codex exec --json`) best-effort.

### Python deletion (user Q2)

Full removal in Phase 2, safely ordered: (1) capture golden fixtures — CLI
JSON output, registry round-trip corpora including legacy/corrupt/CJK metas —
from the Python implementation while it still runs; (2) port the fake-pi
integration suite to Rust test helpers; (3) delete `orc_pkg/`, `.venv`,
`pyproject.toml`, `requirements.txt`, pytest plumbing; (4) `install.sh`
becomes Rust-only (build + symlink), `uninstall.sh` updated. The compatibility
contract survives as fixtures, not as a second implementation.

### Remote access (user Q3 — decision made)

**SSH is the remote story; no web server.** The daemon makes this free:
`ssh box` then `pi-orchestra attach` gives the full experience, because a TUI
renders wherever your terminal is (this is the one place a TUI beats every
GUI). Works with mosh for flaky links. No `orc serve`, no browser surface, no
extra attack surface. Documented in the README as the recommended pattern.

### Trigger word

As rev 2: skills + `codex/AGENTS-block.md` learn "pi-orchestra" as an alias
trigger, `orc task` board maintenance, worker-pool choice (offer
`default_workers`, never assume), `ORC_PANE_ID` awareness, a re-orientation
note for resumed brains, the five v3-review wording fixes, and a hermes block
if local inspection shows hermes reads an AGENTS.md-equivalent.

## UI/UX direction

Subject world: the orchestra pit. Conductor (brain), players (workers), the
score (board), the stage (workspace). Two committed themes: **ember** (warm
charcoal stage, brass/amber live-states, bone text, oxblood attention) and
**phosphor** (CRT monochrome with semantic exceptions) — both already exist
as token systems and extend to the new surfaces. Anti-slop commitments hold:
no stock-ratatui uniform bordered boxes, state always written as words,
typography via glyph weight/spacing hierarchy (a TUI's typography is its
spacing discipline), empty states that teach real commands.

**HOME — the season.** Session cards (double-height rows with topology
glyph, receipts, attention badges); `n` new session: brain → worker pool
(hermes + pi-m3 preselected) → cwd; `enter` attach.

**STAGE — the flagship.** Floating pane cards over a quiet stage backdrop —
zellij proves floating panes in cells work; ours are arranged in **ensemble
layout** (conductor left-of-center, players arced right) with drag (mouse) /
swap (keys), resize, zoom-to-solo, per-session persistence. Cards: arc-glyph
corners, half-block shadow, header rail (harness, run id, status word, live
token/cost ticker), brass edge-light on focus. Between conductor and players:
**baton lines** — braille Béziers under the cards, dim filaments idle, light
pulses traveling on real events, labels settling into the timeline.
Leader key `ctrl-g` (double-tap sends literal); all other input verbatim to
the focused pane.

**SCORE — the board.** Columns Backlog / Assigned / Running / Review / Done;
cards with T-id, title, worker chip, status word, exact-or-`~` tokens,
isolation mark when the task has a worktree, history popover (actor-attributed
moves). Keyboard moves + mouse drag both invoke `orc task move`. `g` flies to
the assignee's stage card. Review column carries diff stats for worktree
tasks (`+412 −88 · 9 files`).

**RUNS — the ledger.** The v3 instrument panel (quota fuel gauges, sparkline,
attention-first table, search, session timeline) ported into the client, now
event-driven instead of poll-rebuilt.

View cycling with `V`, help with `?`, settings with `,` — the key language v3
users already know.

## Phasing

**Phase 0 — fix-first.** Review fixes 1–3 (+6) as the first commits on
`v4-bench` (v3-rust already merged to `main` unfixed via PR #1); full
existing gates (pytest still exists here, cargo suite, live smoke 10/10).

**Phase 1 — spike (go/no-go).** Prototype: `orcd` skeleton hosting two PTYs
(Claude Code + hermes, live), client rendering both panes via the chosen vt
parser (bake-off: `vt100` vs `termwiz` vs `alacritty_terminal`), one braille
Bézier with a tachyonfx pulse, synchronized-output frames, adaptive clock.
Measure: redraw fidelity for full-screen harness TUIs, input latency, CPU at
4 panes + flood, detach/reattach with screen replay, kitty-keyboard/paste/
mouse passthrough. Record hermes's headless shape for Phase 5. Verdict to
`docs/notes/2026-07-11-tui-spike.md`. **Fallback if PTY embedding fails
fidelity:** companion mode — the brain stays in the user's own terminal; the
client shows workers, board, and flow (registered runs never needed PTYs).
Do not grind silently; write the pivot down.

**Phase 2 — daemon + client shell + HOME + STAGE + Python deletion.**
Productionize orcd (socket protocol in orc-proto, multi-client attach,
tracing, soak test), client compositor + floating cards + ensemble layout +
focus/leader routing, HOME + three-step new session, pane↔run attribution,
conductor-down basic recovery (respawn with `resume_args`), golden-fixture
capture → Python fully deleted → Rust-only installer with `pi-orchestra`
launcher (`orc top` aliases to the client's RUNS view).

**Phase 3 — tasks + worktrees + SCORE + skills.** tasks.rs + `orc task`
(add/assign/start/review/done/drop/move/diff/merge, `--isolate`,
actor-attributed), worktree lifecycle (create/branch/diff/squash-merge/prune,
non-git degrade), SCORE view with keyboard + mouse drag moves, board↔stage
flying link, skills/AGENTS updates (trigger alias, board maintenance, worker
pool, resume re-orientation, five review fixes, hermes block if applicable).

**Phase 4 — baton lines + RUNS + polish.** Event stream → directional pulses
with per-kind shape/tempo; session-open settle animation; view transitions;
reduced-motion path; RUNS port; frame/latency benchmarks recorded in README.

**Phase 5 — adapters + docs + dogfood.** hermes adapter first (from Phase 1
notes; interactive-pane degrade if headless is absent — never fake exact
usage), claude/codex best-effort. README + guide rewrite; VHS captures of
HOME/STAGE/SCORE. **Dogfood gate:** build at least one Phase 5 deliverable by
saying "orchestrate" with the brain on the STAGE and tasks on the SCORE
(worktree-isolated); friction log in `docs/notes/` is a deliverable.

Cut order under pressure: claude/codex adapters → view transitions and
settle animations (keep event pulses) → mouse drag on STAGE (keyboard swap
stays) → RUNS extras beyond the v3 port. Never cut: Phase 0, the daemon and
detach/reattach, conductor-down recovery, passthrough integrity,
single-writer mutations, actor-attributed moves, worktree isolation, the
hermes+pi default pool, fail-open quota, baton lines (static degrade
acceptable; absence is not), the engineering standards.

## Risks

1. **vt parsing fidelity for full-screen harness TUIs** — the existential
   risk; spike with the real four harnesses, named fallback (companion mode).
2. **Daemon correctness** (orphaned PTYs, socket protocol versioning, replay
   consistency) — mitigated by zellij's proven blueprint, protocol
   round-trip tests, pid records + reap-on-restart, soak test.
3. **Effect-driven CPU burn** — adaptive clock is a hard requirement with an
   idle-quiet verification test; tachyonfx effects are budgeted per frame.
4. **Worktree edge cases** (dirty base, submodules, non-git cwd) — degrade
   with plain words; never auto-resolve conflicts; merge is always explicit.
5. **hermes adapter unknowns** — headless/usage shape unverified; inspect
   locally first, degrade honestly.
6. **Losing the Python oracle** — golden fixtures captured before deletion
   pin the contract; cross-language parity becomes fixture parity.

## Resolved decisions (from user review)

1. Brain death → daemon + conductor-down recovery (core, Phase 2/4). ✔
2. Python → fully deleted in Phase 2, fixtures first. ✔
3. Remote → SSH + `pi-orchestra attach`; no web server. ✔
4. Worktree-per-task → in scope, Phase 3. ✔
5. Themes → ember + phosphor only. ✔

## Approval gate

No implementation, branch, or swarm until the user approves this revision.
After approval: create `v4-bench` from `main` (which already carries this
document), land Phase 0 fix-first as its opening commits, then the Phase 1
spike before any feature work.
