#![allow(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessRegistry, create_session, write_harness_registry};
use serde_json::Value;

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
