# pi-orchestra v4 — the Bench: multi-harness orchestration workspace

**Date:** 2026-07-11
**Status:** Proposed — awaiting explicit user approval
**Branch after approval:** `v4-bench`, created from `main` after the fix-first items land on `v3-rust` and it merges to `main`
**Prereq:** the three ordered fixes in `docs/reviews/2026-07-11-v3-rust-review.md` (quota timeout, steering turn boundary, TUI quota refresh)

## Product position

pi-orchestra today is a delegation registry with a control-plane TUI: brains
(Claude Code / Codex) shell out to `orc`, workers (pi + MiniMax-M3) run headless,
and `orc top` observes. The operator still lives in N disconnected terminal
windows and reconstructs the swarm in their head.

v4 changes the product contract to:

> Run every harness of a working session — one brain, N workers — inside a single
> pi-orchestra window; see the instruction flow between them and the task board
> they are burning down; and do all of it without ever getting between a harness
> and its own I/O.

The origin idea is the [advisor tool pattern](https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool)
inverted: instead of a cheap executor consulting an expensive advisor
server-side, an expensive brain dispatches cheap executors — and pi-orchestra is
the *inter-harness* coordination fabric that no single harness provides, because
each harness (Claude Code subagents, Codex, agent teams) only orchestrates
within itself.

## What exists (do not rebuild)

- **v3 Rust core** (`rust/crates/orc-core`): registry (atomic JSON, tolerant
  models, orphan reconcile), quota gate, pi json/rpc runner with steering inbox,
  retry/handoff, metrics, search, notifications. 5–20× faster than Python.
- **v3 ratatui console** (`orc top`): attention-first dashboard, session
  workspace with controller→worker topology, timeline, themes (`ember`,
  `phosphor`), settings. Strong, reviewed visual identity.
- **Skills/blocks**: `pi-delegate` (auto), `orchestrate` (keyword-gated),
  `codex/AGENTS-block.md` — the "trigger word" mechanism already works; v4
  extends it, it does not invent it.
- **Python v2** (`orc_pkg`): current default install; v4 makes Rust the default
  and keeps Python only as the cross-language parity oracle in tests.

## Research: prior art and what we take from each

| Source | What it proves | Take | Reject |
|---|---|---|---|
| [Advisor tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool) (Claude docs) | Two-tier model pairing is an officially endorsed pattern | The brain/worker framing, honest cost receipts | Server-side coupling — ours is process-level and harness-agnostic |
| [BridgeSpace](https://www.bridgemind.ai/products/bridgespace) (Tauri 2 + Rust) | 1–16 terminal grid + kanban + agent config in one window is a shippable product | Grid presets (1/2/4/6/9/16), workspace-per-project, kanban next to terminals | GUI app; command blocks; we stay a pure TUI, no Tauri |
| [claude-squad](https://github.com/smtg-ai/claude-squad) (Go) | Multi-agent terminal manager works; harness-agnostic via configurable launch commands | Configurable harness commands; instance list UX; background daemon idea for detach | tmux dependency as the pane engine (we embed PTYs; tmux only as P2 fallback) |
| Claude Code [agent teams](https://code.claude.com/docs/en/agent-teams) | Shared task list + mailbox + lead/teammate model; split panes need tmux/iTerm2 and exclude Ghostty | Task list file semantics (claim, depends-on, file locking); "lead is fixed"; idle notifications | Single-harness lock-in — pi-orchestra's whole point is crossing harness boundaries |
| [fulsomenko/kanban](https://github.com/fulsomenko/kanban) (Rust/ratatui) | A serious ratatui kanban exists: 3 view modes cycled with `V`, vim keys, JSON envelope with atomic writes, file watching, MCP server | View-mode cycling, card anatomy (status word, points→tokens, prefix ids), atomic JSON + watch, crate split (core/domain/persistence/tui) | Sprints, SQLite backend, undo/redo (YAGNI for run-scoped boards) |
| [tui-term](https://github.com/a-kenji/tui-term) + [portable-pty](https://crates.io/crates/portable-pty) + vt100 | Embedding live PTYs as ratatui widgets is an established (if WIP) path | portable-pty for spawn/resize; vt100-family parser; render grid into ratatui buffer | tui-term's experimental controller; we own the pane lifecycle |

## Non-negotiable constraints (carried forward + new)

- All implementation commits land on `v4-bench`. Never force-push; merge to
  `main` only at explicit gates (see phasing).
- Never modify `~/.pi/agent/*`, `~/.claude/settings.json`, `~/.codex/config.toml`.
  Installer appends only marked blocks with backups, as today.
- Registry stays plain JSON/text at `~/.orchestra/runs/<id>/…`; all new files
  (sessions, tasks, bench state) follow the same atomic temp+fsync+rename
  discipline and additive-field tolerance. Python readers must not break.
- Quota gate fails open on transport problems; every network/subprocess call is
  bounded by a timeout.
- The TUI contains no emojis; every color comes from theme tokens; state is
  words, never color alone.
- **New — the prime directive of the bench:** pi-orchestra must never hinder a
  harness. Embedded terminals get full raw keyboard passthrough while focused,
  no chrome overlays their cells, orc never intercepts or proxies a harness's
  API traffic, and coordination happens only through the filesystem contract
  (registry, inbox, tasks) that harnesses opt into via `orc` CLI calls.

## Architecture

### Control plane vs data plane

```
            ┌────────────────────── pi-orchestra (one process) ─────────────────────┐
            │  HOME · BENCH · BOARD · RUNS views (ratatui)                          │
            │        │ owns PTYs (portable-pty)          observes (mtime cache)     │
            │        ▼                                        ▲                     │
            │  ┌───────────┐ ┌───────────┐ ┌───────────┐      │                     │
            │  │ BRAIN pty │ │ W1 pty    │ │ W2 pty    │      │                     │
            │  │ claude    │ │ pi/M3     │ │ pi/M3     │      │                     │
            │  └─────┬─────┘ └─────┬─────┘ └─────┬─────┘      │                     │
            └────────┼─────────────┼─────────────┼────────────┼─────────────────────┘
                     │ runs `orc run/send/task…` │            │
                     ▼             ▼             ▼            │
              ~/.orchestra/{runs,sessions,config}  ───────────┘  (single source of truth)
```

- The **data plane** is each harness talking to its own provider. orc never sits
  in that path.
- The **control plane** is the filesystem: brains call `orc` (via the existing
  skills / AGENTS blocks), which writes runs, inbox messages, and tasks; the TUI
  renders exclusively from those files plus its own PTY buffers. The connection
  lines in the bench are *derived from registry linkage*, not from sniffing
  traffic — a pane launched by the bench exports `ORC_SESSION` and
  `ORC_PANE_ID`, and every run created from inside it carries those, which is
  what lets the UI draw brain→worker edges truthfully.
- Headless use keeps working unchanged: a brain in a plain terminal that says
  "orchestrate" still gets the full flow; the bench is an optional host, not a
  requirement. This is the amplify-don't-hinder guarantee in structural form.

### Process model

The bench process owns the PTY children (brain + any interactively hosted
workers). Registered `orc run` workers stay exactly as today: detached processes
supervised by their runner, surviving a bench crash. Consequences, stated
honestly:

- If the bench exits, hosted *interactive* panes (the brain harness) die with
  it, like closing a terminal window. v4.0 documents this; detach/reattach via a
  background pane-host daemon is P2 (claude-squad solves it with tmux — we may
  offer an optional tmux backend then).
- Registered worker *runs* never die with the bench — the registry/runner
  contract already guarantees that.

### Session model

`orc` with no arguments (and the new `pi-orchestra` alias) opens HOME: past
sessions with receipts, `n` starts a new one. A session record —
`~/.orchestra/sessions/<id>/session.json` (already reserved by the v3 spec) —
gains additive fields:

```json
{
  "id": "orch-20260711-104200-auth-refactor",
  "title": "auth refactor",
  "brain": {"harness": "claude", "cwd": "/path/to/repo"},
  "workers_default": "pi-m3",
  "layout": "grid4",
  "created_at": "…"
}
```

New-session flow (three inline steps, no modal maze): pick brain harness →
pick default worker harness (default `pi-m3`, deliberately overridable to any
configured harness — the product is unbiased) → pick cwd. The bench opens with
the brain pane running.

### Task model (the kanban substrate)

`~/.orchestra/sessions/<id>/tasks/<task-id>.json`, one file per task (atomic
writes, no cross-task lock contention; the agent-teams docs show file-per-task
with claim locking works):

```json
{
  "id": "T3",
  "title": "draft registry.rs from interface spec",
  "status": "running",            // backlog | assigned | running | review | done | dropped
  "assignee_run": "20260711-…-a1b2",  // links card ↔ run ↔ pane edge
  "depends_on": ["T1"],
  "notes": "verify against tests/registry.rs first",
  "created_by": "brain",
  "updated_at": "…"
}
```

CLI: `orc task add|assign|start|review|done|drop|list --json --session <id>`.
The brain maintains the board through these commands — the `orchestrate` skill
gains a step: *decompose into `orc task add` cards first, mark transitions as
you dispatch/verify*. The BOARD view is pure projection; the TUI never writes
tasks. `review` is the honest column: worker output is untrusted until the
brain verifies, and the board must show that gap rather than jumping to done.

### Harness registry (config)

`~/.orchestra/config.json` gains:

```json
{
  "harnesses": {
    "claude": {"cmd": "claude", "args": [], "roles": ["brain"]},
    "codex":  {"cmd": "codex",  "args": [], "roles": ["brain"]},
    "hermes": {"cmd": "hermes", "args": [], "roles": ["brain"]},
    "pi-m3":  {"cmd": "pi", "args": ["--provider","minimax","--model","MiniMax-M3"],
                "roles": ["brain","worker"], "adapter": "pi"}
  },
  "default_worker": "pi-m3",
  "max_parallel_workers": 3,
  "bench": {"leader_key": "ctrl-g", "max_panes": 16, "default_layout": "auto"}
}
```

Any harness is a command + args; unknown harnesses work as plain interactive
panes with no adapter (they just don't get registered-run superpowers). The
settings view exposes harness list, default worker, max workers, leader key.

### Worker adapters (unbiased workers, made real)

Today `orc run` is pi-only. v4 introduces an adapter seam in `runner.rs`:

- `pi` adapter — the existing json/rpc lifecycle, reference implementation.
- `claude` adapter — `claude -p --output-format stream-json` (headless);
  best-effort usage extraction.
- `codex` adapter — `codex exec --json`; best-effort.

Adapters normalize to the same registry meta (status, exit mapping, tokens
exact-or-estimated with basis marked). Steering/kill semantics differ per
adapter; an adapter declares capabilities (`steerable`, `exact_usage`) and the
UI/CLI degrade honestly (a non-steerable run shows why, exactly like json-mode
runs do today). pi/M3 stays the default; the point is choice, not churn.

### Trigger word

Already 90% built. v4 additions: the skills and `codex/AGENTS-block.md` learn
(a) the word "pi-orchestra" as an alias trigger for the orchestrate flow,
(b) `orc task` board maintenance, (c) to tell the user "open `pi-orchestra` to
watch the bench" — and, when running *inside* a bench pane (detected via
`ORC_PANE_ID`), to skip the sales pitch and just coordinate. Apply the five
skill-wording fixes from the v3 review while touching these files.

### Crate layout additions

```
rust/crates/
├── orc-core/            (exists; runner grows adapter seam; + tasks.rs, session.rs)
├── orc-pty/             new: portable-pty spawn/resize/kill, vt100-family parser,
│                        pane buffer → ratatui surface, input encoder
├── orc-tui/             (exists; grows)
│   └── src/
│       ├── home.rs      session shelf + new-session flow
│       ├── bench.rs     pane grid, focus router, leader key, zoom
│       ├── gutter.rs    connection channels + pulse animation (from registry events)
│       ├── board.rs     kanban projection of tasks/
│       └── …            existing dashboard/session/topology/timeline stay as RUNS view
└── orc-cli/             (exists; + `orc task`, `orc bench`/default-command HOME)
```

Dependency additions need justification, as before: `portable-pty` and one
vt100 parser (evaluate `vt100`, `wezterm-term`/`termwiz`, `alacritty_terminal`
in the spike; pick one, prefer the smallest that passes the spike). Still no
tokio; PTY reads are threads feeding the existing event loop.

## UI/UX direction

One design system across four surfaces, cycled with `V` (bench → board → runs)
from inside a session; HOME is the front door. Everything extends the existing
`ember`/`phosphor` tokens — v4 adds no third theme and no new color outside
`theme.rs`.

**The signature element is the connection gutter.** Between the brain pane and
the worker column runs a 1–2 cell channel drawn from theme line tokens. Idle
edges are a dim dotted line; a registry event (dispatch, steering delivery,
completion, handoff) sends a single braille pulse traveling the channel in the
event's direction, with a timestamped label that fades to the timeline. This is
the one place motion is spent (consistent with v3's "restrained motion" rule);
it is functional — you can see *which* worker just got instructions — not
decorative. The swiggly line the user asked for, earning its keep.

### HOME — session shelf

```
PI-ORCHESTRA                                          5h ▮▮▮▮▮▯▯ 71% · wk 45%
──────────────────────────────────────────────────────────────────────────────
  SESSIONS
  ▸ orch-20260711-auth-refactor    BRAIN claude    3 workers   running
       6 tasks · 2 done · 412.3k tok · $0.11                        12m ago
    orch-20260710-v3-inventory     BRAIN codex     3 workers   done
       260.2k tok exact · $0.0649                                    1d ago
──────────────────────────────────────────────────────────────────────────────
  n new session · enter open bench · r runs view · , settings · q quit
```

### BENCH — the terminal grid

```
╔ BRAIN · CLAUDE ══════════════════╗ ┊ ┌ W1 · pi · M3 · run a1b2 ──────┐
║                                  ║ ┊ │ running · 34.1k tok · $0.01   │
║   (claude code, live, full       ║⣿┊ │ (live worker output)          │
║    passthrough while focused)    ║ ┊ └───────────────────────────────┘
║                                  ║ ┊ ┌ W2 · pi · M3 · run c3d4 ──────┐
╚══════════════════════════════════╝ ┊ │ assigned · waiting on T1      │
                                     ┊ └───────────────────────────────┘
 orch-…-auth-refactor · focus BRAIN · ^g leader · V board · z zoom · tab panes
```

- Brain pane: double-line border + `BRAIN · <HARNESS>` label. Workers: single
  line, run id, status word, live receipts in the title row. Grid presets
  1/2/4/6/9/16 (BridgeSpace parity); `auto` packs brain-left/workers-right up
  to 4, then grids.
- **Focus model:** the focused pane gets every key, verbatim — including the
  keys Claude Code and Codex own. One configurable leader key (default
  `ctrl-g`, chosen because none of the target harnesses bind it) pops focus to
  orc chrome for one command (`tab` cycle, `z` zoom, `V` views, `x` close pane,
  `q` quit); `ctrl-g ctrl-g` sends a literal ctrl-g through. The status rail is
  the only permanent chrome; nothing ever overlays pane cells.
- Worker panes are optional views onto registered runs (tailing the same
  output.log the runner writes) — closing a worker pane never kills the run;
  `x` asks before actually killing.

### BOARD — kanban projection

```
 BACKLOG 2      ASSIGNED 1      RUNNING 2        REVIEW 1        DONE 3
┌────────────┐ ┌────────────┐ ┌─────────────┐ ┌─────────────┐ ┌────────────┐
│ T6 quota   │ │ T4 → W2    │ │ T2 · W1     │ │ T3 · W2     │ │ T1 ✓brain  │
│ history    │ │ blocked on │ │ runner.rs   │ │ awaiting    │ │ 73.6k tok  │
│ docs       │ │ T2         │ │ 12m · 41k   │ │ brain check │ │ $0.025     │
└────────────┘ └────────────┘ └─────────────┘ └─────────────┘ └────────────┘
 j/k/h/l move · enter card detail · g jump to pane · V bench · tasks: brain-owned
```

Cards carry a status word, assignee link, exact-or-`~` tokens. `g` on a card
jumps to the assigned worker's pane — board and bench are two projections of
one session. The TUI does not move cards; the brain does, through `orc task`
(read-only invariant preserved). Card ids `T1…` follow fulsomenko-style
prefixes; a `review` column exists because worker output is untrusted until
verified — the board must show that honestly.

### RUNS — existing v3 surfaces

The attention-first dashboard and session workspace remain as the third view,
unchanged except for gaining `V` cycling and task-aware timeline entries.

## Phasing

**Phase 0 — fix-first + merge base.** Apply review fixes 1–3 on `v3-rust`
(quota timeouts, steering turn counting + fixed fake-pi fixture, TUI quota
refresh thread; fold in finding 6's keychain timeout). Gates: full pytest +
cargo test/clippy/fmt + live smoke 10/10. Then merge `v3-rust` → `main`
(review verdict authorizes merge after 1–3) and branch `v4-bench`.

**Phase 1 — PTY spike (go/no-go).** Prototype `orc-pty`: run Claude Code, pi,
and codex in embedded panes; verify full-screen TUI rendering fidelity, key
passthrough (including paste, mouse, ctrl keys), resize, CPU at 4 panes,
scrollback memory bounds. Decide parser crate. **Fallback if no-go:** pivot the
bench to a tmux-backed backend (claude-squad model) and record why. Nothing
else in Phase 2+ starts until this verdict is written down.

**Phase 2 — session home + bench.** `orc-pty` productionized, HOME shelf,
new-session flow, bench grid + focus router + leader key + zoom, `pi-orchestra`
alias in install.sh, harness registry in config + settings view. Rust becomes
the default install; Python stays as test oracle.

**Phase 3 — tasks + board + skills.** `tasks.rs` + `orc task` CLI (+ JSON
output), BOARD view, `orchestrate`/`pi-delegate`/AGENTS-block updates (board
maintenance, "pi-orchestra" trigger alias, the five review wording fixes),
`ORC_PANE_ID` attribution.

**Phase 4 — connection gutter + flow.** gutter.rs pulses from registry/inbox
events, board↔bench jumps, task-aware timeline.

**Phase 5 — worker adapters + docs + dogfood.** Adapter seam with pi as
reference, claude/codex adapters best-effort behind capability flags. README/
guide/install rewrite around v4. **Dogfood gate:** use the orchestra itself
(brain in the bench, keyword-triggered) to build one real feature of this
phase — the friction log (`docs/notes/`) is a deliverable, as it was for v3.
VHS captures of HOME/BENCH/BOARD for the README.

Explicit cut order under pressure: adapters beyond pi (P5) → gutter animation
(keep static edges) → grid presets beyond 1/2/4 → board card detail view.
Never cut: focus passthrough integrity, registry/task atomicity, read-only TUI,
fail-open quota, no-emoji standard, Phase 0.

## Risks and open questions (with recommendations)

1. **PTY embedding fidelity** is the existential risk — hence spike-first with
   a named fallback (tmux backend). Watch: vt100 crates differ on modern
   sequences (synchronized output, OSC, kitty keyboard protocol) that Claude
   Code emits; test with the real harnesses, not `vim`.
2. **Leader key collisions.** `ctrl-g` default, configurable; must audit
   against claude/codex/pi keymaps in the spike and document the double-tap
   literal escape.
3. **Detach/reattach** deliberately punted to P2 (documented in README as
   "bench panes are a window, registered runs are durable").
4. **Log amplification** (193 MB/260k tok in v3 dogfood) — Rust runner already
   strips cumulative snapshots; bench worker panes must tail with the existing
   bounded-memory tail, never slurp.
5. **Who writes tasks?** Only the brain (via CLI) in v4.0. Human card-moves
   from the TUI would break the read-only invariant; if wanted later, they go
   through the same CLI, shelled out like kill/send today.
6. **Python retirement.** Recommendation: v4 flips the default installer to
   Rust and moves Python behind `--python`; delete it only in a later major
   after a deprecation note. The parity suite still runs both.

## Approval gate

No implementation, branch, or swarm until the user approves this design. After
approval: Phase 0 on `v3-rust`, merge, create `v4-bench` carrying this
document, then Phase 1 spike before any feature work.
