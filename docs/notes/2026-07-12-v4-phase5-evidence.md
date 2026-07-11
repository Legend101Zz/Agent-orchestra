# v4 Phase 5 evidence and friction log

**Date:** 2026-07-12

**Branch:** `v4-phase5` from verified `origin/main`
`43c0c5463d13b2e6a7ad4978a6e8ea6aa88e1313`.

## Scope and protected local state

The Phase 5 branch preserved the pre-existing modifications to
`tools/run-v4-phase2-ghostty.sh` and
`docs/prompts/2026-07-11-v4-phase4-phase5-next-session.md`, plus untracked
`findings.md`. None is part of this phase. No Phase 6 work, web UI, or provider
proxy was added.

The local shell did not provide `deleg8` (`command not found`), so no imagined
MiniMax review worker was credited. The actual `orc` command was present.

## Adapter capability proof

The following local help/probes were performed before changing adapter code:

```text
hermes --help
  -z, --oneshot PROMPT  One-shot mode ... print ONLY the final response text

pi --help
  -p, --print; --mode text|json|rpc; --provider; --model; --no-session

pi --list-models minimax
  minimax  MiniMax-M3  1M context

pi --offline --provider minimax --model MiniMax-M3 --no-tools --no-session -p \
  'Reply with exactly PI_M3_CAPABILITY_OK and nothing else.'
  => PI_M3_CAPABILITY_OK (exit 0)
```

`orc adapter list --json` now makes capability and degradation visible without
contacting a provider. Hermes is declared for interactive panes and bounded
`-z` delivery only; no steering or exact-usage claim is made. pi is declared
for interactive panes, `-p --no-session` delivery, and RPC steering; its usage
remains exact only after the completed pi event carries usage. Claude and Codex
remain interactive-only because their local help was not verified for this
Phase 5 adapter surface.

The existing user registry predated the new `dispatch_args`, so the command
correctly reported Hermes and pi delivery unavailable there. It was not
rewritten. A fresh isolated registry contains the verified declarations.

## Real Bench dogfood

All dogfood state is isolated under
`/tmp/pi-orchestra-v4-phase5-dogfood-20260712`; its build uses
`/tmp/pi-orchestra-v4-final/release` and does not alter the live installed
target.

1. A Bench session with Hermes brain and Hermes worker was created through the
   daemon protocol. Both durable panes were running.
2. Sending a compact one-line JSON `input` request acknowledged a literal
   `orchestrate` prompt to the brain pane. The first pretty-printed multiline
   request failed as separate malformed protocol messages; this is a framing
   constraint, not prompt receipt. The corrected request was acknowledged.
3. The Hermes brain inspected commands but did not complete the requested
   board delegation in the bounded trial; it searched the global orchestra
   home instead. Because Hermes has no verified AGENTS-equivalent instruction
   hook, the result is recorded as friction, not claimed as autonomous success.
4. The original checkout was intentionally dirty due preserved local state;
   `orc task add --isolate` correctly returned `ISOLATION UNAVAILABLE: base
   checkout is dirty`. A clean temporary Git worktree was then used as the
   Bench cwd, preserving the real checkout.
5. In session `pi-orchestra-v4-phase5-d-1783795501-0001`, the human actor
   created isolated `T0001`, assigned and started it against the live Hermes
   worker pane, then ran:

   ```sh
   orc dispatch send T0001 hermes \
     'Reply with exactly HERMES_DOGFOOD_OK. Do not edit any files.' \
     --pane pi-orchestra-v4-phase5-d-1783795501-0001-worker-1 \
     --session pi-orchestra-v4-phase5-d-1783795501-0001 \
     --actor human --timeout 120 --json
   ```

   The durable record `D-hermes-1783795513112-pi-orchestra-v4-phase5-d-0000`
   had `status: confirmed`, `exit_code: 0`, and stdout
   `HERMES_DOGFOOD_OK`. `T0001` history then contained `delivery_confirmed`
   with its pane linkage. This proves dispatch receipt, not Hermes steering or
   exact usage.

6. The isolated release `orc run` completed a real MiniMax M3 task:

   ```text
   run 20260712-001528-reply-with-exactly-pi-or-e605
   status done; exit 0; PI_ORC_RUN_OK
   exact tokens: input 1288, output 32, cache_read 256, total 1576
   exact cost_usd: 0.000440
   ```

   The run printed this required relay verbatim:

   ```text
   ORC WARNING: MiniMax quota low — 5h window 70% / weekly 21% remaining. Consider pausing delegation.
   ```

## Friction and remaining limits

- `deleg8` depends on the installer-owned shell block. It was absent in this
  already-open shell; use `orc run` directly or source a correctly installed
  shell block.
- Direct daemon socket clients must send one compact JSON object per line.
  Pretty JSON is intentionally rejected per line.
- Worktree isolation refuses a dirty base checkout. This is protective; use a
  clean worktree or commit/stash only with the owner's permission.
- Hermes can receive confirmed one-shot work but cannot yet self-orient from an
  AGENTS-equivalent hook, steer durably, or report exact usage.
- Claude/Codex adapter work was intentionally cut by the approved Phase 5
  cut order. They remain interactive panes, not falsely advertised workers.

## Verification record

Focused adapter and dispatch tests passed before the final full workspace gate:

```text
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo fmt --check
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo test -p orc-core --test bench --test dispatch --locked
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo test -p orc-cli --locked
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo build --locked --release
```

Final complete gate passed with the required isolated target:

```text
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo fmt --check                         PASS
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo clippy --all-targets -- -D warnings PASS
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo test --locked                       PASS
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final RUSTDOCFLAGS='-D warnings' cargo doc --no-deps PASS
CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-final cargo build --locked --release           PASS
```

An actual isolated-HOME build/install/reinstall/uninstall also passed when the
isolated install HOME was paired with the existing read-only Rust toolchain
locations (`CARGO_HOME=/Users/comreton/.cargo` and
`RUSTUP_HOME=/Users/comreton/.rustup`). The second install left one owned zsh
block and one owned Codex block; uninstall removed only its links/blocks and
preserved that HOME's `.orchestra` data. A bare replacement `HOME` without
those toolchain variables cannot find Rustup's configured toolchain; this is a
test-environment requirement, not an installer mutation.

The production-only source audit (excluding `#[cfg(test)]` tails) found no
`.unwrap(` or `.expect(` in `orc-core` or `orc-daemon`. `git diff --check`
passed. The Phase 5 protected-path checksums reproduced exactly:

```text
33ecb4a6c902fdacfb6085c7673438d1b95777413dc19a68e6c7bcb6ddaa7ce3  ~/.pi/agent/settings.json
782d65bc1b6d446916ee3149891e5c34ac2b505543e4d2a7ebf37a9bad698997  ~/.claude/settings.json
f0a989ad75b992ef16d4feb1ccba245634dc233aaaf05738362cac303b1ef31c  ~/.codex/config.toml
42ca4da60d0a3d3fa921985bffda88cae88d923715780ee39af3fa91bdd0fd01  ~/.local/bin/orc
```

The live `orc` symlink still targets
`/Users/comreton/.local/share/pi-orchestra/target/release/orc`. It was only
observed, never changed. Commit, merge, push, and final remote-main proof are
recorded after the intentional shipping step.
