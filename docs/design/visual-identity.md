# pi-orchestra visual identity — distilled spec (REV 2026.07d)

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

## Message in flight

*(Added REV 2026.07b for #38. The baton spec above is unchanged — this is a
second vocabulary sharing the same geometry, not an amendment to the first.
The two are deliberately separable: the baton says a pane **is producing**, a
message says a discrete thing **was sent**, and the one-direction rule belongs
to the baton alone.)*

- The ambient pulse is a *condition* — it loops for as long as output flows.
  A message is an *event*: it has a source, a destination and an outcome, so
  it traverses its connector exactly **once** and lands. It never loops.
- Packet = a single directional cell, not the pulse's three-cell ramp:
  `▶` travelling conductor → worker, `◀` travelling worker → conductor.
  ASCII column: `>` and `<`. Direction is the meaning, so the glyph carries it
  and colour never has to.
- One cell every ~30 ms — deliberately faster than the 110 ms ambient sweep.
  A dispatch should read as a snap, not as flow.
  *(REV 2026.07c, #49. This read "~60 ms/frame, two cells/frame" — the same
  33⅓ cells per second, but expressed as a frame counter, and implemented as
  one: the packet stood still for 60 ms and then jumped two cells, so it was
  ~16 fps of visible motion however often the shell repainted. Position is now
  a continuous function of the elapsed clock, so the speed is unchanged and the
  travel is smooth. There is no "frame" left to name.)*
- **No trail.** The smoothness above comes from cadence, not from persistence
  behind the head. A trail costs the "single directional cell" rule two lines
  up, and with it the *shape* leg of the three-way separation below — the one
  leg that survives when colour is removed. The argument, and the two forms
  that were considered, are in `findings.md` under 2026-07-31.
- Colour from the map, never a new slot: `brain` outbound (the conductor's
  intent leaving), `confirmed` on a confirmed delivery, `failed` on a failure.
- It rides **over** the connector's current state; the rail underneath keeps
  whatever it was already doing. A worker can be mid-pulse and receive a
  dispatch in the same frame, and both must stay legible.
- Reduced-motion equivalent: no travel. The connector holds solid in the
  message's colour for the traverse duration, then the emote lands. Same
  information, no packet anywhere on the rail.
- Distinguishable from the ambient pulse by all three of shape (one cell vs
  three), behaviour (crosses once vs loops) and colour — so removing any one
  of them still leaves the two tellable apart.

**Connection topology.** Where the ambient rail is drawn is now a function of
how many workers there are — one rail at mid-height stops meaning anything past
one worker. The routing spec (elbows → bus → single rail, with its honest
degradations at and below 80×24) lives in
`docs/notes/2026-07-29-stage-circuit-topology.md`; it is new design rather than
a transcription of the identity HTML, which has no multi-worker topology at
all, so it is written up for review before being promoted into this sheet.

## Signature moments

- **✓ TASK CONFIRMED** — a tactile stamp: row flashes reverse-video for one
  frame, glyph stamps in scaling down, settles to a steady badge with a gold
  (`confirmed`) underline. This is the app's **one** landing language; anything
  that arrives somewhere uses it rather than inventing a second vocabulary.
- **The landing emote** (added REV 2026.07b for #38) — what a
  *message in flight* does when it reaches its destination pane. Same three
  beats as ✓ TASK CONFIRMED, with the glyph and slot naming the outcome:
  `◑ DISPATCHED` in `brain` on the receiving worker, `✓ CONFIRMED` in
  `confirmed` on the conductor, `✕ FAILED` in `failed`. It is short-lived —
  it holds for ~1.2 s and then leaves, restoring the row underneath exactly.
  It never takes focus, never blocks input, and never survives the pane that
  raised it. Under reduced motion the flash frame is dropped: it appears
  already settled, holds, and leaves.
- **The departure beat** (added REV 2026.07c for #49) — the mirror of the
  landing emote, on the pane a message is *leaving*: `▶ HANDING OFF` on the
  conductor as a brief goes out, `◀ ANSWERING` on the worker as its answer
  comes back. Same three beats and the same slot as the landing it belongs to,
  because this is still the one landing language; what differs is the word,
  which is a present participle rather than a past one, and the glyph, which is
  the packet's own directional cell. It **cannot outlive the packet it
  announces**: it lasts while the packet is still crossing its first twelve
  cells of route and stops when the packet lands, so on a short wire it is over
  before the emote starts. Under reduced motion nothing travels, so there is no
  departure to show and the landing emote carries the whole event.
  *(This first read "no hold of its own", which overclaimed. Every route the
  router plans in the wide layout exceeds twelve cells, so in practice the beat
  is a constant 360 ms — the wall-clock cost of crossing the sheet's own rail.
  It is bounded by real geometry rather than by a number picked to look right,
  and it starts at a real event; it is not derived from anything the worker
  did, and STAGE does not claim to show how long a hand-off took.)*
- **⏻ CONDUCTOR DOWN** — calm and recoverable, never alarming: muted coral,
  slow breath (not a blink), elapsed time, clear `R` to recover. Workers hold
  their last state.

## The brief sidecar

*(Added REV 2026.07d for #49 phase 3. A third vocabulary: the baton says a pane
**is producing**, a message says a discrete thing **was sent**, and the sidecar
says **what** was sent and to whom.)*

- A **band** at the top of a worker card's inner area, opened by `<leader> i`,
  at most one on the stage. It is not modal: it never takes focus, never
  swallows a keystroke, and the chord is its own exit. Bare `i` still reaches
  the pane.
- **Drawn after the pane's cell blit.** This is the load-bearing rule and the
  one the `⏻ CONDUCTOR DOWN` overlay currently gets wrong — that one is drawn
  before the blit and is erased by the pane's own grid in the same frame.
  Anything drawn inside a pane's inner area must come after it.
- **Nothing is resized and nothing is lost.** The hosted CLI keeps its full
  grid; the band covers rows and the next frame after closing restores them from
  the daemon's own buffer. The band states its own cost on its rule row:
  `COVERING 7 OF 20 ROWS · ⌃g i closes`.
- **The `▌` rail runs down every band row in the `brain` slot** — the
  conductor's accent inside the worker's card. With colour removed the rail is
  still a shape, which is what keeps the band separable on the monochrome tier.
- **The header always carries the negation**, at every width, degrading
  `sidecar worker, not this pane's CLI` → `not this pane's CLI` → `not this
  pane` → `not here`. It never says "delivered to this pane": `deliver`
  auto-selects a seated pane so the record usually names one, but the work is
  done by a separate child process. Saying otherwise would restate the belief
  #45 was filed about.
- **Truncation replaces, never appends.** The last brief row is *replaced* by
  `… +{N} more lines · {size} brief`, so a count can never be mistaken for
  content, and `{N}` is exact rather than "many".
- **No new slot.** `brain`, `fg`, `muted`, `faint` and `overlay` already exist;
  adding one would reorder three palettes and every golden's legend.
- **No motion.** The band's appearance is anchored to a keypress, and a
  fade-in would be a duration chosen for looks — which #49's acceptance check 6
  forbids. Reduced motion therefore changes nothing about it.
- **Glyph register additions:** `▌` rail (`|`), `▤` open (`[B]`), `⏷` more
  (`v`), `⏶` clipped (`^`), `…` elide (`...`).

**Its vocabulary is bounded by what was observed, not by what the worker did.**
Whether anything is visible at all is the child's decision — `read_until(b'\n')`
means a block-buffering worker delivers nothing until it exits — so the band may
say *"no complete line observed since T"* and may never say "thinking", "quiet",
"idle" or "no output". The four ways there can be nothing to show get four
different sentences rather than one.

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
