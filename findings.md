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
- **Visual identity source:** `docs/design/visual-identity/` (interactive
  HTML + screenshots), distilled to `docs/design/visual-identity.md`.
  Three themes (nocturne flagship / ember / phosphor mono), 17 semantic
  slots, glyph register with ASCII fallbacks, baton pulse spec.
