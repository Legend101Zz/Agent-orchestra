#![allow(unsafe_code)]

//! Regression tests for the pipe-buffer deadlock (issue #28).
//!
//! `invoke_harness` used to pipe stdout/stderr, poll `try_wait()`, and only
//! drain the pipes *after* the child exited. Once a worker filled the OS pipe
//! buffer (~64 KB) it blocked in `write()`, so it could never exit, so the
//! parent never drained — a deadlock that surfaced as `DISPATCH TIMEOUT`.
//!
//! Real workers hit this immediately: `pi --mode json` emits ~4 KB of JSON for a
//! one-word answer, and a measured agentic run produced 148 KB. These fixtures
//! flood far past the buffer and must still be delivered, with the persisted
//! record staying bounded.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessConfig, HarnessRegistry, create_session, write_harness_registry};
use orc_core::dispatch::{
    self, DeliveryStatus, DispatchActor, DispatchRequest, MAX_CAPTURED_BYTES, TRUNCATION_MARKER,
};
use orc_core::tasks::{NewTask, TaskActor, TaskStatus, assign_task, start_task};

/// Bytes each fixture writes — comfortably past both the pipe buffer and
/// `MAX_CAPTURED_BYTES`, so a deadlock and a truncation bug are both caught.
const FLOOD_BYTES: usize = 2 * 1024 * 1024;

/// Delivery bound for the flood tests. A deadlocked child never exits, so on the
/// pre-fix code the dispatch burns this whole budget and fails with `Timeout`;
/// a draining implementation finishes in well under a second.
const FLOOD_TIMEOUT_SEC: u64 = 15;

/// The worker's conclusion, emitted as the very LAST thing it writes. A real
/// agentic worker buries its answer at the end, after the session header,
/// reasoning and tool calls — so a capture that keeps only the head throws the
/// answer away and hands the conductor a useless preamble.
const ANSWER: &str = "FLOOD-WORKER-FINAL-ANSWER: zero matches found";

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fresh_home(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "orc-dispatch-flood-{label}-{}-{nonce}",
        std::process::id()
    ))
}

/// A worker that floods one stream with `FLOOD_BYTES` and then exits 0.
///
/// `stream` is the redirection applied to the flood: empty for stdout, `1>&2`
/// for stderr. Both directions matter — draining only stdout leaves the child
/// blocked on a full stderr pipe.
fn flood_harness(home: &Path, stream: &str) -> HarnessConfig {
    let bin = home.join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let script = bin.join("flood-worker.sh");
    let lines = FLOOD_BYTES / 1024;
    fs::write(
        &script,
        format!(
            r#"#!/bin/sh
# Emits {FLOOD_BYTES} bytes, then exits cleanly. awk keeps this fast enough that
# the test measures the drain strategy, not shell loop overhead.
# The LAST line is the answer, mirroring a real agentic worker: session header
# and reasoning first, conclusion last.
awk 'BEGIN {{ line = sprintf("%1023s", "x"); for (i = 0; i < {lines}; i++) print line }}' {stream}
echo "{ANSWER}" {stream}
exit 0
"#
        ),
    )
    .expect("write flood worker script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod flood worker");

    HarnessConfig {
        command: "/bin/sh".to_owned(),
        args: vec![script.to_string_lossy().into_owned()],
        resume_args: Vec::new(),
        roles: vec!["worker".to_owned()],
        adapter: "flood-worker".to_owned(),
        dispatch_args: vec!["--oneshot".to_owned()],
        dispatch_uses_stdin: false,
        dispatch_timeout_sec: FLOOD_TIMEOUT_SEC,
        extra: Default::default(),
    }
}

fn setup(home: &Path, label: &str, stream: &str) -> String {
    fs::create_dir_all(home).expect("create fresh home");
    // SAFETY: every test that mutates ORC_HOME serializes through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", home) };
    let mut registry = HarnessRegistry::default();
    registry
        .harnesses
        .insert("flood-worker".to_owned(), flood_harness(home, stream));
    registry.default_workers = vec!["flood-worker".to_owned()];
    write_harness_registry(&registry).expect("write harness registry");
    let cwd = home.join(format!("cwd-{label}"));
    fs::create_dir_all(&cwd).expect("create session cwd");
    let session =
        create_session("codex", &["flood-worker".to_owned()], &cwd).expect("create flood session");
    session.id
}

fn running_task(session: &str, title: &str) -> orc_core::tasks::Task {
    let task = orc_core::tasks::add_task(
        session,
        TaskActor::Brain,
        NewTask {
            title: title.to_owned(),
            ..NewTask::default()
        },
    )
    .expect("add flood task");
    assign_task(
        session,
        &task.id,
        "flood-worker".to_owned(),
        None,
        TaskActor::Brain,
    )
    .expect("assign flood task");
    let running = start_task(session, &task.id, TaskActor::Brain).expect("start flood task");
    assert_eq!(running.status, TaskStatus::Running.as_str());
    running
}

fn dispatch_flood(session: &str, task: &str) -> orc_core::dispatch::DispatchRecord {
    let delivered = dispatch::dispatch(&DispatchRequest {
        session: session.to_owned(),
        task: task.to_owned(),
        actor: DispatchActor::Brain,
        harness: "flood-worker".to_owned(),
        pane_id: None,
        run: None,
        prompt: "flood the pipe".to_owned(),
        timeout_sec: Some(FLOOD_TIMEOUT_SEC),
    })
    .expect("dispatch must not error");
    assert_eq!(
        delivered.status,
        DeliveryStatus::Confirmed.as_str(),
        "the flood worker must receive its brief"
    );
    let deadline = Instant::now() + Duration::from_secs(FLOOD_TIMEOUT_SEC);
    loop {
        let record = dispatch::read_dispatch(session, &delivered.id).expect("read flood dispatch");
        if record.is_terminal() {
            return record;
        }
        assert!(
            Instant::now() < deadline,
            "flood dispatch never reached terminal state: {record:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_worker_flooding_stdout_is_drained_while_it_runs() {
    let _guard = lock();
    let home = fresh_home("stdout");
    let session = setup(&home, "stdout", "");
    let task = running_task(&session, "flood stdout");

    let started = Instant::now();
    let record = dispatch_flood(&session, &task.id);
    let elapsed = started.elapsed();

    assert_eq!(
        record.status,
        DeliveryStatus::Confirmed.as_str(),
        "a worker writing {FLOOD_BYTES} bytes must still be delivered; \
         got {:?} / {:?}",
        record.failure_kind,
        record.error
    );
    assert_eq!(record.exit_code, Some(0));
    // The pre-fix code burned the entire timeout budget deadlocked. Draining
    // concurrently finishes in well under it; assert with generous headroom so
    // the test measures the deadlock, not machine speed.
    assert!(
        elapsed.as_secs() < FLOOD_TIMEOUT_SEC,
        "dispatch took {elapsed:?} — the pipe was not drained while the child ran"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_worker_flooding_stderr_is_drained_while_it_runs() {
    let _guard = lock();
    let home = fresh_home("stderr");
    let session = setup(&home, "stderr", "1>&2");
    let task = running_task(&session, "flood stderr");

    let started = Instant::now();
    let record = dispatch_flood(&session, &task.id);
    let elapsed = started.elapsed();

    assert_eq!(
        record.status,
        DeliveryStatus::Confirmed.as_str(),
        "draining stdout alone still leaves the child blocked on a full stderr \
         pipe; got {:?} / {:?}",
        record.failure_kind,
        record.error
    );
    assert_eq!(record.exit_code, Some(0));
    assert!(
        elapsed.as_secs() < FLOOD_TIMEOUT_SEC,
        "dispatch took {elapsed:?} — stderr was not drained while the child ran"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_flooded_record_stays_bounded_and_says_it_was_truncated() {
    let _guard = lock();
    let home = fresh_home("bounded");
    let session = setup(&home, "bounded", "");
    let task = running_task(&session, "flood bounded");

    let record = dispatch_flood(&session, &task.id);
    assert_eq!(record.status, DeliveryStatus::Confirmed.as_str());

    // Bounded: the durable record must never grow with worker chatter.
    assert!(
        record.stdout.len() <= MAX_CAPTURED_BYTES + TRUNCATION_MARKER.len() + 64,
        "captured stdout is {} bytes, past the {MAX_CAPTURED_BYTES}-byte cap",
        record.stdout.len()
    );
    // Honest: a truncated capture says so rather than looking like the whole run.
    assert!(
        record.stdout.contains(TRUNCATION_MARKER),
        "a truncated capture must be marked; got tail {:?}",
        &record.stdout[record.stdout.len().saturating_sub(120)..]
    );

    let stored = dispatch::read_dispatch(&session, &record.id).expect("read durable dispatch");
    assert_eq!(stored, record, "the persisted record must round-trip");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_conductor_receives_the_workers_answer_not_just_its_preamble() {
    let _guard = lock();
    let home = fresh_home("answer");
    let session = setup(&home, "answer", "");
    let task = running_task(&session, "flood then answer");

    let record = dispatch_flood(&session, &task.id);
    assert_eq!(record.status, DeliveryStatus::Confirmed.as_str());

    // The whole point of delegating: the brain must get the worker's
    // conclusion. Found live on 2026-07-27 — a head-only capture handed the
    // conductor `pi --mode json`'s session header and thinking, so it redid the
    // work itself rather than using the delegation.
    assert!(
        record.stdout.contains(ANSWER),
        "the worker's final answer was truncated away; capture ends with {:?}",
        &record.stdout[record.stdout.len().saturating_sub(160)..]
    );
    // Still bounded, and still honest about the gap it dropped.
    assert!(
        record.stdout.len() <= MAX_CAPTURED_BYTES + TRUNCATION_MARKER.len() + 64,
        "capture grew to {} bytes",
        record.stdout.len()
    );
    assert!(record.stdout.contains(TRUNCATION_MARKER));
    let _ = fs::remove_dir_all(&home);
}
