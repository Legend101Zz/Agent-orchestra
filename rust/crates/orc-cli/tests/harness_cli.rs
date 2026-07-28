//! End-to-end coverage for `pio harness list` auto-discovery (issue #3).
//!
//! These tests run the built `pio` binary against a hermetic `PATH` containing
//! fake harness executables, so discovery is deterministic and never depends on
//! what is installed on the machine running the suite.

#![allow(unsafe_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "orc-harness-{label}-{}-{nonce}",
        std::process::id()
    ))
}

/// Write one fake harness script that prints a version for `--version`.
fn fake_harness(bin: &Path, name: &str, version: &str) {
    let path = bin.join(name);
    fs::write(
        &path,
        format!("#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo \"{version}\"\n  exit 0\nfi\nexit 0\n"),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Write a fake harness whose `--version` exits non-zero with noisy stderr.
///
/// Models a real CLI that does not understand `--version`: the error text must
/// never be recorded as the harness "version".
fn failing_harness(bin: &Path, name: &str) {
    let path = bin.join(name);
    fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo \"error: unrecognized option --version\" >&2\n  echo \"usage: noisy second stderr line\" >&2\n  exit 1\nfi\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake `pi` that answers `--list-models` with a fixed real-shaped table.
///
/// Uses only `echo` (a shell builtin), not `cat`, since the hermetic test
/// `PATH` contains only this fixture's own `bin` directory — no external
/// `cat` binary would resolve there.
fn fake_pi_with_list_models(bin: &Path) {
    let path = bin.join("pi");
    fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = \"--list-models\" ]; then\n  echo 'provider  model                   context  max-out  thinking  images'\n  echo 'minimax   MiniMax-M2.7            204.8K   131.1K   yes       no'\n  echo 'minimax   MiniMax-M3              1M       128K     yes       yes'\n  exit 0\nfi\nexit 0\n",
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

/// Run `pio` with a hermetic PATH (only `bin`) and an isolated ORC_HOME.
fn run(home: &Path, bin: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .env("ORC_HOME", home)
        .env("PATH", bin)
        .output()
        .unwrap()
}

fn fixture_registry() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tools/fixtures/discovered-harnesses.json")
}

#[test]
fn harness_list_shows_all_five_with_three_present_and_two_unavailable() {
    let root = root("list");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    // Three of the five known harnesses are on PATH; codex and opencode are not.
    fake_harness(&bin, "claude", "claude 1.2.3");
    fake_harness(&bin, "hermes", "hermes 9.9");
    fake_harness(&bin, "pi", "pi 0.4.0");

    let plain = run(&home, &bin, &["harness", "list"]);
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    let text = String::from_utf8_lossy(&plain.stdout);
    // Every known harness appears — found or missing, never hidden.
    for name in ["claude", "codex", "hermes", "pi", "opencode"] {
        assert!(text.contains(name), "missing harness {name} in:\n{text}");
    }
    // Found harnesses report availability and their resolved path.
    assert!(text.contains("on PATH \u{b7} available"));
    assert!(text.contains(&bin.join("claude").to_string_lossy().into_owned()));
    // Missing harnesses are explicitly marked unavailable.
    assert!(text.contains("NOT ON PATH \u{b7} unavailable"));

    // JSON output lists exactly the five known harnesses with correct state.
    let json = run(&home, &bin, &["harness", "list", "--json"]);
    assert!(json.status.success());
    let rows: Vec<Value> = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(rows.len(), 5);
    let available: Vec<&str> = rows
        .iter()
        .filter(|row| row["available"] == Value::Bool(true))
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(available, ["claude", "hermes", "pi"]);
    let pi = rows.iter().find(|row| row["name"] == "pi").unwrap();
    assert_eq!(pi["version"], "pi 0.4.0");

    let _ = fs::remove_dir_all(&root);
}

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

#[test]
fn harness_list_is_additive_and_preserves_unknown_fields() {
    let root = root("additive");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    // Seed a pre-existing registry carrying unknown fields at every layer.
    fs::copy(fixture_registry(), home.join("harnesses.json")).unwrap();
    // `pi` is on PATH now, with a different path/version than the seed recorded.
    fake_harness(&bin, "pi", "pi 2.0-fresh");

    let listed = run(&home, &bin, &["harness", "list"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );

    let written: Value =
        serde_json::from_slice(&fs::read(home.join("harnesses.json")).unwrap()).unwrap();

    // Unknown fields survive the discovery write at every additive layer.
    assert_eq!(written["future_top_level_field"]["count"], 42);
    assert_eq!(written["app"]["future_app_field"], "kept-app-value");
    assert_eq!(
        written["discovered"]["pi"]["unknown_capability_field"][0],
        "steer"
    );
    // The user-editable configured harness is untouched.
    assert_eq!(written["harnesses"]["hermes"]["adapter"], "hermes");

    // first_seen is preserved from the seed; path/last_seen/version refresh.
    let pi = &written["discovered"]["pi"];
    assert_eq!(pi["first_seen"], "2026-01-01T00:00:00+00:00");
    assert_ne!(pi["last_seen"], "2026-01-02T00:00:00+00:00");
    assert_eq!(pi["version"], "pi 2.0-fresh");
    assert_eq!(
        pi["path"],
        bin.join("pi").to_string_lossy().into_owned().as_str()
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn failed_version_probe_records_no_version_and_keeps_stored_fallback() {
    // Regression for the reviewer's finding: a harness that rejects `--version`
    // (non-zero exit, noisy stderr) must never have its error text recorded as
    // the "version". A previously stored version must survive as the fallback.
    let root = root("probe-fail");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    // Seed a registry that already holds a good stored version for `pi`.
    fs::copy(fixture_registry(), home.join("harnesses.json")).unwrap();
    // Both are on PATH but fail `--version`; `claude` has no stored version.
    failing_harness(&bin, "claude");
    failing_harness(&bin, "pi");

    let json = run(&home, &bin, &["harness", "list", "--json"]);
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let rows: Vec<Value> = serde_json::from_slice(&json.stdout).unwrap();
    let claude = rows.iter().find(|row| row["name"] == "claude").unwrap();
    let pi = rows.iter().find(|row| row["name"] == "pi").unwrap();
    // Found on PATH, but a failed probe records NO version for claude...
    assert_eq!(claude["available"], Value::Bool(true));
    assert_eq!(claude["version"], Value::Null);
    // ...and pi keeps its previously stored version as the fallback.
    assert_eq!(pi["available"], Value::Bool(true));
    assert_eq!(pi["version"], "pi 0.0.1-preexisting");

    // Plain output shows "version unknown", never the stderr error text.
    let plain = run(&home, &bin, &["harness", "list"]);
    let text = String::from_utf8_lossy(&plain.stdout);
    assert!(text.contains("version unknown"));
    assert!(!text.contains("unrecognized option"));

    // And nothing garbage is persisted to the additive registry.
    let raw = fs::read_to_string(home.join("harnesses.json")).unwrap();
    assert!(
        !raw.contains("unrecognized option"),
        "error text leaked into registry:\n{raw}"
    );
    let written: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(written["discovered"]["claude"]["version"], Value::Null);
    assert_eq!(
        written["discovered"]["pi"]["version"],
        "pi 0.0.1-preexisting"
    );

    let _ = fs::remove_dir_all(&root);
}

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
            "harness",
            "add",
            "pi-m2",
            "--like",
            "pi-m3",
            "--provider",
            "minimax",
            "--model",
            "MiniMax-M2.7",
            "--json",
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
            "harness",
            "add",
            "pi-bogus",
            "--like",
            "pi-m3",
            "--provider",
            "minimax",
            "--model",
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
            "harness",
            "add",
            "pi-manual",
            "--like",
            "pi-m3",
            "--provider",
            "minimax",
            "--model",
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
            "harness",
            "add",
            "claude-opus",
            "--like",
            "claude",
            "--provider",
            "anthropic",
            "--model",
            "claude-opus-5",
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
            "harness",
            "add",
            "x",
            "--like",
            "does-not-exist",
            "--provider",
            "a",
            "--model",
            "b",
        ],
    );
    assert!(!added.status.success());
    assert!(String::from_utf8_lossy(&added.stderr).contains("unknown harness"));

    let _ = fs::remove_dir_all(&root);
}

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
