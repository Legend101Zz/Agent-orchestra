#![allow(unsafe_code)]

//! End-to-end CLI tests for the Phase 4A `orc dispatch` command path.
//!
//! Each test creates a fresh ORC_HOME, a fake harness under a temporary
//! `bin/` directory, and exercises the public CLI binary. The fake worker
//! is a shell script so the tests never reach a real model provider.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{
    HarnessConfig, HarnessRegistry, SessionPaneRecord, create_session, write_harness_registry,
    write_session,
};
use serde_json::Value;

fn root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "orc-dispatch-cli-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fixture_harness(home: &Path) -> HarnessConfig {
    let bin = home.join("bin");
    fs::create_dir_all(&bin).expect("create bin");
    let script = bin.join("fake-worker.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
echo "cli-fake-stdout ${@: -1}"
echo "cli-fake-stderr" 1>&2
exit 0
"#,
    )
    .expect("write fake worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod fake worker");
    HarnessConfig {
        command: "/bin/sh".to_owned(),
        args: vec![script.to_string_lossy().into_owned()],
        resume_args: Vec::new(),
        roles: vec!["worker".to_owned()],
        adapter: "fake-worker".to_owned(),
        dispatch_args: vec!["--oneshot".to_owned()],
        dispatch_uses_stdin: false,
        dispatch_timeout_sec: 30,
        extra: Default::default(),
    }
}

fn setup(label: &str) -> (PathBuf, PathBuf, String) {
    let root = root(label);
    let home = root.join("orchestra");
    fs::create_dir_all(&home).expect("create home");
    // SAFETY: this test sets ORC_HOME only for its own isolated root.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let cwd = root.join("cwd");
    fs::create_dir_all(&cwd).expect("create cwd");
    let mut registry = HarnessRegistry::default();
    registry
        .harnesses
        .insert("fake-worker".to_owned(), fixture_harness(&home));
    registry.default_workers = vec!["fake-worker".to_owned()];
    write_harness_registry(&registry).expect("persist harness registry");
    let mut session =
        create_session("codex", &["fake-worker".to_owned()], &cwd).expect("create session");
    session.panes.push(SessionPaneRecord {
        id: "session-pane-1".to_owned(),
        harness: "fake-worker".to_owned(),
        role: "worker".to_owned(),
        state: "running".to_owned(),
        pid: None,
        down_at: None,
        extra: Default::default(),
    });
    write_session(&session).expect("persist worker pane");
    (root, home, session.id)
}

fn orc(root: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .env("ORC_HOME", home)
        .env("HOME", root.join("empty-home"))
        .output()
        .unwrap_or_else(|error| panic!("run orc {args:?}: {error}"))
}

fn add_running_task(root: &Path, home: &Path, session: &str, title: &str) -> String {
    let cwd = root.join("cwd");
    let added = orc(
        root,
        home,
        &[
            "task",
            "add",
            title,
            "--session",
            session,
            "--actor",
            "brain",
            "--json",
        ],
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let added: Value = serde_json::from_slice(&added.stdout).expect("parse task add");
    let id = added["id"].as_str().expect("task id").to_owned();
    for command in [
        vec![
            "task",
            "assign",
            &id,
            "fake-worker",
            "--session",
            session,
            "--json",
        ],
        vec![
            "task",
            "start",
            &id,
            "--session",
            session,
            "--actor",
            "brain",
            "--json",
        ],
    ] {
        let output = orc(root, home, &command);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let _ = cwd;
    id
}

#[test]
fn cli_dispatch_send_list_and_show_record_actor_session_and_pane_linkage() {
    let _guard = lock();
    let (root, home, session) = setup("happy");
    let task = add_running_task(&root, &home, &session, "cli dispatch happy path");

    let dispatched = orc(
        &root,
        &home,
        &[
            "dispatch",
            "send",
            &task,
            "fake-worker",
            "summarize diff",
            "--session",
            &session,
            "--pane",
            "session-pane-1",
            "--run",
            "W-cli",
            "--actor",
            "brain",
            "--json",
        ],
    );
    assert!(
        dispatched.status.success(),
        "{}",
        String::from_utf8_lossy(&dispatched.stderr)
    );
    let record: Value = serde_json::from_slice(&dispatched.stdout).expect("parse dispatched json");
    assert_eq!(record["status"], "confirmed");
    assert_eq!(record["actor"], "brain");
    assert_eq!(record["harness"], "fake-worker");
    assert_eq!(record["task"], task);
    assert_eq!(record["pane_id"], "session-pane-1");
    assert_eq!(record["run"], "W-cli");
    assert!(
        record["command_line"]
            .as_str()
            .unwrap()
            .contains("summarize diff")
    );
    assert!(
        record["stdout"]
            .as_str()
            .unwrap()
            .contains("cli-fake-stdout summarize diff")
    );
    let dispatch_id = record["id"].as_str().expect("dispatch id").to_owned();

    let listed = orc(
        &root,
        &home,
        &["dispatch", "list", "--session", &session, "--json"],
    );
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let listed: Value = serde_json::from_slice(&listed.stdout).expect("parse listed");
    let records = listed.as_array().expect("array of records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["id"], dispatch_id);

    let shown = orc(
        &root,
        &home,
        &[
            "dispatch",
            "show",
            &dispatch_id,
            "--session",
            &session,
            "--json",
        ],
    );
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let shown: Value = serde_json::from_slice(&shown.stdout).expect("parse shown");
    assert_eq!(shown["id"], dispatch_id);
    assert_eq!(shown["status"], "confirmed");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cli_dispatch_through_a_missing_harness_returns_failed_with_an_explicit_error() {
    let _guard = lock();
    let (root, home, session) = setup("missing");
    let task = add_running_task(&root, &home, &session, "cli dispatch missing executable");
    let sent = orc(
        &root,
        &home,
        &[
            "dispatch",
            "send",
            &task,
            "missing-fixture",
            "noop",
            "--session",
            &session,
            "--json",
        ],
    );
    assert_eq!(sent.status.code(), Some(1));
    let record: Value = serde_json::from_slice(&sent.stdout).expect("parse dispatched");
    assert_eq!(record["status"], "failed");
    let error = record["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("UNKNOWN HARNESS") || error.contains("CAPABILITY UNAVAILABLE"),
        "missing harness must surface an explicit error; got {error:?}"
    );
    let _ = fs::remove_dir_all(root);
}
