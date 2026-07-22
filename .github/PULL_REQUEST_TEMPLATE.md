## Issue

Closes #

## What changed

## Evidence (per acceptance check)

<!-- For each acceptance check in the issue: the command you ran and its output. -->

## Gates

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
- [ ] `cargo build --release --locked`
- [ ] `progress.md` entry appended
- [ ] Stayed inside the issue's allowed paths

## Deviations from the contract

<!-- "None" or an explicit list with reasons. -->
