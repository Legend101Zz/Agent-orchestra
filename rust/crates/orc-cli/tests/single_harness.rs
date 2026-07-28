#![allow(unsafe_code)]

//! Issue #12 acceptance coverage for honest single-harness operation.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessConfig, HarnessRegistry, write_harness_registry};
use orc_core::single_harness::SINGLE_HARNESS_MESSAGE;
use serde_json::Value;

const MANDATED_SINGLE_HARNESS_MESSAGE: &str = "One capable harness detected. Parallel cross-harness deliberation is unavailable. Running a sequential plan with self-review.";

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn root(label: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "pio-single-harness-{label}-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture(label: &str) -> (PathBuf, PathBuf) {
    let home = root(label);
    let cwd = home.join("repo");
    let bin = home.join("bin");
    fs::create_dir_all(&cwd).expect("create fixture repo");
    fs::create_dir_all(&bin).expect("create fixture bin");
    git(&cwd, &["init", "-b", "main"]);
    git(
        &cwd,
        &["config", "user.email", "single-harness@example.invalid"],
    );
    git(&cwd, &["config", "user.name", "Single Harness Test"]);
    fs::write(cwd.join("README.md"), "fixture\n").expect("write fixture");
    git(&cwd, &["add", "README.md"]);
    git(&cwd, &["commit", "-m", "fixture"]);

    let worker = bin.join("solo.sh");
    fs::write(
        &worker,
        r#"#!/bin/sh
case "$*" in
  *"Review task"*)
    printf '%s\n' '{"verdicts":[{"check":"artifact exists","verdict":"pass","evidence":"test -f sequential-artifact.txt: exit 0"}]}'
    ;;
  *)
    printf '%s\n' "implemented by solo"
    printf '%s\n' "sequential evidence" > sequential-artifact.txt
    ;;
esac
exit 0
"#,
    )
    .expect("write solo fixture");
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).expect("chmod solo fixture");

    let registry = HarnessRegistry {
        harnesses: BTreeMap::from([(
            "solo".to_owned(),
            HarnessConfig {
                command: worker.to_string_lossy().into_owned(),
                args: Vec::new(),
                resume_args: Vec::new(),
                roles: vec!["brain".to_owned(), "worker".to_owned()],
                adapter: "solo".to_owned(),
                dispatch_args: vec!["--fixture".to_owned()],
                dispatch_uses_stdin: false,
                dispatch_timeout_sec: 30,
                extra: BTreeMap::new(),
            },
        )]),
        default_workers: vec!["solo".to_owned()],
        max_parallel_workers: 1,
        concurrency: BTreeMap::new(),
        app: Default::default(),
        discovered: BTreeMap::new(),
        extra: BTreeMap::new(),
    };
    {
        let _guard = lock();
        // SAFETY: serialized by `lock()`; only the registry writer reads this
        // process-global variable before the guard is released.
        unsafe { std::env::set_var("ORC_HOME", &home) };
        write_harness_registry(&registry).expect("write solo registry");
    }
    (home, cwd)
}

fn pio(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .env("ORC_HOME", home)
        .env("HOME", home.join("empty-home"))
        .output()
        .unwrap_or_else(|error| panic!("run pio {args:?}: {error}"))
}

fn pio_json(home: &Path, args: &[&str]) -> Value {
    let output = pio(home, args);
    assert!(
        output.status.success(),
        "pio {args:?} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("pio {args:?} did not emit JSON: {error}"))
}

#[test]
fn launch_with_one_fixture_harness_prints_the_exact_spec_message() {
    assert_eq!(
        SINGLE_HARNESS_MESSAGE, MANDATED_SINGLE_HARNESS_MESSAGE,
        "the product notice must remain byte-exact against issue #12"
    );
    let (home, cwd) = fixture("launch");
    let output = pio(
        &home,
        &[
            "session",
            "create",
            "--brain",
            "solo",
            "--worker",
            "solo",
            "--cwd",
            cwd.to_str().expect("UTF-8 cwd"),
            "--json",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _: Value = serde_json::from_slice(&output.stdout).expect("session JSON stays parseable");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("{MANDATED_SINGLE_HARNESS_MESSAGE}\n")
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn one_harness_runs_the_isolated_implementer_then_self_reviewer_pipeline() {
    let (home, cwd) = fixture("pipeline");
    let created = pio_json(
        &home,
        &[
            "session",
            "create",
            "--brain",
            "solo",
            "--worker",
            "solo",
            "--cwd",
            cwd.to_str().expect("UTF-8 cwd"),
            "--json",
        ],
    );
    let session = created["id"].as_str().expect("session id").to_owned();

    let planned = pio_json(
        &home,
        &[
            "orch",
            "plan",
            "sequential fixture",
            "--session",
            &session,
            "--isolate",
            "--objective",
            "Create the fixture artifact, then review it.",
            "--allowed",
            "sequential-artifact.txt",
            "--check",
            "artifact exists",
            "--json",
        ],
    );
    let task = planned["tasks"][0]["id"]
        .as_str()
        .expect("task id")
        .to_owned();
    let worktree = planned["tasks"][0]["worktree"]["path"]
        .as_str()
        .expect("isolated worktree")
        .to_owned();
    assert_ne!(Path::new(&worktree), cwd);

    let delegated = pio_json(
        &home,
        &[
            "orch",
            "delegate",
            "solo",
            "--session",
            &session,
            "--task",
            &task,
            "--json",
        ],
    );
    assert_eq!(delegated["dispatches"][0]["harness"], "solo");
    let implemented = pio_json(
        &home,
        &[
            "orch",
            "await",
            &task,
            "--session",
            &session,
            "--timeout",
            "10",
            "--json",
        ],
    );
    assert_eq!(
        implemented["dispatches"][0]["execution_status"],
        "succeeded"
    );
    assert!(
        Path::new(&worktree)
            .join("sequential-artifact.txt")
            .is_file()
    );
    assert!(
        !cwd.join("sequential-artifact.txt").exists(),
        "the implementer must not write into the base checkout"
    );

    let reviewed = pio_json(
        &home,
        &["orch", "review", &task, "--session", &session, "--json"],
    );
    assert_eq!(reviewed["dispatches"][0]["harness"], "solo");
    assert!(
        reviewed["note"]
            .as_str()
            .is_some_and(|note| note.contains("self-review (only one capable harness)"))
    );
    let review_finished = pio_json(
        &home,
        &[
            "orch",
            "await",
            &task,
            "--session",
            &session,
            "--timeout",
            "10",
            "--json",
        ],
    );
    assert_eq!(
        review_finished["dispatches"][0]["execution_status"],
        "succeeded"
    );

    let finished = pio_json(
        &home,
        &["orch", "finish", &task, "--session", &session, "--json"],
    );
    let report = &finished["tasks"][0]["report"];
    assert_eq!(finished["tasks"][0]["status"], "done");
    assert_eq!(report["executor"], "solo");
    assert_eq!(report["reviewer"], "solo");
    assert_eq!(report["review_mode"], "self_review");
    assert_eq!(report["verdicts"][0]["verdict"], "pass");
    assert!(
        report["verdicts"][0]["evidence"]
            .as_str()
            .is_some_and(|evidence| evidence.contains("exit 0"))
    );
    let _ = fs::remove_dir_all(home);
}
