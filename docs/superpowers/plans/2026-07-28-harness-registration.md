# Harness Auto-Registration + Model Profiles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Any `KNOWN_HARNESSES` name (e.g. `opencode`) becomes usable via `session create`/`orch delegate` the moment it's found on `PATH`, and a new `pio harness add` command lets a multi-model harness like `pi` be registered under additional named model profiles (e.g. `pi-claude`), with auto-probing of the harness's own model list where possible and a validated manual fallback otherwise.

**Architecture:** `discovery::discover()` gains a gap-filling insert into `HarnessRegistry.harnesses` for any known-adapter name not already registered (Part A). A new `orc_core::harness_models` module holds one parser per adapter that can list its own models (`pi`, `opencode`) behind a `ModelProbe` enum, used both by a new `pio harness add` CLI command (Part B) and by nothing else — it's a thin, isolated capability.

**Tech Stack:** Rust, existing `orc-core`/`orc-cli` crates, `clap` derive, `serde_json`, `anyhow`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-28-harness-registration-design.md`. **Issue:** #33.

## Global Constraints

- Allowed paths: `rust/crates/orc-core/`, `rust/crates/orc-cli/` (per issue #33).
- All five AGENTS.md gates must pass before each commit that touches Rust code: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, `cargo build --release --locked` (all from `rust/`).
- No new dependencies.
- `pio harness list --json`'s existing shape (a flat 5-element array of `HarnessDiscovery`) must not change — `orc-cli/tests/harness_cli.rs` asserts `rows.len() == 5` today. The new "registered profiles" display is plain-text only.
- Never overwrite an existing `HarnessRegistry.harnesses` entry — auto-registration only fills gaps.
- Provider/model args are only written for the `pi` adapter in this issue (see spec's non-goals); any other adapter's `--provider`/`--model` request is rejected with a clear message, not guessed at.
- Branch: `issue-33-harness-registration` (already created off `main`, spec already committed as `1c27089`).

---

### Task 1: Auto-register any known harness on discovery (Part A)

**Files:**
- Modify: `rust/crates/orc-core/src/invocation.rs` (add a small crate-visible helper)
- Modify: `rust/crates/orc-core/src/discovery.rs` (`discover()` + new private fn + new test)
- Modify: `rust/crates/orc-core/src/bench.rs:147-152` (doc comment accuracy)
- Modify: `rust/crates/orc-cli/tests/harness_cli.rs` (new end-to-end test)

**Interfaces:**
- Produces: `pub(crate) fn has_invocation_template(adapter: &str) -> bool` in `orc_core::invocation`, used by `discovery.rs`.
- Produces: private `fn register_default_profile(harnesses: &mut BTreeMap<String, HarnessConfig>, name: &str)` in `discovery.rs`, unit-tested directly (same pattern as the existing `record_discovery` test).

- [x] **Step 1: Add the crate-visible template-check helper**

In `rust/crates/orc-core/src/invocation.rs`, immediately after the closing brace of `fn template_for` (the function ends right before the doc comment for `resolve_worker_invocation`, around line 187), insert:

```rust
/// Whether `adapter` has a known non-interactive invocation template.
///
/// Used by discovery to decide whether auto-registering a bare profile for a
/// newly-found executable would actually be dispatchable, without duplicating
/// the adapter list `template_for` already owns.
#[must_use]
pub(crate) fn has_invocation_template(adapter: &str) -> bool {
    template_for(adapter).is_some()
}
```

- [x] **Step 2: Write the failing unit test for gap-filling registration**

In `rust/crates/orc-core/src/discovery.rs`, add `HarnessConfig` to the existing `use crate::bench::{...}` import (it currently imports `DiscoveredHarness, HarnessRegistry, load_harness_registry, read_harness_registry, write_harness_registry` — add `HarnessConfig` to that list, alphabetically after `DiscoveredHarness`).

Then, inside the existing `#[cfg(test)] mod tests { ... }` block, change its two import lines:

```rust
    use super::{HarnessRegistry, present, record_discovery, register_default_profile};
    use crate::bench::{DiscoveredHarness, HarnessConfig};
```

(adding `register_default_profile` to the first line, `HarnessConfig` to the second — both currently import fewer names).

Then, after the last test (`present_returns_every_known_harness_and_merges_history`), add:

```rust
    #[test]
    fn register_default_profile_fills_gaps_and_never_overwrites() {
        let mut harnesses = BTreeMap::from([(
            "claude".to_owned(),
            HarnessConfig {
                command: "custom-claude-path".to_owned(),
                args: vec!["--hand-edited".to_owned()],
                resume_args: Vec::new(),
                roles: vec!["brain".to_owned()],
                adapter: "claude".to_owned(),
                dispatch_args: Vec::new(),
                dispatch_uses_stdin: false,
                dispatch_timeout_sec: 0,
                extra: BTreeMap::new(),
            },
        )]);

        // An existing entry is never touched, even though its adapter has a
        // template.
        register_default_profile(&mut harnesses, "claude");
        assert_eq!(harnesses["claude"].command, "custom-claude-path");
        assert_eq!(harnesses["claude"].args, vec!["--hand-edited".to_owned()]);

        // A known adapter with no existing entry gets a minimal usable
        // profile: both roles, empty dispatch_args so invocation.rs's
        // template synthesis (not a hardcoded flag list) drives dispatch.
        register_default_profile(&mut harnesses, "opencode");
        let opencode = &harnesses["opencode"];
        assert_eq!(opencode.command, "opencode");
        assert_eq!(opencode.adapter, "opencode");
        assert_eq!(
            opencode.roles,
            vec!["brain".to_owned(), "worker".to_owned()]
        );
        assert!(opencode.dispatch_args.is_empty());

        // A name with no invocation template is never inserted.
        register_default_profile(&mut harnesses, "no-such-adapter");
        assert!(!harnesses.contains_key("no-such-adapter"));
    }
```

- [x] **Step 3: Run the test to verify it fails**

Run: `cd rust && cargo test -p orc-core --lib discovery::tests::register_default_profile_fills_gaps_and_never_overwrites -- --exact`
Expected: FAIL to compile — `register_default_profile` does not exist yet.

- [x] **Step 4: Implement `register_default_profile` and wire it into `discover()`**

In `rust/crates/orc-core/src/discovery.rs`, add this private function after `record_discovery` (before `probe_version`):

```rust
/// Insert a minimal usable profile for `name` when none exists yet and the
/// adapter has a working invocation template. Never overwrites an existing
/// entry — hand-configured or previously auto-registered. Leaves
/// `dispatch_args` empty so `invocation::resolve_worker_invocation` derives
/// the actual invocation from the adapter's template (path 2) instead of a
/// second hardcoded flag list living here.
fn register_default_profile(harnesses: &mut BTreeMap<String, HarnessConfig>, name: &str) {
    if harnesses.contains_key(name) || !crate::invocation::has_invocation_template(name) {
        return;
    }
    harnesses.insert(
        name.to_owned(),
        HarnessConfig {
            command: name.to_owned(),
            args: Vec::new(),
            resume_args: Vec::new(),
            roles: vec!["brain".to_owned(), "worker".to_owned()],
            adapter: name.to_owned(),
            dispatch_args: Vec::new(),
            dispatch_uses_stdin: false,
            dispatch_timeout_sec: 0,
            extra: BTreeMap::new(),
        },
    );
}
```

Then modify `discover()` to call it. The current body (around line 72-93):

```rust
pub fn discover(probe_versions: bool) -> Result<Vec<HarnessDiscovery>> {
    let mut registry = load_harness_registry()?;
    let now = now_iso();
    for name in KNOWN_HARNESSES {
        let Some(path) = locate_executable(name) else {
            continue;
        };
        let path_str = path.to_string_lossy().into_owned();
        let stored_version = registry
            .discovered
            .get(*name)
            .and_then(|record| record.version.clone());
        let version = if probe_versions {
            probe_version(&path).or(stored_version)
        } else {
            stored_version
        };
        record_discovery(&mut registry.discovered, name, path_str, version, &now);
    }
    write_harness_registry(&registry)?;
    Ok(present(&registry))
}
```

becomes (one new line before `write_harness_registry`):

```rust
pub fn discover(probe_versions: bool) -> Result<Vec<HarnessDiscovery>> {
    let mut registry = load_harness_registry()?;
    let now = now_iso();
    for name in KNOWN_HARNESSES {
        let Some(path) = locate_executable(name) else {
            continue;
        };
        let path_str = path.to_string_lossy().into_owned();
        let stored_version = registry
            .discovered
            .get(*name)
            .and_then(|record| record.version.clone());
        let version = if probe_versions {
            probe_version(&path).or(stored_version)
        } else {
            stored_version
        };
        record_discovery(&mut registry.discovered, name, path_str, version, &now);
        register_default_profile(&mut registry.harnesses, name);
    }
    write_harness_registry(&registry)?;
    Ok(present(&registry))
}
```

- [x] **Step 5: Run the test to verify it passes**

Run: `cd rust && cargo test -p orc-core --lib discovery::tests -- --exact`
Expected: all `discovery::tests::*` PASS, including the new one.

- [x] **Step 6: Fix the now-inaccurate doc comment on `HarnessRegistry.discovered`**

In `rust/crates/orc-core/src/bench.rs`, the `discovered` field doc (around line 147-152) currently reads:

```rust
    /// PATH-discovered harness executables keyed by harness name.
    ///
    /// Populated by `pio harness list` (auto-discovery). Additive and
    /// independent of [`Self::harnesses`], which stays user-editable.
    #[serde(default)]
    pub discovered: BTreeMap<String, DiscoveredHarness>,
```

Change the second paragraph to reflect Part A:

```rust
    /// PATH-discovered harness executables keyed by harness name.
    ///
    /// Populated by `pio harness list` (auto-discovery). Additive; discovery
    /// also fills gaps in [`Self::harnesses`] for known-adapter names with no
    /// existing profile (issue #33), but never overwrites an existing entry
    /// there, hand-configured or previously auto-registered.
    #[serde(default)]
    pub discovered: BTreeMap<String, DiscoveredHarness>,
```

- [x] **Step 7: Write the failing end-to-end test proving the reported bug is fixed**

In `rust/crates/orc-cli/tests/harness_cli.rs`, add (after `harness_list_shows_all_five_with_three_present_and_two_unavailable`):

```rust
#[test]
fn harness_list_then_session_create_can_use_a_newly_discovered_harness() {
    // Reproduces the exact reported failure: `opencode` is a KNOWN_HARNESSES
    // name with a working invocation template, but before issue #33 nothing
    // ever put it into `harnesses`, so `session create --worker opencode`
    // failed with "unknown worker harness: opencode" even right after
    // `harness list` reported it "available".
    let root = root("discover-then-session");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    let cwd = root.join("cwd");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&cwd).unwrap();
    fake_harness(&bin, "opencode", "opencode 1.0.0");

    let listed = run(&home, &bin, &["harness", "list"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );

    let created = run(
        &home,
        &bin,
        &[
            "session",
            "create",
            "--brain",
            "opencode",
            "--worker",
            "opencode",
            "--cwd",
            &cwd.to_string_lossy(),
            "--json",
        ],
    );
    assert!(
        created.status.success(),
        "session create with a newly auto-registered harness should succeed: {}",
        String::from_utf8_lossy(&created.stderr)
    );

    let _ = fs::remove_dir_all(&root);
}
```

- [x] **Step 8: Run the test to verify it fails, then run all discovery/harness tests to verify it passes**

Run (before Step 4/6, if you're doing this test-first — otherwise confirm it now passes since Steps 1-6 are already done): `cd rust && cargo test -p orc-cli --test harness_cli -- --exact`
Expected: all tests in `harness_cli.rs` PASS, including `harness_list_then_session_create_can_use_a_newly_discovered_harness`.

- [x] **Step 9: Run the full workspace test suite and commit**

Run: `cd rust && cargo test --workspace`
Expected: all green.

```bash
cd "/Volumes/Mrigesh SSD/Agent-orchestra"
git add rust/crates/orc-core/src/invocation.rs rust/crates/orc-core/src/discovery.rs rust/crates/orc-core/src/bench.rs rust/crates/orc-cli/tests/harness_cli.rs
git commit -m "$(cat <<'EOF'
feat: auto-register any known harness with an invocation template

pio harness list discovered opencode but never made it usable —
only the four hardcoded defaults ever landed in
HarnessRegistry.harnesses, so `session create --worker opencode`
failed even right after harness list reported it available.

discover() now fills that gap for any KNOWN_HARNESSES name with a
working invocation template and no existing entry, never touching
a hand-configured or previously auto-registered profile.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Per-adapter model listing (`orc_core::harness_models`)

**Files:**
- Create: `rust/crates/orc-core/src/harness_models.rs`
- Modify: `rust/crates/orc-core/src/lib.rs` (register the module)

**Interfaces:**
- Consumes: `crate::quota::command_output_with_timeout(command: &mut std::process::Command, timeout: Duration) -> std::io::Result<Option<std::process::Output>>` (already `pub(crate)` in `orc-core`, same crate).
- Produces: `pub enum ModelProbe { NoProber, Failed(String), Models(Vec<(String, String)>) }` and `pub fn probe(adapter: &str, command: &str) -> ModelProbe`, both consumed by Task 3.

- [x] **Step 1: Write the failing parser unit tests**

Create `rust/crates/orc-core/src/harness_models.rs` with just the test module first:

```rust
//! Best-effort "what models can this harness run" probing.
//!
//! Each multi-model harness has its own way of listing what it can run:
//! `pi --list-models` prints a table, `opencode models` prints bare
//! `provider/model` lines. There is no universal way to ask an arbitrary CLI
//! this question, so this module offers one parser per adapter it actually
//! knows and reports plainly when an adapter has none — callers (`pio
//! harness add`) fall back to trusting hand-supplied provider/model values
//! in that case, never guessing.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_real_pi_list_models_table() {
        let text = "provider  model                   context  max-out  thinking  images\n\
                     minimax   MiniMax-M2.7            204.8K   131.1K   yes       no\n\
                     minimax   MiniMax-M2.7-highspeed  204.8K   131.1K   yes       no\n\
                     minimax   MiniMax-M3              1M       128K     yes       yes\n";
        assert_eq!(
            parse_pi_table(text),
            vec![
                ("minimax".to_owned(), "MiniMax-M2.7".to_owned()),
                ("minimax".to_owned(), "MiniMax-M2.7-highspeed".to_owned()),
                ("minimax".to_owned(), "MiniMax-M3".to_owned()),
            ]
        );
    }

    #[test]
    fn parses_the_real_opencode_models_lines() {
        let text = "opencode/big-pickle\nminimax-coding-plan/MiniMax-M3\nopenai/gpt-5\n";
        assert_eq!(
            parse_opencode_lines(text),
            vec![
                ("opencode".to_owned(), "big-pickle".to_owned()),
                ("minimax-coding-plan".to_owned(), "MiniMax-M3".to_owned()),
                ("openai".to_owned(), "gpt-5".to_owned()),
            ]
        );
    }

    #[test]
    fn probe_returns_no_prober_for_an_unknown_adapter() {
        assert_eq!(probe("claude", "claude"), ModelProbe::NoProber);
    }

    #[test]
    fn probe_reports_failed_when_the_command_does_not_exist() {
        match probe("pi", "definitely-not-a-real-command-xyz") {
            ModelProbe::Failed(_) => {}
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
```

- [x] **Step 2: Run the tests to verify they fail**

Add `pub mod harness_models;` to `rust/crates/orc-core/src/lib.rs` (alphabetically, between `pub mod dispatch_supervisor;` and `pub mod inbox;`).

Run: `cd rust && cargo test -p orc-core --lib harness_models -- --exact`
Expected: FAIL to compile — none of `parse_pi_table`, `parse_opencode_lines`, `probe`, `ModelProbe` exist yet.

- [x] **Step 3: Implement the module**

Insert the implementation above the `#[cfg(test)]` block in `rust/crates/orc-core/src/harness_models.rs`:

```rust
use std::process::Command;
use std::time::Duration;

use crate::quota::command_output_with_timeout;

/// Bounded upper limit for one model-list probe. Longer than the 2s version
/// probe in `discovery.rs` since a model list can involve real provider
/// config work, not just printing a static string.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of asking a harness what models it can run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelProbe {
    /// This adapter has no known way to list its models.
    NoProber,
    /// The probe ran but did not succeed (spawn failure, timeout, nonzero
    /// exit, or output that didn't parse into any models).
    Failed(String),
    /// Parsed `(provider, model)` pairs, in the order the harness printed
    /// them.
    Models(Vec<(String, String)>),
}

/// Probe `command` (the harness's configured executable name or path) for
/// its available models, using the parser for `adapter` if one exists.
#[must_use]
pub fn probe(adapter: &str, command: &str) -> ModelProbe {
    match adapter {
        "pi" => probe_pi(command),
        "opencode" => probe_opencode(command),
        _ => ModelProbe::NoProber,
    }
}

fn probe_pi(command: &str) -> ModelProbe {
    let mut cmd = Command::new(command);
    cmd.arg("--list-models");
    run_and_parse(&mut cmd, parse_pi_table)
}

fn probe_opencode(command: &str) -> ModelProbe {
    let mut cmd = Command::new(command);
    cmd.arg("models");
    run_and_parse(&mut cmd, parse_opencode_lines)
}

fn run_and_parse(command: &mut Command, parse: impl Fn(&str) -> Vec<(String, String)>) -> ModelProbe {
    let output = match command_output_with_timeout(command, PROBE_TIMEOUT) {
        Ok(Some(output)) => output,
        Ok(None) => return ModelProbe::Failed(format!("timed out after {PROBE_TIMEOUT:?}")),
        Err(error) => return ModelProbe::Failed(format!("failed to run: {error}")),
    };
    if !output.status.success() {
        return ModelProbe::Failed(format!(
            "exited with {}",
            output
                .status
                .code()
                .map_or_else(|| "no code".to_owned(), |code| code.to_string())
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let models = parse(&text);
    if models.is_empty() {
        ModelProbe::Failed("no models found in output".to_owned())
    } else {
        ModelProbe::Models(models)
    }
}

/// Parse `pi --list-models`'s table: a header line starting with `provider`,
/// then whitespace-separated columns with provider and model first.
fn parse_pi_table(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !line.trim_start().starts_with("provider"))
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let provider = columns.next()?;
            let model = columns.next()?;
            Some((provider.to_owned(), model.to_owned()))
        })
        .collect()
}

/// Parse `opencode models`'s bare `provider/model` lines.
fn parse_opencode_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let (provider, model) = line.trim().split_once('/')?;
            Some((provider.to_owned(), model.to_owned()))
        })
        .collect()
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cd rust && cargo test -p orc-core --lib harness_models -- --exact`
Expected: all 4 tests PASS.

- [x] **Step 5: Run the full workspace test suite and commit**

Run: `cd rust && cargo test --workspace`
Expected: all green.

```bash
cd "/Volumes/Mrigesh SSD/Agent-orchestra"
git add rust/crates/orc-core/src/harness_models.rs rust/crates/orc-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat: per-adapter model-list probing for pi and opencode

Adds orc_core::harness_models::probe, one parser per adapter that
can list its own models. Everything else reports NoProber plainly
rather than guessing — each CLI's model-listing surface is
genuinely different (verified pi --list-models and opencode models
output by hand).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `pio harness add` CLI command (Part B)

**Files:**
- Modify: `rust/crates/orc-cli/src/main.rs` (`HarnessCommand` enum, `dispatch_harness`, imports)
- Modify: `rust/crates/orc-cli/tests/harness_cli.rs` (new tests)

**Interfaces:**
- Consumes: `orc_core::harness_models::{probe, ModelProbe}` (Task 2), `orc_core::bench::{HarnessConfig, load_harness_registry, write_harness_registry}` (existing).
- Produces: `pio harness add <key> --like <existing-key> [--provider <p>] [--model <m>] [--list-models] [--json]`, consumed only by end users / Task 4's display work (no other task calls into this).

- [x] **Step 1: Add the `Add` variant to `HarnessCommand`**

In `rust/crates/orc-cli/src/main.rs`, in the `HarnessCommand` enum (currently `List` and `Cap`), add a third variant after `Cap`:

```rust
    /// Register a new named harness profile, e.g. another model of `pi`.
    ///
    /// Copies `command`/`adapter`/`roles`/`resume_args`/`dispatch_args`/
    /// `dispatch_uses_stdin`/`dispatch_timeout_sec` from `--like`, then sets
    /// `--provider`/`--model` as the new profile's `args` — supported for
    /// the `pi` adapter today. `--list-models` probes and prints without
    /// registering anything.
    Add {
        /// New registry key, e.g. `pi-claude`. Required unless --list-models.
        key: Option<String>,
        /// Existing registry key to copy command/adapter/roles from.
        #[arg(long)]
        like: String,
        /// Provider name, validated against the harness's own model list
        /// when one can be probed.
        #[arg(long)]
        provider: Option<String>,
        /// Model name, validated the same way as --provider.
        #[arg(long)]
        model: Option<String>,
        /// Probe and print the harness's available models; register nothing.
        #[arg(long)]
        list_models: bool,
        #[arg(long)]
        json: bool,
    },
```

- [x] **Step 2: Write the failing CLI tests**

In `rust/crates/orc-cli/tests/harness_cli.rs`, add this helper near `fake_harness`/`failing_harness`:

```rust
/// A fake `pi` that answers `--list-models` with a fixed real-shaped table.
fn fake_pi_with_list_models(bin: &Path) {
    let path = bin.join("pi");
    fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = \"--list-models\" ]; then\n  cat <<'EOF'\nprovider  model                   context  max-out  thinking  images\nminimax   MiniMax-M2.7            204.8K   131.1K   yes       no\nminimax   MiniMax-M3              1M       128K     yes       yes\nEOF\n  exit 0\nfi\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake `pi` that errors on `--list-models` (simulating a broken/old
/// binary), to test the "probe failed, trust manual input" fallback.
fn fake_pi_that_fails_list_models(bin: &Path) {
    let path = bin.join("pi");
    fs::write(&path, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}
```

Then add these tests at the end of the file (before the final closing, i.e. after the last existing test):

```rust
#[test]
fn harness_add_registers_a_validated_model_profile() {
    let root = root("add-ok");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fake_pi_with_list_models(&bin);

    let added = run(
        &home,
        &bin,
        &[
            "harness", "add", "pi-m2", "--like", "pi-m3", "--provider", "minimax", "--model",
            "MiniMax-M2.7", "--json",
        ],
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let json: Value = serde_json::from_slice(&added.stdout).unwrap();
    assert_eq!(json["key"], "pi-m2");

    let written: Value =
        serde_json::from_slice(&fs::read(home.join("harnesses.json")).unwrap()).unwrap();
    assert_eq!(written["harnesses"]["pi-m2"]["command"], "pi");
    assert_eq!(written["harnesses"]["pi-m2"]["adapter"], "pi");
    assert_eq!(
        written["harnesses"]["pi-m2"]["args"],
        serde_json::json!(["--provider", "minimax", "--model", "MiniMax-M2.7"])
    );
    // The source profile is untouched.
    assert_eq!(
        written["harnesses"]["pi-m3"]["args"],
        serde_json::json!(["--provider", "minimax", "--model", "MiniMax-M3"])
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn harness_add_rejects_a_model_the_harness_cannot_run() {
    let root = root("add-bad-model");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fake_pi_with_list_models(&bin);

    let added = run(
        &home,
        &bin,
        &[
            "harness", "add", "pi-bogus", "--like", "pi-m3", "--provider", "minimax", "--model",
            "NoSuchModel",
        ],
    );
    assert!(!added.status.success());
    let error = String::from_utf8_lossy(&added.stderr);
    assert!(error.contains("is not a model"), "{error}");
    assert!(
        error.contains("MiniMax-M2.7"),
        "expected valid choices listed: {error}"
    );

    let written: Value =
        serde_json::from_slice(&fs::read(home.join("harnesses.json")).unwrap()).unwrap();
    assert!(written["harnesses"].get("pi-bogus").is_none());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn harness_add_list_models_prints_without_registering() {
    let root = root("add-list");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fake_pi_with_list_models(&bin);

    let listed = run(
        &home,
        &bin,
        &["harness", "add", "--like", "pi-m3", "--list-models"],
    );
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let text = String::from_utf8_lossy(&listed.stdout);
    assert!(text.contains("minimax/MiniMax-M2.7"));
    assert!(text.contains("minimax/MiniMax-M3"));

    let written: Value =
        serde_json::from_slice(&fs::read(home.join("harnesses.json")).unwrap()).unwrap();
    let keys: Vec<&str> = written["harnesses"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys.len(), 4, "no new key should be registered: {keys:?}");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn harness_add_skips_validation_when_the_probe_itself_fails() {
    let root = root("add-probe-fails");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fake_pi_that_fails_list_models(&bin);

    let added = run(
        &home,
        &bin,
        &[
            "harness", "add", "pi-manual", "--like", "pi-m3", "--provider", "minimax", "--model",
            "SomeFutureModel",
        ],
    );
    assert!(
        added.status.success(),
        "a failed probe must not block manual registration: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    let written: Value =
        serde_json::from_slice(&fs::read(home.join("harnesses.json")).unwrap()).unwrap();
    assert_eq!(
        written["harnesses"]["pi-manual"]["args"],
        serde_json::json!(["--provider", "minimax", "--model", "SomeFutureModel"])
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn harness_add_rejects_non_pi_adapters_for_now() {
    let root = root("add-wrong-adapter");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();

    let added = run(
        &home,
        &bin,
        &[
            "harness", "add", "claude-opus", "--like", "claude", "--provider", "anthropic",
            "--model", "claude-opus-5",
        ],
    );
    assert!(!added.status.success());
    let error = String::from_utf8_lossy(&added.stderr);
    assert!(error.contains("'pi' adapter"), "{error}");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn harness_add_rejects_an_unknown_like_key() {
    let root = root("add-unknown-like");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();

    let added = run(
        &home,
        &bin,
        &[
            "harness", "add", "x", "--like", "does-not-exist", "--provider", "a", "--model", "b",
        ],
    );
    assert!(!added.status.success());
    assert!(String::from_utf8_lossy(&added.stderr).contains("unknown harness"));

    let _ = fs::remove_dir_all(&root);
}
```

- [x] **Step 3: Run the tests to verify they fail**

Run: `cd rust && cargo test -p orc-cli --test harness_cli -- --exact harness_add`
Expected: FAIL to compile — `HarnessCommand::Add` isn't handled in `dispatch_harness`, and `pio` doesn't recognize `add`.

- [x] **Step 4: Implement the `Add` handler**

In `rust/crates/orc-cli/src/main.rs`, add `HarnessConfig` to the existing `use orc_core::bench::{...}` import line (currently `create_session, list_sessions, load_harness_registry, write_harness_registry` — add `HarnessConfig` alphabetically first), and add `use orc_core::harness_models::{self, ModelProbe};` near the other `use orc_core::...` lines.

In `dispatch_harness`'s `match command { ... }`, add a new arm after the `Cap { ... } => { ... }` arm (before the closing `}` of the match):

```rust
        HarnessCommand::Add {
            key,
            like,
            provider,
            model,
            list_models,
            json,
        } => {
            let mut registry = load_harness_registry()?;
            let source = registry
                .harnesses
                .get(&like)
                .cloned()
                .with_context(|| format!("unknown harness: {like}"))?;

            if list_models {
                let probe = harness_models::probe(&source.adapter, &source.command);
                return print_model_probe(&like, &probe, json);
            }

            let key = key.ok_or_else(|| {
                anyhow!("harness add requires <key> unless --list-models")
            })?;
            let provider = provider.ok_or_else(|| {
                anyhow!("harness add requires --provider (use --list-models to see choices)")
            })?;
            let model = model.ok_or_else(|| {
                anyhow!("harness add requires --model (use --list-models to see choices)")
            })?;

            if source.adapter != "pi" {
                anyhow::bail!(
                    "harness add only supports provider/model profiles for the 'pi' adapter \
                     today; {like} uses adapter '{}'",
                    source.adapter
                );
            }

            if let ModelProbe::Models(models) = harness_models::probe(&source.adapter, &source.command) {
                if !models.iter().any(|(p, m)| *p == provider && *m == model) {
                    let choices = models
                        .iter()
                        .map(|(p, m)| format!("{p}/{m}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    anyhow::bail!(
                        "'{provider}/{model}' is not a model {like} can run; valid choices: {choices}"
                    );
                }
            }

            let mut config = source;
            config.args = vec![
                "--provider".to_owned(),
                provider.clone(),
                "--model".to_owned(),
                model.clone(),
            ];
            registry.harnesses.insert(key.clone(), config);
            write_harness_registry(&registry)?;

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "key": key,
                        "like": like,
                        "provider": provider,
                        "model": model,
                    }))?
                );
            } else {
                println!("registered {key} (like {like}) \u{b7} {provider}/{model}");
            }
            Ok(0)
        }
```

Then add this helper function near `dispatch_harness` (e.g. right after it):

```rust
fn print_model_probe(key: &str, probe: &ModelProbe, json: bool) -> Result<i32> {
    match probe {
        ModelProbe::Models(models) => {
            if json {
                let rows: Vec<_> = models
                    .iter()
                    .map(|(provider, model)| serde_json::json!({"provider": provider, "model": model}))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                for (provider, model) in models {
                    println!("{provider}/{model}");
                }
            }
            Ok(0)
        }
        ModelProbe::NoProber => {
            eprintln!(
                "no automatic model listing for {key}; run its own model-listing command \
                 yourself (e.g. its --help), then pass --provider/--model to `harness add`"
            );
            Ok(1)
        }
        ModelProbe::Failed(reason) => {
            eprintln!(
                "could not probe {key}'s models ({reason}); pass --provider/--model manually"
            );
            Ok(1)
        }
    }
}
```

- [x] **Step 5: Run the tests to verify they pass**

Run: `cd rust && cargo test -p orc-cli --test harness_cli -- --exact harness_add`
Expected: all 6 new tests PASS.

- [x] **Step 6: Run clippy and the full workspace test suite, then commit**

Run: `cd rust && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: both clean.

```bash
cd "/Volumes/Mrigesh SSD/Agent-orchestra"
git add rust/crates/orc-cli/src/main.rs rust/crates/orc-cli/tests/harness_cli.rs
git commit -m "$(cat <<'EOF'
feat: pio harness add for registering named model profiles

pi-m3 was one hardcoded profile of the generic pi binary with no
way to register another (e.g. pi-claude) without hand-editing
~/.orchestra/harnesses.json. `pio harness add <key> --like <key>
--provider --model` copies command/adapter/roles from an existing
profile and validates the pair against the harness's own model
list when a prober exists (pi today), rejecting bad pairs with the
real valid choices and writing nothing. --list-models probes
without registering.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Surface registered profiles in `pio harness list`, final gates, PR

**Files:**
- Modify: `rust/crates/orc-cli/src/main.rs` (`HarnessCommand::List` handler + new helper)
- Modify: `rust/crates/orc-cli/tests/harness_cli.rs` (new test)
- Modify: `progress.md`, `LOG.md`

**Interfaces:**
- Consumes: `orc_core::bench::{HarnessConfig, load_harness_registry}` (existing/Task 1).
- Produces: nothing consumed by later tasks — this is the last task.

- [x] **Step 1: Write the failing test for the new display**

In `rust/crates/orc-cli/tests/harness_cli.rs`, add:

```rust
#[test]
fn harness_list_shows_registered_profiles_with_their_model() {
    let root = root("list-profiles");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();

    let plain = run(&home, &bin, &["harness", "list"]);
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    let text = String::from_utf8_lossy(&plain.stdout);
    assert!(text.contains("registered profiles:"));
    // pi-m3 is one of the four defaults, written the first time the
    // registry is created (by load_harness_registry inside discover()).
    assert!(
        text.contains("pi-m3") && text.contains("minimax/MiniMax-M3"),
        "{text}"
    );
    // --json output is unchanged: still exactly the 5 KNOWN_HARNESSES rows.
    let json = run(&home, &bin, &["harness", "list", "--json"]);
    let rows: Vec<Value> = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(rows.len(), 5);

    let _ = fs::remove_dir_all(&root);
}
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cd rust && cargo test -p orc-cli --test harness_cli -- --exact harness_list_shows_registered_profiles_with_their_model`
Expected: FAIL — no "registered profiles:" text is printed yet.

- [x] **Step 3: Implement the display**

In `rust/crates/orc-cli/src/main.rs`, in `dispatch_harness`'s `HarnessCommand::List { json } => { ... }` arm, the current non-json branch ends with the `for harness in &harnesses { ... }` loop. Add a call right after that loop, still inside the `else` block:

```rust
            } else {
                for harness in &harnesses {
                    if harness.available {
                        println!(
                            "{:<10} on PATH · available     {}",
                            harness.name,
                            harness.path.as_deref().unwrap_or("?"),
                        );
                        println!(
                            "    {}",
                            harness.version.as_deref().unwrap_or("version unknown")
                        );
                    } else {
                        println!("{:<10} NOT ON PATH · unavailable", harness.name);
                    }
                }
                print_registered_profiles()?;
            }
```

Then add these two helpers near `print_model_probe` (from Task 3):

```rust
/// Print every registered harness profile with its configured provider/model
/// when one is set (currently only the `pi` adapter's `--provider <p>
/// --model <m>` shape, written by `harness add` and the `pi-m3` default).
fn print_registered_profiles() -> Result<()> {
    let registry = load_harness_registry()?;
    if registry.harnesses.is_empty() {
        return Ok(());
    }
    println!("\nregistered profiles:");
    for (key, config) in &registry.harnesses {
        let roles = config.roles.join("+");
        match provider_model_args(config) {
            Some((provider, model)) => {
                println!("  {key:<12} {} ({roles}) \u{b7} {provider}/{model}", config.command);
            }
            None => {
                println!("  {key:<12} {} ({roles})", config.command);
            }
        }
    }
    Ok(())
}

/// Extract `(provider, model)` from a profile's `args` when they carry the
/// `--provider <p> --model <m>` shape this module writes.
fn provider_model_args(config: &HarnessConfig) -> Option<(&str, &str)> {
    let mut iter = config.args.iter();
    let mut provider = None;
    let mut model = None;
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--provider" => provider = iter.next().map(String::as_str),
            "--model" => model = iter.next().map(String::as_str),
            _ => {}
        }
    }
    provider.zip(model)
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cd rust && cargo test -p orc-cli --test harness_cli -- --exact`
Expected: every test in `harness_cli.rs` PASSes, including the new one and the existing `harness_list_shows_all_five_with_three_present_and_two_unavailable` (still asserting `rows.len() == 5`).

- [x] **Step 5: Run every gate from AGENTS.md**

```bash
cd "/Volumes/Mrigesh SSD/Agent-orchestra/rust"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo build --release --locked
```

Expected: all five clean.

- [x] **Step 6: Update `progress.md` and `LOG.md`, commit, push, open the PR**

Append a dated entry to `progress.md` (top-level, following the existing format) summarizing: the bug (opencode discovered but unusable), Part A's fix, Part B's `pio harness add` with pi/opencode probing and validated-or-manual registration, and that all gates pass.

In `LOG.md`, add issue #33 to the status board table (below #11's row) with status `👀` and branch `issue-33-harness-registration`, and append a ship-log entry under the existing entries following the same format as the `#11`/`#30` entries (what changed, why, what you did NOT do).

```bash
cd "/Volumes/Mrigesh SSD/Agent-orchestra"
git add rust/crates/orc-cli/src/main.rs rust/crates/orc-cli/tests/harness_cli.rs progress.md LOG.md
git commit -m "$(cat <<'EOF'
feat: surface registered harness profiles in pio harness list

Adds a "registered profiles" section to the plain-text output
showing each profile's command, roles, and configured provider/
model when set. --json output is intentionally unchanged (existing
tests assert it stays the flat 5-row KNOWN_HARNESSES array).

Closes #33.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
git push -u origin issue-33-harness-registration
gh pr create --repo Legend101Zz/Agent-orchestra \
  --title "Auto-register known harnesses + pio harness add for model profiles" \
  --body "Closes #33. See docs/superpowers/specs/2026-07-28-harness-registration-design.md for the design."
```

---

## Self-Review Notes

- **Spec coverage:** Goal 1 (auto-register on discovery) → Task 1. Goal 2 (register command) → Task 3. Goal 3 (auto-probe then manual fallback) → Task 2 (probers) + Task 3 (fallback logic + tests for both the probe-fails and probe-succeeds-but-rejects paths). Goal 4 (list shows provider/model) → Task 4. Non-goals (no generic multi-adapter args abstraction, no edit/remove) are respected — `Add`'s adapter check hard-rejects anything but `pi`.
- **Type consistency:** `ModelProbe` defined once in Task 2, imported unchanged in Task 3 (`harness_models::{self, ModelProbe}`) and matched by variant name (`Models`, `NoProber`, `Failed`) consistently across `dispatch_harness`'s `Add` arm and `print_model_probe`. `HarnessConfig` field names (`command`, `args`, `adapter`, `roles`, `resume_args`, `dispatch_args`, `dispatch_uses_stdin`, `dispatch_timeout_sec`, `extra`) match the real struct in `bench.rs` throughout.
- **`--json` compatibility confirmed:** Task 4's own test explicitly re-asserts `rows.len() == 5` after the display change, and the pre-existing `harness_list_shows_all_five_with_three_present_and_two_unavailable`/`harness_list_is_additive_and_preserves_unknown_fields`/`failed_version_probe_records_no_version_and_keeps_stored_fallback` tests were traced by hand against Task 1's auto-registration logic — none of their assertions touch the `harnesses` map's key count or contents in a way the new gap-filling insert would break.
