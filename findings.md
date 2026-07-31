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

## Moving the checkout breaks every installed link (2026-07-27)

`~/Agent-orchestra` was moved to `/Volumes/Mrigesh SSD/Agent-orchestra` to free
disk space. Everything `install.sh` creates is an **absolute symlink into the
checkout**, so the move left all of them dangling at once:

```
DANGLING ~/.claude/skills/{pi-delegate,orchestrate,deliberate}
DANGLING ~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py
```

The failure is quiet in the worst way: Claude Code's `UserPromptSubmit` hook
points at a dead path, so `delegate:` simply stops being detected — no error the
conductor sees, just the feature silently not happening. The `~/.zshrc` block
has the same shape.

**Fix: re-run `./install.sh` from the new location.** It is idempotent and
explicitly replaces dead symlinks (`install_skill` has a dangling-link branch);
`ln -sfn` re-points the hook. Verify with
`printf '{"prompt":"delegate: x"}' | ~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py`.

Two related traps:
- The new path **contains a space** — quote it everywhere (`git -C "/Volumes/Mrigesh SSD/pi-orchestra"`).
  Unquoted paths fail in a way that looks like "repo missing", not "bad quoting".
- The binaries are NOT affected: `~/.local/bin/pio` links into
  `~/.local/share/pi-orchestra/target/release/`, which did not move.

**Correction (2026-07-30): the live checkout is now
`/Volumes/Mrigesh SSD/pi-orchestra`, not `Agent-orchestra`.** This entry
originally called `pi-orchestra` "a different, older checkout" — that was true
when it was written and is not any more. Verified today: `pi-orchestra` tracks
`origin/main` and holds the merge of PR #47; `Agent-orchestra` is parked on the
unmerged `issue-12-single-harness-mode` branch, roughly the #12 era. Both have
the same remote (`Legend101Zz/Agent-orchestra`), which is what makes the mix-up
easy. `docs/WORKFLOW.md` carried the same stale line and is corrected too.

**The installed links are split across the two checkouts, and nobody meant
that.** Measured 2026-07-30:

```
~/.claude/skills/{pi-delegate,orchestrate,deliberate} -> Agent-orchestra/skills/…   (stale checkout)
~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py -> pi-orchestra/shell/…      (live checkout)
~/.zshrc line 140                                      -> pi-orchestra/shell/orchestra.zsh
```

Harmless *today* only because `diff -rq` says the two `skills/` trees are
byte-identical. It stops being harmless the moment a skill changes on `main` or
`Agent-orchestra` is deleted to reclaim the disk it was moved for. Fix by
re-running `./install.sh` from `pi-orchestra` — it replaces the links in place.

## 2026-07-29 — One theme record: `harnesses.json` is authoritative (#37)

`theme` used to live in two files that nothing kept in step, so
`pio config set theme nocturne` reported success and changed nothing on
screen: the CLI wrote `~/.orchestra/config.json`, while the client rendered
`registry.app.theme` from `~/.orchestra/harnesses.json`.

**Decision: `harnesses.json`'s `app.theme` is the single authoritative
record. `config.json`'s `theme` survives only as a derived mirror.**

The registry wins because it is what the daemon serves on `Home`, which is
what actually gets rendered — the other file could only ever be a claim about
the palette. Both files survive because `config.json` predates the decision
and other tools (and older builds) read it.

How the two are kept from disagreeing, all in `orc-core/src/control.rs`:

- `control::theme()` is the only read path. It answers from the registry;
  `config.json` is consulted **only** when no registry exists yet, so a
  machine installed before #37 keeps the choice it already had instead of
  silently jumping to the flagship.
- `control::set_theme()` is the only write path. It writes the registry, then
  refreshes the derived copy in `config.json`. Both writes are atomic and
  preserve unknown fields.
- `control::read_config_value()` overlays the authoritative theme, so every
  reader of `config.json` — `pio config get/list`, and the standalone
  `orc-tui` ledger — sees the truth even if the file on disk went stale from
  a hand-edited registry.
- `control::set_config("theme", …)` delegates to `set_theme` rather than
  writing the key it was handed.
- Unknown names are resolved to the flagship **on write**, not just on read,
  so no durable record holds a name nothing can render.
- `install.sh` no longer seeds `"theme":"ember"` into a fresh `config.json`.
  That seed disagreed with the registry default (`nocturne`) from the moment
  of install and is what made the split visible in the first place.

Two related notes:

- **`orc_tui::Theme` now knows all three names.** It previously resolved only
  ember and phosphor, so `Theme::named("nocturne")` silently answered EMBER.
  Its new `NOCTURNE` is transcribed from the identity map through the same
  slot correspondence `Theme::runs_theme()` uses, and
  `theme::tests::the_embedded_and_standalone_ledgers_resolve_every_name_the_same_way`
  pins the two to each other. Its `EMBER`/`PHOSPHOR` remain the older,
  pre-#13 approximations — restyling them is identity work (#13), not this.
- **`PROTOCOL_VERSION` did not bump for `SetTheme`/`ThemeSet`.** Both enums
  are externally tagged and additive, and no pair of builds can reach a new
  variant anyway: the hello handshake compares `BUILD_IDENTIFIER`, so mixed
  builds are refused before any command is sent. Verified live against a real
  July-12 daemon binary in both directions. The version is reserved for a
  change to the meaning of an existing message.

## 2026-07-30 — "monochrome emits no colour" is a claim about the map, not the screen (#39)

`ColorTier` (truecolor / ansi256 / ansi16 / monochrome) is detected once from
`NO_COLOR`, `TERM` and `COLORTERM`, and **everything the theme map answers with
— the 17 slots and the 7-stop trigger gradient alike — is resolved through it**
in `Theme::resolve`. On monochrome that is `Color::Reset` throughout, which is
why the trigger token has to stay readable by bold plus the `◆ DELEGATE` title
badge rather than by colour. Truecolor is byte-identical to pre-#39 output;
every committed golden still matches.

**One colour is deliberately outside that rule: `Theme::pane_color`**, which
replays a hosted pane's own SGR from the wire at every tier, monochrome
included. It is **not tier-gated on purpose** — quantising or discarding another
program's output would be editing it, and in practice the hosted process
inherits `NO_COLOR` itself and stops emitting colour on its own. Only the
`Slot` fallback (a pane's *default* fg/bg) goes through the tier.

So a harness printing colour inside its pane can still put colour on a
`NO_COLOR` screen. That is documented on `pane_color` itself, and it is the
reason the module doc says "monochrome emits no colour" as a claim about **what
pi-orchestra paints**, not about the terminal. Whether `pane_color` *should*
strip colour under `NO_COLOR` is an open decision, deliberately not taken in
#39 — the ask there was honesty, and the review's blocking finding was that two
sentences #39 itself added had claimed more than the code did.

Related, same issue: `no_hex_literals_outside_the_theme_map` walks `src/`
**recursively**, and its exemption compares the **whole relative path** against
`THEME_MAP`, not the file name. The name-based version was sound only while the
walk was flat; once it recursed, any `src/<anydir>/theme.rs` escaped the scan —
including, ironically, the `src/theme/mod.rs` + `src/theme/palette.rs` split
this 1100-line file is heading for. If the map ever moves, repoint `THEME_MAP`;
a dedicated assertion fails by name rather than letting the gate scan its own
table.

## 2026-07-31 — Decision 2: the packet gets no trail, and the reason is the ASCII column (#49)

Issue #49 asked for a fading trail behind the in-flight packet, and left the
call open: propose a trail row for the design sheet, or get the smoothness from
cadence and intensity alone. **Decided: no trail.** Three pieces of evidence,
in the order they mattered.

**1. The sheet defines the packet as one cell, and makes that load-bearing.**
`docs/design/visual-identity.md:106` — "Packet = a single directional cell, not
the pulse's three-cell ramp" — and `:120-122` — "Distinguishable from the
ambient pulse by all three of shape (one cell vs three), behaviour (crosses
once vs loops) and colour — so removing any one of them still leaves the two
tellable apart." A three-cell trail deletes the *shape* leg outright; a
two-cell one halves it. On the monochrome tier colour is already gone
(`Theme::resolve` answers `Color::Reset` throughout), and behaviour needs time
to observe, so with colour removed **shape is the only instantaneous
discriminator left**. Spending it on decoration is not affordable.

**2. The ASCII column has no unclaimed directional character.** A Unicode-only
trail (`▸ ›` / `◂ ‹`) is genuinely defensible against the baton — triangles and
guillemets cannot be confused with `▓ ▒ ░ ─ · ━`. But `baton.rs:84-96` already
claims `- # + : . =`, `circuit.rs` collapses every wire junction to `+`, and
the packet owns `> <`. What is left (`,` `` ` `` `'`) does not read as an arrow
or as a fade, so an ASCII trail would break `:108`'s promise that direction is
carried by the glyph. A vocabulary that exists at one tier and not the other is
exactly what "Design in layers, each stands alone" forbids.

**3. The chunkiness was never the absence of persistence.** It was
`FLIGHT_STEP = 2` cells per `FLIGHT_FRAME_MS = 60` — the packet was never drawn
in an odd-numbered cell at all, however fast the loop polled. Making position a
continuous function of the elapsed clock halves the jump and roughly doubles
the drawn frames at the same speed, which is where the smoothness comes from.

**A trap worth recording.** The issue said a trail drawn from `▓▒░·─━` "would
fail" the shipped test at `orc-app/src/lib.rs:6667`. It would not.
`the_message_vocabulary_survives_no_color_and_the_ascii_column` asserts on the
return value of `circuit::packet()` and never counts painted buffer cells, so a
trail of *any* glyph would have gone straight past it. The decision is now
enforced by `the_packet_is_one_cell_and_draws_no_trail`, which counts packet
cells on the actual route in the rendered buffer, and which a deliberately
added two-cell trail fails.

**What the decision cost, and what it bought back.** Nothing visible was given
up, and one real defect came out of it: the packet was shipping as `bold+dim`.
`paint_cell` merges modifiers into whatever the cell already carries
(`Cell::set_style` inserts rather than replaces), and the rail underneath a
packet is `Slot::Faint`, i.e. DIM — so `theme.state(Slot::Brain)`'s BOLD landed
on top of it and the golden's own legend recorded
`j fg=#5ad1c8 bg=#0a0c11 mod=bold+dim`. The packet now clears the dim it
inherits. That is the whole of the "intensity" half: one bright cell on a dim
rail.

## 2026-07-31 — `delivery_confirmed` means "the worker took it", never "it answered" (#49)

Recorded because it was believed twice, once in the shipped code and once in
the first version of issue #49 itself.

`dispatch_supervisor::mark_started` is invoked from the `on_started` callback
at `invoke_harness`, **immediately after `command.spawn()`** and before any
wait. Its detail string says so in as many words: *"dispatch {} delivered to
{}; worker running"*. `persist_terminal`'s success arm — which sets
`execution_status`, `exit_code`, `stdout`, `stderr` and `usage` — appended no
task history at all. So the last durable word the board ever had about a
finished delegation was that its process had started.

The completion event is now `execution_succeeded` / `execution_failed`
(`review_execution_*` for a reviewer dispatch), written by `persist_terminal`
once the child has exited *and* `Drain::finish` has joined both reader threads,
so the answer it announces is EOF-complete. It is appended **before**
`write_dispatch`, deliberately: that makes "the dispatch is terminal" imply
"the board has been told", which is what lets a test wait on one and assert the
other instead of racing two writers. It is best-effort and its failure is
durable on `record.warnings` rather than propagated — propagating would abort
`execute` before it removes the supervisor spec and calls `drain_queued`, so
one contended board would stall every queued dispatch in the session.

**`delivery_confirmed` was not silenced, and the reason is not obvious.** The
tempting simplification is that `assigned` always precedes it, so the receipt
is redundant. It does not always precede it: `orch::review` dispatches a
reviewer without ever calling `assign_task`, and `orch::delegate` skips
`assign_task` when the task is already `running` — a retry against the same
worker. On both paths `delivery_confirmed` is the only record that a brief left
the conductor. It is therefore classified `(Outbound, Confirmed)`: the outbound
journey landing, which is what it always was.

**A watermark hazard the wake path exposes.** `note_task_events` used to
advance its per-task history watermark *before* checking whether the task had
an `assignee_run` to aim at. `pio orch delegate` passes no run to
`assign_task`, and it is the detached supervisor's `record_delivery` — another
process, later — that writes the link, so a board read landing between the two
consumed `assigned` and the outbound packet was never raised. #45 never saw
this because the board was only read when a pane spoke; #49's board watcher
reads it the moment the file changes, which is exactly that window. The
watermark now moves only once there is a wire to aim at.

**Still open — and the fix does *not* need `orc-daemon`, contrary to what this
entry first said.** `task_board` truncates `TaskSummary.history` to the last
eight entries and the watermark is a length into that window, so once a task's
real history passes eight, `history.len()` pins at 8, `seen` pins at 8, and
`skip(seen)` on an 8-element window is empty forever: every later event is
silently unanimated, including `execution_succeeded` itself. Adding the
completion event brings a full contracted lifecycle to nine.

The watermark is `StageState::seen_history` — `orc-app`, inside #49's allowed
paths — and it is a *length* by choice, not by necessity. Anchoring it on the
last-seen entry's identity (`at`/`actor`/`action`/`to`) and locating that in
the current window, or keeping the window's entry identities and raising the
set difference, both fix it without touching the daemon and both survive a
sliding window. A daemon-side total-length field would be cleaner still, but
it is not what makes the fix possible. Deferred to its own issue on scope,
not on impossibility.
