# pi-orchestra — agent instructions

You are working on **pi-orchestra**: one expensive conductor, a bench of
cheap workers, all in one terminal. Rust workspace under `rust/`, three
binaries: `orcd` (daemon), `pi-orchestra` (ratatui TUI), `orc` (headless CLI).
State lives in plain additive JSON under `~/.orchestra`.

Naming: the user-facing CLI is being renamed `orc` → **`pio`** (`orcd` →
`piod`) in issue #17. New user-facing text always says `pio`; internal crate
names (`orc-core` etc.), `ORC_*` env vars, and `~/.orchestra` stay.

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

## Where you are allowed to put files (checkouts, worktrees, build dirs)

**Everything for this repo lives on the external SSD. Never write a checkout,
a worktree, or a `target/` dir under `/Users/comreton` — the system disk has
~35 GB free and one worktree with a debug + release build is 2–4 GB.**

```
/Volumes/Mrigesh SSD/pi-orchestra                  ← the checkout (main)
/Volumes/Mrigesh SSD/pi-orchestra-worktrees/<slug> ← every worktree, one per issue
```

- Review or parallel work gets a worktree, and it goes in
  `pi-orchestra-worktrees/` — named for its issue (`issue-49`), not `wt-*` at
  the volume root:
  ```bash
  git -C "/Volumes/Mrigesh SSD/pi-orchestra" \
      worktree add "/Volumes/Mrigesh SSD/pi-orchestra-worktrees/issue-<N>" issue-<N>-<slug>
  ```
- **Check the SSD is mounted before you touch anything, and stop if it is
  not.** Do not silently fall back to `$HOME`, do not clone somewhere
  "convenient", do not create the directory — a missing mount means the volume
  is unplugged, and a checkout that appears at `/Volumes/Mrigesh SSD/…` with no
  disk behind it lands on the system disk instead. Report back and wait:
  ```bash
  mountpoint=/Volumes/Mrigesh\ SSD
  if [ ! -d "$mountpoint/pi-orchestra/.git" ]; then
    echo "STOP: external SSD not mounted (no $mountpoint/pi-orchestra/.git). Ask before continuing."
    exit 1
  fi
  ```
- Note the **space in the path** — quote it everywhere. Unquoted, it fails in a
  way that reads as "repo missing" rather than "bad quoting".
- Delete a worktree when its issue merges: `git worktree remove <path>`. They
  do not shrink on their own and each carries its own `target/`.
- Scratch files, logs and one-off scripts go in the harness's own scratchpad
  directory, never in the repo and never in `$HOME`.

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
- **An issue's allowed-path list is a scope fence for *code*. It never gags the
  process files.** `progress.md`, `LOG.md`, `task_plan.md` and `findings.md`
  live at the repo root and are always in scope, because this file and
  `docs/WORKFLOW.md` require updating them on every issue — the first two
  unconditionally, `task_plan.md` as item 2 of the reading list, `findings.md`
  as the home for durable decisions. Write an issue's allowed paths as if those
  four were listed, and do not make a session choose between two rules.
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
- Update `LOG.md` (the human's dashboard) whenever an issue changes state:
  implementers set 👀 + branch and write a plain-English ship-log entry;
  reviewers set 🧪 (or back to 🔨 with what must be fixed) and append a
  one-line verdict under the entry. Never delete ship-log history.
- Secrets: `GH_TOKEN` from env only; never write tokens into files, code,
  or logs.
