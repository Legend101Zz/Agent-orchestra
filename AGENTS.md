# pi-orchestra — agent instructions

You are working on **pi-orchestra**: one expensive conductor, a bench of
cheap workers, all in one terminal. Rust workspace under `rust/`, three
binaries: `orcd` (daemon), `pi-orchestra` (ratatui TUI), `orc` (headless CLI).
State lives in plain additive JSON under `~/.orchestra`.

## Read first, in this order

1. `docs/WORKFLOW.md` — the issue → branch → review → merge loop. Follow it.
2. `task_plan.md` — current program status and issue map.
3. `docs/superpowers/specs/2026-07-22-v1-universal-delegation-design.md` —
   the V1 product spec (positioning, concepts, roadmap).
4. `docs/design/visual-identity.md` — for ANY TUI/visual work: semantic color
   slots (nocturne/ember/phosphor), glyph register, baton spec. Widget code
   references slot names, never hex literals.
5. The GitHub issue you're implementing — its task contract is binding.

## Codebase map

- `rust/crates/orc-core` — domain logic: `registry.rs` (harnesses),
  `adapter.rs`, `dispatch.rs` (confirmed delivery), `tasks.rs` (board),
  `quota.rs` (guard), `runner.rs`, `metrics.rs`, `inbox.rs`, `control.rs`.
- `rust/crates/orc-daemon` — `orcd`: owns PTYs, durable sessions, Unix
  socket at `~/.orchestra/orcd.sock`.
- `rust/crates/orc-app` — the TUI: HOME / STAGE / SCORE / RUNS screens.
- `rust/crates/orc-cli` — `orc`: run/rpc/task/dispatch/list/quota.
- `rust/crates/orc-proto`, `orc-pty` — protocol and PTY plumbing.
- `skills/`, `codex/AGENTS-block.md`, `shell/orchestra.zsh` — harness-side
  integrations installed by `install.sh`.
- Tests live next to each crate (`tests/`); fixtures under `tools/fixtures/`.

## Non-negotiable gates (run from `rust/`, all must pass before pushing)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo build --release --locked
```

## Conventions

- One issue = one branch (`issue-<N>-<slug>`) = one merge. Stay inside the
  issue's allowed paths; if the contract is wrong, stop and comment on the
  issue — do not improvise.
- Commits: imperative, prefixed `feat:` / `fix:` / `docs:` / `test:` /
  `refactor:`, body explains why. Never commit directly to `main`.
- TUI state must degrade honestly: never claim a capability that wasn't
  probed; unavailable ≠ hidden. Every state pairs a glyph with color
  (see visual identity: color is never load-bearing alone).
- Durable JSON under `~/.orchestra` is additive — never write a migration
  that breaks old records; readers tolerate unknown fields.
- Keep files focused; prefer new modules over growing a file past ~600 lines.
- Update `progress.md` (append a dated entry) and the issue (evidence per
  acceptance check) before you finish.
- Secrets: `GH_TOKEN` from env only; never write tokens into files, code,
  or logs.
