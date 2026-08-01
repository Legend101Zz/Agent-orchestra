# V1 foundations — crate & prior-art decisions

Date: 2026-07-22 · Status: binding for V1 implementation issues · Produced
by: issue #16 (Claude research session, no code) · Spec:
`2026-07-22-v1-universal-delegation-design.md`

Method: crates.io metadata + upstream repos verified 2026-07-22; harness
flags verified against the **locally installed binaries** (ground truth, not
docs): claude 2.1.217, codex-cli 0.145.0, opencode 1.18.4, hermes 0.18.2,
pi 0.80.7. Prior art mined: BloopAI/vibe-kanban (Rust, closest cousin),
smtg-ai/claude-squad, opencode, NousResearch/hermes-agent, togethercomputer/moa,
OpenRouter Fusion.

Existing-stack constraint respected: the workspace is **fully synchronous**
(no tokio; ureq, signal-hook, portable-pty, std::process). Nothing below
changes that for existing crates.

---

## 1. MCP server SDK — `rmcp` (official), isolated in the new `orc-mcp` crate (#8)

**Chosen:** `rmcp` v2.2.0 · Apache-2.0 · official
`modelcontextprotocol/rust-sdk` — repo pushed the day of this research,
3.6k★, active issue triage. Features: `server` + `macros` (default), stdio
transport (`transport-async-rw`). Tool schemas come from `schemars` via the
`#[tool]` proc macro — which is exactly the schema crate chosen in §5, so
contract types can derive one schema used by both CLI docs and MCP tools.

**The tokio tension, resolved:** rmcp requires tokio (`sync, macros, rt,
time` only — not `full`). The workspace is sync and stays sync. The daemon
protocol boundary already exists (`orc-proto` over the Unix socket), so
`orc-mcp` is a thin translator: MCP stdio in → existing sync daemon calls
out. It runs a self-contained `current_thread` tokio runtime that never
leaks into `orc-core`/`orc-daemon`. Issue #8 already allows a new crate.

**Rejected:**
- *Hand-rolled JSON-RPC 2.0 over stdio* (zero new deps — tempting for a
  sync codebase): the protocol surface is more than a dispatch loop —
  initialize/capabilities handshake, protocol-version negotiation,
  cancellation, progress notifications, and a spec that still moves.
  Conformance drift would be our bug tail forever; rmcp gives it free.
- *`mcp-sdk`* v0.0.3: last publish Jan 2025, ~12k downloads total. Dead.
- *`mcpr`* v0.2.3: last publish Mar 2025, ~11k downloads. Dead.

## 2. Headless harness invocation — probe-driven adapter templates (#4, #6)

**Chosen pattern:** one invocation template per harness, selected by probe
results (never by version pinning — vibe-kanban pins CLI versions and pays
for it with constant bump releases; `pio doctor` probes instead). Spawn
per delegation with `std::process::Command` in sync `orc-core` — no
runtime change. Capture the session id from the first structured event and
store it on the task for resume.

Ground-truth invocation table (verified locally, 2026-07-22):

| Harness | One-shot | Structured output | Resume | Cwd | Usage |
|---|---|---|---|---|---|
| claude 2.1.217 | `claude -p "<brief>"` | `--output-format stream-json --verbose` (NDJSON events; `json` for final blob) | `--resume <sid>` / `-c`; `--fork-session` | spawn cwd + `--add-dir` | usage/cost fields in result event |
| codex 0.145.0 | `codex exec "<brief>"` (or stdin) | `--json` (JSONL events); `--output-schema <file>`; `-o <file>` last message | `codex exec resume <sid>\|--last` | `-C <dir>`; `--skip-git-repo-check` | token counts in events |
| opencode 1.18.4 | `opencode run "<brief>"` | `--format json` (raw JSON events) | `-s <sid>` / `-c`; `--fork` | `--dir <dir>` | in events |
| hermes 0.18.2 | `hermes -z "<brief>"` (prints only final answer) | final text; `--usage-file <path>` writes JSON usage report | `--resume <sid>` / `--continue [name]`; `--pass-session-id` | spawn cwd; has own `--worktree` (don't use; ours isolates) | `--usage-file` |
| pi 0.80.7 | `pi -p "<brief>"` | `--mode json` (`rpc` for bidirectional) | `--session-id <id>` (creates if missing — deterministic), `-c`, `--resume` | spawn cwd + `--session-dir` | in json events |

Permissions for unattended runs: claude `--permission-mode` /
`--allowedTools` (never default to `--dangerously-skip-permissions`);
codex `-s <sandbox_mode>`; opencode `--auto` (flagged dangerous — surface
to user); hermes `--yolo` (same); pi tool config. Contract
forbidden-actions map onto these per-harness.

**Prior art mined:**
- *vibe-kanban* (Rust; `crates/executors/src/executors/*.rs`) — the
  closest cousin. Drives claude with `--verbose
  --output-format=stream-json --input-format=stream-json
  --permission-prompt-tool=stdio` — a **bidirectional control protocol**
  (`control_request`/`can_use_tool` events) mediating permissions
  per-call, plus `--resume-session-at <uuid>` to truncate history. Drives
  codex via `codex app-server` (JSON-RPC server), opencode via
  `opencode serve --hostname 127.0.0.1 --port 0` + HTTP. Steal: executor
  abstraction, worktree lifecycle. Avoid: version pinning, and the
  server-mode complexity until V1 proves spawn-per-task insufficient.
- *claude-squad* (Go): tmux + worktrees puppeteering interactive TUIs —
  that's our hosted-pane PTY mode (already built on portable-pty), not
  delegation. Validates the two-mode split.
- *aider*: `--message` one-shot + `--yes`; same shape, nothing new.

**V1 decision:** simple one-shot spawn + JSON parsing. The fancier
channels (claude `--input-format stream-json`, codex `app-server`,
opencode `serve`) are recorded as the V1.5 upgrade path when we need
mid-task permission mediation or lower latency.

## 3. Git worktree management — shell out to the `git` CLI (#11)

**Chosen:** `git worktree add/list/remove/prune` via
`std::process::Command`, parsing `git worktree list --porcelain` (stable,
documented, line-oriented). Version: whatever `git` is on PATH (worktree
porcelain stable since git 2.17, 2018). License: n/a (external tool). One
small module in `orc-core`; zero new dependencies.

Why: the repo currently has **no** git dependency; worktree operations are
porcelain-level lifecycle commands, not object-database work; the git CLI
is guaranteed present in any repo pi-orchestra manages. claude-squad and
workmux shell out for exactly this. Cleanup semantics stolen from
vibe-kanban: worktree + branch per task; on cleanup remove the worktree
and delete the branch only if unmerged work was abandoned deliberately —
never silently.

**Rejected:**
- *`git2`* v0.21.0 (MIT/Apache-2.0, rust-lang org): binds libgit2 1.9 —
  the final 1.x line, with 2.0 API/ABI changes ahead; worktree API covers
  add/open but the prune/list ergonomics still push you to the CLI;
  vibe-kanban uses it *and still* needed a dedicated `worktree-manager`
  crate. A C build dependency for a lifecycle we can drive in ~100 lines
  of porcelain calls is bad trade.
- *`gix`* (gitoxide) v0.85.0 (MIT/Apache-2.0): impressive and active, but
  worktree *management* (add/remove/prune) is still maturing
  (checkout-focused today), the API churns fast, and the dependency tree
  is large. Re-evaluate for V2 if we start reading objects/status
  in-process.

## 4. Retry/backoff + rate-limit detection — `backon` + our own signal table (#7)

**Chosen:** `backon` v1.6.0 · Apache-2.0 · MSRV 1.85 (ours: 1.91) ·
Xuanwo/backon, ~63M downloads, zero dependencies, stable since 1.0. Key
fit: first-class **blocking** retry (`BlockingRetryable` + `StdSleeper`) —
no async runtime needed, matching the sync workspace. Exponential +
jitter built in; `adjust()` supports dynamic backoff (e.g. honoring a
retry-after hint parsed from harness output).

Rate-limit **detection** is ours, not a crate's: a per-harness signal
table (patterns in structured error events / stderr) maintained next to
the adapter templates, feeding quota guard v2's backoff + ORC WARNING
channel. Exact per-harness signal strings are an open question (below) to
be captured with fixtures during #4/#7 — do not guess them in code.

**Rejected:**
- *`backoff`*: RUSTSEC-2025-0012 — officially unmaintained. No.
- *`tokio-retry`*: async-only; would drag tokio into `orc-core`.
- *`retry`* v2.2.0: alive but minimal — fixed strategies, no dynamic
  adjustment; backon strictly dominates at the same weight.

## 5. Schema & validation — `schemars` + serde, additive-JSON pattern kept (#3, #5)

**Chosen:** `schemars` v1.2.1 · MIT · MSRV 1.74 · ~351M downloads (the
de-facto standard; last publish Feb 2026). Contract v2 and
`harnesses.json` types derive `Serialize + Deserialize + JsonSchema`.
Synergy: rmcp's `#[tool]` macro consumes schemars 1.x schemas — one
derive powers the MCP tool surface (#8), `pio task add` validation (#5),
and the registry shape (#3).

Additive-JSON invariant (AGENTS.md) is a serde pattern, not a crate:
every persisted struct carries
`#[serde(flatten)] extra: serde_json::Map<String, Value>` so unknown
fields survive round-trips; write tests that prove a future-field file
survives read→write. (This is the existing convention — keep it.)

**Rejected:**
- *`garde`* v0.23 / *`validator`*: field-level input validation (email,
  ranges) — wrong niche; contract validation is structural + semantic and
  lives in our code.
- *`jsonschema`* v0.48.5 (instance validation): not needed at runtime —
  serde's typed parsing *is* our validation. Worth considering later as a
  **dev-dependency** to assert emitted schemas validate fixture files;
  noted, not chosen.
- *`typify`* (schema→Rust): wrong direction; our types are the source of
  truth.

## 6. Snapshot testing — keep `TestBackend`, adopt `insta` for the buffers (all UI issues)

**Chosen:** `insta` v1.48.0 · Apache-2.0 · MSRV 1.66 · ~81M downloads,
last publish Jun 2026 (Mitsuhiko) — as **dev-dependency only**, with the
`cargo-insta` review tool. Pattern (also the one the ratatui community
converged on): render into `ratatui::backend::TestBackend`, snapshot the
buffer with `assert_snapshot!`. Existing hand-rolled buffer assertions in
`orc-tui`/`orc-app` keep working; new/updated UI tests move to `.snap`
files so a visual change is a reviewable diff + `cargo insta review`, not
a hand-edited expected-buffer literal inside a 2 600-line `lib.rs`.

**Rejected:**
- *Status quo (hand-rolled expected buffers)*: every intentional visual
  change means hand-rewriting buffer literals — high-friction, and diffs
  drown reviewers. It's the reason snapshot tooling exists.
- *`expect-test`* (rust-analyzer style): solid, but no redactions, no
  review UI, and the TUI ecosystem's examples/patterns are insta-shaped.

## 7. Prior-art scan — what to steal, what to avoid (V2 planning; one page)

**hermes-agent #344** (multi-agent umbrella): draws the exact line the V1
spec draws — isolated children returning summaries is *delegation*; true
orchestration adds specialized roles, dependency-aware DAGs, cooperation,
crash recovery, health monitoring, shared state. Hermes today: depth ≤2,
≤3 parallel children, no retry, no synthesis. **Steal:** its sub-issue
decomposition — acceptance criteria + *independent judge* for delegation
quality (#356) is literally our issue #11; adversarial debate (#376) and
shared memory pools (#377) map to our V2/V2.5. **Avoid:** shipping
parallel dispatch without synthesis or retry — that's the gap it's
complaining about.

**hermes-agent #38952** (configurable MoA): hardcoded frontier panel +
max reasoning = "stupidly expensive", deliberately default-off. **Steal
the config axes for V2 deliberate mode:** reference models, aggregator,
temperatures, reasoning effort, minimum successful references,
session-level on/off, explicit per-request routing (slash-command
equivalent = our `deliberate:` trigger). Our spec already commits to
this; #38952 is the evidence it matters.

**togethercomputer/moa** (Apache-2.0, reference implementation): layered
proposers → aggregator; knobs: aggregator, reference models, temperature,
`rounds` (layers−1), parallelism, multi-turn. +7.6% AlpacaEval 2.0 over
GPT-4o using only OSS models. **Steal:** the minimal panel shape (N
proposers, 1 aggregator, optional refinement rounds) as V2's starting
topology.

**OpenRouter Fusion:** judge produces a *structured analysis* —
consensus, contradictions, partial coverage, unique insights, blind
spots — then writes the final answer from it. **Steal that report format**
for deliberate mode's "consensus/disagreement/blind-spot report" (spec
already names it). Cost reality: request price = sum of all panel +
judge completions; their Quality preset ≈ 3× solo Opus. **This is
pi-orchestra's differentiator validated:** Fusion burns one metered pool;
we spread panels across sunk-cost subscriptions.

**claude-squad / vibe-kanban** (operational prior art): covered in §2/§3.
One-line verdicts — claude-squad: tmux+worktree interactive parallelism =
our hosted-pane mode, validates the split; vibe-kanban: best-in-class
executor abstraction to steal, version-pinning treadmill to avoid.

---

## Binding summary (what implementation issues must use)

| Issue | Binding decision |
|---|---|
| #3 | schemars-derived registry types; `#[serde(flatten)]` extra-field pattern; record path + version string + mtime per harness |
| #4 | probe matrix per §2 table; capabilities from probes, never version pins; cache keyed on binary identity |
| #5 | schemars v1.2.1 derives on contract v2; additive-JSON flatten pattern + round-trip test |
| #6 | §2 invocation templates; sync `std::process::Command`; session-id capture for resume; honest refusal from probe results |
| #7 | backon v1.6.0 blocking retry; per-harness rate-limit signal table (fixtures first) |
| #8 | rmcp v2.2.0 (`server`+`macros`, stdio) in new `orc-mcp` crate; self-contained current_thread tokio; translate to existing daemon protocol |
| #11 | git CLI shell-out for worktrees; `--porcelain` parsing; vibe-kanban cleanup semantics |

## Open questions (listed, not researched — timebox honored)

1. **Exact rate-limit signal strings** per harness (claude stream-json
   error subtype, codex event, opencode event, hermes stderr, pi json) —
   capture empirically with fixtures during #4/#7.
2. **claude stream-json event schema stability** across versions — the
   probe should record schema shape, and parsers must tolerate unknown
   event types (same additive philosophy as our JSON).
3. **hermes one-shot resume semantics** — `-z` with `--resume` +
   `--pass-session-id` interaction needs a live experiment before #6
   relies on it.
4. **opencode `run` vs `serve --attach`** for repeated dispatches (cold
   MCP boot cost per run) — measure during #6; switch only if spawn cost
   is real.
5. **codex `app-server`** JSON-RPC mode vs `exec` — richer control,
   more surface; defer unless #6 hits `exec` limits.
6. **MCP progress/cancellation notifications** — which conductors
   actually consume them; affects how much of rmcp's surface #8 wires up.
