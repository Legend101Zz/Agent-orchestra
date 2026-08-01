# v4 Phase 3 evidence

**Date:** 2026-07-11

## Scope and branch proof

Work stayed on `v4-bench`. Before editing, `git fetch origin`, `git status
--short --branch`, `git rev-parse HEAD`, `git rev-parse origin/v4-bench`, and
`git ls-remote origin refs/heads/v4-bench` all identified
`24ce363f686a4f0052970d776ab87e1a3b40974c`. The only worktree entry was the
pre-existing `?? findings.md`; it was not read, staged, changed, or committed.

The audited pre-existing Phase 3 commits were `499b3b9` (task board),
`5f9bdff` (worktrees), and `24ce363` (SCORE surface). The audit found that
the CLI advertised `task diff`/`task merge` but rejected both, a symlinked
worktree root could escape the owned root, isolation history forced `human`,
SCORE lacked its contract-level interactions/evidence, and installer skill
links could overwrite or delete user content.

## Shipped commits

- `7dfd3a2 fix: enable task worktree CLI lifecycle`
- `cf883ff fix: harden task worktree ownership checks`
- `69b84d5 fix: preserve user install content and isolate builds`
- `db687b3 feat: complete score board interactions`
- `f27e5c5 fix: focus linked stage pane from score`

## Verification

All Rust commands used `CARGO_TARGET_DIR=/tmp/pi-orchestra-v4-phase3-final-target`:

```sh
cd rust
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps
cargo build --locked --release
```

All passed. The test run includes compatibility-oracle and fake-pi suites,
installer integration, real-binary task CLI diff/merge JSON, atomic/additive
task records, corrupt sibling tolerance, task concurrency, temporary-Git
worktree diff/merge/drop, symlink-root refusal, daemon protocol/recovery, and
SCORE snapshots.

The SCORE TestBackend test `score_snapshots_and_drag_parser_cover_the_two_themes_and_required_sizes`
rendered and inspected ember and phosphor at `150x44` and exactly `72x30`.
It verifies review diff, dependency block, history, error context, token text,
and SGR drag parsing. This is terminal-buffer snapshot evidence, not a macOS
application screenshot; no macOS image claim is made.

The final source audit used:

```sh
rg -n '\\.(unwrap|expect)\\(' crates/orc-core/src crates/orc-daemon/src
rg --files -g '*.py' -g 'pyproject.toml' -g 'requirements*.txt' -g 'pytest.ini' -g 'tox.ini'
```

The first returned no production `unwrap`/`expect`; the second found no Python
runtime, package, or test plumbing.

## Installer and Hermes checks

The Rust installer test performs isolated-HOME install, reinstall, and
uninstall with user content. It proves idempotent owned blocks, no duplicate
markers, protected-content survival, and removes only repository-owned skill
symlinks. `install.sh` now builds to an isolated install target by default;
the live target behind `~/.local/bin/orc` was not built or modified.

An actual build-mode isolated-HOME exercise also passed (not only the test):

```sh
HOME="$TMP/home" CARGO_HOME=/Users/comreton/.cargo RUSTUP_HOME=/Users/comreton/.rustup ./install.sh
HOME="$TMP/home" CARGO_HOME=/Users/comreton/.cargo RUSTUP_HOME=/Users/comreton/.rustup ./install.sh
HOME="$TMP/home" ./uninstall.sh
```

It checked one shell marker, one AGENTS marker, owned command symlinks,
survival of a pre-existing `~/.claude/skills/pi-delegate`, and link removal.

Local `hermes --help` was inspected. It exposes sessions, skills, hooks, and
configuration but no demonstrated AGENTS.md-equivalent project-instruction
hook. Therefore no Hermes block was installed; the source AGENTS block states
this explicitly.

## Caveat

The final protected-path manifest reproduced all user configuration values,
including the pre-work Codex checksum
`f0a989ad75b992ef16d4feb1ccba245634dc233aaaf05738362cac303b1ef31c`.
The live `~/.local/bin/orc` symlink remained broken exactly as captured. One
platform-managed `~/.pi/agent/sessions/...pi-orchestra...jsonl` checksum
changed from `c31cfd565b16dddec158cdcc6ec86783b853e7bced0e5dff35faf337efb1736d`
to `551f1d459ded97a4e3daaa1bab24445f66d29431644effac0db17aeb7521e41b`
during the agent session. It was never opened, edited, restored, or otherwise
modified by repository work; restoring it would itself violate the protected
path rule. This means the literal all-path checksum gate is an environmental
non-pass, not a configuration change.

The required read-only MiniMax audit was attempted twice via the isolated Rust
`orc` binary. Both runs failed with `orc: pi executable not found on PATH`
(exit 127), so neither produced findings and neither was used. The manual
audit and verified tests above are the evidence for this phase.

Phase 4 is intentionally not started: no RUNS port, baton/view-transition
animation, reduced-motion work, or adapters were added.
