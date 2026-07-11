#![allow(unsafe_code)]

//! Bounded brain-to-worker command path integration tests.
//!
//! Each test creates an isolated ORC_HOME, configures a fake harness, and
//! exercises the public `orc_core::dispatch` surface. The fake harness is a
//! shell script under a temporary directory so the tests never spawn a
//! real model provider.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{
    HarnessConfig, HarnessRegistry, create_session, load_harness_registry, write_harness_registry,
};
use orc_core::dispatch::{self, DeliveryStatus, DispatchActor, DispatchRecord, DispatchRequest};
use orc_core::registry::atomic_write_json;
use orc_core::tasks::{NewTask, TaskActor, TaskStatus, assign_task, start_task};
use serde_json::json;

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
        "orc-dispatch-core-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run git {args:?}: {error}"));
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture_harness(home: &Path) -> (HarnessConfig, HarnessConfig) {
    let bin = home.join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let worker_script = bin.join("fake-worker.sh");
    fs::write(
        &worker_script,
        r#"#!/bin/sh
# invoked as: sh <script> [--oneshot] <prompt>
echo "fake-worker-stdout ${@: -1}"
echo "fake-worker-stderr" 1>&2
exit 0
"#,
    )
    .expect("write fake worker script");
    fs::set_permissions(&worker_script, fs::Permissions::from_mode(0o755))
        .expect("chmod fake worker script");

    let missing_script = bin.join("missing.sh");
    let mut missing_config = HarnessConfig {
        command: "/path/that/does/not/exist/fake-worker".to_owned(),
        args: Vec::new(),
        resume_args: Vec::new(),
        roles: vec!["worker".to_owned()],
        adapter: "missing-fixture".to_owned(),
        dispatch_args: vec!["--oneshot".to_owned()],
        dispatch_uses_stdin: false,
        dispatch_timeout_sec: 30,
        extra: Default::default(),
    };
    missing_config.extra.insert(
        "fixture_script".to_owned(),
        json!(missing_script.to_string_lossy()),
    );

    let worker_config = HarnessConfig {
        command: "/bin/sh".to_owned(),
        args: vec![worker_script.to_string_lossy().into_owned()],
        resume_args: Vec::new(),
        roles: vec!["worker".to_owned()],
        adapter: "fake-worker".to_owned(),
        dispatch_args: vec!["--oneshot".to_owned()],
        dispatch_uses_stdin: false,
        dispatch_timeout_sec: 30,
        extra: Default::default(),
    };
    (worker_config, missing_config)
}

fn setup_session_with_harness(home: &Path, label: &str) -> (PathBuf, HarnessRegistry) {
    fs::create_dir_all(home).expect("create fresh home");
    // SAFETY: tests that mutate ORC_HOME serialize through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", home) };
    let (worker, missing) = fixture_harness(home);
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("fake-worker".to_owned(), worker);
    registry
        .harnesses
        .insert("missing-fixture".to_owned(), missing);
    registry.default_workers = vec!["fake-worker".to_owned()];
    write_harness_registry(&registry).expect("write harness registry");
    let cwd = home.join(format!("cwd-{label}"));
    fs::create_dir_all(&cwd).expect("create session cwd");
    let session = create_session("codex", &["fake-worker".to_owned()], &cwd)
        .expect("create dispatch session");
    (cwd, registry_for(&session))
}

fn registry_for(_session: &orc_core::bench::BenchSession) -> HarnessRegistry {
    load_harness_registry().expect("reload harness registry")
}

fn running_task(home: &Path, session: &str, title: &str) -> orc_core::tasks::Task {
    let task = orc_core::tasks::add_task(
        session,
        TaskActor::Brain,
        NewTask {
            title: title.to_owned(),
            ..NewTask::default()
        },
    )
    .expect("add dispatch task");
    assign_task(
        session,
        &task.id,
        "fake-worker".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign dispatch task");
    let running = start_task(session, &task.id, TaskActor::Brain).expect("start dispatch task");
    assert_eq!(running.status, TaskStatus::Running.as_str());
    let _ = home; // silence unused warning while reserving for future fixtures
    running
}

#[test]
fn dispatch_through_a_fake_worker_is_confirmed_records_actor_and_pane_linkage() {
    let _guard = lock();
    let home = fresh_home("confirmed");
    let (_cwd, _registry) = setup_session_with_harness(&home, "confirmed");
    let session_id = std::fs::read_dir(home.join("sessions"))
        .expect("sessions dir")
        .filter_map(Result::ok)
        .next()
        .expect("one session")
        .path()
        .join("session.json");
    let session_id = std::fs::read_to_string(&session_id).expect("read session json");
    let session_id: serde_json::Value =
        serde_json::from_str(&session_id).expect("parse session json");
    let session_id = session_id["id"].as_str().expect("session id").to_owned();
    let task = running_task(&home, &session_id, "happy path dispatch");

    let record = dispatch::dispatch(&DispatchRequest {
        session: session_id.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "fake-worker".to_owned(),
        pane_id: Some(format!("{session_id}-worker-1")),
        run: Some("W-1".to_owned()),
        prompt: "summarize diff".to_owned(),
        timeout_sec: Some(30),
    })
    .expect("dispatch must succeed");
    assert_eq!(record.status, DeliveryStatus::Confirmed.as_str());
    assert_eq!(record.actor, "brain");
    assert_eq!(record.harness, "fake-worker");
    assert_eq!(record.task, task.id);
    assert_eq!(
        record.pane_id.as_deref(),
        Some(format!("{session_id}-worker-1").as_str())
    );
    assert_eq!(record.run.as_deref(), Some("W-1"));
    assert_eq!(record.exit_code, Some(0));
    assert!(record.stdout.contains("fake-worker-stdout summarize diff"));
    assert!(record.stderr.contains("fake-worker-stderr"));
    assert!(record.command_line.contains("/bin/sh"));
    assert!(record.command_line.contains("fake-worker"));
    assert!(record.command_line.contains("--oneshot"));
    assert!(record.command_line.contains("summarize diff"));
    assert!(record.failure_kind.is_none());
    assert!(record.error.is_none());

    let stored = dispatch::read_dispatch(&session_id, &record.id).expect("read durable dispatch");
    assert_eq!(stored, record);
    assert!(record.is_confirmed());
    let linked = orc_core::tasks::read_task(&session_id, &task.id).expect("read linked task");
    assert_eq!(linked.assignee_run, record.pane_id);
    assert_eq!(
        linked.history.last().map(|history| history.action.as_str()),
        Some("delivery_confirmed")
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn dispatch_through_a_missing_executable_is_failed_with_a_clear_error() {
    let _guard = lock();
    let home = fresh_home("missing-failed");
    let (_cwd, _registry) = setup_session_with_harness(&home, "missing");
    let session_id = std::fs::read_dir(home.join("sessions"))
        .expect("sessions dir")
        .filter_map(Result::ok)
        .next()
        .expect("one session")
        .path()
        .join("session.json");
    let session_id = std::fs::read_to_string(&session_id).expect("read session json");
    let session_id: serde_json::Value =
        serde_json::from_str(&session_id).expect("parse session json");
    let session_id = session_id["id"].as_str().expect("session id").to_owned();
    let task = running_task(&home, &session_id, "missing executable dispatch");

    let record = dispatch::dispatch(&DispatchRequest {
        session: session_id.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "missing-fixture".to_owned(),
        pane_id: None,
        run: None,
        prompt: "summarize diff".to_owned(),
        timeout_sec: Some(15),
    })
    .expect("dispatch must persist even on failure");
    assert_eq!(record.status, DeliveryStatus::Failed.as_str());
    assert_eq!(record.failure_kind.as_deref(), Some("missing_executable"));
    let error = record.error.as_deref().unwrap_or_default();
    assert!(
        error.contains("MISSING EXECUTABLE"),
        "missing-executable error must be explicit; got {error:?}"
    );
    assert!(!record.is_confirmed());

    let listed = dispatch::list_dispatches(&session_id).expect("list dispatches");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, record.id);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn dispatch_without_a_non_interactive_capability_is_failed_with_capability_unavailable() {
    let _guard = lock();
    let home = fresh_home("capability");
    let (_cwd, mut registry) = setup_session_with_harness(&home, "capability");
    let session_id = std::fs::read_dir(home.join("sessions"))
        .expect("sessions dir")
        .filter_map(Result::ok)
        .next()
        .expect("one session")
        .path()
        .join("session.json");
    let session_id = std::fs::read_to_string(&session_id).expect("read session json");
    let session_id: serde_json::Value =
        serde_json::from_str(&session_id).expect("parse session json");
    let session_id = session_id["id"].as_str().expect("session id").to_owned();
    let task = running_task(&home, &session_id, "missing capability dispatch");

    let mut pi_config = registry
        .harnesses
        .get("pi-m3")
        .cloned()
        .expect("pi-m3 harness");
    pi_config.dispatch_args.clear();
    registry
        .harnesses
        .insert("no-cap-fixture".to_owned(), pi_config);
    write_harness_registry(&registry).expect("persist registry");

    let record = dispatch::dispatch(&DispatchRequest {
        session: session_id.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "no-cap-fixture".to_owned(),
        pane_id: None,
        run: None,
        prompt: "summarize diff".to_owned(),
        timeout_sec: Some(15),
    })
    .expect("dispatch must persist even on capability error");
    assert_eq!(record.status, DeliveryStatus::Failed.as_str());
    assert_eq!(
        record.failure_kind.as_deref(),
        Some("capability_unavailable")
    );
    let error = record.error.as_deref().unwrap_or_default();
    assert!(
        error.contains("CAPABILITY UNAVAILABLE"),
        "missing-capability error must be explicit; got {error:?}"
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn dispatch_history_lists_records_with_newest_first_and_preserves_actor_linkage() {
    let _guard = lock();
    let home = fresh_home("history");
    let (_cwd, _registry) = setup_session_with_harness(&home, "history");
    let session_id = std::fs::read_dir(home.join("sessions"))
        .expect("sessions dir")
        .filter_map(Result::ok)
        .next()
        .expect("one session")
        .path()
        .join("session.json");
    let session_id = std::fs::read_to_string(&session_id).expect("read session json");
    let session_id: serde_json::Value =
        serde_json::from_str(&session_id).expect("parse session json");
    let session_id = session_id["id"].as_str().expect("session id").to_owned();
    let first_task = running_task(&home, &session_id, "first history dispatch");
    let second_task = running_task(&home, &session_id, "second history dispatch");

    let record_first = dispatch::dispatch(&DispatchRequest {
        session: session_id.clone(),
        task: first_task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "fake-worker".to_owned(),
        pane_id: Some("pane-a".to_owned()),
        run: Some("W-1".to_owned()),
        prompt: "first".to_owned(),
        timeout_sec: Some(30),
    })
    .expect("first dispatch");
    let record_second = dispatch::dispatch(&DispatchRequest {
        session: session_id.clone(),
        task: second_task.id.clone(),
        actor: DispatchActor::Human,
        harness: "fake-worker".to_owned(),
        pane_id: Some("pane-b".to_owned()),
        run: Some("W-2".to_owned()),
        prompt: "second".to_owned(),
        timeout_sec: Some(30),
    })
    .expect("second dispatch");

    let history = dispatch::list_dispatches(&session_id).expect("list history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id, record_second.id);
    assert_eq!(history[1].id, record_first.id);
    assert_eq!(history[0].actor, "human");
    assert_eq!(history[1].actor, "brain");
    assert_eq!(history[0].pane_id.as_deref(), Some("pane-b"));
    assert_eq!(history[0].run.as_deref(), Some("W-2"));

    let raw_path = home
        .join("dispatches")
        .join(&session_id)
        .join(format!("{}.json", record_first.id));
    let raw: DispatchRecord =
        serde_json::from_str(&fs::read_to_string(&raw_path).expect("read raw")).expect("parse raw");
    assert_eq!(raw.actor, "brain");
    assert!(raw.command_line.contains("first"));
    atomic_write_json(
        &home
            .join("dispatches")
            .join(&session_id)
            .join("corrupt.json"),
        &json!("not a dispatch record"),
    )
    .expect("write corrupt sibling");
    let listed = dispatch::list_dispatches(&session_id).expect("list tolerates corrupt");
    assert_eq!(listed.len(), 2);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn dispatch_prompt_above_the_bounded_limit_is_rejected_before_invocation() {
    let _guard = lock();
    let home = fresh_home("bound");
    let (_cwd, _registry) = setup_session_with_harness(&home, "bound");
    let session_id = std::fs::read_dir(home.join("sessions"))
        .expect("sessions dir")
        .filter_map(Result::ok)
        .next()
        .expect("one session")
        .path()
        .join("session.json");
    let session_id = std::fs::read_to_string(&session_id).expect("read session json");
    let session_id: serde_json::Value =
        serde_json::from_str(&session_id).expect("parse session json");
    let session_id = session_id["id"].as_str().expect("session id").to_owned();
    let task = running_task(&home, &session_id, "bounded prompt dispatch");

    let oversize = "x".repeat(dispatch::MAX_CAPTURED_BYTES + 8);
    let error = dispatch::dispatch(&DispatchRequest {
        session: session_id.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "fake-worker".to_owned(),
        pane_id: None,
        run: None,
        prompt: oversize,
        timeout_sec: Some(15),
    })
    .expect_err("oversized prompt must be refused");
    assert!(
        error.to_string().contains("refactor into a smaller prompt"),
        "unexpected error: {error}"
    );
    assert!(
        dispatch::list_dispatches(&session_id)
            .expect("list")
            .is_empty()
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn dispatch_from_a_temporary_git_worktree_runs_with_assigned_runner_and_succeeds() {
    let _guard = lock();
    let home = fresh_home("git");
    let repo = home.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "dispatch@example.invalid"]);
    git(&repo, &["config", "user.name", "Dispatch Test"]);
    fs::write(repo.join("story.txt"), "one\n").expect("write initial");
    git(&repo, &["add", "story.txt"]);
    git(&repo, &["commit", "-m", "initial"]);
    let (_cwd, _registry) = setup_session_with_harness(&home, "git");

    // SAFETY: this test serializes the process-wide registry root.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let session = create_session("codex", &["fake-worker".to_owned()], &repo)
        .expect("create session in repo");
    let session_id = session.id.clone();

    let task = orc_core::tasks::add_task(
        &session_id,
        TaskActor::Brain,
        NewTask {
            title: "git worktree dispatch".to_owned(),
            isolate: true,
            ..NewTask::default()
        },
    )
    .expect("add isolated task");
    let worktree_path = PathBuf::from(
        task.worktree
            .as_ref()
            .and_then(|worktree| worktree.path.clone())
            .expect("worktree path"),
    );
    assert!(worktree_path.is_dir());

    assign_task(
        &session_id,
        &task.id,
        "fake-worker".to_owned(),
        Some("W-git".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign isolated task");
    start_task(&session_id, &task.id, TaskActor::Brain).expect("start isolated task");

    let record = dispatch::dispatch(&DispatchRequest {
        session: session_id.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "fake-worker".to_owned(),
        pane_id: Some("pane-git".to_owned()),
        run: Some("W-git".to_owned()),
        prompt: "show me diff".to_owned(),
        timeout_sec: Some(30),
    })
    .expect("dispatch must succeed from a git worktree");
    assert_eq!(record.status, DeliveryStatus::Confirmed.as_str());
    assert_eq!(record.run.as_deref(), Some("W-git"));
    assert!(record.command_line.contains("show me diff"));
    assert!(worktree_path.is_dir(), "worktree must remain intact");
    let _ = fs::remove_dir_all(home);
}
