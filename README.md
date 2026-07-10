# pi-orchestra

Delegation and orchestration layer: Claude Code / Codex (the expensive "brains")
offload heavy, long-context work to **pi** running **MiniMax-M3** (1M context,
~$0.30/$1.20 per 1M tokens), with every delegated run registered on disk,
quota-gated against your MiniMax coding plan, and visible in a btop-style TUI.

```
Claude Code (brain)                     Codex (brain)
   │  skill: pi-delegate (auto)            │  ~/.codex/AGENTS.md block
   │  skill: orchestrate (keyword-gated)   │
   └───────────────┬───────────────────────┘
                   ▼  bash
              orc CLI ──── quota check → spawn pi → tee output → registry
                   ▼
   pi -p / --mode rpc  --provider minimax --model MiniMax-M3 --no-session
                   ▼
        ~/.orchestra/runs/<id>/{meta.json, output.log, inbox/}
                   ▲
        orc top  (Textual TUI: run table, log tail, kill keys, quota bars)
```

## Install / uninstall

```bash
./install.sh     # venv, ~/.local/bin/orc symlink, ~/.zshrc block, skills, codex block
./uninstall.sh   # removes symlinks + marked blocks; keeps ~/.orchestra data
```

Everything is additive. `~/.pi/agent/*`, `~/.claude/settings.json`, and
`~/.codex/config.toml` are never touched (checksum-verified). Backups are written
before any append (`*.pi-orchestra.bak`).

## Cheat sheet

| I want to…                       | Command |
|----------------------------------|---------|
| Delegate one task                | `deleg8 "task"` or `deleg8 "task" /path` |
| Streaming delegation             | `pi-rpc "task"` |
| Watch everything (control plane) | `orc top` |
| List / inspect / kill runs       | `orc list` / `orc show <id>` / `orc kill <id>` |
| Usage, cost & savings report     | `orc stats` (`--json` for machines) |
| Group a swarm as one session     | `export ORC_SESSION="orch-…"` before `orc run`, or `--session ID` |
| Check MiniMax quota              | `orc quota` (exit 0 ok / 2 warn / 3 block / 4 unknown) |
| Force past a quota block         | add `--force` (only if you accept the risk) |
| Fail fast on a stalled worker    | `orc run "task" --idle-timeout 120` |
| Different model, one-off         | `pi -p --offline --provider minimax --model MiniMax-M2.5 "task"` (unregistered) |

- **Claude/Codex auto-delegate** heavy tasks (10+ files, big inputs, batch/refactor
  work) via the `pi-delegate` skill; they must relay `ORC WARNING`/`ORC BLOCKED`
  lines to you verbatim.
- **Say "orchestrate"** in your prompt to trigger multi-worker mode (quota check →
  ≤3 parallel workers → verified synthesis). Ordinary prompts never trigger it.
- **Cost:** a PONG round-trip measured $0.00014; a typical delegation (~50k in /
  5k out) ≈ $0.02 API-equivalent; a 500k-token scan ≈ $0.17. On the coding plan
  these draw down the 5-hour/weekly windows shown in `orc top`. Both `orc run`
  (json mode) and `orc rpc` record exact usage + cost from pi's `agent_end`
  event in `meta.json`; chars/4 estimates (marked `~`) are only a fallback.

## Control plane (`orc top`)

A btop-style instrument cluster over `~/.orchestra`, auto-refreshing every 2 s:

- **Quota** — gradient fuel-gauge meters for the 5-hour and weekly windows with
  warn/block notches, reset countdown, and a braille history sparkline (sampled
  to `quota_history.jsonl` on every API fetch).
- **Tiles** — the delegated-value hero (`saved $X · Nx vs brain rates`, with an
  honest "% exact basis" line), tokens today (workers + 🧠/🤖 brains), cost
  today, active workers.
- **Sessions** — orchestrated swarms group into expandable rows (`ORC_SESSION`);
  `enter` on a run opens a drill-in screen with **Flow** (brain→workers DAG),
  **Conversation** (markdown prompt/reply, thinking behind `t`), **Log** (live
  tail, `/` search, `w` wrap) and **Meta** tabs; `esc` comes back.
- **Keys** — `j/k` nav · `enter` open · `x` kill (twice to confirm) · `n` new
  task · `s` sort · `/` filter · `t` theme · `?` help · `q` quit. Mouse works
  everywhere.
- **Themes** — `ember` (default) and `phosphor` (CRT green); `t` cycles and
  persists to `~/.orchestra/config.json`.

Screenshots: `docs/orc-top-screenshot.svg` · `docs/orc-session-screenshot.svg` ·
`docs/orc-top-phosphor.svg` (regenerate with
`.venv/bin/python tools/make_screenshots.py`).

## Usage accounting (`orc stats`)

Three blocks, honest about precision: **WORKERS** (registry; exact tokens+cost
where pi reported usage, `~` estimates otherwise), **DELEGATED VALUE** (worker
tokens priced at brain API list rates vs what MiniMax actually cost — the
number this project exists for), **BRAINS** (parsed locally from
`~/.claude/projects/**.jsonl` and `~/.codex/sessions/**`, cached by mtime,
labeled API-equivalent since subscriptions are flat-rate; `n/a` when absent).

## Config (`~/.orchestra/config.json`)

| Key | Default | Meaning |
|-----|---------|---------|
| `warn_pct` | 25 | warn when min(5h %, weekly %) at or below this |
| `block_pct` | 10 | refuse to spawn (exit 3) at or below this; `--force` overrides |
| `cache_ttl_sec` | 60 | quota API cache |
| `max_parallel_workers` | 3 | ceiling the orchestrate skill respects |
| `idle_timeout_sec` | 300 | kill a worker that produces no output for this long (exit 124) |
| `theme` | `ember` | `orc top` theme: `ember` or `phosphor` (the `t` key persists here) |

## Troubleshooting

- **`orc quota` says unknown** — your key may not be a coding-plan subscription key,
  or the endpoint is down. Delegation still works; gating is skipped with a warning.
- **Worker killed with exit 124** — the MiniMax API stalls sometimes (observed ~50 %
  of long-prompt calls on 2026-07-10). Retry once; the skills know to. Note that in
  `pi -p` mode output is buffered per turn, so `idle_timeout_sec` must exceed your
  longest expected single turn.
- **Killed runs showing `failed`** — pi traps SIGTERM and exits 143; orc classifies
  143 or an inbox kill-marker as `killed`. If you see otherwise, check `orc show <id>`.
- **TUI looks wrong after a Textual upgrade** — `orc top` is pinned to the repo venv;
  re-run `./install.sh` after changing `requirements.txt`.

## Lessons from building this (advisor-pattern dogfood)

This repo was built with the exact flow it implements: MiniMax-M3 (via pi) drafted
`registry.py`, `quota.py`, `runner.py`, and `control.py` from interface+test specs;
the main brain reviewed, fixed, and integrated. Findings baked into the skills:

1. **Always pass `--offline`** — pi's startup network checks caused a 5-minute hang.
2. **`--thinking low` for tightly-specified drafts** — default thinking produced
   4-minute stalls; low took 10–47 s with equal quality on spec-driven files.
3. **~50 % of long-prompt calls stalled or aborted** — hence the idle watchdog,
   the retry-once rule, and treating worker output as untrusted.
4. Worker draft quality was high (fsync, corrupt-meta tolerance added unprompted)
   but with real bugs each time: an invented `"auth"` JSON wrapper, a dropped
   docstring quote. **Review is not optional.**
5. **pi rpc exits when stdin closes** — heredoc one-shots silently die; orc holds
   stdin open until `agent_end`.
6. pi's `agent_end` carries exact token usage and cost — recorded into the registry.
