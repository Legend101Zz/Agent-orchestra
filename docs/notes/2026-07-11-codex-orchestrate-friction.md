# Codex orchestrate friction — orc v3 Rust dogfood

**Started:** 2026-07-11  
**Brain:** Codex  
**Implementation branch:** `v3-rust`

This is a running, evidence-based log of friction encountered while using the
repository's own `orchestrate` flow to build v3. Worker output is treated as untrusted
until checked against repository files and tests.

## Session 1 — port inventories and interface draft

- Session: `orch-20260710-205821-v3`
- Pre-run quota: 97% remaining in the five-hour window, 46% weekly, level `ok`, API
  source. No `ORC WARNING:` or `ORC BLOCKED:` line was emitted.
- Workers:
  - `20260710-205821-read-the-approved-v3-des-6875`
  - `20260710-205822-audit-the-complete-curre-a03a`
  - `20260710-205822-using-docs-superpowers-s-3642`

### Skill wording and CLI mismatch

The orchestration guidance says to prefer `--thinking low` for tightly specified
drafts, but `orc run` and `orc rpc` do not expose a thinking-level option. The caller
cannot follow that recommendation through the registered orchestration path. The
worker prompts were tightened and explicitly asked for concise reasoning instead.

### Initial ergonomics

- Session grouping is easy to apply only when all launches share one shell containing
  the exported `ORC_SESSION`. A copied command issued later in another shell silently
  loses that grouping unless the environment or `--session` is repeated.
- Background launch output is only a bare run identifier. That is script-friendly,
  but the human has to retain the preceding session value separately.
- `orc top` is the only live view suggested by the flow; the CLI does not provide a
  session-scoped watch command.
- Registered JSON-mode worker logs grew to roughly 0.8–4.0 MB each during the first
  few minutes because raw agent/tool events are retained. This is good evidence but
  makes naive cross-run full-log search and frequent reparsing untenable; v3 needs
  file-offset/mtime-aware parsing.
- The growth then became extreme: active logs reached roughly 49 MB and 65 MB within
  four minutes. Inspection confirmed why: each `message_update` contains a small
  `assistantMessageEvent.delta` (49 bytes in the sampled event) plus a cumulative
  `message` snapshot (52,627 bytes in that same event). Persisting every raw update
  therefore produces near-quadratic log growth as an answer grows. A compatible v3
  optimization should retain the event type and delta needed by Python readers while
  omitting the redundant cumulative snapshot from stored `message_update` records;
  `agent_end` must remain intact for exact usage.

### Local gate setup

- The pre-change Python baseline is green: 86 tests passed in 15.83 seconds.
- Rust 1.91.1 is installed.
- `hyperfine`, `vhs`, and `termshot` were not installed when the session started.
  The requested benchmark and terminal capture steps therefore need explicit tool
  setup rather than assuming the developer machine already has them.

### Worker-quality review

Two of three workers completed successfully on their first attempt. Their reports were
useful as checklists, but they demonstrate why direct verification is mandatory:

- The interface-draft worker invented an interactive `handoff --confirm/--yes` flow,
  UUIDv7 prompt ids, extra session fields, and dependencies that the approved design
  did not request. It also proposed hand-written HTTPS as an alternative to a sync
  client, which is not a reasonable TLS implementation plan.
- The registry inventory correctly captured the subtle two-stage killed semantics
  (meta exit `-15`, shell exit `130`) and the `starting`/null-PID reconciliation edge.
  However, several `file:line` citations did not match the current files, it described
  the reverse-sort id tie-break in the wrong direction, and it introduced an
  unapproved `controller` metadata field.
- Both reports are being used as leads, not specifications. Current source and tests
  remain authoritative.

The third worker also completed successfully on its first attempt, but was the slowest
and used 119,174 tokens for an inventory prompt. Its audit accurately found the
missing intervention commands, positional selection restore, absent benchmark hooks,
and remaining emoji use. It also incorrectly described the exact-cost fallback in
`metrics._run_cost_usd` and misstated some test counts, so its arithmetic and inventory
claims were rechecked locally.

### Session receipt

All three workers finished with exit 0:

| Run suffix | Exact tokens | Cost |
|---|---:|---:|
| `6875` registry/runtime inventory | 73,674 | $0.025429 |
| `a03a` CLI/tests/TUI audit | 119,174 | $0.019775 |
| `3642` Rust interface/test draft | 67,355 | $0.019711 |
| **Total** | **260,203** | **$0.064915** |

Post-run quota was 89% in the five-hour window and 45% weekly, level `ok`, API source.
The three raw `output.log` files occupied approximately 98 MB, 30 MB, and 65 MB —
about 193 MB total for 260k exact tokens. This storage amplification is itself a
high-priority dogfood finding.
