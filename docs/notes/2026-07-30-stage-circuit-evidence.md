# #38 evidence — STAGE as a live circuit

Branch `issue-38-stage-circuit`, five commits off `main` @ `358b625`.
Routing design: `2026-07-29-stage-circuit-topology.md`.

## Gates

All five, from `rust/`:

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass — 124 tests, `orc-app` 92 (was 72) |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | pass |
| `cargo build --release --locked` | pass |

**Two pre-existing flakes observed, neither caused by this branch.** The branch
touches exactly one crate — `orc-app` — and both flakes are elsewhere.

1. `background_dispatch::delegate_confirms_while_running_and_cap_one_queues_until_real_exit`
   failed once and passed on re-run. This is the sub-1s wall-clock budget
   already filed in `task_plan.md` as storage-dependent.
2. `orc-cli::quota_guard::cli_dispatch_at_cap_is_queued_then_drains_and_the_cap_setter_persists`
   failed 1 run in 8. Measured rather than assumed, using the interleaved A/B
   on the same volume that `task_plan.md` used for the discovery flake:

   | | passed |
   |---|---|
   | `issue-38-stage-circuit` | 9/10 |
   | `origin/main` @ `358b625` (worktree, same volume) | 9/10 |

   Identical, so pre-existing. It is a **fourth** member of the bounded-probe
   flake family `task_plan.md` records, and was not named there; added.

The new AC9 measurement deliberately does not add a fifth: its ceiling is 16 ms
against a measured 0.157 ms, two orders of magnitude of headroom.

## AC3 — fluid drag, counted

The acceptance check says a wall-clock "feels smooth" claim is not evidence, so
this is a count. `a_drag_issues_no_daemon_traffic_until_the_mouse_comes_up`
drives 60 drag frames through the real `route_raw_mouse` → `stage_areas` →
`sync_stage_geometry` path against a scripted daemon that reports every request
line that crosses the socket.

| | `resize` | `update_layout` | total |
|---|---|---|---|
| shipped behaviour (measured by reverting the guard) | 59 | 60 | **119** |
| this branch, during the drag | 0 | 0 | **0** |
| this branch, on release | 1 | 1 | **2** |

The 119 was measured, not estimated: the guard was removed and the test's
assertion swapped for a print. Each of those 119 was a *blocking* round-trip on
the UI thread — `resize` reaches the daemon, calls `TIOCSWINSZ` and makes the
hosted CLI reflow its whole screen; `update_layout` re-reads `session.json`,
mutates it and writes it back through an `fsync`.

## AC4 — no blocking I/O on the draw path while the mouse is down

Same test, the `during` row: exactly zero. The client says nothing to the
daemon between mouse-down and mouse-up. The frame still follows the cursor
because that is a local repaint.

## AC9 — repaint cost with six workers

`six_workers_all_producing_still_repaint_inside_one_frame`: six workers all
mid-pulse, one message in flight with its emote showing, 150×44, 200 frames
after a 10-frame warm-up.

| profile | ms/frame | budget |
|---|---|---|
| `--release` (what `./install.sh` builds) | **0.23** | 16 ms |
| debug (`cargo test`) | **3.5** | 16 ms |

The 16 ms budget is the animating cadence itself: if a repaint did not fit in
one, the loop could not keep the rate it asks for.

**Treat these as an order of magnitude, not a figure.** The same code measured
0.157 / 6.223 earlier in the session and 0.23 / 3.5 now, and review measured
3.679 debug against a note claiming 6.223 — a spread of roughly 2× in both
directions, on an idle-vs-loaded laptop. Bisecting confirmed the drift is
environmental rather than code: disabling the loom clip changes release by
0.003 ms, and borrowing the route-length map instead of cloning it changes
nothing measurable. What the measurement supports is the headroom — two
orders of magnitude under the budget in the profile users actually run — not
any particular decimal.

## AC1 / AC2 / AC5 / AC7 / AC8 — committed goldens

New golden files, each pinning text **and** per-cell style:

| file | pins |
|---|---|
| `stage-1-workers.txt` | one worker: still the sheet's straight `◆ ────▓▒░───── ●` |
| `stage-3-workers.txt` | trunk with three taps, three independent traffic states |
| `stage-6-workers.txt` | six connectors, three live and three idle at once |
| `stage-80x24.txt` | AC8: the inlaid fallback at the sheet's minimum viewport |
| `stage-message-dispatch.txt` | AC5: `◆ ····▶······· ●` |
| `stage-message-return.txt` | AC5: `◆ ·······◀···· ●` |
| `stage-message-output.txt` | AC5: `◆ ────▓▒░───── ●` |

The five existing `stage-*.txt` goldens moved by exactly one row — the port now
sits at the centre of the fan rather than at the stage's vertical middle — with
their rail content unchanged.

## Verified by mutation

Four defects were re-introduced deliberately to confirm the tests catch them,
rather than trusting that they would:

| mutation | caught by |
|---|---|
| drop `stage_changed` from the repaint guard | `the_idle_rail_is_painted_not_merely_computed_when_output_stops` |
| stub `baton_needs_repaint` to `false` | that, plus `reduced_motion_repaints_the_steady_to_idle_transition_too` |
| never record the painted rail state | that, plus `a_trigger_animates_the_rainbow_but_never_the_rail` |
| restore the per-frame geometry sync | `a_drag_issues_no_daemon_traffic_until_the_mouse_comes_up` |

## Defects the work surfaced that the issue did not list

- **A mouse release re-armed the drag.** SGR reports a release as the same
  button code with an `m` suffix and the press branch keyed on the code alone,
  so letting go over a title row started a fresh drag. Pre-existing; it would
  have become much worse once geometry defers while a drag is in flight.
- **A drag's own writes were invisible to the layout debounce.** `layout` was
  both what the client wanted and implicitly what it had sent, so a move was
  never persisted unless a clamp happened to change it. Intent is now tracked
  separately.
- **Endpoint clearance blanked a junction**, leaving a hole in the trunk at the
  port row. Caught by reading the 6-worker golden, not by reasoning.
- **The ASCII endpoints are three cells wide** and a wire cell is one column, so
  drawing them sheared the row.
- **The inlaid fallback overwrote each pane's title.** Caught by an existing
  test asserting the worker's name is on screen at 72 columns.
- **`baton_row`, the test helper, matched the wrong row.** It took the first row
  containing any rail character, but `·` is also a title separator and `─` is
  the pane border, both above the rail. Every baton test using it was passing
  for the wrong reason.

## Decisions pinned by test, for review

- **A worker's connector carries that worker's output only.** The conductor's
  own output lights nothing ambient; its traffic is a discrete message. This is
  what makes AC2 mean anything, and it also stopped the conductor's pulse
  holding the whole loop at 16 ms while animating nothing.
- **A trigger animates the rainbow, never the rail.** `has_live_trigger` reads
  what a pane *displays*; the rail reports what a pane *produced*. The trigger
  keeps its own 120 ms cadence instead of holding the shell at the baton's
  16 ms. This was the "signals are inverted" note in the #13 carry-over,
  pinned per its request to pin it either way.
- **Geometry defers entirely rather than throttling to ~100 ms.** AC3 allows
  either; AC4 only allows the former.
- **The routing spec is in `docs/notes/`, not the sheet.** It is new design —
  the identity HTML has no multi-worker topology — so it is written up for
  review before promotion. `docs/design/visual-identity.md` gained only the
  message-in-flight amendment that was approved, and the HTML needs no
  amendment at all because the baton's one-direction rule is untouched.

## Live run (2026-07-30)

Everything above is `TestBackend`. This section is the real binaries: `piod`
hosting real PTYs, the real `pi-orchestra` client driven through a real PTY at
150×44, with its frames read back through the project's own VT emulator
(`orc-pty`) so what is quoted is what a terminal would show.

Isolated: private `ORC_HOME`, private socket, and a harness registry pointing
every "agent" at a shell script that prints a banner and a tick per second — no
real agent CLI was launched and no tokens were spent. The session was created
through the actual `n` → brain → workers → cwd flow, not by writing records.

**Topology, 1 conductor + 3 workers, live:**

```
╭ ◆ CLAUDE  running ──────────────────────╮                   │[agent] tick 3   │
│[agent] tick 1                           │▐ ╭────────────── ●│  PI-M3
│                                         │▐ ░
│                                         │▐ ▓
│                                         │▐◆┼──▓▒░─────── ●│  HERMES
│                                         │▐ │
│                                         │▐ ╰·············· ●│  CLAUDE (worker)
```

Three independent connectors, the `◆┼` port, `╭`/`╰` trunk ends and `├` taps —
and note the bottom rail is `··············`, a **decayed idle rail**. That is
the #13 carry-over fix visible in the running app: before it, that rail would
have been a packet stranded mid-sweep.

**Edge-resize, live.** Pressing the conductor's right edge and dragging 25
columns left moved the border from column 77 to column 52, and the wiring
stretched with it — the rail grew from 12 cells to 37, sampling onto the longer
route. The border tracked the cursor *during* the drag (local repaint) and
stayed put after release.

**Message in flight, live.** A task assigned to worker-2 through `pio task
assign` while attached:

```
│▐◆┼──▶▒░─────── ●│                     <- outbound packet crossing
╭ ● HERMES  running · ◑ TASK DISPATCHED ───╮   <- emote on the receiving worker
```

It landed on HERMES and on no other pane — per-worker attribution in the real
app. The emote held for four frames (~1.6 s at the capture rate, consistent
with the 1.2 s hold) and left.

**The return, live.** Marking that task done, the packet is visible travelling
back and rounding the corner:

| frame | screen |
|---|---|
| 397 | `╰·········◀···· ●` — leaving worker-3 |
| 440 | `▐ ◀` — travelling **up the vertical trunk** |
| 473 | `◆◀············ ●` — arriving at the conductor's port |
| 502+ | `◆ CLAUDE running · ✓ TASK CONFIRMED` — emote on the **conductor** |

That is the two-vocabulary design end to end: the ambient rail keeps its
one-direction rule, and the discrete message crosses back the other way as its
own thing.

### One limitation the live run exposed

**A task event is only noticed on the next pane-output tick.** The client
re-reads the task board inside the `UiEvent::Snapshot` arm, so if every pane is
silent a dispatch will not animate until something prints. Observed directly: a
session whose fake agents had stopped showed a correct board (`assigned`,
`moved` → `done` in the history) and no animation at all, because no snapshot
arrived to trigger the re-read.

This is pre-existing — the board refresh has always lived in that arm — and it
is invisible in practice, because a worker that has just been dispatched to is
about to produce output. It is recorded here rather than fixed because moving
the board refresh onto its own signal is a change to how the client watches the
daemon, which is outside this issue.


## Review round 1 (FIX, one item)

**The clipping test was vacuous.** `a_dragged_pane_clips_the_wiring…` aimed its
fixture at `routes[0]…max()`, which is the far end of a *spur* (column 79), not
the trunk (column 65). The pane landed six columns clear and its rows missed the
one spur row in range by one, so `clip` had nothing to remove and deleting
`clip` entirely still passed all 97 tests.

Rebuilt on the case that actually occurs — one worker dragged across another
worker's spur row — and it now carries a positive assertion as well as the
property: the covered cell is gone *and* the uncovered part of the same wire
survives, so the loom was clipped rather than never having reached that far.
Mutation-verified this time: removing `clip` fails with
`wire cell (70, 18) is painted inside pane 3`.

The guard itself was correct; only its test was not. That was the one guard on
this branch that skipped the mutation check every other one got, which is why
review called FIX rather than ACCEPT — correctly.

**Both non-blocking notes are also addressed.**

- `Routing` was computed and documented as "so the UI can say so" but never read
  outside its own tests — the claims-a-capability shape #13 was pulled up on.
  It is now read: at widths where the router gives up, STAGE's legend leads with
  `connectors inlaid — too narrow to route`. It leads rather than trails because
  the width at which routing fails is also the width at which the legend gets
  clipped. That is what AC8's "stated fallback" was asking for, and the
  `stage-80x24.txt` golden now shows it. Mutation-verified: blanking the note
  fails two tests.
- The router ran up to four times per frame (`route_lengths()` from three call
  sites plus `render_circuit`'s own `plan()`). It now runs **once**, in
  `render_stage`, recorded on `StageState` for the three consumers. Ordering
  matters and cost a round: recording *after* the pane loop left every emote one
  frame late, because `emotes` needs a route's length to know whether a message
  has landed. It is recorded immediately after `pane_areas` is written, before
  anything asks.