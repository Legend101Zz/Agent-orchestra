# orc v3 — Rust runtime and operator console

**Date:** 2026-07-11  
**Status:** Proposed — awaiting explicit user approval  
**Branch after approval:** `v3-rust`, created from `main`

## Product position

`orc top` v2 is a strong instrument panel but a weak control plane. It shows quota,
run state, cost, sessions, and logs with a recognizable visual identity. The missing
loop is intervention: an operator cannot steer a live RPC worker, retry a failed run,
or hand incomplete work to another worker without reconstructing context outside the
product.

v3 changes the product contract to:

> Inspect, intervene in, and recover a running swarm without leaving `orc top`, while
> keeping the registry plain, interoperable, and honest about uncertain usage data.

The Rust migration exists to remove Python/venv startup cost and to make a responsive,
stable control plane practical at hundreds of runs. It is not permission to change the
registry contract or silently diverge from Python behavior.

## Non-negotiable constraints

- All implementation commits land on `v3-rust`. Do not commit to, merge into, or
  force-push `main`.
- Do not modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`, or the user's `~/.local/bin/orc` symlink.
- Preserve `~/.orchestra/runs/<id>/{meta.json,output.log,inbox/}` as plain JSON/text.
- JSON writes use a temporary file in the destination directory, flush and sync it,
  then atomically rename it.
- The owning runner is the only writer of its `meta.json`. The TUI does not write run
  metadata; actions go through CLI commands or atomic inbox messages.
- Rust must read old metadata without `session`, exact token usage, v3 attention
  fields, or a recorded controller model. Python must continue reading Rust-created
  runs and ignoring additive fields it does not understand.
- Python remains the default install. `install.sh --rust` is opt-in and must not
  replace the live symlink during development or test work.
- The TUI contains no emojis. Status and identity use text, restrained geometric
  marks, and line work from theme tokens.

## Verified v2 gaps

The following are implementation gaps, not speculative requests:

- The original design reserves `{"type":"prompt","message":"..."}` inbox messages,
  but the Python RPC loop only checks `kill-*.json`. There is no steering CLI command
  or TUI composer.
- Retry is an instruction in the brain-facing skill, not a product action.
- User-defined budgets, burn alerts, and completion/failure notifications do not
  exist.
- The 24-hour activity strip counts run starts rather than token volume or completed
  work.
- Exact delegated-value calculations split input/output correctly; estimated runs
  are conservatively treated as all-input. The hero number does not make that
  uncertainty visually prominent enough.
- Brain token totals can be dominated by cached Codex input, which makes a combined
  headline easy to misread.
- The table is cleared and rebuilt every two seconds. Selection is restored by row
  position, not stable identity, and the approach does not scale gracefully.
- Search is local to one run log. There is no cross-run output search or durable
  session timeline.
- The first-run state is dead space. Material settings remain hand-edited JSON.
- The session Flow tab is visually pleasant but operationally static: it does not
  expose steering, retry, handoff, or failure recovery in context.

## Prioritized feature contract

### P0 — parity and registry safety

The Rust binary must match Python behavior for registry creation and reconciliation,
quota retrieval and gating, runner lifecycle, exact usage extraction, signals, idle
timeouts, sessions, and `list/show/kill/stats` before it becomes a feature platform.

Parity includes:

- `pi -p --mode json` and `pi --mode rpc`, always with `--offline` and the explicit
  MiniMax provider/model.
- Keychain lookup through `security`, then read-only fallback to
  `~/.pi/agent/auth.json`.
- Coding-plan quota selection using `model_name: "general"`, 60-second cache, history
  append, and existing exit codes.
- PID liveness reconciliation for orphaned runs.
- SIGTERM mapping to killed/143 behavior and idle-watchdog exit 124.
- Exact token and cost capture from `agent_end`, with old estimated metadata retained.

### P0 — intervention and recovery

#### Live steering

Add `orc send <run-id> "message"` for a running RPC worker. It atomically creates an
inbox prompt message. The Rust RPC runner watches prompt messages, writes them to the
held-open pi stdin, and marks delivery without rewriting `meta.json`.

Delivery history remains durable by moving a consumed file to an additive processed
location under `inbox/` or by an equally plain atomic acknowledgement convention.
Python readers ignore these files. A JSON-mode one-shot run rejects steering with a
clear explanation rather than pretending delivery succeeded.

The TUI exposes the same path through an inline composer. It shows queued, delivered,
and unavailable states. It never implies the worker understood a message merely
because stdin accepted it.

#### Retry

Add `orc retry <run-id>` for an infrastructure-equivalent rerun. It inherits task,
cwd, brain, session, provider, and model, permits an edited task, creates a new run,
and records an additive `retry_of` link. It never overwrites or reopens the old run.

#### Brain-reviewed handoff

Add `orc handoff <run-id> "remaining-work brief"`. This is distinct from retry:

1. The brain inspects the previous output, repository changes, and verification
   evidence.
2. The brain identifies completed and remaining work.
3. A new M3 worker receives a focused continuation brief, the original objective,
   the prior run identifier, and instructions to verify existing work before editing.
4. The new run records an additive `handoff_from` link and stays in the same session.

Obvious timeout or token/context-limit signatures may mark an additive
`attention: "handoff_needed"` and `failure_kind`, but ambiguous semantic
incompleteness is never inferred from untrusted prose alone. Handoff launch is not
automatic; brain review is the required judgment gate.

### P0 — advisory session budgets

Session budgets are visibility and alerting tools, not execution governors.

- Show actual spend, advisory budget, remaining headroom, and an uncertainty marker.
- Warn at configurable percentages and when a session crosses its advisory budget.
- Do not kill an in-flight worker, block a new worker, or pre-allocate fixed thinking
  budgets per worker.
- Keep the existing provider quota block behavior for compatibility. Provider quota
  is a separate safety mechanism and still requires explicit `--force` authorization
  to bypass.
- Do not claim real-time cost precision. Exact cost normally arrives only at
  `agent_end`; active-session projections are labeled estimates.

An additive session record may live under `~/.orchestra/sessions/<id>/session.json`
if needed for mutable session-level preferences. It must use the same atomic JSON
discipline. Run metadata remains independently readable, and Python commands must
continue to operate if the session record is absent.

### P0 — scalable live rendering

- Cache directory and file mtimes and parse only changed metadata.
- Diff snapshots by stable run identity instead of clearing the table.
- Keep selection anchored to a run/session key across refreshes and sorting.
- Render the visible viewport rather than materializing hundreds of styled rows.
- Page and mouse-wheel navigation operate over the same stateful selection model.
- Polling remains acceptable; async runtime complexity is not justified for local
  filesystem reads. Quota/network work and notification subprocesses run off the
  render path through small worker threads or equivalent bounded jobs.

### P1 — attention-first dashboard

The top-level hierarchy changes from five competing metric tiles to an operator
briefing:

1. Provider quota and advisory session budget.
2. Runs needing attention: failed, stalled, or awaiting handoff.
3. Active session progress and worker capacity.
4. A smaller value receipt with exact/estimated basis visible beside the number.

Activity becomes completed token volume over time where exact data exists, with
estimated volume visually distinct and run count retained as a secondary annotation.
Brain accounting separates fresh input, cached input, and output instead of presenting
one inflated token headline.

Empty state teaches the real workflow with copyable CLI examples and the relevant
keys. It does not fill space with decorative copy.

### P1 — actionable notifications

Notifications are enabled for actionable events by default:

- failed;
- stalled/idle timeout;
- handoff needed;
- whole-session completion.

Ordinary successful worker completion is quiet by default to avoid swarm spam.
On macOS, notifications use `osascript` through a subprocess. Failure to notify never
changes run status. Configuration supports disabling notifications or opting into all
completions.

### P1 — search and history

- Cross-run search covers run identifiers, tasks, sessions, and output text.
- Results retain run identity and line context and open the correct detail view.
- A session timeline orders dispatch, prompt delivery, completion, timeout, kill,
  retry, and handoff events from durable registry/inbox evidence.
- Full animated replay is secondary. It ships only if source events contain enough
  trustworthy timing information; no timestamps are invented.

### P2 — settings surface

Expose material settings in the TUI: warning thresholds, provider block threshold,
idle timeout, notifications, advisory budget defaults, parallel-worker display limit,
and theme. Changes shell through one Rust config command and use atomic JSON writes.
The TUI does not directly mutate configuration or run metadata.

Uncommon or experimental settings stay in `config.json`; the settings view must not
become a generic form dump.

## Session workspace design

### Default wide layout

The session screen opens as a split operational workspace:

- **Left, approximately 36%:** controller-to-worker topology and session state.
- **Right, approximately 64%:** the selected worker's conversation, live output,
  failure explanation, and contextual actions.

The topology is functional, not decorative. It contains:

- a controller node labeled `CODEX`, `CLAUDE`, or `HUMAN`;
- an optional `brain_model` label when the caller records it, with a graceful old-meta
  fallback to the controller name alone;
- worker nodes explicitly labeled `MINIMAX M3`;
- meaningful connections for dispatch, delivered follow-ups, retry, and handoff;
- status word, elapsed time, exact/estimated tokens, cost, and attention state;
- a selected path emphasized while unrelated branches recede;
- retry and handoff shown as new linked workers so history is never rewritten.

For narrow terminals, the split becomes a vertical topology followed by the selected
worker panel. The same information hierarchy survives; columns are not merely crushed.

### Visual language

- No emojis or pictographic identity shortcuts.
- No default ratatui white text, uniform boxes, or border around every element.
- Use negative space, alignment, line hierarchy, typographic contrast, and a small set
  of geometric marks.
- Every color comes from the shared theme token module.
- `ember` remains warm charcoal/amber rather than orange-on-black everywhere.
- `phosphor` remains a committed monochrome personality, with semantic exceptions only
  where necessary for errors and accessibility.
- State is always written as a word and never encoded by color alone.
- Motion is restrained to an optional low-frequency cell change on an active path.
  No glowing, pulsing panels, or decorative animation.
- The bottom action strip is contextual. A running RPC worker offers `SEND` and
  `KILL`; a failed run offers `RETRY` and `HANDOFF`; a completed run offers
  `HANDOFF` only when the operator or brain identifies remaining work.

Additional detail views remain available for Conversation, Log, Timeline, and Meta,
but the topology and current action state are visible in the default workspace.

## Rust architecture

Create a Cargo workspace under `rust/` with a binary named `orc`, built at
`rust/target/release/orc`.

Suggested module boundaries:

```text
rust/
├── Cargo.toml
└── crates/
    ├── orc-core/
    │   └── src/
    │       ├── registry.rs       atomic JSON, compatibility, reconciliation
    │       ├── model.rs          tolerant run/session/config structures
    │       ├── quota.rs          credentials, remains API, cache/history
    │       ├── runner.rs         json and RPC lifecycle
    │       ├── inbox.rs          kill, steering, delivery acknowledgements
    │       ├── control.rs        list, show, kill, retry, handoff, config
    │       ├── metrics.rs        exact/estimated accounting
    │       ├── notification.rs   actionable event policy and osascript
    │       └── search.rs         bounded cross-run search
    ├── orc-tui/
    │   └── src/
    │       ├── app.rs            event loop and commands
    │       ├── snapshot.rs       mtime cache and stable identity diff
    │       ├── dashboard.rs      attention-first overview
    │       ├── session.rs        responsive split workspace
    │       ├── topology.rs       controller/M3 layout and connections
    │       ├── timeline.rs       durable event reconstruction
    │       ├── theme.rs          all visual tokens
    │       └── widgets/          product-specific render primitives
    └── orc-cli/
        └── src/main.rs           clap surface, process exit mapping
```

The dependency floor is `clap`, `serde`, `serde_json`, and `anyhow` for the core,
with `ratatui` and `crossterm` for the TUI. Additional crates require a concrete
portability or correctness justification. Do not introduce Tokio merely to poll local
files and one subprocess.

Rust structs use defaults and optional fields rather than `deny_unknown_fields`.
Unknown Python or future metadata survives read-only operations. Commands that update
a run must begin from the parsed JSON object and preserve unrecognized fields.

## Compatibility and parity strategy

### Fixtures and golden behavior

- Reuse the Python suite's fake-pi scenarios against the Rust binary.
- Create golden metadata fixtures for old estimated runs, exact runs, sessions,
  corrupt metadata, missing optional fields, killed runs, orphaned PIDs, idle timeout,
  and RPC `agent_end` usage.
- Compare meaningful JSON structure and exit behavior, not whitespace from human
  tables unless table output is itself the compatibility contract.
- Test that prompt delivery is one-time and that an acknowledgement remains durable.

### Cross-language round trips

1. Rust creates and completes a run; Python `orc list/show/stats` reads it.
2. Python creates and completes a run; Rust `list/show/stats/top` reads it.
3. Rust reads an old meta, performs an allowed update, and preserves unknown fields.
4. Both implementations tolerate the absence of additive session records and v3
   linkage fields.

### Regression gates

- `.venv/bin/python -m pytest -q` stays green.
- `cargo fmt --check` passes.
- `cargo clippy -- -D warnings` passes.
- `cargo test` passes.
- The protected files' checksums remain unchanged.
- No test or install step replaces `~/.local/bin/orc` without a later explicit user
  decision.

## Benchmark plan

Build release mode before measuring. Use `hyperfine` for at least:

```sh
hyperfine 'python -m orc_pkg list' 'rust/target/release/orc list'
hyperfine 'python -m orc_pkg quota --json' 'rust/target/release/orc quota --json'
```

The quota comparison uses a warm cache. Record shell, machine, run count, warmup,
sample count, mean, range, and speedup in the README. If the result is noisy or the
Rust path is not materially faster, report that rather than choosing favorable runs.

## Implementation phasing after approval

### Phase 1 — orchestrated inventory and specifications

- Run `orc quota`, create one `ORC_SESSION`, and launch no more than three attributed
  `--brain codex` workers.
- Assign registry/interface inventory, test/CLI/theme inventory, and tightly specified
  Rust draft/documentation work.
- Monitor via `orc list --json`, review all outputs as untrusted, and retry a failed
  worker only once with a tighter prompt.
- Keep `docs/notes/2026-07-11-codex-orchestrate-friction.md` current with command,
  wording, watchdog, output-quality, and control-plane friction.

### Phase 2 — Rust parity core

- Implement tolerant models, atomic registry, quota, runner, lifecycle, signals,
  sessions, and parity CLI commands.
- Add golden and cross-language tests before building v3-only behavior.
- Keep commits small and conventional; push `v3-rust` as verified slices land.

### Phase 3 — intervention core and ratatui console

- Implement send, retry, handoff, advisory budgets, notifications, search, timeline,
  settings command, stable snapshot/diff, and viewport rendering.
- Build the attention-first dashboard and responsive split session workspace.
- Verify both themes and narrow/wide terminal behavior from the same tokens.
- Capture judge-facing terminal output with `vhs` or `termshot` in `docs/`.

### Phase 4 — proof and handoff

- Run Rust formatting, lint, unit/integration tests, the full Python suite, cross-language
  round trips, benchmarks, and `tests/live_smoke.sh` once with the Rust binary first on
  `PATH` without changing the symlink.
- Update README, guide, and `install.sh --rust`, leaving Python as default.
- Include `orc stats` as the orchestration cost receipt.
- Push `v3-rust` and provide shipped/cut/risk/benchmark/judge-scrutiny notes.
- Do not merge. The judge decides.

## Explicit cuts before core quality

If scope pressure appears, cut in this order:

1. animated session replay;
2. rich settings forms beyond material controls;
3. optional all-success notifications;
4. secondary dashboard decoration.

Do not cut registry parity, steering, retry, brain-reviewed handoff, advisory budget
visibility, stable rendering, the split session workspace, or the no-emoji visual
standard.

## Approval gate

No implementation branch, Rust workspace, worker swarm, code edit, commit, or push is
allowed until the user explicitly approves this design. After approval, create
`v3-rust` from `main`, carry this design document onto it, and begin the Phase 1
orchestrated inventory.
