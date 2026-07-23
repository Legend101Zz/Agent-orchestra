#![allow(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessRegistry, create_session, write_harness_registry};
use serde_json::Value;

/// Serialize the process-global `ORC_HOME` writes these tests share, so
/// in-process `create_session` calls never race across parallel tests.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fresh_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("orc-task-cli-{}-{nonce}", std::process::id()))
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git").arg("-C").arg(cwd).args(args).output();
    let output = output.unwrap_or_else(|error| panic!("run git {args:?}: {error}"));
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn orc(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .env("ORC_HOME", home)
        .output()
        .unwrap_or_else(|error| panic!("run orc {args:?}: {error}"))
}

#[test]
fn task_diff_and_merge_are_reachable_from_the_cli_and_json_is_meaningful() {
    let _guard = lock();
    let root = fresh_root();
    let home = root.join("home");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap_or_else(|error| panic!("create repo: {error}"));
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "task-cli@example.invalid"]);
    git(&repo, &["config", "user.name", "Task CLI"]);
    fs::write(repo.join("story.txt"), "one\n").unwrap_or_else(|error| panic!("write: {error}"));
    git(&repo, &["add", "story.txt"]);
    git(&repo, &["commit", "-m", "initial"]);

    // SAFETY: this test uses its own temporary registry root.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    write_harness_registry(&HarnessRegistry::default())
        .unwrap_or_else(|error| panic!("write harnesses: {error}"));
    let session = create_session("codex", &["pi-m3".to_owned()], &repo)
        .unwrap_or_else(|error| panic!("create session: {error}"));

    let added = orc(
        &home,
        &[
            "task",
            "add",
            "CLI worktree",
            "--isolate",
            "--session",
            &session.id,
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
    let task: Value =
        serde_json::from_slice(&added.stdout).unwrap_or_else(|error| panic!("task json: {error}"));
    let id = task["id"].as_str().unwrap_or_default().to_owned();
    let worktree = task["worktree"]["path"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!id.is_empty());
    assert!(!worktree.is_empty());
    fs::write(Path::new(&worktree).join("story.txt"), "one\ntwo\n")
        .unwrap_or_else(|error| panic!("edit worktree: {error}"));

    let diff = orc(
        &home,
        &["task", "diff", &id, "--session", &session.id, "--json"],
    );
    assert!(
        diff.status.success(),
        "{}",
        String::from_utf8_lossy(&diff.stderr)
    );
    let diff: Value =
        serde_json::from_slice(&diff.stdout).unwrap_or_else(|error| panic!("diff json: {error}"));
    assert_eq!(diff["insertions"], 1);
    assert_eq!(diff["files"], 1);

    git(Path::new(&worktree), &["add", "story.txt"]);
    git(Path::new(&worktree), &["commit", "-m", "task change"]);
    for command in [
        vec!["task", "assign", &id, "pi-m3", "--session", &session.id],
        vec!["task", "start", &id, "--session", &session.id],
        vec!["task", "review", &id, "--session", &session.id],
    ] {
        let output = orc(&home, &command);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let merged = orc(
        &home,
        &[
            "task",
            "merge",
            &id,
            "--session",
            &session.id,
            "--actor",
            "brain",
            "--json",
        ],
    );
    assert!(
        merged.status.success(),
        "{}",
        String::from_utf8_lossy(&merged.stderr)
    );
    let merged: Value = serde_json::from_slice(&merged.stdout)
        .unwrap_or_else(|error| panic!("merge json: {error}"));
    assert_eq!(merged["worktree"]["state"], "merged");
    assert_eq!(
        fs::read_to_string(repo.join("story.txt")).unwrap_or_default(),
        "one\ntwo\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn contract_flags_round_trip_through_add_show_and_brief() {
    let _guard = lock();
    let root = fresh_root();
    let home = root.join("home");
    let cwd = root.join("cwd");
    fs::create_dir_all(&cwd).unwrap_or_else(|error| panic!("create cwd: {error}"));
    // SAFETY: this test uses its own temporary registry root.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    write_harness_registry(&HarnessRegistry::default())
        .unwrap_or_else(|error| panic!("write harnesses: {error}"));
    let session = create_session("codex", &["pi-m3".to_owned()], &cwd)
        .unwrap_or_else(|error| panic!("create session: {error}"));

    // AC2: `task add` with the contract flags creates a full contract.
    let added = orc(
        &home,
        &[
            "task",
            "add",
            "Build the widget",
            "--objective",
            "A working widget exists.",
            "--allowed",
            "src/widget.rs",
            "--allowed",
            "tests/widget.rs",
            "--forbidden",
            "no new dependencies",
            "--check",
            "widget renders",
            "--check",
            "tests pass",
            "--artifact",
            "branch with code + tests",
            "--reviewer",
            "claude",
            "--timeout",
            "600",
            "--max-retries",
            "2",
            "--max-tokens",
            "50000",
            "--max-usd-cents",
            "250",
            "--session",
            &session.id,
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
    let task: Value =
        serde_json::from_slice(&added.stdout).unwrap_or_else(|error| panic!("task json: {error}"));
    let id = task["id"].as_str().unwrap_or_default().to_owned();
    assert_eq!(task["contract"]["objective"], "A working widget exists.");
    assert_eq!(task["contract"]["allowed_paths"][0], "src/widget.rs");
    assert_eq!(task["contract"]["allowed_paths"][1], "tests/widget.rs");
    assert_eq!(task["contract"]["forbidden"][0], "no new dependencies");
    assert_eq!(task["contract"]["acceptance_checks"][1], "tests pass");
    assert_eq!(
        task["contract"]["expected_artifact"],
        "branch with code + tests"
    );
    assert_eq!(task["contract"]["reviewer"], "claude");
    assert_eq!(task["contract"]["limits"]["timeout_sec"], 600);
    assert_eq!(task["contract"]["limits"]["max_retries"], 2);
    assert_eq!(task["contract"]["budget"]["max_tokens"], 50000);
    assert_eq!(task["contract"]["budget"]["max_usd_cents"], 250);

    // AC2: `task show` displays the contract to a human.
    let shown = orc(&home, &["task", "show", &id, "--session", &session.id]);
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let shown = String::from_utf8_lossy(&shown.stdout);
    for needle in [
        "A working widget exists.",
        "src/widget.rs",
        "no new dependencies",
        "widget renders",
        "claude",
    ] {
        assert!(shown.contains(needle), "show missing {needle:?}:\n{shown}");
    }

    // AC3: the dispatch brief contains all contract sections verbatim.
    let brief = orc(&home, &["task", "brief", &id, "--session", &session.id]);
    assert!(
        brief.status.success(),
        "{}",
        String::from_utf8_lossy(&brief.stderr)
    );
    let brief = String::from_utf8_lossy(&brief.stdout);
    for needle in [
        "## Objective",
        "A working widget exists.",
        "## Allowed paths",
        "- src/widget.rs",
        "- tests/widget.rs",
        "## Forbidden",
        "- no new dependencies",
        "## Expected artifact",
        "branch with code + tests",
        "## Acceptance checks",
        "1. widget renders",
        "2. tests pass",
        "## Limits",
        "timeout 600s · max 2 retries",
        "## Reviewer",
        "claude",
        "## Budget",
        "50000 tokens · $2.50",
    ] {
        assert!(brief.contains(needle), "brief missing {needle:?}:\n{brief}");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_task_added_without_contract_flags_has_no_contract() {
    let _guard = lock();
    let root = fresh_root();
    let home = root.join("home");
    let cwd = root.join("cwd");
    fs::create_dir_all(&cwd).unwrap_or_else(|error| panic!("create cwd: {error}"));
    // SAFETY: this test uses its own temporary registry root.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    write_harness_registry(&HarnessRegistry::default())
        .unwrap_or_else(|error| panic!("write harnesses: {error}"));
    let session = create_session("codex", &["pi-m3".to_owned()], &cwd)
        .unwrap_or_else(|error| panic!("create session: {error}"));

    let added = orc(
        &home,
        &[
            "task",
            "add",
            "plain task",
            "--session",
            &session.id,
            "--json",
        ],
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let task: Value =
        serde_json::from_slice(&added.stdout).unwrap_or_else(|error| panic!("task json: {error}"));
    assert!(
        task.get("contract").is_none(),
        "uncontracted task leaked a contract key: {task}"
    );
    let _ = fs::remove_dir_all(root);
}
