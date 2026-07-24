# Findings (durable discoveries & decisions)

Older findings: the v3-rust review findings that previously lived here are
resolved (v4 Phases 0–6) and preserved in git history and
`docs/reviews/2026-07-11-v3-rust-review.md`.

## 2026-07-22 — V1 program setup

- **Positioning locked:** "turn the pile of AI subscriptions you already pay
  for into one orchestra." Differentiator vs OpenRouter Fusion/MoA services:
  panels spread across sunk-cost subscriptions, not one metered pool.
- **Skills teach intent; CLI/MCP performs the operation.** Skills alone give
  inconsistent invocation — every dependable action must be an `orc` verb
  and (where supported) an MCP tool.
- **Trigger highlighting reality check:** Claude Code and Codex input UIs are
  closed — no ultrathink-style highlight possible there; acknowledge via
  hook/status output instead. Highlighting IS possible where we own the
  renderer (hosted panes) and in extensible harnesses (pi).
- **Credential sharing is out, permanently.** V3 collaboration = capability
  advertisement + artifact exchange; credentials never leave a machine.
  Provider ToS make account-proxying a non-starter for an OSS project.
- **code-puppy integration surface:** reads root `AGENTS.md`
  (also `.code_puppy/AGENTS.md`), custom slash commands from
  `.agents/commands/*.md`, JSON agents in `~/.code_puppy/agents/`, models in
  `~/.code_puppy/extra_models.json`; MCP via `/mcp`; default agent prefers
  files ≤600 lines.
- **CLI naming (2026-07-22):** user-facing command is `pio` (daemon `piod`),
  chosen over `pioh` (awkward to type) and keeping `orc` (weak brand tie).
  Scope of rename is user-facing only — crate names, `ORC_*` env vars,
  `~/.orchestra` stay (issue #17).
- **Research-first (2026-07-22):** crate/prior-art choices for V1's new
  surface (MCP SDK, headless harness invocation, worktrees, backoff, schema)
  are decided once in issue #16 by a Claude session with web access, recorded
  as a decision doc, and bind the implementation issues — instead of each
  puppy session picking dependencies ad hoc.
- **Visual identity source:** `docs/design/visual-identity/` (interactive
  HTML + screenshots), distilled to `docs/design/visual-identity.md`.
  Three themes (nocturne flagship / ember / phosphor mono), 17 semantic
  slots, glyph register with ASCII fallbacks, baton pulse spec.

## 2026-07-24 — Capability probe: advertisement ≠ runtime (from #6 real-CLI review)

- **The `pio doctor` probe is help-token based and correctly per-harness.**
  Each capability is proven by finding a harness-specific flag in that harness's
  `--help` corpus (`probe/profiles.rs`). Verified live: Hermes (empty
  structured-output and working-dir proof tokens) probes those two **false**,
  while claude/codex/pi/opencode advertise and probe all eight **true**. It is
  NOT rubber-stamping. (An earlier reviewer note claiming "identical capabilities
  for all five" was a bug in the *inspection* script — it sorted the report's
  `capabilities` map keys instead of filtering by `value == true`; the doctor
  output is a full `{cap: bool}` map, so every capability name appears as a key.)
- **What a positive probe guarantees:** the flag is *advertised*, not that the
  one-shot invocation actually *runs* on this host. Live proof: codex advertises
  `exec` / `--json` / `-C` (all probe true) yet `codex exec` refuses to start in
  a non-git dir without `--skip-git-repo-check`. No help token can surface that.
- **Where runtime quirks live:** per-adapter invocation *templates*
  (`orc-core/src/invocation.rs`, the `fixed` flags slot), NOT the probe and NOT
  as synthetic capabilities. Codex's `--skip-git-repo-check` is the first entry;
  it is permissive only — never a sandbox/approval-skip (#16). Rule of thumb for
  a new harness: `profiles.rs` = *what it advertises*; `invocation.rs` = *what it
  takes to actually launch it*.
- **Deliberately NOT done:** no execution/"smoke" probe that spawns a real
  one-shot to verify it runs. That would make `pio doctor` incur real model
  calls (cost + latency) on every refresh; the template layer already carries the
  known runtime quirks. Revisit only if per-harness launch quirks proliferate.
