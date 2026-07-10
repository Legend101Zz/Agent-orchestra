# NEXT SESSION PROMPT — orc top v2: a control plane worth staring at

Copy everything below the line into a fresh Claude Code (Opus/Fable) or Codex session
started in `/Users/comreton/Desktop/pi-orchestra`.

---

## Who you are for this session

You are the most ruthless, conversion-obsessed founder-designer alive, and you are
also the engineer who ships it. The current `orc top` works but looks like a Textual
default — stock blue accent, flat table, no soul. You are replacing it with a control
plane people screenshot and post. Every panel must earn its pixels: if a widget
doesn't help the operator decide *"is my orchestra healthy, what is it costing me,
what do I kill?"* in under two seconds, it doesn't ship.

**Anti-slop design directive (non-negotiable):** You tend to converge toward generic,
on-distribution outputs. Avoid the "AI slop" aesthetic: no stock Textual blue
(`#0178d4`), no default theme, no predictable layouts, no cookie-cutter component
patterns. In a TUI your "typography" is glyph craft: box-drawing weights, braille
graphs, block-element gradients, casing, letter-spacing of labels, deliberate density.
Make creative, distinctive choices that feel genuinely designed for *an orchestra of
AI workers* — surprising and delightful, not templated. Think outside the box.

## Project context (read this, then verify in the repo)

`pi-orchestra` lets brains (Claude Code, Codex) delegate heavy work to a cheap
1M-context worker (pi CLI running MiniMax-M3), with quota gating and a file registry.
Read `README.md` and `docs/superpowers/specs/2026-07-10-pi-orchestra-design.md` first.

- CLI: `orc` (`orc_pkg/` package, Python 3.14, repo venv `.venv`, Textual 8.2.8).
  Subcommands: run, rpc, list, show, kill, quota, top, hidden `_exec`.
- Registry: `~/.orchestra/runs/<id>/{meta.json, output.log, inbox/}` — plain JSON,
  atomic writes, single-writer rule (only the owning orc process writes meta;
  the TUI is read-only and must stay that way — kills/new tasks shell out to
  `python -m orc_pkg kill/run`).
- Tests: `.venv/bin/python -m pytest -q` → 26 passing now. Keep them passing.
  Live checks: `tests/live_smoke.sh` (costs cents, uses real MiniMax).
- Verified facts you must not re-derive wrong:
  - Always pass `--offline` on every pi invocation (startup net check can hang 5 min).
  - `pi -p --mode json` and `pi --mode rpc` both emit JSONL events; assistant text
    arrives as `{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":…}}`;
    terminal event `{"type":"agent_end","messages":[…]}`; assistant messages carry
    exact `"usage":{"input":…,"output":…,"cacheRead":…,"totalTokens":…,"cost":{"total":…}}`.
  - pi rpc exits when stdin closes — hold stdin open until `agent_end`.
  - pi traps SIGTERM → exit 143; orc maps 143/inbox-marker to status `killed`.
  - MiniMax quota API: `GET https://api.minimax.io/v1/token_plan/remains`, coding plan
    is the `model_name:"general"` entry (5-hour + weekly `*_remaining_percent`).
  - The MiniMax API stalls on ~half of long calls some days: the idle watchdog
    (`idle_timeout_sec`, exit 124) is load-bearing. `--thinking low` for drafts.
- Protected: never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`. Registry schema changes must be backward compatible
  (old runs without new fields must still render).

## Inspiration to actually study (don't skip)

- **btop++** — <https://github.com/aristocratos/btop> — the target vibe for the main
  dashboard: rounded panels with titles embedded in the border, *gradient* meters,
  braille/block history graphs, mouse + vim keys, theme files. Steal the feeling of
  "dense but instantly legible instrument cluster".
- **Posting** — <https://github.com/darrenburns/posting> — the target vibe for the
  session drill-in: app-like Textual UI, tabbed request/response panes, syntax
  highlighting, command palette, "jump mode" keyboard nav, custom named themes.
- **toolong** — <https://github.com/Textualize/toolong> — log tailing done right
  (live tail, scrollback, search); model the Log tab on it.
- **dolphie** — <https://github.com/charles-001/dolphie> — a Textual metrics
  dashboard in production; good patterns for metric tiles + graphs.
- **netext** — <https://github.com/mahrz24/netext> — terminal node-graph rendering
  for networkx graphs with a native Textual widget (click events). Candidate for the
  orchestration diagram. Evaluate it honestly: if it fights you, hand-roll the
  diagram with box-drawing + connectors instead. Textual also ships a `Sparkline`
  widget, and `textual-plotext` exists for charts.
- **ccusage** — <https://github.com/ryoppippi/ccusage> (and
  <https://ccusage.com>) — proves brain-side usage is parseable: Claude Code writes
  one JSONL per session under `~/.claude/projects/<slug>/*.jsonl`, each line with
  per-call `usage` (input/output/cache tokens, model). Parse those locally (do NOT
  add a Node dependency); Codex equivalents live under `~/.codex/sessions/` —
  support best-effort, degrade gracefully to "n/a".

## The work

### 1 · Data layer first (make the UI honest)

1. **Exact usage for one-shot runs**: switch `runner._exec` to `pi -p --mode json …`;
   keep echoing extracted text deltas to the caller's stdout (callers must still see
   plain text!), write raw events to `output.log`, and record exact
   `tokens {input, output, cache_read, total, cost_usd}` in meta (reuse
   `_extract_usage`, already written for rpc). Estimates remain only as fallback.
2. **Session grouping**: `orc run/rpc --session <id>` and `ORC_SESSION` env var →
   `meta["session"]` (absent = ungrouped; old runs must still work). Update the
   `orchestrate` skill (`skills/orchestrate/SKILL.md`) and the Codex block
   (`codex/AGENTS-block.md`) to export `ORC_SESSION="orch-<timestamp>-<slug>"` so a
   swarm shows up as one session.
3. **Metrics module** (`orc_pkg/metrics.py`) + `orc stats [--json]`:
   - Worker side (exact): aggregate registry tokens/cost by brain, by session, by
     day; runs count by status.
   - Brain side (estimate): parse `~/.claude/projects/**/*.jsonl` usage lines and
     `~/.codex/sessions/**` (best effort) → tokens by model for today / this week.
     Label these clearly as *API-equivalent value* (subscriptions are flat-rate).
   - The hero number: **"delegated value"** — API-equivalent cost of the tokens the
     workers burned at *brain* prices vs what MiniMax actually cost. That's the
     product's reason to exist; put it where eyes land first.
   - Cache the parse (it can be MBs) with mtime-based invalidation. Unit-test with
     fixture JSONL files; never let a malformed line crash the TUI.

### 2 · Dashboard redesign (btop energy)

Replace the current single-table layout with an instrument cluster:

- **Top strip**: quota as two gradient meters (green→amber→red *within* the bar, btop
  style) with threshold ticks at warn/block, reset countdowns, and a compact history
  sparkline of quota over time (sample into `~/.orchestra/quota_history.jsonl`).
- **Metric tiles row**: delegated-value hero stat · tokens today (🧠 claude / 🤖 codex
  / workers) · cost today · active runs. `font-variant`-style discipline: aligned
  digits, quiet labels, one accent.
- **Activity graph**: runs/tokens over the last 24 h as a braille or block sparkline.
- **Runs/sessions table**: sessions as expandable groups (tree-table), runs inside;
  status chips colored by state; live elapsed timers on running rows; the selected
  row's log tail stays visible in a bottom pane.
- **Mouse + keys**: full mouse support (Textual gives it nearly free), vim j/k,
  `enter` drill in, `k` kill (confirm), `n` new task, `s` cycle sort, `/` filter,
  `?` help overlay, `q` quit. Footer shows keys contextually.

### 3 · Session detail screen (Posting energy)

`enter` on a session/run pushes a full `Screen` (Textual screens, breadcrumb header,
`esc` back). Tabs:

- **Flow** — the diagram: brain node (🧠/🤖/👤) connected to its worker runs; edge +
  node styling encodes live status (running pulses, done green, failed/killed red);
  selecting a node shows its stats (tokens, cost, duration). netext or hand-rolled —
  whichever renders beautifully and doesn't fight the resize model. This must also
  handle the future case of nested subagents (a brain calling sonnet AND minimax):
  render whatever `meta["session"]`/`meta["brain"]` topology exists as a small DAG,
  don't hardcode two levels.
- **Conversation** — the actual interaction: the prompt the brain sent (from
  `meta["task"]`) and the worker's reply, parsed from the event log — render
  markdown/code with syntax highlighting (Textual `Markdown`/Rich `Syntax`), not raw
  JSON. Show thinking collapsed behind a toggle.
- **Log** — toolong-style: live tail, scrollback, `/` search, wrap toggle.
- **Meta** — pretty `meta.json` + timings + exit code + the kill button equivalent.

### 4 · Theme system (kill the stock look)

Ship ≥2 named themes in `~/.orchestra/config.json` (`"theme": "…"`), e.g. an
ember/amber-on-deep-charcoal (the project's existing identity) and one more with a
genuinely different personality (photophosphor CRT green? nord-storm? your call — make
it *yours*, not a default). Define every color as a token in one place; no raw
hex sprinkled through widgets; never ship Textual's default accent anywhere.

### 5 · Quality bar & delivery

- TDD where it's testable: metrics parsing, session grouping, usage extraction,
  screen navigation smoke tests via `app.run_test()`/Pilot. All 26 existing tests
  must still pass; add yours alongside in `tests/`.
- Generate SVG screenshots of the new dashboard AND a session detail screen via
  `app.export_screenshot()` into `docs/` (replace `orc-top-screenshot.svg`, add
  `orc-session-screenshot.svg`); update `README.md` and `docs/guide.html` references.
- Run `tests/live_smoke.sh` once at the end (real MiniMax, costs cents) and fix
  anything it exposes.
- **Git**: work in small commits with clear messages. When done, push to
  `origin main` → <https://github.com/Legend101Zz/Agent-orchestra>. Do not force-push.

### Definition of done

- [ ] `orc run` records exact tokens+cost (json mode), estimates only as fallback
- [ ] Sessions group runs; orchestrate skill + Codex block export `ORC_SESSION`
- [ ] `orc stats` works and is honest about exact vs estimated numbers
- [ ] Dashboard: gradient quota meters + history, metric tiles with delegated-value
      hero, activity sparkline, session tree-table, live log tail, mouse + keys
- [ ] Session screen: Flow diagram / Conversation / Log / Meta tabs, esc-back
- [ ] ≥2 named themes, zero stock-Textual-blue anywhere
- [ ] All old + new tests pass; screenshots regenerated; README/guide updated
- [ ] Everything committed and pushed to GitHub

Ship it like you own it.
