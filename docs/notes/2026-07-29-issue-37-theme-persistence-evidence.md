# Issue #37 — evidence: theme persistence + the `<leader> t` switcher

Branch `issue-37-theme-persistence`, off `main` @ `8b47bf1`.
Two halves in one branch, as the carry-over comment on #37 requires: the
switcher (#13's finding 1) and this issue's original persistence scope.

## What moved

| Crate | Change |
|---|---|
| `orc-core` | `control::{THEMES, resolve_theme, theme, set_theme}`; `read_config_value` overlays the authoritative theme; `set_config("theme", …)` delegates |
| `orc-proto` | `ClientRequest::SetTheme`, `ServerResponse::ThemeSet`; `PROTOCOL_VERSION` unchanged (rationale in its doc comment) |
| `orc-daemon` | `SetTheme` → `control::set_theme` |
| `orc-app` | `apply_theme` / `cycle_theme` / `route_leader` / `handle_leader_chord`; `LeaderAction::Theme`; `BenchClient::set_theme`; `resolve_initial_theme`; `ThemeName::next`; `?` help |
| `orc-tui` | `NOCTURNE`; `Theme::named` resolves all three; `other()` cycles three; `App::new` defaults to the flagship |
| `install.sh` | dropped the duplicated `"theme"` seed (declared deviation, below) |

## The three traps, and what holds them shut

1. **Three copies of the live theme.** `shell.theme`, `shell.runs.theme`, and
   `StageState`'s own field. `apply_theme` writes all three;
   `leader_t_cycles_every_screen_together_from_every_screen` asserts all three
   on all four screens, for a full nocturne → ember → phosphor → nocturne lap.
2. **The chord existed only on STAGE and SCORE.** HOME is the launch screen.
   `route_leader` now arms and consumes the chord on HOME, SCORE, and RUNS from
   one table; STAGE keeps `RawRouter` (byte-at-a-time: literal re-send,
   bracketed paste) and gained `LeaderAction::Theme`.
3. **`orc-app` must never write `~/.orchestra`** (`lib.rs:4`). It doesn't:
   persistence is `ClientRequest::SetTheme` → daemon → `control::set_theme`.
   A failed round trip degrades to a session-only switch with the reason on the
   message line.

## Acceptance checks

### 1. Cycle, quit, relaunch — comes back in the chosen theme

The client's half is `<leader> t` → `cycle_theme` → `BenchClient::set_theme`;
the daemon's half is the write; the relaunch half is `resolve_initial_theme`
against the daemon's `Home`. Driven end to end over a real socket
(`piod` on an isolated `ORC_HOME`):

```
hello        -> welcome
home         -> theme = nocturne
set_theme    -> {'type': 'theme_set', 'theme': 'phosphor'}
home         -> theme = phosphor          <- what a relaunch reads
set unknown  -> {'type': 'theme_set', 'theme': 'nocturne'}
home         -> theme = nocturne
set ember    -> {'type': 'theme_set', 'theme': 'ember'}    <- "EMBER" requested
home         -> theme = ember
```

Tests: `orc-daemon … set_theme_persists_through_the_daemon_and_the_next_home_reports_it`,
`orc-app … a_relaunch_opens_in_the_persisted_theme`,
`orc-app … leader_t_cycles_every_screen_together_from_every_screen`.

### 2. `pio config set theme <x>` changes what the client renders

Not read back from the file — read from the daemon's `Home`, which is the
answer the client actually renders:

```
$ pio config set theme phosphor
daemon Home  -> theme = phosphor
```

Test: `orc-cli … config_set_theme_reaches_the_record_the_client_renders`.

### 3. `config get`, the file, and the rendered palette agree

```
harnesses.json app.theme = ember      <- authoritative
config.json    theme     = ember      <- derived mirror
pio config get theme     = "ember"
daemon Home    theme     = ember      <- what is rendered
```

One source of truth, both files surviving: decision and mechanism recorded in
`findings.md` (2026-07-29 entry). A hand-edited registry is still reported
correctly — `a_hand_edited_registry_is_reported_rather_than_the_stale_config_copy`.

### 4. Unknown names, old configs, additive writes

`resolve_theme` answers the flagship for `""`, `"   "`, `"teal"`,
`"ember-dark"`, `"nocturn"`, `"🎨"`, and resolves on **write** as well as read,
so no durable record holds an unrenderable name. A pre-#37 `config.json` (no
registry) keeps the theme it already had, and its unknown fields survive the
write; unknown fields in `harnesses.json` (top level and inside `app`) survive
too. Writes use the registry's atomic temp/flush/sync/rename path and leave no
`.tmp-` files behind. Tests: all six in `orc-core/tests/theme_config.rs`.

### 5. Protocol honesty

**`PROTOCOL_VERSION` does not bump.** Both enums are externally tagged and
additive, and no pair of builds can reach a new variant: the hello handshake
compares `BUILD_IDENTIFIER`, so mixed builds are refused first. Proven in both
directions against real older binaries, not simulations:

```
# new client -> older daemon (a July-12 orcd build)
Error: the running daemon predates this client (client build 0.4.0+8b47bf1651d6)
       — detach other clients, then run `orc daemon restart`

# older client (~/.local/bin/pi-orchestra @ 125585a) -> this daemon
Error: daemon build 0.4.0+8b47bf1651d6 does not match client build 0.4.0+125585abf93a
       — detach other clients, then run `orc daemon restart`

# version mismatch, and an unknown variant: explicit, connection survives
{'type': 'error', 'message': 'protocol mismatch: client 2, daemon 1'}
malformed protocol message: unknown variant `set_wallpaper`, expected one of `he…
```

Test: `orc-proto … set_theme_is_additive_and_does_not_move_the_protocol_version`.

### 6. `orc_tui::Theme::named` resolves all three

It knew only ember and phosphor, so `named("nocturne")` answered EMBER. It now
knows all three, and its `NOCTURNE` is the identity map's row through
`runs_theme()`'s slot correspondence, pinned to it by
`the_embedded_and_standalone_ledgers_resolve_every_name_the_same_way` so the two
ledgers cannot drift. Its `EMBER`/`PHOSPHOR` remain the older pre-#13
approximations; restyling those is identity work (#13), not this issue.

### 7. Gates

```
### GATE 1: cargo fmt --all -- --check                       PASS (no diff)
### GATE 2: cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 12.17s
### GATE 3: cargo test --workspace       TOTAL passed: 267  failed: 0
### GATE 4: RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.28s
### GATE 5: cargo build --release --locked
    Finished `release` profile [optimized] target(s) in 0.37s
```

### Gate 3 and the external-SSD flake — checked, not waved away

On the external-SSD checkout, `cargo test --workspace` failed intermittently
(~3 runs in 6) on `background_dispatch::delegate_confirms_while_running…` and
`quota_guard::cli_dispatch_at_cap…`, while `main` passed 6/6 — which looks like
a regression until you control for the disk. `task_plan.md` already records that
`background_dispatch.rs:195`'s sub-1s wall-clock budget is storage-dependent,
and my first comparison was confounded: the branch was on `/Volumes/Mrigesh SSD`
and the `main` worktree was on internal `/tmp`.

Re-run on equal footing — the same commit checked out to `/tmp`:

```
external SSD : background_dispatch  ~4.5-4.8s, 1 failure in 5
internal disk: background_dispatch  3.28-3.49s, 0 failures in 8
internal disk: main @ 8b47bf1       3.39-3.65s, 0 failures in 8
internal disk: full workspace       267 passed, 0 failed  (x3, commit 2112865)
internal disk: full workspace       267 passed, 0 failed  (x4, merge ad96e41)
```

**And the decisive one: unmodified `main` flakes the same way on this storage.**
A `main` worktree built on the SSD (`094e475`, no changes from this branch) fails
`quota_guard::cli_dispatch_at_cap_is_queued_then_drains_and_the_cap_setter_persists`
— one of the exact tests the branch failed on — in 1 of 6 full-workspace runs.
The earlier "main passed 6/6" was on internal disk, which is what made the branch
look guilty.

A third test joins the same family after the merge:
`harness_cli::harness_list_is_additive_and_preserves_unknown_fields` asserts a
probed `--version` string, and `discovery.rs:35` bounds that probe at
`Duration::from_secs(2)`. Under parallel load on the SSD the probe times out and
falls back to the stored version. It passes in isolation, and this branch does not
touch `discovery.rs`, `probe.rs`, or `quota.rs` at all (`git diff --stat` over
those three paths is empty).

Same code, same suite: the difference is the storage. The delegate path also never
calls anything this branch touched — `read_config_value` / `theme` / `set_theme`
appear only in orc-cli's `config` subcommand.

## Carry-over checks (from the #13 comment on #37)

- **Cycles on all four screens, asserted in one test including `stage.theme`
  and `runs.theme`** — `leader_t_cycles_every_screen_together_from_every_screen`.
- **After `t` in RUNS, `shell.runs.theme == shell.theme.runs_theme()`, never
  `orc_tui::EMBER` or `orc_tui::PHOSPHOR`** — asserted there and in
  `bare_t_in_runs_takes_the_shell_path_not_the_ledgers_own_switcher`.
- **No `error: unrecognized subcommand` on any message line** — `t` is
  intercepted in `route_runs_key` before it can reach
  `orc_tui::App::cycle_theme`; the test asserts every one of the four message
  lines is empty after each switch.
- **`?` help updated** — it now documents `<leader> t` and `pio config set
  theme`, instead of pointing at `~/.orchestra/harnesses.json` as the way to
  change things.

## Mutation testing

Fifteen mutations, each caught by the test that is supposed to protect it:

| # | Mutation | Caught by |
|---|---|---|
| M1 | `apply_theme` skips `stage.theme` | `leader_t_cycles_every_screen_together…` |
| M2 | `apply_theme` skips `runs.theme` | same |
| M3 | RUNS `t` falls through to the ledger | `bare_t_in_runs_takes_the_shell_path…` |
| M4 | STAGE chord drops `b't'` | `leader_t_cycles_every_screen_together…` |
| M5 | leader never arms outside STAGE | `the_leader_chord_reaches_home_and_runs…` |
| M6 | SCORE loses the shared table | `score_keeps_its_documented_chord…` |
| M7 | relaunch ignores the stored theme | `a_relaunch_opens_in_the_persisted_theme` |
| M8 | help drops the THEME block | `help_snapshots_cover_first_use…` |
| M9 | `config.json` wins over the registry | `the_registry_wins_when_the_two_files_disagree` |
| M10 | `set_config` stops routing `theme` | `set_config_theme_routes_to_the_authoritative_record` |
| M11 | `set_theme` stops mirroring | `set_theme_writes_the_registry_and_derives_the_config_copy` |
| M12 | unknown name stored verbatim | `set_config_theme_routes…` + the CLI test |
| M13 | `orc-tui` forgets nocturne | `the_embedded_and_standalone_ledgers_resolve…` |
| M14 | daemon acks `SetTheme` without persisting | `set_theme_persists_through_the_daemon…` |
| M15 | `PROTOCOL_VERSION` bumped | `set_theme_is_additive_and_does_not_move…` |

M12 survived its first run — but that was my filter naming the wrong test, not
a gap: `theme()` resolves on read, so the unit assertion could not tell
"resolved on write" from "resolved on read". The assertion was strengthened to
check the value **stored** in `harnesses.json`, and M12 is caught at unit level
too.

## Review fix 1 — the client→daemon seam (2026-07-29)

The reviewer found the one place the suite had no teeth, and it was the seam AC1
rests on. Reproduced before fixing: replacing `cycle_theme`'s round trip with a
no-op, keeping the local `apply_theme`, left `cargo test --workspace` at
**267 passed, 0 failed** — the number this note originally cited as evidence.
`<leader> t` still recoloured every screen; the theme just stopped surviving a
relaunch.

Structural cause: every orc-app test drove the switcher with `commands: None`,
so the `Some(commands)` branch never executed. The old mutation table bracketed
the gap — M7 covers what a relaunch *reads*, M14 covers the daemon persisting a
request it *receives* — and nothing covered the *send*. `BenchClient::set_theme`
had no test at all.

Three tests now close it, built on the `scripted_daemon` + `read_request_line`
helpers already in the file, each driving the real key-press path against a live
Unix socket rather than calling the writer directly:

| Test | What it pins |
|---|---|
| `leader_t_emits_set_theme_for_the_name_it_just_cycled_to` | the **bytes on the wire** are `{"type":"set_theme","theme":"ember"}` |
| `a_refused_save_keeps_the_switch_and_reports_it_on_the_message_line` | a daemon answering `error` keeps the switch on all three copies and puts `theme not saved: …` on the message line |
| `the_client_adopts_the_name_the_daemon_says_it_wrote` | the client renders the name the daemon reports storing |

The third also discharges the reviewer's non-blocking note: `ServerResponse::ThemeSet`
documented that it carries the resolved name "so a client that asked for something
unrecognised learns what was written", and `cycle_theme` discarded it with `Ok(_)`.
Rather than soften the doc, the client adopts it — the screen cannot show a palette
the durable record disagrees with.

Mutation-tested four ways, each caught by exactly the test that owns it:

| Mutation | Result |
|---|---|
| the reviewer's exact no-op (round trip removed) | all three fail (`the client must send a request when the theme is cycled: Timeout`) |
| send a fixed name instead of the cycled one | 2 fail (wire assertion + adopt) |
| swallow the save failure (`Err(_) => String::new()`) | 1 fails (refusal) |
| ignore the daemon's resolved name (`Ok(_) =>`) | 1 fails (adopt) |

Gate 3 is now **270 passed, 0 failed** on internal disk.

### A third member of the bounded-probe flake family

`doctor_cli::doctor_probes_capability_combinations_and_persists_to_registry` failed
once under compile load and passes 6/6 in isolation. It rides `probe.rs:55`'s
`HELP_PROBE_TIMEOUT = 5s`, the same shape as `discovery.rs:35`'s 2 s version probe
(which the reviewer A/B'd against `origin/main`: branch 8/8, main 7/8) and
`background_dispatch.rs:195`'s sub-1s budget. This branch touches none of
`probe.rs`, `discovery.rs`, or `quota.rs` — `git diff 8b47bf1` over all three is
empty. Three tests, one cause, worth its own issue.

### Not changed, with reasoning

`route_leader` runs ahead of the `raw_input_view` guard that holds back bare `?`
and `V`. STAGE's `RawRouter` already arms the leader while forwarding every other
byte to the focused pane, so intercepting the chord inside a text input is the
*consistent* behaviour rather than the divergent one, and it keeps `<leader> q`
reachable from inside the launch flow. The leader is always a `ctrl-<letter>` byte
and cannot be typed as text.

## Declared deviation from the allowed paths

`install.sh` is not in #37's allowed paths, but it seeded
`{"…","theme":"ember"}` into a fresh `config.json`, disagreeing with the
registry default (`nocturne`) from the moment of install. Left alone it would
defeat acceptance check 3 on every new machine — the file on disk would
contradict `config get` and the screen until the first write. The seed key is
removed; one line, nothing beyond the split this issue exists to close.

## Noted, not fixed (out of scope)

- The other RUNS settings keys (`n` notifications, `w`/`W`, `b`/`B`, `+`/`-`)
  still shell out through `orc_tui::App::invoke` and will print the same
  `unrecognized subcommand` inside the embed. #37 limits `orc-tui` to theme-name
  resolution, so restructuring `invoke` needs its own issue.
- `pio config set leader_key` does not reach `app.leader_key` — the same
  two-file split, one field over. The help text still points at the file for the
  leader key rather than claiming a command that does not work.
