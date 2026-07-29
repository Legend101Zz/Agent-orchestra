//! `pio config` and the client must agree about the theme.
//!
//! Before issue #37 `pio config set theme <x>` wrote `config.json` while the
//! client rendered `harnesses.json`'s `app.theme`, so the command reported
//! success and changed nothing on screen. These tests drive the real binary
//! and check the record the daemon actually serves to the client.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// A home per test, named rather than timestamped: these run in parallel in
/// one binary, and a coarse clock hands two of them the same nanosecond.
fn fresh_home(test: &str) -> PathBuf {
    std::env::temp_dir().join(format!("orc-theme-cli-{}-{test}", std::process::id()))
}

fn pio(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .env("ORC_HOME", home)
        .output()
        .unwrap_or_else(|error| panic!("run pio {args:?}: {error}"))
}

fn stdout_json(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "parse stdout {:?}: {error}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read json")).expect("parse json")
}

#[test]
fn config_set_theme_reaches_the_record_the_client_renders() {
    let home = fresh_home("set");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).expect("create home");

    for name in ["phosphor", "ember", "nocturne"] {
        pio(&home, &["config", "set", "theme", name]);

        // What the daemon serves to the client on `Home`.
        assert_eq!(
            read_json(&home.join("harnesses.json"))["app"]["theme"],
            Value::String(name.to_owned()),
            "`config set theme {name}` must change what the client renders"
        );
        // What the CLI reports back.
        assert_eq!(
            stdout_json(&pio(&home, &["config", "get", "theme"])),
            Value::String(name.to_owned())
        );
        // And the derived copy, so a pre-#37 reader agrees rather than drifts.
        assert_eq!(
            read_json(&home.join("config.json"))["theme"],
            Value::String(name.to_owned())
        );
        assert_eq!(
            stdout_json(&pio(&home, &["config", "list"]))["theme"],
            Value::String(name.to_owned())
        );
    }

    // An unknown name resolves to the flagship instead of being written raw.
    pio(&home, &["config", "set", "theme", "chartreuse"]);
    assert_eq!(
        stdout_json(&pio(&home, &["config", "get", "theme"])),
        Value::String("nocturne".to_owned())
    );
    assert_eq!(
        read_json(&home.join("harnesses.json"))["app"]["theme"],
        Value::String("nocturne".to_owned())
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_hand_edited_registry_is_reported_rather_than_the_stale_config_copy() {
    let home = fresh_home("hand-edited");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).expect("create home");
    pio(&home, &["config", "set", "theme", "ember"]);

    // Someone edits the registry by hand — still the authoritative record.
    let mut registry = read_json(&home.join("harnesses.json"));
    registry["app"]["theme"] = Value::String("phosphor".to_owned());
    fs::write(
        home.join("harnesses.json"),
        serde_json::to_vec_pretty(&registry).expect("encode registry"),
    )
    .expect("write registry");

    assert_eq!(
        stdout_json(&pio(&home, &["config", "get", "theme"])),
        Value::String("phosphor".to_owned()),
        "config get must not answer from the derived copy"
    );
    assert_eq!(
        read_json(&home.join("config.json"))["theme"],
        Value::String("ember".to_owned()),
        "fixture check: the stale copy is still on disk"
    );

    let _ = fs::remove_dir_all(&home);
}
