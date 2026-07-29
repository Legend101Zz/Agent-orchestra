# STAGE connection topology — routing spec (#38)

Status: **proposed**, implemented on `issue-38-stage-circuit`. Written up here
rather than straight into `docs/design/visual-identity.md` because it is new
design, not a transcription: the identity HTML has no multi-worker topology to
transcribe (see "What the sheet actually contains" below). Promote into the
sheet once reviewed.

## Why the shipped rail had to change

`render_stage` drew **one** rail, at `area.y + area.height / 2`, whenever
`width >= 100 && panes >= 2`. Three facts follow:

- With three workers there is still one horizontal line at mid-height, pointing
  at the seam between workers 1 and 2 — wired to nothing in particular.
- Its x came from `conductor_width(area)`, not from `state.pane_areas`, so after
  the first drag the rail pointed at empty stage.
- `apply_snapshot` raised one global pulse if *any* pane's `sequence` moved, so
  even the single rail could not say which worker produced the output.

The rail was decoration. The point of this spec is to make it information.

## What the sheet actually contains

Checked before designing, so this does not silently contradict the source of
truth. In `docs/design/visual-identity/Pi-Orchestra Identity.dc.html`:

- `_fillBaton` draws **one** 12-cell rail with a hardcoded `◆ ` prefix and ` ●`
  suffix. There is no worker loop.
- `_fillScreens`' STAGE block places pre-baked strings by array index —
  `[bIdle, bIdle, bLive, bIdle, …]` — hand-positioned at the vertical middles of
  two worker panes. No geometry, no routing, no junction glyph, no trunk.
- The chrome says "1 conductor + 3 workers" but two worker panes are drawn.
  Three workers appear only in the zoom mockup, with no batons at all.
- Prose at HTML:341 asserts the intent this spec implements: *"Baton filaments
  run from the conductor to each worker."*
- Zero occurrences of `drag`, `mouse`, `n8n`, `edge` or `bezier`. Every
  interaction the sheet specifies is keyboard.

So the sheet states the *intent* (a filament per worker) and never resolves the
geometry. This spec resolves it and leaves the baton's own frame maths — which
**is** specified, and is out of scope for #38 — untouched.

## The routing

Two modes, chosen by whether a horizontal gutter exists between the conductor's
right edge and the leftmost worker's left edge.

### Wide mode — elbows over a shared trunk

The n8n shape: one output port on the conductor, a vertical trunk in the
gutter, one horizontal spur per worker into its left edge.

```
 ╭─ conductor ──╮
 │              │
 │              ├──┬──────────▶ ╭─ worker 1 ─╮
 │              │  │            ╰────────────╯
 │              │  ├──────────▶ ╭─ worker 2 ─╮
 │              │  │            ╰────────────╯
 ╰──────────────╯  ╰──────────▶ ╭─ worker 3 ─╮
```

- **Port row** — the conductor's vertical middle, clamped inside its pane.
- **Trunk column** — two columns right of the conductor's shadow, clamped to
  stay at least two columns left of the nearest worker. When the gutter is too
  narrow for a distinct trunk this clamp collapses it onto the port column,
  which *is* the "bus" degradation the issue listed as the middle option: the
  trunk hugs the conductor's edge and the spurs get longer. No separate code
  path, no threshold to tune.
- **Spur row** — each worker's vertical middle, clamped inside that worker.
- **Junctions** are chosen from which sides a trunk cell actually connects
  (up / down / left / right), so a tap reads as `├`, the top of the trunk as
  `╭`, the bottom as `╰`, and a port row that is also a tap as `┼`. Rounded
  corners, matching the panes' `border::ROUNDED`.

Geometry is derived from `state.pane_areas` — the rects actually painted — so
the wiring follows a dragged pane instead of pointing at where it used to be.

### Narrow mode — the honest fallback

Below ~100 columns `stage_areas` stacks every pane full-width, so there is no
gutter to route through and a left-to-right connector would be meaningless.
Today the rail is simply **not drawn**, which is below the sheet's own 80×24
minimum and is exactly the silent disappearance AC8 forbids.

Instead each worker keeps a connector *at its own threshold*: a six-cell rail
inlaid into its top border, carrying that worker's own state.

It is right-aligned rather than tucked against the left corner because the
title grows from the left — harness name, state, a `✓ TASK CONFIRMED` stamp and
a trigger badge all share that run of border, and an inlay at the corner
overwrote the pane's own name.

```
 ╭─ ● CONDUCTOR LIVE ──────────────────╮
 ╰─────────────────────────────────────╯
 ╭─ ● WORKER-1 LIVE ──────────── ▓▒░───╮   <- live: the packet still travels
 ╰─────────────────────────────────────╯
 ╭─ ● WORKER-2 LIVE ──────────── ······╮   <- idle: the dim dotted base
 ╰─────────────────────────────────────╯
```

It costs no rows, it exists at every width down to 80×24 and at every worker
count, and motion still reads. What it gives up is *routing* — you can no
longer see the path, only the endpoint state. That trade is stated rather than
hidden, which is the requirement.

## How the baton rides a path of any length

`baton.rs` is out of scope for #38 and stays byte-identical: twelve cells,
seven frames, 110 ms each, one direction. A route is a *polyline* whose length
varies per worker, so the fixed twelve-cell rail is **sampled** onto it — path
cell `j` of `L` takes rail cell `j * 12 / L`. The packet stretches on a long
wire and compresses on a short one; the frame maths never changes.

Where the sampled rail cell is a packet cell the wire's box-drawing glyph is
replaced by the packet's block, so the packet visibly rounds the corners. Where
it is not, the wire draws its own shape in the rail's slot. One rail, per
worker, on an arbitrary path.

## Ordering

Drawn back to front: the static loom (every wire, idle), then each worker's
packet along its own path, then any message in flight. Later paints win, so a
live worker's packet overwrites the idle loom underneath it and a dispatch
overwrites both.

## Constraints this must not break

- Connector rects must **not** enter `state.pane_areas`. `persist_stage_layout`
  early-returns when `pane_areas.len() != panes.len()`, and `route_raw_mouse`
  maps a click to a pane by index into it — extra entries would silently
  disable layout persistence and misroute the mouse.
- The wire alphabet is a module-local table keyed on `GlyphTier`, following
  `baton::Cell::symbol`'s precedent, **not** new `Glyph` register variants. The
  register is the concept vocabulary and is out of scope (#13); box-drawing
  chrome already lives outside it (`border::ROUNDED`).
- Colour goes through `theme.state(Slot)` only. No new `Slot` — adding one
  reorders three palettes and regenerates all eighteen golden snapshots.

## Evidence

Recorded in `docs/notes/2026-07-29-stage-circuit-evidence.md` alongside the
drag-RPC counts and the six-worker repaint measurement.
