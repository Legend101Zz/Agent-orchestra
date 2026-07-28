# Harness auto-registration + model-profile registration (2026-07-28)

## Problem

`pio harness list` (issue #3/#4, already merged) discovers executables on
`PATH` and persists availability/version history to
`HarnessRegistry::discovered`. It never touches `HarnessRegistry::harnesses`
— the map `session create`, `orch delegate`, and everything else actually
validate worker/brain names against. Only four profiles exist there, hardcoded
in `HarnessRegistry::default()`: `claude`, `codex`, `hermes`, `pi-m3`.

Consequence: `opencode` is fully wired end-to-end (an invocation template
exists for it in `invocation.rs`, `spawn_guard.rs` already special-cases its
concurrency cap) and `pio harness list` reports it "available" — but
`pio session create --worker opencode` fails with `unknown worker harness:
opencode`, because nothing ever inserts it into `harnesses`. The same trap
awaits any future name added to `discovery::KNOWN_HARNESSES`.

Separately: `pi-m3` is not really "a harness," it's one hardcoded profile of
the generic `pi` binary pinned to `--provider minimax --model MiniMax-M3`.
`pi` (and, differently, `opencode`) can run many models/providers, but there
is no way to register another named profile (e.g. `pi-claude`) without
hand-editing `~/.orchestra/harnesses.json`.

## Goals

1. Any name in `discovery::KNOWN_HARNESSES` that resolves on `PATH` becomes
   immediately usable (as both brain and worker) the moment `pio harness
   list` runs — no more silent "unknown worker harness" for something the
   tool already knows how to invoke.
2. A CLI command to register additional named model-profiles of a
   multi-model harness, without hand-editing JSON.
3. That registration command tries to auto-discover the harness's own model
   list first; falls back to a clear manual flow (type the provider/model
   yourself) when auto-discovery isn't supported or fails, with explicit
   instructions for what to run to find the value and how to retry.
4. `pio harness list` surfaces which provider/model a profile is configured
   for, where one is set.

## Non-goals

- A universal "list models" abstraction across every possible harness CLI.
  Each tool's surface differs (`pi --list-models` prints a table; `opencode
  models` prints bare `provider/model` lines; others may expose neither).
  This spec adds probers for `pi` and `opencode` (both verified working
  below) behind a small per-adapter match; anything else takes the manual
  path with a clear message, not a best-effort guess.
- Editing/removing existing profiles (`pio harness cap` already exists for
  concurrency; nothing here touches it).
- Changing role semantics or the capability-probe system (`pio doctor`,
  issue #4) — auto-registered and manually-added profiles are still subject
  to the existing capability probe before a dispatch actually uses them.

## Part A — auto-register any known harness on discovery

In `discovery::discover()` (`orc-core/src/discovery.rs`), after recording
presence into `registry.discovered` for a `KNOWN_HARNESSES` name that
resolved on `PATH`: if `registry.harnesses` has **no** entry keyed exactly by
that name, insert one:

```rust
HarnessConfig {
    command: name.to_owned(),
    args: vec![],
    resume_args: vec![],
    roles: vec!["brain".to_owned(), "worker".to_owned()],
    adapter: name.to_owned(),
    dispatch_args: vec![],       // empty ⇒ invocation.rs synthesizes from
    dispatch_uses_stdin: false,  // the adapter's template (path 2 in
    dispatch_timeout_sec: 0,     // resolve_worker_invocation), same as any
    extra: BTreeMap::new(),      // hand-written template-driven entry.
}
```

This only fires when `invocation::template_for(name)` returns `Some(_)` —
i.e. we actually know how to run it non-interactively — so it never creates a
profile that would immediately fail every dispatch. It never overwrites an
existing entry (default-seeded or hand/auto-added), so `pi-m3`'s hand-tuned
config is untouched and a user's own edits survive. Existing tests for
`discover()` gain a case: a `KNOWN_HARNESSES` name with an invocation
template and no prior registry entry ends up in `harnesses` after one
`discover()` call, with the four pre-existing defaults unchanged.

## Part B — `pio harness add`: named model-profiles

```
pio harness add <key> --like <existing-key> [--provider <name>] [--model <name>] [--list-models] [--json]
```

- `<key>`: new registry key, e.g. `pi-claude`.
- `--like <existing-key>`: required. Copies `command`, `adapter`, `roles`,
  `resume_args`, `dispatch_args`, `dispatch_uses_stdin`, `dispatch_timeout_sec`
  from that existing profile (e.g. `pi-m3` → command `pi`, adapter `pi`).
  Rejects if `<existing-key>` doesn't exist, with the same "unknown harness"
  error shape `pio harness cap` already uses.
- `--provider` / `--model`: written into the new profile's `args` as
  `["--provider", provider, "--model", model]` when the source (`--like`)
  profile's adapter is `pi` (matching the existing `pi-m3` shape exactly).
  For any other adapter, they're rejected with a message naming which
  adapters currently support model-profile flags (see below) — this spec
  ships one working adapter (`pi`) rather than guessing flag syntax for
  every tool; support for further adapters is additive later work, not
  blocking here.
- `--list-models`: probe-only mode. Runs the per-adapter prober (below),
  prints the result, and exits without registering anything (`<key>` and
  `--provider`/`--model` may be omitted in this mode).
- Validation before write: when a prober exists for the source adapter,
  `harness add` re-runs it and checks `(provider, model)` literally appears
  in the returned list. If it doesn't, the command exits non-zero, prints
  the exact valid choices from the probe, and writes nothing. If no prober
  exists for that adapter, this check is skipped (manual/no-validation
  path) and the profile is written as given — this is the explicit
  "auto-list when possible, else trust the manual input" behavior asked
  for.

### Probers (Part B's per-adapter model listing)

Two adapters get a real prober; the match is exhaustive over known adapters
and returns `None` (⇒ manual path) for anything else:

- **`pi`**: run `<command> --list-models`, parse the tabular output. Verified
  real output:
  ```
  provider  model                   context  max-out  thinking  images
  minimax   MiniMax-M2.7            204.8K   131.1K   yes       no
  minimax   MiniMax-M2.7-highspeed  204.8K   131.1K   yes       no
  minimax   MiniMax-M3              1M       128K     yes       yes
  ```
  Parse: skip the header line (starts with `provider`), split each
  subsequent line on whitespace, take the first two columns as
  `(provider, model)`. Bounded timeout matching the existing
  `VERSION_PROBE_TIMEOUT` pattern in `discovery.rs` (2s) — a hung prober
  must not hang the CLI.
- **`opencode`**: run `<command> models`, parse bare `provider/model` lines.
  Verified real output:
  ```
  opencode/big-pickle
  minimax-coding-plan/MiniMax-M3
  openai/gpt-5
  ...
  ```
  Parse: split each non-empty line on the first `/`; left is provider,
  right is model. (`opencode` is not registered against the `pi`-shaped
  `--provider/--model` args format in this spec — its own model-selection
  flag syntax is future work if/when someone needs an `opencode` profile
  variant; today's ask is specifically the `pi` case, and the prober is
  still useful standalone via `--list-models` for visibility.)

Both probers live in a new small module, e.g. `orc-core/src/model_probe.rs`,
returning `Vec<(String, String)>` (provider, model) or a typed error
distinguishing "adapter has no known prober" from "the prober ran but
failed/timed out" so `harness add` can print the right message in each case.

### `pio harness list` display

Add one line per entry that has provider/model args set (detected by the
same `["--provider", ..., "--model", ...]` shape written above), e.g.:

```
pi-m3      on PATH · available     /Users/.../pi
    0.80.7 · minimax/MiniMax-M3
pi-claude  on PATH · available     /Users/.../pi
    0.80.7 · anthropic/claude-sonnet-5
```

`--json` output includes the same provider/model pair as additive fields on
the existing discovery row where present.

## Testing

- `discover()` gains a fixture case proving a `KNOWN_HARNESSES` entry not in
  the four defaults gets auto-inserted into `harnesses` with a working
  template, and that re-running `discover()` doesn't clobber a
  since-hand-edited version of that same entry.
- `model_probe`: unit tests against the exact captured `pi --list-models`
  and `opencode models` output above (as fixture strings, not live process
  calls) proving both parsers extract the right `(provider, model)` pairs.
- `pio harness add`: CLI test registering `pi-claude --like pi-m3 --provider
  X --model Y` against a fixture `pi` script that emits a fixed
  `--list-models` table; asserts success when `(X, Y)` is in the table,
  failure with the real choices listed when it isn't, and that a failed
  attempt writes nothing to the registry.
- `pio harness add ... --list-models` prints without registering; asserts
  the registry file is unchanged afterward.
- An adapter with no prober (e.g. a fixture harness using adapter `claude`)
  registers straight through without the validation step.

## Out of scope, restated

No changes to role semantics, capability probing, dispatch, or existing
`pio harness cap`. No editing/removing of profiles. No generic multi-adapter
model-flag abstraction beyond the one (`pi`) shipped here.
