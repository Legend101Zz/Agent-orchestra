# pi-orchestra visual identity — distilled spec (REV 2026.07)

Source of truth: `docs/design/visual-identity/Pi-Orchestra Identity.dc.html`
(interactive, three-theme live preview) plus `screenshots/`. This file is the
implementation-ready distillation for ratatui work. Widget code references
**semantic slot names, never hex literals** — themes swap by remapping slots.

## The metaphor

- **◆ conductor** — the one expensive brain. Plans, decomposes, delegates.
  Rendered in `brain` accent, always the top-left anchor, heavier active border.
- **● the bench** — pool of cheap workers awaiting dispatch. Each seat resolves
  a harness on PATH. `worker` accent; filled circle seated, hollow when idle.
- **╺━━╸ the baton** — filament connecting conductor to worker. Idle = dim
  dotted rule; a pulse travels along it whenever a pane produces output.
  The signature motion of the app.
- **⏻ durable session** — panes survive detach and crash; sessions are a shelf
  of reattachable cards with health, worker count, last-seen.

## Three themes

| theme | role | character |
|---|---|---|
| **nocturne** | FLAGSHIP | Stage at night. Near-black blue, cool teal conductor, periwinkle bench, warm gold confirmations. |
| **ember** | anchor | Warm charcoal + brass, firelit study; olive confirmations for contrast on the warm base. |
| **phosphor** | anchor · mono | CRT green. One hue, five luminances; state via brightness/bold/reverse-video. The 16-color-safe purist tier. |

## Color tokens (hex; extracted from the HTML palettes)

| slot | usage | nocturne | ember | phosphor |
|---|---|---|---|---|
| `bg` (bg.base) | house lights / deepest | `#0a0c11` | `#15110c` | `#050c07` |
| `surface` (bg.surface) | panel fill | `#10131b` | `#1d1710` | `#08130c` |
| `overlay` (bg.overlay) | floating pane / modal | `#171b26` | `#282016` | `#0c1c12` |
| `border` (border.dim) | inactive pane frame | `#262c3a` | `#3d3122` | `#164a29` |
| `border-hi` (border.active) | focused pane frame | `#39425a` | `#5a4630` | `#1f6b3a` |
| `fg` (fg.default) | 80% of all text | `#c4cad6` | `#e9ddc7` | `#48f57a` |
| `muted` (fg.muted) | metadata / labels | `#727b8f` | `#9a8a6c` | `#279c4b` |
| `faint` (fg.faint) | disabled / hint keys | `#454c60` | `#5f5238` | `#155f2e` |
| `brain` | ◆ conductor accent | `#5ad1c8` | `#d7a355` | `#a9ffc3` |
| `worker` | ● bench accent | `#8ea2ff` | `#cf8148` | `#48f57a` |
| `confirmed` | ✓ task confirmed | `#e6b450` | `#a9bd63` | `#d6ffe2` |
| `pending` | ◔ queued | `#8a93a6` | `#8f7d5c` | `#279c4b` |
| `failed` | ✕ failed / dead | `#e07a80` | `#d1704a` | `#b6ff54` |
| `avail` | ● available on PATH | `#6fd08c` | `#b3c56a` | `#7dff9e` |
| `unavail` | ○ not on PATH | `#565e70` | `#6a5c42` | `#155f2e` |
| `sel` | selection fill | `#1c2740` | `#33270f` | `#0f3d22` |
| `glow` | motion/pulse accent | `#5ad1c8` | `#d7a355` | `#48f57a` |

ANSI-256 fallback: use the nearest xterm cube/greyscale index per slot when
truecolor is not detected (reference column in `screenshots/01-tokens.png`,
e.g. nocturne bg 233, surface 234, overlay 235, border 238/240, fg 251/244/240).
Compute the remaining indices with a nearest-color pass and snapshot-test them.
Respect `NO_COLOR`: fall through to the 16-color / phosphor tier.

## Glyph register (`screenshots/02-tokens.png`)

Every state has a symbol so color is never load-bearing alone.

| concept | glyph | nerd-font | ASCII fallback |
|---|---|---|---|
| conductor / brain | ◆ | nf-md-brain | `(*)` or `[C]` |
| worker · seated | ● | nf-cod-server-process | `[w]` |
| worker · idle seat | ○ | nf-cod-circle-outline | `( )` |
| baton filament | ━ | box-drawing (native) | `->` |
| output pulse | ⠿ | braille (native) | `~` |
| task confirmed | ✓ | nf-fa-check | `[x]` / `OK` |
| queued / pending | ◔ | nf-md-timer-sand | `...` / `o` |
| in progress | ◑ | nf-md-progress-clock | `>>` |
| failed / dead | ✕ | nf-fa-times | `X` |
| conductor down | ⏻ | nf-md-power-sleep | `DOWN` |
| detached (durable) | ⊘ | nf-md-lan-disconnect | `~/~` |
| available on PATH | ● | nf-fa-check-circle | `+` |
| unavailable | ○ | nf-fa-circle-o | `-` |

## Type

Primary face: **JetBrains Mono** (Nerd Font Mono variant so icons are
single-cell). Weight is simulated, never a font axis: regular body, bold
emphasis, reverse-video selection, dim metadata. Hierarchy recipe: 80% of
content `fg`, headers bold, metadata dim, status in its semantic color,
accent reserved for interactive/active.

## Baton pulse spec

- Packet = 3-cell window `░▒▓ → ▓▒░` swept left→right over the rail's dim `─` base.
- 12 cells, ~110 ms/frame, loops while output flows; one direction only
  (conductor → worker). Color ramp within the packet: `brain` at tail →
  `confirmed` at head.
- Trigger on a pane's stdout tick; decay to idle dotted rail after 400 ms silence.
- Reduced-motion equivalent: solid accent rail when active, dim dotted rail
  when idle, count badge updated ≤1×/sec. No travel, no sweep.
- Braille ⠿ spinner = conductor thinking; block sparkline ▁▂▃▅▇ = worker tok/s.

## Signature moments

- **✓ TASK CONFIRMED** — a tactile stamp: row flashes reverse-video for one
  frame, glyph stamps in scaling down, settles to a steady badge with a gold
  (`confirmed`) underline.
- **⏻ CONDUCTOR DOWN** — calm and recoverable, never alarming: muted coral,
  slow breath (not a blink), elapsed time, clear `R` to recover. Workers hold
  their last state.

## Degradation tiers

Design in layers, each stands alone: (1) monochrome must be *usable*,
(2) 16 ANSI colors must be *readable*, (3) truecolor makes it *beautiful*.
Orthogonal switches: reduced motion; minimum viewport 80×24 (below → resize
prompt).

## Principles

- Semantic color only — remove all color and the UI still works via layout,
  glyphs, reverse-video.
- Never color alone — every state pairs with a glyph (and a label where it matters).
- Context-sensitive footer: show what's actionable now, never the whole keymap.
- Async everything; Esc cancels; spinners for indeterminate work.

Borrowed patterns: gitui (semantic theme slots), bottom/ratatui (sparklines,
gauges, constraint layouts), zellij (focused-pane frames, status-bar modes),
atuin (ledger columns, exit badges, fuzzy search), television (preview panes),
Nerd Fonts (Mono icon variants).
