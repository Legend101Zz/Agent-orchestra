//! End-to-end coverage for `pio doctor` capability probing (issue #4).
//!
//! Tests run the built `pio` binary against a hermetic `PATH` of fake harness
//! executables that advertise chosen flags in their `--help` output, so probing
//! is deterministic and never depends on what is installed on the host. The
//! fakes use only shell builtins (`printf`/`echo`) because the probe inherits
//! only this hermetic `PATH`.

#![allow(unsafe_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("orc-doctor-{label}-{}-{nonce}", std::process::id()))
}

/// Write a fake harness whose `--help` (any args) prints the given flag tokens.
fn fake_harness(bin: &Path, name: &str, tokens: &str) {
    let path = bin.join(name);
    fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{tokens}'\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Write a fake harness that also appends one line to `log` on every run.
fn counting_harness(bin: &Path, name: &str, tokens: &str, log: &Path) {
    let path = bin.join(name);
    fs::write(
        &path,
        format!(
            "#!/bin/sh\necho probe >> '{}'\nprintf '%s\\n' '{tokens}'\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(home: &Path, bin: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .env("ORC_HOME", home)
        .env("PATH", bin)
        .output()
        .unwrap()
}

fn reports(home: &Path, bin: &Path, args: &[&str]) -> Vec<Value> {
    let mut full = vec!["doctor", "--json"];
    full.extend_from_slice(args);
    let out = run(home, bin, &full);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn row<'a>(reports: &'a [Value], name: &str) -> &'a Value {
    reports.iter().find(|r| r["name"] == name).unwrap()
}

fn has(report: &Value, capability: &str) -> bool {
    report["capabilities"][capability].as_bool().unwrap()
}

#[test]
fn doctor_probes_capability_combinations_and_persists_to_registry() {
    let root = root("combos");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();

    // Three available harnesses chosen so that EVERY capability is detected
    // present in at least one and absent in at least one available harness:
    //   claude  — advertises all seven flags → all 8 caps (cancel derived).
    //   hermes  — one-shot + usage-file only → worker; proves usage without json.
    //   pi      — model only, NO one-shot → "limited"; proves cancel not derived.
    // codex and opencode stay off PATH → honest "unavailable" rows.
    fake_harness(
        &bin,
        "claude",
        "-p --resume --model --output-format --permission-mode --add-dir",
    );
    fake_harness(&bin, "hermes", "-z --usage-file");
    fake_harness(&bin, "pi", "--model");

    let rows = reports(&home, &bin, &[]);
    assert_eq!(
        rows.len(),
        5,
        "all five known harnesses are always presented"
    );

    // claude: full conductor — every capability true, including derived cancel.
    let claude = row(&rows, "claude");
    assert_eq!(claude["available"], Value::Bool(true));
    assert_eq!(claude["role"], "conductor");
    for capability in [
        "non_interactive",
        "resume",
        "tools",
        "model_select",
        "structured_output",
        "usage_reporting",
        "cancellation",
        "working_dir",
    ] {
        assert!(has(claude, capability), "claude should have {capability}");
    }

    // hermes: worker — usage present, structured/resume/tools/model/cwd absent.
    let hermes = row(&rows, "hermes");
    assert_eq!(hermes["role"], "worker");
    assert!(has(hermes, "non_interactive"));
    assert!(has(hermes, "usage_reporting"));
    assert!(has(hermes, "cancellation")); // derived from non-interactive
    assert!(!has(hermes, "structured_output"));
    assert!(!has(hermes, "resume"));
    assert!(!has(hermes, "working_dir"));

    // pi: limited — only model select; no one-shot means no derived cancellation.
    let pi = row(&rows, "pi");
    assert_eq!(pi["role"], "limited");
    assert!(has(pi, "model_select"));
    assert!(!has(pi, "non_interactive"));
    assert!(!has(pi, "cancellation"));

    // Unavailable harnesses are presented, never hidden.
    for name in ["codex", "opencode"] {
        let report = row(&rows, name);
        assert_eq!(report["available"], Value::Bool(false));
        assert_eq!(report["role"], "unavailable");
    }

    // Results serialize into harnesses.json under discovered.<name>.probe.
    let written: Value =
        serde_json::from_slice(&fs::read(home.join("harnesses.json")).unwrap()).unwrap();
    let claude_probe = &written["discovered"]["claude"]["probe"];
    assert!(
        claude_probe["capabilities"]
            .as_array()
            .unwrap()
            .contains(&Value::from("structured_output"))
    );
    assert!(claude_probe["binary"]["size"].as_u64().unwrap() > 0);
    assert!(claude_probe["probed_at"].is_string());
    // pi's probe records model_select but not one-shot.
    let pi_caps = written["discovered"]["pi"]["probe"]["capabilities"]
        .as_array()
        .unwrap();
    assert!(pi_caps.contains(&Value::from("model_select")));
    assert!(!pi_caps.contains(&Value::from("non_interactive")));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn doctor_plain_output_matches_spec_table_with_unavailable_row() {
    let root = root("table");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fake_harness(
        &bin,
        "claude",
        "-p --resume --model --output-format --permission-mode --add-dir",
    );
    fake_harness(&bin, "hermes", "-z --usage-file");

    let out = run(&home, &bin, &["doctor"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);

    // Spec table shape: display · installed · role · summary for present rows.
    assert!(text.contains("Claude Code"));
    assert!(text.contains("installed"));
    assert!(text.contains("conductor"));
    assert!(text.contains("Hermes"));
    // An explicit unavailable row for every missing harness, never hidden.
    for display in ["Codex", "OpenCode", "Pi/MiniMax"] {
        let line = text
            .lines()
            .find(|line| line.contains(display))
            .unwrap_or_else(|| panic!("missing row for {display} in:\n{text}"));
        assert!(
            line.contains("unavailable"),
            "row for {display} must be marked unavailable: {line}"
        );
    }
    // The capability matrix pairs a glyph with every state (never color alone).
    assert!(text.contains("CAPABILITIES"));
    assert!(text.contains('\u{2713}')); //  present
    assert!(text.contains('\u{2717}')); //  not advertised

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn doctor_probe_cache_invalidates_when_binary_changes() {
    let root = root("cache");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let log = root.join("pi-probe.log");
    // Only pi is on PATH, so only pi is probed (one help_argv → one line/probe).
    counting_harness(&bin, "pi", "-p --model", &log);

    let count = |log: &Path| {
        fs::read_to_string(log)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    };

    // First run probes; model select is detected.
    let first = reports(&home, &bin, &[]);
    assert_eq!(count(&log), 1);
    assert!(has(row(&first, "pi"), "model_select"));

    // Second run with an unchanged binary is a cache hit — no re-probe.
    let cached = reports(&home, &bin, &[]);
    assert_eq!(count(&log), 1, "unchanged binary must not be re-probed");
    // probed_at is stable across a cache hit.
    assert_eq!(
        row(&first, "pi")["probed_at"],
        row(&cached, "pi")["probed_at"]
    );

    // Change the binary (different content → different size, and bump mtime):
    // the cache key (path/mtime/size) no longer matches, forcing a re-probe.
    counting_harness(&bin, "pi", "-p", &log); // drops --model
    let handle = fs::OpenOptions::new()
        .write(true)
        .open(bin.join("pi"))
        .unwrap();
    handle
        .set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();

    let changed = reports(&home, &bin, &[]);
    assert_eq!(count(&log), 2, "changed binary must be re-probed");
    assert!(
        !has(row(&changed, "pi"), "model_select"),
        "re-probe reflects the new capability set"
    );

    // --refresh forces a re-probe even when identity matches.
    reports(&home, &bin, &["--refresh"]);
    assert_eq!(count(&log), 3, "--refresh always re-probes");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn doctor_write_preserves_unknown_fields_at_every_layer() {
    let root = root("additive");
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    // Seed a registry carrying unknown fields at top level, app, and inside a
    // discovered harness's probe (including an unknown capability slug).
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tools/fixtures/probed-harnesses.json"),
        home.join("harnesses.json"),
    )
    .unwrap();
    // A fresh claude on PATH forces a dirty write of the whole registry.
    fake_harness(&bin, "claude", "-p --resume --output-format");

    let rows = reports(&home, &bin, &[]);
    // The seeded pi (not on PATH) is presented unavailable but its stored probe
    // still exposes only the KNOWN typed capabilities — "telepathy" is dropped.
    let pi = row(&rows, "pi");
    assert_eq!(pi["available"], Value::Bool(false));
    assert!(has(pi, "non_interactive"));
    assert!(has(pi, "resume"));
    // Unknown slug never surfaces as a known capability column.
    assert!(pi["capabilities"].get("telepathy").is_none());

    // Every unknown field survived doctor's additive write.
    let written: Value =
        serde_json::from_slice(&fs::read(home.join("harnesses.json")).unwrap()).unwrap();
    assert_eq!(written["future_top_level_field"]["count"], 42);
    assert_eq!(written["app"]["future_app_field"], "kept-app-value");
    let probe = &written["discovered"]["pi"]["probe"];
    assert_eq!(probe["unknown_probe_field"][0], "kept-probe");
    assert_eq!(probe["binary"]["unknown_identity_field"], "kept-identity");
    // The unknown capability slug is preserved verbatim in the stored set.
    assert!(
        probe["capabilities"]
            .as_array()
            .unwrap()
            .contains(&Value::from("telepathy"))
    );

    let _ = fs::remove_dir_all(&root);
}
