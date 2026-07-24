#![allow(unsafe_code)]

//! End-to-end CLI tests for quota guard v2 (issue #7): the concurrency cap,
//! the queued exit code, the ORC WARNING channel, `dispatch drain`, and the
//! `harness cap` setter. A slot is held via the core API to simulate a worker
//! in flight, so the CLI subprocess sees a full cap and must queue — proving
//! AC1/AC3 through the real `pio` binary without a slow real backoff (the
//! backoff schedule itself is covered at the core level in `quota_guard.rs`).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessConfig, HarnessRegistry, create_session, write_harness_registry};
use orc_core::spawn_guard::{DEFAULT_LEASE_TTL, acquire_slot};
use serde_json::Value;

fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "orc-quota2-cli-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn worker_script(home: &Path, sentinel: &Path) -> String {
    let bin = home.join("bin");
    fs::create_dir_all(&bin).expect("create bin");
    let script = bin.join("capped-worker.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho ran > '{}'\necho \"cli-capped-stdout ${{@: -1}}\"\nexit 0\n",
            sentinel.display()
        ),
    )
    .expect("write worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod worker");
    script.to_string_lossy().into_owned()
}

fn setup(label: &str) -> (PathBuf, PathBuf, String, PathBuf) {
    let root = root(label);
    let home = root.join("orchestra");
    fs::create_dir_all(&home).expect("create home");
    // SAFETY: this test sets ORC_HOME only for its own isolated root.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let cwd = root.join("cwd");
    fs::create_dir_all(&cwd).expect("create cwd");
    let sentinel = root.join("worker-ran.marker");
    let script = worker_script(&home, &sentinel);
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert(
        "capped".to_owned(),
        HarnessConfig {
            command: "/bin/sh".to_owned(),
            args: vec![script],
            resume_args: Vec::new(),
            roles: vec!["worker".to_owned()],
            adapter: "hermes".to_owned(),
            dispatch_args: vec!["--oneshot".to_owned()],
            dispatch_uses_stdin: false,
            dispatch_timeout_sec: 30,
            extra: Default::default(),
        },
    );
    registry.default_workers = vec!["capped".to_owned()];
    write_harness_registry(&registry).expect("persist registry");
    let session = create_session("codex", &["capped".to_owned()], &cwd).expect("create session");
    (root, home, session.id, sentinel)
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
            "capped",
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
    id
}

#[test]
fn cli_dispatch_at_cap_is_queued_then_drains_and_the_cap_setter_persists() {
    let _guard = lock();
    let (root, home, session, sentinel) = setup("queue");

    // AC3 UX: set the cap through the CLI; it must persist to the registry.
    let capped = orc(&root, &home, &["harness", "cap", "capped", "1", "--json"]);
    assert!(
        capped.status.success(),
        "{}",
        String::from_utf8_lossy(&capped.stderr)
    );
    let cap: Value = serde_json::from_slice(&capped.stdout).expect("parse cap json");
    assert_eq!(cap["effective_cap"], 1);
    assert_eq!(cap["override"], 1);

    let task = add_running_task(&root, &home, &session, "cli queued dispatch");

    // Hold the single slot to simulate a worker already in flight.
    let held = acquire_slot("capped", 1, DEFAULT_LEASE_TTL, Some("in-flight"))
        .expect("acquire held slot")
        .expect("slot free to hold");

    // AC1: the CLI dispatch is QUEUED (exit 75 EX_TEMPFAIL), not spawned, and
    // the warning surfaces on the ORC WARNING channel (stderr).
    let queued = orc(
        &root,
        &home,
        &[
            "dispatch",
            "send",
            &task,
            "capped",
            "do it",
            "--session",
            &session,
            "--json",
        ],
    );
    assert_eq!(
        queued.status.code(),
        Some(75),
        "queued dispatch uses EX_TEMPFAIL"
    );
    let record: Value = serde_json::from_slice(&queued.stdout).expect("parse queued json");
    assert_eq!(record["status"], "queued");
    assert!(
        !sentinel.exists(),
        "no worker may spawn while the cap is full"
    );
    let warned = String::from_utf8_lossy(&queued.stderr);
    assert!(
        warned.contains("ORC WARNING") && warned.contains("concurrency cap"),
        "cap warning must reach the ORC WARNING channel: {warned}"
    );

    // Draining while the slot is still held changes nothing (still queued).
    let noop = orc(
        &root,
        &home,
        &["dispatch", "drain", "--session", &session, "--json"],
    );
    assert!(noop.status.success());
    let drained_none: Value = serde_json::from_slice(&noop.stdout).expect("parse drain json");
    assert_eq!(drained_none.as_array().map(Vec::len), Some(0));
    assert!(!sentinel.exists(), "still capped, still not spawned");

    // Free the slot; draining now runs the queued dispatch to confirmed.
    drop(held);
    let drained = orc(
        &root,
        &home,
        &["dispatch", "drain", "--session", &session, "--json"],
    );
    assert!(
        drained.status.success(),
        "{}",
        String::from_utf8_lossy(&drained.stderr)
    );
    let records: Value = serde_json::from_slice(&drained.stdout).expect("parse drain json");
    let records = records.as_array().expect("drain array");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["status"], "confirmed");
    assert_eq!(records[0]["id"], record["id"], "the same queued record ran");
    assert!(sentinel.exists(), "the worker ran once the slot freed");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cli_harness_cap_clear_restores_the_adapter_default() {
    let _guard = lock();
    let (root, home, _session, _sentinel) = setup("clear");

    // hermes adapter default is 3; an override sets 1, --clear restores 3.
    let set = orc(&root, &home, &["harness", "cap", "capped", "1", "--json"]);
    let set: Value = serde_json::from_slice(&set.stdout).expect("parse set");
    assert_eq!(set["effective_cap"], 1);

    let cleared = orc(
        &root,
        &home,
        &["harness", "cap", "capped", "--clear", "--json"],
    );
    assert!(
        cleared.status.success(),
        "{}",
        String::from_utf8_lossy(&cleared.stderr)
    );
    let cleared: Value = serde_json::from_slice(&cleared.stdout).expect("parse clear");
    assert_eq!(
        cleared["effective_cap"], 3,
        "cleared override falls back to adapter default"
    );
    assert!(cleared["override"].is_null());

    let _ = fs::remove_dir_all(root);
}
