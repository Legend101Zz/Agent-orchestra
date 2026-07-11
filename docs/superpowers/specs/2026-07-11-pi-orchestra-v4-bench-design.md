# pi-orchestra v4 — the Bench: multi-harness orchestration workspace

**Date:** 2026-07-11 (rev 2 — presentation layer pivoted from ratatui to a Tauri
desktop app after user review; worker defaults changed to hermes + pi)
**Status:** Proposed — awaiting explicit user approval
**Branch after approval:** `v4-bench`, created from `main` after the fix-first
items land on `v3-rust` and it merges to `main`
**Prereq:** the three ordered fixes in `docs/reviews/2026-07-11-v3-rust-review.md`
(quota timeout, steering turn boundary, TUI quota refresh)

## Product position

pi-orchestra today is a delegation registry with a control-plane TUI: brains
(Claude Code / Codex) shell out to `orc`, workers run headless, and `orc top`
observes. The operator still lives in N disconnected terminal windows and
reconstructs the swarm in their head.

v4 changes the product contract to:

> Run every harness of a working session — one brain, N workers — inside a
> single pi-orchestra window; see the instruction flow between them as living
> connections and the task board they are burning down; and do all of it
> without ever getting between a harness and its own I/O.

The origin idea is the [advisor tool pattern](https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool)
inverted: instead of a cheap executor consulting an expensive advisor
server-side, an expensive brain dispatches cheap executors — and pi-orchestra
is the *inter-harness* coordination fabric that no single harness provides,
because each harness (Claude Code subagents/agent teams, Codex, hermes
subagents) only orchestrates within itself.

## Presentation layer decision (the rev-2 pivot)

The user's bar: floating cards, smooth glowing Bézier connectors between them,
shadows, rounded panels, draggable kanban — a UI that feels like a polished web
app, not a character grid. A ratatui TUI physically cannot render that; apps
like Claude Code, lazygit, and k9s only *simulate* polish through spacing and
color within terminal cells. Something that genuinely draws
`card ── smooth glowing curve ── card` is a browser UI, a desktop WebView, a
custom GPU renderer, or terminal pixel-graphics protocols. Options weighed:

| Option | Verdict | Why |
|---|---|---|
| **A. Tauri 2 desktop app** (Rust backend + WebView frontend, xterm.js terminals) | **Chosen** | Exactly BridgeSpace's stack; [tauri-plugin-pty](https://github.com/Tnze/tauri-plugin-pty) and reference apps prove xterm.js + portable-pty over Tauri IPC works; Vibe Kanban proves Rust-backend + web-frontend for agent orchestration; full CSS/SVG freedom for cards, connectors, drag-and-drop; the app owns its window, so browser chrome can't steal keystrokes from a live harness |
| B. Local web server + browser tab (`orc serve`) | Deferred to P2 | Same frontend could be served later for remote use, but a browser tab hosting an interactive brain is a UX hazard (`cmd+w` kills your session, shortcuts collide) and it never feels like an app |
| C. Custom GPU-rendered terminal/UI (wgpu/gpui) | Rejected | Warp/Zed-class engineering effort; unjustifiable for this project |
| D. Kitty graphics protocol inside the TUI | Rejected | Static pixels in cells; not interactive, not draggable, terminal-dependent |

**The ratatui `orc top` console is kept, frozen as the SSH/fallback control
plane.** It is already built, reviewed, and reads the same files; it gets bug
fixes and task-list awareness but no new surfaces. The flagship UI is the app.
The frontend talks to the backend through a thin transport trait over Tauri
IPC so option B (serving the same UI over WebSocket) stays cheap later.

## What exists (do not rebuild)

- **v3 Rust core** (`rust/crates/orc-core`): registry (atomic JSON, tolerant
  models, orphan reconcile), quota gate, pi json/rpc runner with steering
  inbox, retry/handoff, metrics, search, notifications. 5–20× faster than
  Python.
- **v3 ratatui console** (`orc top`): attention-first dashboard, session
  workspace, timeline, `ember`/`phosphor` themes — now the fallback view.
- **Skills/blocks**: `pi-delegate` (auto), `orchestrate` (keyword-gated),
  `codex/AGENTS-block.md` — the trigger-word mechanism exists; v4 extends it.
- **Python v2** (`orc_pkg`): kept only as the cross-language parity oracle;
  v4 makes Rust the default install.

## Research: prior art and what we take from each

| Source | What it proves | Take | Reject |
|---|---|---|---|
| [Advisor tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool) | Two-tier model pairing is an endorsed pattern | Brain/worker framing, honest cost receipts | Server-side coupling — ours is process-level, harness-agnostic |
| [BridgeSpace](https://www.bridgemind.ai/products/bridgespace) (Tauri 2 + Rust) | 1–16 terminal panes + kanban + agent config in one desktop window ships | The stack (Tauri), workspace-per-project, terminals-beside-board | Rigid grid layouts (we do a stage, below); command blocks |
| [Vibe Kanban](https://github.com/BloopAI/vibe-kanban) (Rust + React) | Kanban as the command center for coding agents works; Rust backend owns git/terminals, web UI owns the board | Board-centric orchestration UX, task→workspace→terminal linkage | Its worktree-per-task model (our unit is the run/session, worktrees optional); Node server process |
| [claude-squad](https://github.com/smtg-ai/claude-squad) (Go) | Harness-agnostic launcher via configurable commands | Configurable harness commands; detach-survivability thinking | tmux as pane engine |
| Claude Code [agent teams](https://code.claude.com/docs/en/agent-teams) | Shared task list + mailbox + lead/teammate model | File-based task semantics (claim, depends_on, locking); idle notifications | Single-harness lock-in — crossing harnesses is our moat |
| [fulsomenko/kanban](https://github.com/fulsomenko/kanban) (Rust) | Serious task-domain modeling in Rust | Card anatomy, prefix ids (T1…), atomic JSON + file watching, crate split | Sprints, SQLite, undo/redo (YAGNI) |
| [hermes-agent](https://github.com/nousresearch/hermes-agent) (Nous Research) | Open-source multi-provider agent CLI with subagents, 6 terminal backends | First-class harness, **default worker** alongside pi; inspect `hermes --help` locally for headless/usage-reporting shape | Its gateway/messaging surface (out of scope) |
| [tauri-plugin-pty](https://github.com/Tnze/tauri-plugin-pty), [tauri-terminal](https://github.com/marc2332/tauri-terminal) | xterm.js + portable-pty in Tauri 2 is an established pattern | PTY host architecture, xterm.js WebGL renderer | Depending on the plugin blindly — spike validates throughput first |

## Non-negotiable constraints (carried forward + new)

- All implementation commits land on `v4-bench`. Never force-push; merge to
  `main` only at explicit gates.
- Never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`. Installer appends only marked blocks with backups.
- Registry stays plain JSON/text at `~/.orchestra/runs/<id>/…`; all new files
  (sessions, tasks) use atomic temp+fsync+rename and additive-field tolerance.
  Python readers must not break (parity tests prove it).
- Quota gate fails open on transport problems; every network/subprocess call
  is bounded by a timeout.
- **Single-writer rule, restated for the app:** the frontend never writes
  registry/task files. Every mutation — launching, killing, steering, moving a
  task card — goes through orc-core command functions (same code the CLI
  uses), and every mutation is recorded with its actor (`brain` | `human`).
- **The prime directive of the bench:** pi-orchestra must never hinder a
  harness. Terminal panes get verbatim keyboard passthrough while focused
  (xterm.js gets the raw stream; app-level shortcuts use chords no harness
  binds, and `cmd+w`-style window shortcuts are intercepted, never delivered
  as pane death). orc never proxies a harness's API traffic; coordination is
  filesystem-only, opted into via `orc` CLI calls.
- TUI (fallback) keeps its standards: no emojis, theme tokens only, state as
  words. The app inherits the same token discipline (below).

## Architecture

### Control plane vs data plane

```
   ┌──────────────────── orchestra-app (Tauri 2, one process) ────────────────────┐
   │  WebView UI: HOME · STAGE · SCORE · RUNS   (React/TS, xterm.js, SVG layer)   │
   │        │ IPC (typed commands + event stream)                                 │
   │  Rust side: PTY host (portable-pty) · orc-core calls · fs watcher → events   │
   │        │ owns interactive PTYs                 observes (mtime cache)        │
   │        ▼                                            ▲                        │
   │  BRAIN pty (claude) · W panes (hermes, pi/M3, …)    │                        │
   └────────┼─────────────────────────────────────────────┼────────────────────────┘
            │ harness runs `orc run/send/task…`           │
            ▼                                             │
     ~/.orchestra/{runs,sessions,config}  ────────────────┘   single source of truth
     ▲
     └── orc CLI (headless brains, skills) · orc top (ratatui fallback) — unchanged consumers
```

- The **data plane** is each harness talking to its own provider. orc never
  sits in that path.
- The **control plane** is the filesystem. Brains call `orc` (via skills /
  AGENTS blocks), which writes runs, inbox messages, and tasks; the app
  renders from those files plus its own PTY buffers. Connector lines are
  *derived from registry linkage*, not traffic sniffing: a pane launched from
  the stage exports `ORC_SESSION` and `ORC_PANE_ID`, every run created inside
  it carries those, and that is what lets the UI truthfully draw brain→worker
  curves.
- Headless use keeps working unchanged — a brain in a plain terminal that says
  "orchestrate" gets the full flow; the app is an optional host, not a
  requirement.

### Process model

The app owns interactive PTY children (the brain, and any worker harness
opened interactively). Registered `orc run` workers stay as today: detached,
runner-supervised, surviving an app crash. Stated honestly: closing the app
closes interactive panes like closing a terminal window (documented);
registered runs never die with it. Detach/reattach for interactive panes is
P2.

### Session model

Launching `pi-orchestra` opens HOME: past sessions with receipts; a new
session takes three steps (brain harness → worker pool → cwd).
`~/.orchestra/sessions/<id>/session.json`:

```json
{
  "id": "orch-20260711-104200-auth-refactor",
  "title": "auth refactor",
  "brain": {"harness": "claude", "cwd": "/path/to/repo"},
  "worker_pool": ["hermes", "pi-m3"],
  "layout": {"mode": "ensemble"},
  "created_at": "…"
}
```

### Task model (the board substrate)

`~/.orchestra/sessions/<id>/tasks/<task-id>.json`, one file per task (atomic
writes; agent-teams shows file-per-task with claim locking works):

```json
{
  "id": "T3",
  "title": "draft registry.rs from interface spec",
  "status": "running",            // backlog | assigned | running | review | done | dropped
  "assignee_run": "20260711-…-a1b2",
  "worker": "hermes",
  "depends_on": ["T1"],
  "created_by": "brain",
  "history": [{"at": "…", "actor": "brain", "to": "running"}],
  "updated_at": "…"
}
```

CLI: `orc task add|assign|start|review|done|drop|move|list --json --session <id>`.
The brain maintains the board through these commands; the human may drag a
card in the app, which invokes the same `orc task move` path with
`actor: "human"` — visible in the card's history, so brain and human never
silently overwrite each other's judgment. `review` is the honest column:
worker output is untrusted until the brain (or human) verifies, and the board
must show that gap rather than jumping to done.

### Harness registry and worker defaults

`~/.orchestra/config.json` gains:

```json
{
  "harnesses": {
    "claude": {"cmd": "claude", "args": [], "roles": ["brain","worker"], "adapter": "claude"},
    "codex":  {"cmd": "codex",  "args": [], "roles": ["brain","worker"], "adapter": "codex"},
    "hermes": {"cmd": "hermes", "args": [], "roles": ["brain","worker"], "adapter": "hermes"},
    "pi-m3":  {"cmd": "pi", "args": ["--provider","minimax","--model","MiniMax-M3"],
               "roles": ["brain","worker"], "adapter": "pi"}
  },
  "default_workers": ["hermes", "pi-m3"],
  "max_parallel_workers": 3,
  "app": {"leader_chord": "cmd+shift+space", "max_panes": 16}
}
```

**Worker choice is the user's, and the default pool is hermes + pi
(MiniMax-M3).** The new-session flow and the `orchestrate` skill both offer
the pool and accept any configured harness — claude, codex, hermes, pi, or a
custom command. Unknown harnesses still work as plain interactive panes; they
just lack registered-run superpowers until an adapter exists.

### Worker adapters

`runner.rs` grows an adapter seam. Each adapter declares capabilities
(`steerable`, `exact_usage`) and normalizes to the same registry meta; the UI
and CLI degrade honestly when a capability is absent.

- `pi` — existing json/rpc lifecycle; reference implementation.
- `hermes` — **default worker, first new adapter.** Shape verified locally
  first (`hermes --help`): headless invocation, JSON/stream output, usage
  reporting, exit semantics. If hermes lacks a headless mode, it runs as an
  interactive stage pane attributed to the session while the adapter is
  pending — never fake exactness.
- `claude` — `claude -p --output-format stream-json`, best-effort usage.
- `codex` — `codex exec --json`, best-effort.

### Trigger word

Already 90% built. v4 additions: the skills and `codex/AGENTS-block.md` learn
(a) "pi-orchestra" as an alias trigger for the orchestrate flow, (b) `orc
task` board maintenance, (c) worker-pool choice (offer the configured
`default_workers`, never assume), (d) when running inside a stage pane
(`ORC_PANE_ID` set), skip the sales pitch and coordinate. A hermes
instructions block is added if local inspection shows hermes reads an
AGENTS.md-equivalent. Apply the five skill-wording fixes from the v3 review
while touching these files.

### Repository layout

```
rust/crates/
├── orc-core/        (exists; runner adapter seam; + tasks.rs, session.rs, events.rs)
├── orc-cli/         (exists; + `orc task`, `orc session`)
└── orc-tui/         (exists; frozen fallback — bug fixes + task awareness only)
app/                 new: Tauri 2
├── src-tauri/       PTY host, typed IPC commands wrapping orc-core, fs watcher → event stream
└── ui/              React + TypeScript + Vite; xterm.js (WebGL addon); SVG connector layer;
                     dnd-kit board; design tokens shared with the TUI palette
```

The transport between ui and src-tauri is one typed interface (commands +
event subscription) so a future `orc serve` can reuse the UI over WebSocket.

## UI/UX direction

**Subject world: the orchestra pit.** The brain is the conductor; workers are
players; the kanban is the score; the window is the stage. This vocabulary
drives the design so nothing reads as a generic dashboard template.

**Anti-slop commitments.** No cream-paper + terracotta serif landing look; no
near-black + single acid accent; no broadsheet hairline grid. No default
shadcn-looking card grids with uniform rounded rectangles and a stats row. The
palette extends the project's existing ember identity into materials the
subject owns: deep warm charcoal stage (#1a1614-family, not pure black), brass
and amber (instrument metal) for live/active, bone/ivory text, oxblood for
attention states, with `phosphor` surviving as an alternate app theme. One
characterful display face for session titles and view names (a warm grotesque
— specimen chosen during Phase 2 with screenshots, not defaulted to Inter),
a workhorse UI face for controls, and the user's terminal mono inside panes.
Design tokens live in one file mirrored TUI↔app so both feel like one product.

**The signature element — baton lines.** Connectors between the conductor card
and player cards are cubic Bézier curves in an SVG layer *under* the cards:
dim braided filaments when idle; on a registry event (dispatch, steering
delivery, completion, handoff) a pulse of light travels the curve in the
event's direction with a small timestamped label that then settles into the
timeline. Event kinds are distinguished by pulse shape/tempo, not just hue.
This is where the motion budget is spent; everything else animates only on
state change (a card settling when a run completes), and `prefers-reduced-motion`
collapses pulses to state flashes.

### Surfaces (one window, four views)

**HOME — the season.** Past sessions as wide cards with topology thumbnail,
receipts (exact-vs-`~` tokens, cost), and attention badges; one action: new
session (brain → worker pool [hermes + pi preselected] → cwd picker).

**STAGE — the flagship.** Not a rigid grid: an **ensemble layout**. The
conductor terminal card sits center-left; player cards arc around it in
orchestra seating, each a floating rounded panel with soft elevation, a header
rail (harness name, run id, status word, live token/cost ticker) and a live
xterm.js surface. Baton lines connect conductor→players beneath the cards.
Cards are draggable and resizable; layouts persist per session; presets
(solo/duet/quartet/full-16) exist for people who want BridgeSpace-style
grids. Focus follows click; the focused card gets a brass edge-light and
verbatim input. Double-click a header (or leader chord) zooms a card to the
full stage and back. Closing a player card never kills a registered run
without an explicit confirm.

**SCORE — the board.** Kanban columns Backlog / Assigned / Running / Review /
Done drawn on a stage-dark surface with faint staff-line texture (restrained —
a texture, not a theme park). Cards carry T-prefix id, title, worker chip
(hermes / pi-m3 / codex / claude), status word, exact-or-`~` tokens, and a
history popover (who moved it, brain or human). Drag between columns invokes
`orc task move` (actor: human). Clicking a card's worker chip flies to that
player card on the STAGE — board and stage are two projections of one
session.

**RUNS — the ledger.** The v3 attention-first data: quota meters, run table,
session timeline, search — rebuilt as a dense web view over the same
snapshot code, for when you want the instrument panel rather than the stage.

### Interaction guardrails

- Keyboard: full passthrough to focused terminal; app chords use
  modifier+combos audited against claude/codex/hermes/pi keymaps in the spike;
  `cmd+w`/`cmd+q` intercepted with confirm when live panes exist.
- Empty states teach the real workflow with copyable commands (as v3 does).
- Errors state what happened and the next action; no toast confetti; no
  celebration animations when a swarm finishes — a completed session is a
  receipt, not a party.

## Phasing

**Phase 0 — fix-first + merge base.** Apply review fixes 1–3 (+6) on
`v3-rust`; full gates (pytest, cargo test/clippy/fmt, live smoke 10/10); merge
`v3-rust` → `main`; branch `v4-bench`.

**Phase 1 — app spike (go/no-go).** Minimal Tauri 2 app: two xterm.js panes
running Claude Code and hermes interactively via portable-pty, one hardcoded
SVG Bézier between them pulsing on a watched-file change. Must survive:
full-screen TUI redraws at 60fps-ish, paste, mouse, resize, IPC throughput
under `yes`-style output floods, key passthrough audit, quit interception.
Verdict written to `docs/notes/2026-07-11-app-spike.md`. Fallback if Tauri
disappoints: same UI served locally (`orc serve` + browser) — record why.

**Phase 2 — HOME + STAGE.** Session records, home shelf, three-step new
session (worker pool defaults hermes + pi), ensemble layout + drag/resize/
zoom/persist, pane↔run attribution (`ORC_SESSION`/`ORC_PANE_ID`), design
tokens + typography pass with screenshot review, `pi-orchestra` launcher in
`install.sh` (Rust CLI becomes default install; Python behind `--python`).

**Phase 3 — tasks + SCORE + skills.** `tasks.rs` + `orc task` CLI with actor
attribution, SCORE board with drag-to-move, board↔stage flying links, skills
and AGENTS-block updates (board maintenance, "pi-orchestra" trigger, worker
pool choice, five review wording fixes, hermes block if applicable), TUI
gains read-only task list rendering.

**Phase 4 — baton lines + RUNS.** Event stream from fs watcher → pulse
animation on real dispatch/steer/complete/handoff; timeline labels; RUNS view
port of the v3 instrument panel.

**Phase 5 — adapters + docs + dogfood.** hermes adapter first (after local
shape verification), claude/codex best-effort behind capability flags. README
+ guide rewrite with app screenshots/screen recording. **Dogfood gate:** build
at least one Phase 5 deliverable by saying "orchestrate" with the brain
hosted on the STAGE and the SCORE tracking tasks; friction log in
`docs/notes/` is a deliverable.

Cut order under pressure: claude/codex adapters → RUNS view (TUI still covers
it) → layout presets beyond solo/duet/quartet → staff-line texture and
typography extras. Never cut: Phase 0, passthrough integrity, single-writer
mutations, fail-open quota, actor-attributed task moves, the hermes+pi default
pool, baton lines (a static-line degrade is acceptable; absence is not).

## Risks

1. **IPC/render throughput** for flooding terminals — the existential risk;
   spike measures it with WebGL renderer + output batching before anything
   else is built.
2. **Keyboard fidelity** in xterm.js for harness TUIs (kitty keyboard
   protocol, bracketed paste, mouse modes) — spike audits with the real four
   harnesses, not `vim`.
3. **hermes adapter unknowns** — headless mode/usage reporting unverified;
   mitigation: local inspection first, interactive-pane degrade if absent.
4. **Frontend toolchain weight** (Node/Vite enters the repo) — contained to
   `app/ui`; CI gates add `npm build` + typecheck; no server runtime ships.
5. **macOS packaging/signing** — dev builds fine unsigned; distribution
   packaging is explicitly out of v4 scope.
6. **Log amplification** (193 MB/260k tok in v3 dogfood) — player panes tail
   bounded output; xterm.js scrollback capped; never slurp whole logs.

## Open questions (with recommendations)

1. **Detach/reattach for interactive panes** — punted to P2; document "stage
   panes are a window; registered runs are durable."
2. **Python retirement** — v4 flips default install to Rust, Python behind
   `--python` as parity oracle; delete in a later major.
3. **Remote access** (`orc serve`) — P2; transport abstraction keeps it cheap.
4. **Worktree-per-task** (Vibe Kanban model) — attractive, out of v4 scope;
   revisit once the board is proven.
5. **App theme count** — ship ember-derived default + phosphor alternate;
   no theme builder.

## Approval gate

No implementation, branch, or swarm until the user approves this revision.
After approval: Phase 0 on `v3-rust`, merge, create `v4-bench` carrying this
document, then the Phase 1 app spike before any feature work.
