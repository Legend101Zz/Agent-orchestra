#![allow(unsafe_code)]

//! Issue #30 end-to-end acceptance tests for detached `orch delegate`.
//!
//! Every operation goes through a fresh real `pio` process. The fixtures record
//! process-lifetime events on disk, so concurrency and queue draining are
//! observed rather than inferred from returned status words.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessConfig, HarnessRegistry, create_session, write_harness_registry};
use orc_core::spawn_guard::active_slots;
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
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "pio-background-dispatch-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn pio(root: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .env("ORC_HOME", home)
        .env("HOME", root.join("empty-home"))
        .output()
        .unwrap_or_else(|error| panic!("run pio {args:?}: {error}"))
}

fn pio_ok(root: &Path, home: &Path, args: &[&str]) -> Value {
    let output = pio(root, home, args);
    assert!(
        output.status.success(),
        "pio {args:?} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("pio {args:?} did not emit JSON: {error}"))
}

fn fixture_script(root: &Path) -> PathBuf {
    let script = root.join("worker.sh");
    let events = root.join("events.log");
    let alpha = root.join("alpha.started");
    let beta = root.join("beta.started");
    fs::write(
        &script,
        format!(
            r#"#!/bin/sh
prompt="${{@: -1}}"
echo "$prompt start" >> '{events}'
case "$prompt" in
  first)
    sleep 2
    echo "first answer"
    ;;
  second)
    echo "second answer"
    ;;
  alpha)
    echo yes > '{alpha}'
    i=0
    while [ ! -f '{beta}' ] && [ "$i" -lt 100 ]; do sleep 0.02; i=$((i + 1)); done
    [ -f '{beta}' ] || exit 9
    sleep 0.2
    echo "alpha answer"
    ;;
  beta)
    echo yes > '{beta}'
    i=0
    while [ ! -f '{alpha}' ] && [ "$i" -lt 100 ]; do sleep 0.02; i=$((i + 1)); done
    [ -f '{alpha}' ] || exit 9
    sleep 0.2
    echo "beta answer"
    ;;
  crash)
    sleep 30
    echo "must not survive supervisor"
    ;;
esac
echo "$prompt end" >> '{events}'
"#,
            events = events.display(),
            alpha = alpha.display(),
            beta = beta.display(),
        ),
    )
    .expect("write worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod worker");
    script
}

fn setup(label: &str, max_parallel_workers: usize) -> (PathBuf, PathBuf, String) {
    let root = root(label);
    let home = root.join("orchestra");
    let cwd = root.join("cwd");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cwd).expect("cwd");
    // SAFETY: every test in this file holds the process-global lock.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let script = fixture_script(&root);
    let mut registry = HarnessRegistry {
        max_parallel_workers,
        ..HarnessRegistry::default()
    };
    registry.harnesses.insert(
        "fixture".to_owned(),
        HarnessConfig {
            command: "/bin/sh".to_owned(),
            args: vec![script.to_string_lossy().into_owned()],
            resume_args: Vec::new(),
            roles: vec!["worker".to_owned()],
            adapter: "fixture".to_owned(),
            dispatch_args: vec!["--oneshot".to_owned()],
            dispatch_uses_stdin: false,
            dispatch_timeout_sec: 20,
            extra: Default::default(),
        },
    );
    registry.concurrency.insert("fixture".to_owned(), 2);
    write_harness_registry(&registry).expect("registry");
    let session = create_session("codex", &["fixture".to_owned()], &cwd).expect("session");
    (root, home, session.id)
}

fn delegate(root: &Path, home: &Path, session: &str, prompt: &str) -> (Output, Value) {
    let output = pio(
        root,
        home,
        &[
            "orch",
            "delegate",
            "fixture",
            "--session",
            session,
            "--title",
            prompt,
            "--prompt",
            prompt,
            "--dispatch-timeout",
            "20",
            "--json",
        ],
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("delegate output was not JSON: {error}"));
    (output, value)
}

fn await_task(root: &Path, home: &Path, session: &str, task: &str) -> Value {
    pio_ok(
        root,
        home,
        &[
            "orch",
            "await",
            task,
            "--session",
            session,
            "--timeout",
            "10",
            "--poll-interval-ms",
            "10",
            "--json",
        ],
    )
}

#[test]
fn delegate_confirms_while_running_and_cap_one_queues_until_real_exit() {
    let _guard = lock();
    let (root, home, session) = setup("cap-one", 1);

    let started = Instant::now();
    let (first_output, first) = delegate(&root, &home, &session, "first");
    assert!(first_output.status.success());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "delegate blocked for the two-second worker: {:?}",
        started.elapsed()
    );
    assert_eq!(first["dispatches"][0]["status"], "confirmed");
    assert_eq!(first["dispatches"][0]["execution_status"], "running");
    assert!(first["dispatches"][0]["exit_code"].is_null());
    let first_task = first["tasks"][0]["id"].as_str().expect("task").to_owned();

    let (second_output, second) = delegate(&root, &home, &session, "second");
    assert_eq!(second_output.status.code(), Some(75));
    assert_eq!(second["dispatches"][0]["status"], "queued");
    let second_task = second["tasks"][0]["id"].as_str().expect("task").to_owned();
    let events = fs::read_to_string(root.join("events.log")).expect("events");
    assert_eq!(events.lines().collect::<Vec<_>>(), vec!["first start"]);

    let first_done = await_task(&root, &home, &session, &first_task);
    assert_eq!(first_done["dispatches"][0]["execution_status"], "succeeded");
    assert_eq!(first_done["dispatches"][0]["exit_code"], 0);
    assert!(
        first_done["dispatches"][0]["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("first answer")
    );

    // A different pio process sees the durable terminal result, and the first
    // supervisor's real exit automatically drains the queued second task.
    let second_done = await_task(&root, &home, &session, &second_task);
    assert_eq!(
        second_done["dispatches"][0]["execution_status"],
        "succeeded"
    );
    let status = pio_ok(
        &root,
        &home,
        &[
            "orch",
            "status",
            &second_task,
            "--session",
            &session,
            "--json",
        ],
    );
    assert_eq!(status["dispatches"][0]["exit_code"], 0);

    let events = fs::read_to_string(root.join("events.log")).expect("events");
    let lines: Vec<_> = events.lines().collect();
    let first_end = lines.iter().position(|line| *line == "first end").unwrap();
    let second_start = lines
        .iter()
        .position(|line| *line == "second start")
        .unwrap();
    assert!(
        first_end < second_start,
        "second worker started before the first really exited: {lines:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cap_two_workers_overlap_and_both_reach_terminal_state() {
    let _guard = lock();
    let (root, home, session) = setup("overlap", 2);
    let (alpha_output, alpha) = delegate(&root, &home, &session, "alpha");
    let (beta_output, beta) = delegate(&root, &home, &session, "beta");
    assert!(alpha_output.status.success() && beta_output.status.success());
    assert_eq!(alpha["dispatches"][0]["execution_status"], "running");
    assert_eq!(beta["dispatches"][0]["execution_status"], "running");

    let alpha_task = alpha["tasks"][0]["id"].as_str().unwrap();
    let beta_task = beta["tasks"][0]["id"].as_str().unwrap();
    let alpha_done = await_task(&root, &home, &session, alpha_task);
    let beta_done = await_task(&root, &home, &session, beta_task);
    assert_eq!(alpha_done["dispatches"][0]["execution_status"], "succeeded");
    assert_eq!(beta_done["dispatches"][0]["execution_status"], "succeeded");

    let events = fs::read_to_string(root.join("events.log")).expect("events");
    let lines: Vec<_> = events.lines().collect();
    let first_end = lines
        .iter()
        .position(|line| line.ends_with(" end"))
        .expect("an end");
    let alpha_start = lines
        .iter()
        .position(|line| *line == "alpha start")
        .unwrap();
    let beta_start = lines.iter().position(|line| *line == "beta start").unwrap();
    assert!(
        alpha_start < first_end && beta_start < first_end,
        "both starts must precede the first end: {lines:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn dead_supervisor_reconciles_terminal_kills_worker_and_releases_slots() {
    let _guard = lock();
    let (root, home, session) = setup("crash", 1);
    let (output, delegated) = delegate(&root, &home, &session, "crash");
    assert!(output.status.success());
    let task = delegated["tasks"][0]["id"].as_str().unwrap().to_owned();
    let supervisor = delegated["dispatches"][0]["supervisor_pid"]
        .as_u64()
        .expect("supervisor pid")
        .to_string();
    let worker = delegated["dispatches"][0]["worker_pid"]
        .as_u64()
        .expect("worker pid")
        .to_string();

    let killed = Command::new("/bin/kill")
        .args(["-9", &supervisor])
        .status()
        .expect("kill supervisor");
    assert!(killed.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    let reconciled = loop {
        let status = pio_ok(
            &root,
            &home,
            &["orch", "status", &task, "--session", &session, "--json"],
        );
        if status["dispatches"][0]["execution_status"] == "orphaned" {
            break status;
        }
        assert!(Instant::now() < deadline, "supervisor was not reconciled");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        reconciled["dispatches"][0]["failure_kind"],
        "supervisor_lost"
    );
    assert_eq!(active_slots("fixture").expect("harness slots"), 0);
    assert_eq!(
        active_slots(&format!("session:{session}")).expect("session slots"),
        0
    );

    let worker_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let alive = Command::new("/bin/kill")
            .args(["-0", &worker])
            .output()
            .is_ok_and(|output| output.status.success());
        if !alive {
            break;
        }
        assert!(
            Instant::now() < worker_deadline,
            "worker {worker} survived supervisor reconciliation"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn pi_json_transport_persists_readable_answer_and_exact_usage() {
    let _guard = lock();
    let root = root("json-answer");
    let home = root.join("orchestra");
    let cwd = root.join("cwd");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&cwd).expect("cwd");
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let script = root.join("pi-json.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
echo '{"type":"session","id":"transport-noise"}'
echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"READABLE "},"message":{"large":"snapshot"}}'
echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"ANSWER"},"message":{"large":"snapshot"}}'
echo '{"type":"agent_end","messages":[{"role":"assistant","usage":{"input":120,"output":30,"cacheRead":2048,"totalTokens":2198,"cost":{"total":0.0002014}}}]}'
"#,
    )
    .expect("json worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert(
        "pi-json".to_owned(),
        HarnessConfig {
            command: "/bin/sh".to_owned(),
            args: vec![script.to_string_lossy().into_owned()],
            resume_args: Vec::new(),
            roles: vec!["worker".to_owned()],
            adapter: "pi".to_owned(),
            dispatch_args: vec!["--oneshot".to_owned()],
            dispatch_uses_stdin: false,
            dispatch_timeout_sec: 20,
            extra: Default::default(),
        },
    );
    write_harness_registry(&registry).expect("registry");
    let session = create_session("codex", &["pi-json".to_owned()], &cwd).expect("session");
    let output = pio_ok(
        &root,
        &home,
        &[
            "orch",
            "delegate",
            "pi-json",
            "--session",
            &session.id,
            "--title",
            "answer",
            "--prompt",
            "answer",
            "--json",
        ],
    );
    let task = output["tasks"][0]["id"].as_str().unwrap();
    let terminal = await_task(&root, &home, &session.id, task);
    let record = &terminal["dispatches"][0];
    assert_eq!(record["stdout"], "READABLE ANSWER");
    assert!(
        !record["stdout"]
            .as_str()
            .unwrap()
            .contains("message_update"),
        "transport JSON leaked into the answer"
    );
    assert_eq!(record["usage"]["total"], 2198);
    assert_eq!(record["usage"]["input"], 120);
    assert_eq!(record["usage"]["output"], 30);
    assert_eq!(record["usage"]["cache_read"], 2048);
    assert_eq!(record["usage"]["cost_usd"], 0.000201);
    let _ = fs::remove_dir_all(root);
}
