# Issue #39 — visual-identity carry-over from #13: evidence

Branch `issue-39-trigger-tier-grep-gate`, from `main` @ `1406840`.
Two findings, one decision, five gates. Everything below was run, not reasoned.

---

## Finding 1 — the trigger gradient ignored the colour tier

### The decision: (a), collapse it — but collapse *per tier*, not to nothing

The issue offered (a) gate the rainbow on the tier, or (b) keep it and soften
`theme.rs`'s absolutes. **(a).** The deciding argument is the issue's own
sentence about the second half of the finding:

> The same unconditional path means a 16-colour terminal (`TERM=xterm` →
> `Ansi16`) also receives truecolor SGR.

The complaint there is not "a 16-colour terminal gets colour", it is "a
16-colour terminal gets SGR it cannot render". So the fix is not to take the
effect away below truecolor — it is to send each tier something it understands,
which is what every other colour in the crate already does. The gradient became
a row of the map like any other, with the same three columns:

| tier | a trigger token receives |
|---|---|
| `TrueColor` | the seven 24-bit stops — **unchanged**, byte for byte |
| `Ansi256` | the seven nearest xterm-cube indices |
| `Ansi16` | seven distinct base ANSI colours |
| `Monochrome` | nothing. `Theme::trigger_gradient()` answers `None` |

The owner-approved exception from #9 (LOG.md 2026-07-24) is preserved exactly
where it was approved: the stops are still written out as literals rather than
as slot names, and the truecolor rendering is untouched. What is no longer
excepted is the *tier*, which is a different rule and the one #13 introduced.

Evidence that truecolor is untouched: **no existing golden drifted.** Only the
four new `stage-trigger-*.txt` files appeared. Ten committed STAGE goldens
compare byte-for-byte against a build with this change in it.

### The claims now match the behaviour (AC1)

- `theme.rs` module doc — "the monochrome tier drops colour entirely" now says
  *"the slots, and the trigger gradient below, without exception"*, and it is
  true rather than softened.
- `Theme::slot`'s "every slot answers `Color::Reset`" was already true and
  stays.
- `Theme::trigger_gradient` carries the per-tier table above as doc, so the
  answer for `Ansi16`/`Ansi256` is on the record (AC3).
- `render_pane`'s comment no longer claims the span "still reads when colour is
  stripped" as a side note — BOLD is now the unconditional part of that code
  path and the comment says so.
- LOG.md's 2026-07-24 claim that the effect "survives NO_COLOR/mono" because
  the token stays bold was **always true and stays true**; nothing there needed
  rewriting, because that entry is about legibility, exactly as the issue said.

### What each tier actually paints (AC3)

From the new golden legends — the per-cell foreground of the nine cells of
`delegate:`, read out of `tests/snapshots/stage-trigger-*.txt`:

```
stage-trigger-truecolor   d #ff6b6b  e #ffa94d  l #ffe066  e #63e6be  g #4dabf7
                          a #b197fc  t #f783ac  e #ff6b6b  : #ffa94d      (bold)
stage-trigger-ansi256     d ansi203  e ansi215  l ansi221  e ansi79   g ansi75
                          a ansi141  t ansi211  e ansi203  : ansi215      (bold)
stage-trigger-ansi16      d lightred e yellow   l lightyellow e lightgreen
                          g lightblue a lightmagenta t magenta
                          e lightred : yellow                              (bold)
stage-trigger-no-color    all nine: fg=reset bg=reset mod=bold
```

The `Ansi256` column is not trusted — `the_trigger_gradients_ansi256_column_
really_is_the_nearest_index` recomputes a nearest-colour pass over indices
16..=255 (the cube plus the greyscale ramp; 0..=15 are terminal-defined and so
cannot be measured) and asserts the transcribed indices are what it finds. The
design sheet asks for exactly that pass.

The `Ansi16` row is asserted to be a permutation — seven distinct base colours,
no `Rgb`/`Indexed`/`Reset` among them — because seven stops collapsing onto six
colours would put a seam in the gradient.

### The probe *is* driven, not assumed

`no_color_keeps_every_state_distinguishable_by_glyph_bold_and_reverse` still
starts from `ColorTier::from_env` with `NO_COLOR` set alongside
`COLORTERM=truecolor` and `TERM=xterm-256color`, so the tier under test is the
one a real `NO_COLOR` user gets, not one a test picked.

### Why the suite missed it, and what now catches it (AC2)

Both holes the issue named, closed:

1. That test's `all(fg == "reset")` net ran over SCORE and HOME only. It now
   renders **STAGE with a live trigger** and puts it inside the same net.
2. `stage_panes()` writes `codex ready`, which contains no trigger, so not one
   committed golden went through the highlight path. New fixture
   `stage_trigger_panes()` writes `delegate: ship the thing`, gated at all four
   tiers.

Mutation-checked. Reverting `render_pane` to the pre-fix behaviour (every tier
gets the 24-bit stops) fails **three independent tests**:

```
$ # let gradient = Theme::new(theme.name(), ColorTier::TrueColor).trigger_gradient();
$ cargo test -p orc-app --lib
test snapshot::tests::a_live_trigger_is_snapshotted_at_every_colour_tier ... FAILED
test tests::a_trigger_token_degrades_with_the_colour_tier ... FAILED
test tests::no_color_keeps_every_state_distinguishable_by_glyph_bold_and_reverse ... FAILED
test result: FAILED. 96 passed; 3 failed
```

### One consequence, taken deliberately

`repaint_reasons`' `trigger_ambient` held the shell at a 120 ms cadence so the
gradient could step one stop per tick. With no gradient there is nothing to
step, so on the monochrome tier that cadence would repaint a token that cannot
change — which is *precisely* the anti-pattern `StageState::any_live`'s own doc
calls out ("a repaint cadence held open by something that animates nothing is
the same wasted spin the trigger rainbow used to cause"). Gating the gradient
without gating the guard would have created that spin, so the guard moved too:
`trigger_ambient` now also requires `shell.theme.trigger_gradient().is_some()`.

Mutation-checked: removing that clause fails
`a_trigger_asks_for_no_frames_when_there_is_no_gradient_to_slide`, which pins
both halves — truecolor still asks for frames at 120 ms, monochrome asks for
none and the loop settles to its 30 s idle wait.

### Adjacent, and deliberately left alone

`Theme::pane_color` still returns a hosted pane's own SGR colour verbatim at
every tier, monochrome included, so a harness that prints colour inside its pane
will put colour on a `NO_COLOR` screen. That is **not** this finding: it is
documented behaviour with a stated reason ("this replays what the harness
printed; it is not theming"), the app is not claiming otherwise, and in practice
the hosted process inherits `NO_COLOR` itself. Noted here rather than fixed —
out of scope, and worth a decision of its own if the owner wants one. The
consequence for the test above is stated honestly: its all-`reset` assertion
covers what pi-orchestra paints, and its fixture's pane emits no SGR of its own.

---

## Finding 2 — the grep gate did not recurse

### The hole was real (reproduced)

Added the issue's exact demonstration — `src/widgets/mod.rs` containing
`Color::Rgb(0xff, 0x00, 0xaa)` / `Color::Indexed(199)` / `Color::LightMagenta` /
a `"#ff00aa"` string / a doc-comment hex — and declared `pub mod widgets;` in
`lib.rs`. With the **pre-fix** gate (flat `read_dir` + `scanned >= 4`):

```
test theme::tests::no_hex_literals_outside_the_theme_map ... ok
test result: ok. 1 passed; 0 failed
```

Green, with five colour literals in the crate. Confirmed, not assumed.

### AC4 — the recursive gate catches it

Same file, gate as shipped:

```
thread '...no_hex_literals_outside_the_theme_map' panicked at theme.rs:1086:9:
colour literals outside the theme map (7 files scanned):
widgets/mod.rs:4: //! A doc-comment hex, too: #ff00aa
widgets/mod.rs:12: let _hex = "#ff00aa";
widgets/mod.rs:14: .fg(Color::Rgb(0xff, 0x00, 0xaa))
widgets/mod.rs:15: .bg(Color::Indexed(199))
widgets/mod.rs:16: .fg(Color::LightMagenta)
```

All five forms, and the subdirectory is part of the reported name —
`widgets/mod.rs:14`, not a bare `mod.rs:14` that would not say which module.
`src/widgets/` was then deleted; `git status` confirms nothing of it remains.

### AC5 — the floor tracks the tree, not the constant 4

`scanned >= 4` was satisfied by the five top-level files on their own, so it
could never notice a directory the walk skipped. The floor is now the crate's
own **module graph**: `declared_sources` parses `mod x;` / `pub mod x;` from
`lib.rs` and `main.rs` transitively, resolving each to `x.rs` or `x/mod.rs`. It
reads source text, not the directory tree, so it is an independent statement of
what must be scanned — and a colour literal can only reach a build through a
declared module, so it is also the exactly-right set.

Two assertions, and each catches a non-recursing walk on its own. With the walk
crippled back to flat and the demo file present:

```
# per-file coverage
panicked at theme.rs:1074: the gate never looked at widgets/mod.rs,
                           which the crate declares as a module

# count, with the coverage check also removed
panicked at theme.rs:1076: the gate scanned 6 files but the crate declares 8
                           — it is not looking where the widgets are
```

Both mutations reverted; both assertions are in the shipped test.

---

## Gates (from `rust/`)

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --workspace` | 304 passed, 0 failed |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | exit 0 |
| `cargo build --release --locked` | exit 0 |

`orc-app` lib tests 94 → 99 (five added; measured against a stashed tree, not
inferred from the diff), plus its 4 integration tests. One `cargo test
--workspace` run hit the known storage-dependent flake
`orc-cli::delegate_confirms_while_running_and_cap_one_queues_until_real_exit`
(already filed in `task_plan.md`, re-observed on #38); it passed on re-run and
is in a crate this branch does not touch.

### Re-gated after rebasing onto `9431f32`

`docs/post-38-merge` merged as PR #46 while this branch was open, so the branch
was rebased onto the new `main`. The only conflict was three adjacent
status-board rows in LOG.md: `main`'s `#38` (now ✅) and its new `#45` row were
kept, and only the `#39` row is this branch's. `main` changed **no code** between
`1406840` and `9431f32`, and the nine non-LOG files are byte-identical to the
tree gated above — verified with `git diff <pre-rebase> -- . ':(exclude)LOG.md'`
returning empty.

All five gates re-run on the rebased tree: fmt, clippy, doc and release build
green; `cargo test --workspace` went red **once** with the failing name not
captured (output was suppressed on that run), then green on **seven** consecutive
full-workspace runs at 304 passed / 0 failed, plus eight isolated runs of the
suspected flake's own target. Stated as observed rather than attributed: the only
failure positively identified in this session was the filed `orc-cli` flake
above, and this branch touches no `orc-cli` code.
