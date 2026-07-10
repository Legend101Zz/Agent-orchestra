# v3-rust review — verdict and findings

**Reviewer:** Claude (reviewing brain), 2026-07-10/11
**Scope:** branch `v3-rust` (9 commits over `main`), spec `docs/superpowers/specs/2026-07-11-orc-v3-rust-design.md`
**Method:** full source read (Rust + Python), invariant experiments against fake and real pi, cross-language round-trips, reproduced benchmarks, live TUI runs, `tests/live_smoke.sh` with the Rust binary first on PATH.

## Verdict: **fix-first**

The port is high quality — registry discipline, kill semantics, exit codes, compatibility, and the ratatui console all check out, and the benchmark claims reproduce. But two P0 defects gate the merge: live steering (the flagship v3 intervention feature) silently loses the steered turn against real pi, and the quota gate lost Python's network timeout so it can hang — i.e. fail **closed** — when the endpoint stalls. Both are contained, well-understood fixes.

### Ordered fix list (merge after 1–3)

1. **Bound the quota fetch.** Add `timeout_global(Some(15s))` (and a connect timeout) to the `ureq` call in `quota.rs`, matching Python's `urlopen(..., timeout=15)`; also bound the `security` keychain subprocess (Python uses 10 s).
2. **Make steering survive the turn boundary.** In `runner.rs`, count delivered prompts and only finish the RPC loop when `agent_end` events ≥ 1 + delivered prompts (idle watchdog stays as the backstop; drain pending prompts before deciding). Update the fake-pi in `tests/test_rust_parity.py::test_rpc_send_delivers_once_and_acks` to emit **one `agent_end` per prompt** — the current fixture blocks on the follow-up before any `agent_end`, which is exactly the shape that masks this bug.
3. **Refresh quota in the TUI, off the render path.** `App::new` fetches quota synchronously (network before first paint) and the event loop never refetches, so a long-lived `orc top` shows frozen meters and level forever. Fetch on a worker thread on the cache-TTL cadence.

Findings 4–7 below are non-blocking.

## Findings (ranked)

| # | Severity | Where | Failure scenario (one line) |
|---|----------|-------|------------------------------|
| 1 | **P0** | `rust/crates/orc-core/src/runner.rs:378-383`, `control.rs:154` | `orc send` (or TUI `s`) delivers a follow-up mid-turn; the runner breaks on the **first** `agent_end`, drops stdin, and finalizes "done" — the steered turn's output and usage are silently lost while the inbox shows the prompt ack'd as processed. **Live-confirmed** (run `20260710-223438-write-one-short-sentence-b3c0`: follow-up delivered, 1 `agent_end` in log, reply contains only turn 1; also reproduced with a two-turn fake pi — turn-2 usage 24 tokens dropped, meta kept turn-1's 12). |
| 2 | **P0** | `rust/crates/orc-core/src/quota.rs:114-124` | MiniMax endpoint hangs (dropped packets rather than refusal) → `fetch_remains` blocks forever (verified: ureq 3.3.0 `Timeouts::default()` is all-`None`) → `apply_gate` never returns → **every** `orc run/rpc/retry/handoff` and `orc top` startup hangs; violates "quota gate fails open, never blocks work." Python had `timeout=15` (`orc_pkg/quota.py:77`) — this is a port regression. |
| 3 | Medium | `rust/crates/orc-tui/src/app.rs:109-110`, `lib.rs:22-45` | Quota + history are fetched once in `App::new` (blocking, on the startup path) and never again; an operator watching `orc top` for hours sees a frozen quota meter/level while workers burn the window — the v2 floor had live meters, and the spec requires network work off the render path. |
| 4 | Low | `rust/crates/orc-tui/src/app.rs:394-411` | TUI actions shell out synchronously; `orc kill` polls up to 5 s, freezing the whole console mid-render (spec: subprocesses off the render path). |
| 5 | Low | `rust/crates/orc-core/src/model.rs:104-114`, `registry.rs:251` | A hand-edited/legacy meta missing `brain`/`provider`/`model` parses in Python (`dict.get`) but fails Rust's required fields, so `orc list` **silently drops** the run — no panic, but data quietly hidden (verified with fixture). |
| 6 | Low | `rust/crates/orc-core/src/quota.rs:64-73` | `security find-generic-password` runs with no timeout (Python: 10 s); a wedged Keychain hangs quota → same blast radius as #2. Fold into fix 1. |
| 7 | Hygiene | `rust/crates/orc-tui/Cargo.toml:16` | `unicode-width` is declared but unused in `orc-tui` (ratatui handles width internally); remove or use. |

### Invariants verified (all pass)

- `--offline` + explicit `minimax`/`MiniMax-M3` in both JSON and RPC arg sets (`runner.rs:19-40`).
- RPC holds stdin open until `agent_end`; exact usage/cost read from `agent_end` `messages[].usage` (last non-zero, matching Python).
- pi SIGTERM trap → 143 → meta `killed` / `exit_code: -15`, `orc kill` exit 0 (experiment + live smoke check 7).
- Idle watchdog → exit 124, status `failed`, additive `attention: handoff_needed` / `failure_kind: idle_timeout` (experiment).
- Quota gate exits 3 on block with `ORC BLOCKED` line, `--force` proceeds; `unknown` level proceeds (fail-open **at the logic level** — see finding 2 for the transport-level hole). Coding plan = `model_name:"general"`.
- Registry writes: temp file in destination dir + flush + `sync_all` + atomic rename, `create_new` temp, cleanup on error (`registry.rs:67-97`). Unknown JSON fields survive Rust rewrites (verified: `v2_custom_field` preserved through orphan reconcile).
- TUI is read-only: orphan projection is view-only (`snapshot.rs:47-50`); actions go through CLI subprocesses.
- Round-trips: Rust-written runs read by `python -m orc_pkg list/show/stats` and vice versa (parity suite + manual experiments); legacy v2 metas (no `session`/`mode`/exact tokens) render in CLI and TUI; corrupt meta → clean error in Rust (Python `show` tracebacks — Rust is better here).
- Protected files untouched: `~/.pi/agent/*`, `~/.claude/settings.json`, `~/.codex/config.toml` mtimes predate the branch work; `~/.local/bin/orc` still → `bin/orc` (Python).
- No emojis; no raw colors outside `theme.rs`; both themes coherent; CJK task metas render without panic; 72×30 reflows with the information hierarchy intact.

### Gates

`.venv/bin/python -m pytest -q` → **92 passed**. `cargo test` → 13 passed. `cargo clippy --all-targets -- -D warnings` → clean. `cargo fmt --check` → clean. `tests/live_smoke.sh` (Rust orc first on PATH) → **10/10 PASS**.

### Benchmarks (reproduced independently)

hyperfine 1.20, 20 runs / 5 warmups, 500-run seeded fixture, warm quota cache, M-series MacBook Air:

| Command | Python mean | Rust mean | Speedup |
|---------|------------:|----------:|--------:|
| `orc list` (500 runs) | 132.4 ± 8.8 ms | 24.0 ± 3.2 ms | **5.5×** |
| `orc quota --json` (cached) | 99.6 ± 2.1 ms | 5.0 ± 0.4 ms | **20.1×** |

Consistent with the README's claimed 5.84× / 14.43× — the table was measured, not asserted (quota reproduced even faster here).

## Process assessment (orchestrate's first Codex-driven run)

**It worked, with honest accounting.** Registry evidence corroborates `docs/notes/2026-07-11-codex-orchestrate-friction.md` exactly: session `orch-20260710-205821-v3` ran 3 workers, all `brain: codex`, all `done` first-attempt (0 stalls, 0 kills in-session), exact usage 260,203 tokens / **$0.0649** total. Registry-wide: 16 runs, 8 exact (99% of tokens exact-basis), 3 killed + 1 failed are all human-brain smoke/test runs. Worker reports were used correctly as *leads*: the friction log documents invented flags (`--confirm/--yes`), phantom fields, and wrong `file:line` citations that were caught by verification — the "workers are untrusted" rule earned its keep.

Real friction found and worth fixing in the skills (proposals — not applied unilaterally):

1. **`skills/orchestrate/SKILL.md` step 3** — the example says `--brain claude`; a Codex brain following it literally mis-attributes the swarm. Change to `--brain <your-brain> (claude|codex)` and add `--session "$ORC_SESSION"` explicitly to the example command, with a note: *"a command copied into another shell loses the exported `ORC_SESSION`; pass `--session` explicitly when in doubt"* (observed friction: silent grouping loss).
2. **`skills/orchestrate/SKILL.md` step 6** — replace *"total estimated tokens (from `orc list --json` → `tokens.estimated_total`)"* with *"exact tokens and cost where present (`tokens.total`, `tokens.cost_usd`; fall back to `~estimated_total`), plus the `orc stats` receipt"*. The estimated-only wording predates exact usage capture and undersells the accounting the registry now does.
3. **`skills/orchestrate/SKILL.md` step 4** — add: *"do not pass pi options `orc` doesn't expose (there is no `--thinking`); tighten the prompt instead"* — the session lost time to exactly this mismatch.
4. **`codex/AGENTS-block.md`** — same exact-token reporting fix as (2); after fix #2 above lands, also advertise `orc send/retry/handoff` so brains steer through the product instead of reconstructing context.
5. **Log growth**: the friction log's 49–65 MB `message_update` amplification is real; the Rust runner now strips the cumulative snapshot (verified in parity test). Note in `pi-delegate`/`orchestrate` that Python-runner JSON swarms still amplify, so prefer the Rust runner for multi-worker sessions once merged.

## What I did *not* find

No partial-write corruption path (atomic rename everywhere, single-writer honored, kill/reconcile fallback writes match Python's existing behavior). No panic on malformed, legacy, or unicode-heavy metas. No quota fail-closed at the *logic* level (only the transport-level hang of finding 2). No stock-ratatui look: theme tokens are total, the two themes are genuinely distinct, and the empty state teaches the real workflow.

---
*Review artifacts: benchmarks in `/tmp` scratchpad (hyperfine markdown), steering probe run `20260710-223438-*` in `~/.orchestra/runs/`, live smoke transcript 10/10.*
