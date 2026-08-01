# Issue #13 — visual identity v1: evidence

Branch `issue-13-visual-identity-v1`. Everything below was run from `rust/`
on 2026-07-29.

Spec: `docs/design/visual-identity.md`, whose own source of truth is
`docs/design/visual-identity/Pi-Orchestra Identity.dc.html` (the `_fillTokens`,
`_fillGlyphs`, and `_fillBaton` generators).

## What landed

| file | what it owns |
|---|---|
| `orc-app/src/theme.rs` | the single theme map — 17 slots × 3 themes × 4 colour tiers, plus the grep gate |
| `orc-app/src/glyph.rs` | the glyph register — 16 concepts, Unicode + Nerd-Font name + ASCII fallback |
| `orc-app/src/baton.rs` | the baton — 12 cells, 7 frames, 110 ms/frame, 400 ms decay, static reduced-motion rails |
| `orc-app/src/snapshot.rs` | the snapshot harness (test-only) |
| `orc-app/tests/snapshots/*.txt` | 18 committed golden files |

## AC1 — TestBackend snapshots, four screens × three themes, committed and gated

```
$ cargo test -p orc-app --lib snapshot
running 8 tests
test snapshot::tests::the_ansi_256_tier_is_snapshotted_too ... ok
test snapshot::tests::the_ascii_glyph_column_is_snapshotted_too ... ok
test snapshot::tests::the_monochrome_tier_is_snapshotted_too ... ok
test snapshot::tests::every_screen_is_snapshotted_in_every_theme ... ok
test result: ok. 8 passed; 0 failed
```

18 golden files: `{home,stage,score,runs}-{nocturne,ember,phosphor}` plus
`home-no-color`, `score-no-color`, `stage-no-color`, `home-ansi256`,
`home-ascii`, `stage-ascii`.

Each file records **three** things — the symbol grid, a per-cell style key, and
the legend those keys resolve to. The style grid is the point: a theme
regression that changed only colour leaves a text-only snapshot untouched.

Mutation check — one channel of nocturne's `brain` slot, `0x5ad1c8 → 0x5ad1c9`,
which changes no text anywhere:

```
snapshot .../home-nocturne.txt drifted.
  want c fg=#5ad1c8 bg=#10131b mod=bold
  got  c fg=#5ad1c9 bg=#10131b mod=bold
```

Regenerate with `ORC_UPDATE_SNAPSHOTS=1 cargo test -p orc-app`; the diff is the
review.

## AC2 — no hex literals in widget code outside the theme map

`theme::tests::no_hex_literals_outside_the_theme_map` reads every `.rs` under
`orc-app/src/` except `theme.rs` and rejects a `#rrggbb` string, an
`Rgb(`/`Indexed(` constructor with numeric arguments, or any named `Color::`
variant. It asserts it scanned ≥4 files, so it cannot pass by looking nowhere.

`Color::Rgb(red, green, blue)` built from *identifiers* is allowed — that is a
hosted pane replaying its own SGR, not the app theming itself — and that one
path lives in `Theme::pane_color` inside the map anyway.

Mutation check — a literal smuggled into `render_legend`:

```
colour literals outside the theme map (5 files scanned):
lib.rs:1599: let _mutation = ratatui::style::Color::Rgb(0x12, 0x34, 0x56);
test result: FAILED
```

## AC3 — NO_COLOR: every state distinguishable by glyph, bold, reverse

`tests::no_color_keeps_every_state_distinguishable_by_glyph_bold_and_reverse`
drives `ColorTier::from_env` with a real `NO_COLOR` environment (alongside
`COLORTERM=truecolor`, to prove `NO_COLOR` wins), then asserts every one of the
17 slots resolves to `reset`, that no rendered cell carries colour, and that
reverse / bold / dim are all present.

The committed `score-no-color.txt` legend is the whole palette that reaches the
terminal:

```
a fg=reset bg=reset mod=bold
b fg=reset bg=reset mod=-
c fg=reset bg=reset mod=reverse
d fg=reset bg=reset mod=dim
```

…and the board still separates its states, on glyph alone:

```
 BACKLOG             ASSIGNED            RUNNING             REVIEW              DONE
› ◔ T0001 queued b…   ✕ T0002 blocked …   ◑ T0003 running …   ◑ T0004 review b…   ✓ T0005 done bri…
```

HOME does the same for session health (`●` live vs `⏻` down) and PATH
availability (`●` vs `○`).

Mutation check — making `Monochrome` quietly emit its 16-colour row:

```
panicked at lib.rs: assertion `left == right` failed: Bg still emits colour under NO_COLOR
```

## AC4 — baton frames match the spec, reduced motion honoured

`baton::tests::the_sweep_frames_are_the_design_sheets_frames` pins all seven
frames against `_fillBaton` (head at `2f`, body at `2f+1`, tail at `2f+2`, dim
rail elsewhere, twelve cells):

```
▓▒░─────────    ──▓▒░───────    ────▓▒░─────    ──────▓▒░───
────────▓▒░─    ──────────▓▒    ────────────
```

`tests::stage_paints_the_spec_baton_between_the_conductor_and_the_bench`
asserts what STAGE actually paints, endpoints and all:

```
◆ ▓▒░───────── ●     (Sweeping(0))
◆ ──────▓▒░─── ●     (Sweeping(3))
◆ ············ ●     (Idle — 400 ms of silence)
◆ ━━━━━━━━━━━━ ●     (Steady — live, reduced motion)
```

Reduced motion is structural, not a flag the renderer might forget: `State`
only has `Idle` and `Steady` available when `reduced_motion` is set, so a
reduced-motion client cannot paint a packet. Deleting the reduced-motion branch
fails two tests.

## AC5 — all gates

```
$ cargo fmt --all -- --check                                 exit 0
$ cargo clippy --workspace --all-targets -- -D warnings      exit 0
$ cargo test --workspace                                     exit 0   (250 passed, 0 failed)
$ RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps  exit 0
$ cargo build --release --locked                             exit 0
```

## Degradation tiers

`ColorTier` is probed, never assumed, and the probe is a pure function of an
injected environment so it is testable:

| signal | tier |
|---|---|
| `ORC_COLOR=truecolor\|256\|16\|none` | as named — an explicit request outranks every heuristic |
| `NO_COLOR` present (any value) | `Monochrome` |
| `TERM` absent or `dumb` | `Monochrome` |
| `COLORTERM` truecolor/24bit, or `TERM` contains `direct` | `TrueColor` |
| `TERM` contains `256` | `Ansi256` |
| otherwise | `Ansi16` |

The `Ansi256` column is the design sheet's own nearest-index column, not a
recomputation of it (`home-ansi256.txt`: `bg=ansi233`, `surface=ansi234`,
`brain=ansi80`, `muted=ansi244`).

## Glyph tier

`GlyphTier` decides between the register's Unicode column and its ASCII column.
`ORC_GLYPHS=ascii|unicode` is an explicit answer; `ORC_NERD_FONT=1` is the user
asserting a Nerd Font; otherwise `LC_ALL`/`LC_CTYPE`/`LANG` decide, and a
session that is not UTF-8 gets ASCII rather than mojibake. `home-ascii.txt`:

```
  › [w] ╭ bench-alpha · 2/2 workers live · READY
    DOWN ╭ bench-beta · 1/2 workers · CONDUCTOR DOWN · R recovers
  + codex   on PATH · dispatch verified
  - pi-m3   NOT ON PATH · unavailable
```

## Judgement calls, stated plainly

1. **Nerd-font detection is a UTF-8 probe, not a font probe.** A terminal
   cannot be asked which font it is using, and the register's glyphs are
   ordinary geometric-shape and box-drawing codepoints, not Private Use Area
   icons — so UTF-8 is the question that actually decides the column. Each
   entry still records its `nf-*` name (`Glyph::nerd_name`), so a future PUA
   column has its mapping ready. Inventing PUA codepoints from memory would
   have shipped mojibake.

2. **`⏻` collides with itself in the register.** The design sheet gives `⏻` to
   both *durable session* and *conductor down* — which are the two states a
   reader most needs to tell apart on the session shelf. A healthy card takes
   the bench's `●` instead. Distinguishability is the stated principle; a
   literal reading of the register would have broken AC3.

3. **`◑` vs `◐` for *in progress*.** `visual-identity.md` says `◑`, the HTML
   says `◐`. The issue says "implement `docs/design/visual-identity.md`", so
   `◑` it is.

4. **The drop shadow has no slot.** `bg` *is* the darkest colour in the map and
   it is also the stage, so there is nothing darker to cast a shadow with. The
   shadow is drawn in `border` + `DIM`, which reads as a soft edge and invents
   no hex.

5. **`BatonKind` is gone.** Its four event profiles (settle / dispatch /
   complete / failed) predate the spec, which gives the baton one behaviour and
   one direction. Task events now pulse the rail exactly as a stdout tick does;
   confirmed / failed / done still read from the pane title and the SCORE card,
   each with its own glyph.

6. **`nocturne` is now the default.** The sheet calls it FLAGSHIP. New installs
   open in it (`BenchAppConfig::default`, `model::default_theme`); an existing
   `~/.orchestra/harnesses.json` that names ember or phosphor is never
   rewritten, and an unknown name resolves to the flagship rather than failing.

## Known follow-up, out of scope here

`orc-proto/src/lib.rs:477` still documents the wire field as "Theme constrained
to ember or phosphor by the client". `orc-proto` is outside this issue's
allowed paths, so the comment is stale by one theme; the field itself is a free
string and carries `nocturne` correctly.
