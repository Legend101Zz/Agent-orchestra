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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use orc_core::bench::{
    HarnessConfig, HarnessRegistry, create_session, load_harness_registry, write_harness_registry,
};
use orc_core::contract::TaskContract;
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

fn await_terminal(session: &str, delivered: DispatchRecord) -> DispatchRecord {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let record = dispatch::read_dispatch(session, &delivered.id).expect("read dispatch");
        if record.is_terminal() {
            return record;
        }
        assert!(
            Instant::now() < deadline,
            "dispatch never reached terminal state: {record:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
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
    let record = await_terminal(&session_id, record);
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
    // Two events, in order, meaning different things (issue #49). This used to
    // assert that `delivery_confirmed` was the *last* word the board had about
    // a finished dispatch — which was true, and was the bug: the board's final
    // record of every delegation said "the worker process started".
    // `await_terminal` above is what makes this deterministic rather than a
    // race: the supervisor appends the completion event before it writes the
    // terminal dispatch record.
    let words = linked
        .history
        .iter()
        .map(|history| history.action.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        words
            .iter()
            .rev()
            .take(2)
            .rev()
            .copied()
            .collect::<Vec<_>>(),
        vec!["delivery_confirmed", "execution_succeeded"],
        "the brief was taken, and then the worker answered: {words:?}"
    );
    let completion = linked.history.last().expect("completion event");
    assert!(
        completion
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("answered") && detail.contains("exit 0")),
        "and the completion event says what happened: {completion:?}"
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
    let (_cwd, registry) = setup_session_with_harness(&home, "git");
    let worker_script = registry.harnesses["fake-worker"]
        .args
        .first()
        .map(PathBuf::from)
        .expect("fake worker script");
    fs::write(
        &worker_script,
        "#!/bin/sh\npwd > worker-cwd.txt\necho reviewable-output\n",
    )
    .expect("write cwd worker");
    fs::set_permissions(&worker_script, fs::Permissions::from_mode(0o755))
        .expect("chmod cwd worker");

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
            isolate: false,
            contract: Some(TaskContract {
                objective: "Write only inside the isolated task tree.".to_owned(),
                allowed_paths: vec!["worker-cwd.txt".to_owned()],
                forbidden: vec!["do not touch the main checkout".to_owned()],
                acceptance_checks: vec!["main checkout stays untouched".to_owned()],
                ..TaskContract::default()
            }),
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
    let record = await_terminal(&session_id, record);
    assert_eq!(record.status, DeliveryStatus::Confirmed.as_str());
    assert_eq!(record.run.as_deref(), Some("W-git"));
    assert!(record.command_line.contains("show me diff"));
    assert!(worktree_path.is_dir(), "worktree must remain intact");
    assert_eq!(
        record.cwd.as_deref(),
        Some(worktree_path.to_string_lossy().as_ref())
    );
    assert!(
        worktree_path.join("worker-cwd.txt").is_file(),
        "worker output must land in its owned worktree"
    );
    assert!(
        !repo.join("worker-cwd.txt").exists(),
        "main checkout must stay untouched"
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn contracted_dispatch_refuses_to_fall_back_to_the_main_session_cwd() {
    let _guard = lock();
    let home = fresh_home("contract-no-worktree");
    let (cwd, _registry) = setup_session_with_harness(&home, "contract-no-worktree");
    let session_path = fs::read_dir(home.join("sessions"))
        .expect("sessions")
        .filter_map(Result::ok)
        .next()
        .expect("session")
        .path()
        .join("session.json");
    let session: serde_json::Value =
        serde_json::from_slice(&fs::read(session_path).expect("session bytes"))
            .expect("session json");
    let session_id = session["id"].as_str().expect("session id");
    let task = orc_core::tasks::add_task(
        session_id,
        TaskActor::Brain,
        NewTask {
            title: "must isolate".to_owned(),
            contract: Some(TaskContract {
                objective: "Never execute in the shared cwd.".to_owned(),
                allowed_paths: vec!["artifact.txt".to_owned()],
                forbidden: vec!["do not use the main tree".to_owned()],
                acceptance_checks: vec!["shared cwd remains untouched".to_owned()],
                ..TaskContract::default()
            }),
            ..NewTask::default()
        },
    )
    .expect("add contracted task");
    assert_eq!(task.worktree.as_ref().unwrap().state, "unavailable");
    assign_task(
        session_id,
        &task.id,
        "fake-worker".to_owned(),
        None,
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(session_id, &task.id, TaskActor::Brain).expect("start");
    let error = dispatch::dispatch(&DispatchRequest {
        session: session_id.to_owned(),
        task: task.id,
        actor: DispatchActor::Brain,
        harness: "fake-worker".to_owned(),
        pane_id: None,
        run: None,
        prompt: "must not run".to_owned(),
        timeout_sec: Some(30),
    })
    .expect_err("contracted task without isolation must be refused");
    assert!(
        error.to_string().contains("ISOLATION REQUIRED"),
        "unexpected error: {error}"
    );
    assert!(
        fs::read_dir(&cwd).expect("shared cwd").next().is_none(),
        "worker must not write into the shared cwd"
    );
    let _ = fs::remove_dir_all(home);
}

// --- Issue #51 defect 2: a killed supervisor tells the board -----------------

/// A worker that outlives its own supervisor, so the supervisor can be killed
/// while the dispatch is genuinely still `running`.
fn slow_worker(home: &Path) {
    let script = home.join("bin").join("fake-worker.sh");
    fs::write(&script, "#!/bin/sh\nsleep 30\n").expect("write slow worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod slow worker");
}

/// SIGKILL — uncatchable, so nothing cooperative runs in that process
/// afterwards. This is the OOM-kill and the reboot, not a polite shutdown.
fn sigkill(pid: u32) {
    let status = Command::new("/bin/kill")
        .args(["-9", &pid.to_string()])
        .status()
        .expect("run kill");
    assert!(status.success(), "kill -9 {pid} failed");
}

/// Dispatch through the real `pio` binary rather than in-process.
///
/// The supervisor must be a *grandchild*: `dispatch_supervisor::launch` spawns
/// it from whatever process calls `dispatch`, and a SIGKILLed child of a
/// still-running parent is a zombie, which `pid_alive`'s `kill(pid, 0)` reads
/// as alive — correctly, since the process entry does still exist. In
/// production the dispatching `pio` exits immediately and the supervisor is
/// reparented and reaped, so killing it really does make it disappear. Going
/// through the binary reproduces that instead of fighting it.
fn pio_dispatch(home: &Path, session: &str, task: &str, run: &str) -> DispatchRecord {
    let output = Command::new(pio_binary())
        .args([
            "dispatch",
            "send",
            task,
            "fake-worker",
            "sleep for a while",
            "--session",
            session,
            "--run",
            run,
            "--timeout",
            "60",
            "--json",
        ])
        .env("ORC_HOME", home)
        .env("HOME", home.join("empty-home"))
        .output()
        .expect("run pio dispatch send");
    assert!(
        output.status.success(),
        "pio dispatch send failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("pio emitted a dispatch record")
}

fn wait_for_death(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run kill -0")
        .success()
    {
        assert!(Instant::now() < deadline, "pid {pid} never died");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_killed_supervisor_leaves_a_durable_board_event_of_its_own() {
    // Issue #51 defect 2, acceptance check 4. `reconcile_record` marked the
    // dispatch `orphaned` and appended nothing to the board, so the board's
    // last word stayed `delivery_confirmed` — "the worker took the brief and
    // started" — and the task read as running for ever. #50 made that
    // conspicuous rather than causing it: once a completion vocabulary exists,
    // the *absence* of a completion event means something.
    //
    // The supervisor here is a real detached process and it is really
    // SIGKILLed. Reconciliation is then performed by an ordinary reader, which
    // is the whole design question this settles.
    let _guard = lock();
    let home = fresh_home("orphan-board");
    let (_cwd, _registry) = setup_session_with_harness(&home, "orphan-board");
    slow_worker(&home);
    let cwd = home.join("cwd-orphan");
    fs::create_dir_all(&cwd).expect("create session cwd");
    let session_id = create_session("codex", &["fake-worker".to_owned()], &cwd)
        .expect("create orphan session")
        .id;
    let task = running_task(&home, &session_id, "orphan me");

    let record = pio_dispatch(&home, &session_id, &task.id, "W-orphan");
    assert_eq!(record.status, DeliveryStatus::Confirmed.as_str());
    let supervisor = record.supervisor_pid.expect("a detached supervisor");

    // `mark_started` writes the dispatch record before it appends
    // `delivery_confirmed`, so `pio dispatch send` can return between the two.
    // Killing inside that window would test a supervisor that died before it
    // delivered, which is a different thing.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !orc_core::tasks::read_task(&session_id, &task.id)
        .expect("read the board")
        .history
        .iter()
        .any(|entry| entry.action == "delivery_confirmed")
    {
        assert!(
            Instant::now() < deadline,
            "delivery never reached the board"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    sigkill(supervisor);
    wait_for_death(supervisor);

    // An ordinary reader notices. This is the only thing that happens between
    // the kill and the assertions.
    let listed = dispatch::list_dispatches(&session_id).expect("list dispatches");
    let reconciled = listed
        .iter()
        .find(|candidate| candidate.id == record.id)
        .expect("the record survives the listing");
    assert_eq!(
        reconciled.execution_status.as_deref(),
        Some("orphaned"),
        "the dispatch record already said this before #51"
    );
    assert_eq!(reconciled.failure_kind.as_deref(), Some("supervisor_lost"));
    assert!(
        reconciled.warnings.is_empty(),
        "the board took the event: {:?}",
        reconciled.warnings
    );

    // Read the board *after* the reconciling call returned. Reconciliation
    // happens inside the listing, so a `Task` snapshot taken before it — the
    // shape `orch::status` returns — predates its own append.
    let board = orc_core::tasks::read_task(&session_id, &task.id).expect("read the board");
    let words = board
        .history
        .iter()
        .map(|entry| entry.action.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        words.last(),
        Some(&"execution_orphaned"),
        "the board's last word must be the orphaning, not `delivery_confirmed`: {words:?}"
    );
    // Distinguishable from "still running": there is a completion event at all.
    assert!(
        words.contains(&"delivery_confirmed"),
        "the brief really was delivered first: {words:?}"
    );
    // Distinguishable from "the worker failed": a different word, and neither
    // of the two words that mean the worker itself reached a verdict.
    assert!(
        !words.contains(&"execution_failed") && !words.contains(&"execution_succeeded"),
        "an orphaned dispatch is not a failed one and not a successful one: {words:?}"
    );
    assert!(
        board
            .history
            .last()
            .and_then(|entry| entry.detail.as_deref())
            .is_some_and(|detail| detail.contains(&record.id) && detail.contains("supervisor")),
        "and it names the dispatch it is about: {:?}",
        board.history.last()
    );

    let _ = fs::remove_dir_all(home);
}

#[test]
fn concurrent_listers_write_one_orphan_event_and_never_wedge_the_board() {
    // Issue #51 acceptance check 5. Reconciliation now writes the task board,
    // and it runs in any process that lists dispatches — so two of them racing
    // must neither corrupt the board nor deadlock, and must not double-animate
    // by appending the same event twice.
    //
    // Four real `pio` processes, not threads: the board lock is a lock *file*,
    // so processes are the honest unit even though the mechanism makes threads
    // equivalent. The deterministic half of the guarantee — that the dedupe
    // key is the dispatch and not the word — is pinned separately below,
    // because a spawned race is corroboration and not proof: nothing forces
    // the four to overlap.
    let _guard = lock();
    let home = fresh_home("orphan-race");
    let (_cwd, _registry) = setup_session_with_harness(&home, "orphan-race");
    slow_worker(&home);
    let cwd = home.join("cwd-race");
    fs::create_dir_all(&cwd).expect("create session cwd");
    let session_id = create_session("codex", &["fake-worker".to_owned()], &cwd)
        .expect("create race session")
        .id;

    let mut tasks = Vec::new();
    for index in 0..2 {
        let task = running_task(&home, &session_id, &format!("race me {index}"));
        let record = pio_dispatch(&home, &session_id, &task.id, &format!("W-race-{index}"));
        let supervisor = record.supervisor_pid.expect("a detached supervisor");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !orc_core::tasks::read_task(&session_id, &task.id)
            .expect("read the board")
            .history
            .iter()
            .any(|entry| entry.action == "delivery_confirmed")
        {
            assert!(
                Instant::now() < deadline,
                "delivery never reached the board"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        sigkill(supervisor);
        wait_for_death(supervisor);
        tasks.push((task.id, record.id));
    }

    let pio = pio_binary();
    let started = Instant::now();
    let children = (0..4)
        .map(|_| {
            Command::new(&pio)
                .args(["dispatch", "list", "--session", &session_id, "--json"])
                .env("ORC_HOME", &home)
                .env("HOME", home.join("empty-home"))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn pio dispatch list")
        })
        .collect::<Vec<_>>();
    for child in children {
        let output = child.wait_with_output().expect("wait for pio");
        assert!(
            output.status.success(),
            "pio dispatch list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let records: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("pio emitted JSON");
        // Nothing was silently dropped: `list_dispatches` swallows a failing
        // reconcile with `.ok()`, so a propagated board error would make a
        // record vanish from every listing rather than merely go unannounced.
        assert_eq!(
            records.as_array().map(Vec::len),
            Some(2),
            "both records survive every listing: {records}"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "four concurrent listers must not wedge: took {:?}",
        started.elapsed()
    );

    for (task_id, dispatch_id) in &tasks {
        let board = orc_core::tasks::read_task(&session_id, task_id).expect("read the board");
        let orphans = board
            .history
            .iter()
            .filter(|entry| entry.action == "execution_orphaned")
            .count();
        assert_eq!(
            orphans,
            1,
            "exactly one orphan event for dispatch {dispatch_id}, however many \
             processes noticed: {:?}",
            board
                .history
                .iter()
                .map(|entry| entry.action.clone())
                .collect::<Vec<_>>()
        );
        // And the board is still parseable as a whole — a torn write would
        // take the task out of `list_tasks` entirely.
        assert!(
            orc_core::tasks::list_tasks(&session_id)
                .expect("list tasks")
                .iter()
                .any(|task| task.id == *task_id),
            "no torn task file"
        );
    }

    let _ = fs::remove_dir_all(home);
}

/// The `pio` binary this test run built, found the same way the production
/// supervisor launcher finds it: a sibling of `target/<profile>/deps`.
fn pio_binary() -> PathBuf {
    let current = std::env::current_exe().expect("current exe");
    let candidate = current
        .parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("pio"))
        .expect("target dir");
    assert!(
        candidate.is_file(),
        "pio must be built for this test (it is, under `cargo test --workspace`): {}",
        candidate.display()
    );
    candidate
}

// --- Issue #49 phase 2: partial output survives a killed supervisor ---------

/// A worker that says something, then outlives its supervisor.
///
/// `slow_worker` above is silent by design — #51 only needed a live worker to
/// kill. Phase 2 needs one that has genuinely produced output before the kill,
/// because the claim under test is about the bytes, not about the liveness.
fn talking_slow_worker(home: &Path) {
    let script = home.join("bin").join("fake-worker.sh");
    fs::write(
        &script,
        "#!/bin/sh\necho 'PARTIAL WORK BEFORE THE KILL'\necho '{\"verdict\":\"ACCEPT\"}'\nsleep 30\n",
    )
    .expect("write talking slow worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .expect("chmod talking slow worker");
}

#[test]
fn a_killed_supervisor_leaves_its_partial_output_on_disk() {
    // Issue #49 phase 2. This is the durability win stated as a test: before
    // this branch a SIGKILLed supervisor destroyed everything the worker had
    // produced, because `Captured` was in-memory only and `reconcile_record`
    // rewrites the record with `stdout` untouched — i.e. empty.
    //
    // It also pins the hard rule that came out of the design phase: partial
    // text is *never* folded into `record.stdout`. `report::parse_review_verdicts`
    // brace-scans that field, so an orphaned reviewer's half-finished thinking
    // must not be parseable as a verdict. The worker below emits a complete,
    // well-formed verdict object on purpose.
    let _guard = lock();
    let home = fresh_home("orphan-progress");
    let (_cwd, _registry) = setup_session_with_harness(&home, "orphan-progress");
    talking_slow_worker(&home);
    let cwd = home.join("cwd-orphan-progress");
    fs::create_dir_all(&cwd).expect("create session cwd");
    let session_id = create_session("codex", &["fake-worker".to_owned()], &cwd)
        .expect("create orphan session")
        .id;
    let task = running_task(&home, &session_id, "orphan my partial output");

    let record = pio_dispatch(&home, &session_id, &task.id, "W-orphan-progress");
    let supervisor = record.supervisor_pid.expect("a detached supervisor");
    let paths = dispatch::progress_paths(&session_id, &record.id, 1);

    // Wait until the worker's bytes are actually durable, then kill. Killing
    // before they land would test something else entirely.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let text = fs::read_to_string(&paths.stdout_log).unwrap_or_default();
        if text.contains("PARTIAL WORK BEFORE THE KILL") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the worker's output never became durable, so there is nothing to orphan"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    sigkill(supervisor);
    wait_for_death(supervisor);

    // An ordinary reader reconciles, exactly as `pio dispatch list` would.
    let listed = dispatch::list_dispatches(&session_id).expect("list dispatches");
    let reconciled = listed
        .iter()
        .find(|candidate| candidate.id == record.id)
        .expect("the record survives the listing");
    assert_eq!(reconciled.execution_status.as_deref(), Some("orphaned"));

    // The bytes survived the death of the process that wrote them.
    let survived = fs::read_to_string(&paths.stdout_log).expect("the log outlives its supervisor");
    assert!(
        survived.contains("PARTIAL WORK BEFORE THE KILL"),
        "a killed supervisor must leave what its worker had already said; got {survived:?}"
    );

    // …and the record still points at it, having been rewritten by reconcile.
    let progress = reconciled
        .progress
        .as_ref()
        .expect("reconciliation must not drop the progress pointer");
    assert_eq!(progress.attempt, 1);
    assert!(progress.stdout_log.ends_with(".a1.out.log"));

    // The hard rule: none of it reached `stdout`, so nothing can read a
    // half-finished worker's output as an answer.
    assert!(
        reconciled.stdout.is_empty(),
        "mid-flight output must never be folded into `stdout`: {:?}",
        reconciled.stdout
    );
    assert!(
        !reconciled.stdout.contains("ACCEPT"),
        "an orphaned worker's verdict-shaped output must not be parseable as a verdict"
    );
    let _ = fs::remove_dir_all(&home);
}
