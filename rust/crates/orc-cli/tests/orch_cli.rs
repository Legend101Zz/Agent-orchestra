#![allow(unsafe_code)]

//! CLI tests for the normalized `orch_*` control verbs, `pio mcp print-config`,
//! and `pio session` (issue #8).
//!
//! The behavioral parity test proves the CLI verbs and the MCP tools drive the
//! *same* underlying operation: both are thin adapters over `orc_core::orch`.
//! The MCP suite (`orc-mcp/tests/tools.rs`) asserts each tool returns the core
//! `orch::*` outcome; here we assert the CLI verb returns that same outcome, so
//! by the shared core the two surfaces cannot diverge (AC3).

use std::collections::BTreeSet;
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
use orc_core::orch::{self, AwaitRequest, DelegateRequest, OrchActor, Verb};
use serde_json::{Value, json};

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
        "orc-cli-orch-{label}-{}-{nonce}",
        std::process::id()
    ))
}

/// Write a fixture harness registry under the *current* `ORC_HOME`; the caller
/// sets `ORC_HOME` under `lock()` first.
fn write_fixture_registry(home: &Path) {
    write_fixture_registry_with_review_verdict(home, "pass");
}

fn write_fixture_registry_with_review_verdict(home: &Path, verdict: &str) {
    assert!(matches!(verdict, "pass" | "fail"));
    let bin = home.join("bin");
    fs::create_dir_all(&bin).expect("create bin");
    let script = bin.join("fake-worker.sh");
    let script_body = r#"#!/bin/sh
case "$*" in
  *"Acceptance checks:"*)
    echo '{"verdicts":[{"check":"it builds","verdict":"__VERDICT__","evidence":"fixture review returned __VERDICT__"}]}'
    ;;
  *)
    echo "fake-worker-stdout ${@: -1}"
    ;;
esac
exit 0
"#
    .replace("__VERDICT__", verdict);
    fs::write(&script, script_body).expect("write fake worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod fake worker");
    let mut registry = HarnessRegistry::default();
    for config in registry.harnesses.values_mut() {
        config.roles.retain(|role| role == "brain");
    }
    registry.harnesses.insert(
        "fake-worker".to_owned(),
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
        },
    );
    registry.default_workers = vec!["fake-worker".to_owned()];
    write_harness_registry(&registry).expect("write registry");
}

/// Create a fixture session with one running worker pane; returns its id.
fn write_fixture_session(home: &Path) -> String {
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("create cwd");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&cwd)
            .args(args)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "orch-cli@example.invalid"]);
    git(&["config", "user.name", "Orch CLI Test"]);
    fs::write(cwd.join("README.md"), "fixture\n").expect("write fixture");
    git(&["add", "README.md"]);
    git(&["commit", "-m", "fixture"]);
    let mut session =
        create_session("codex", &["fake-worker".to_owned()], &cwd).expect("create session");
    session.panes.push(SessionPaneRecord {
        id: "worker-1".to_owned(),
        harness: "fake-worker".to_owned(),
        role: "worker".to_owned(),
        state: "running".to_owned(),
        pid: None,
        down_at: None,
        extra: Default::default(),
    });
    write_session(&session).expect("write session");
    session.id
}

fn pio(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .env("ORC_HOME", home)
        .env("HOME", home.join("empty-home"))
        .output()
        .unwrap_or_else(|error| panic!("run pio {args:?}: {error}"))
}

fn pio_ok(home: &Path, args: &[&str]) -> Value {
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

/// The deterministic slice of a task that must be identical across surfaces.
fn task_projection(task: &Value) -> Value {
    json!({
        "id": task["id"],
        "title": task["title"],
        "description": task["description"],
        "status": task["status"],
        "depends_on": task["depends_on"],
        "assignee": task["assignee"],
        "assignee_run": task["assignee_run"],
        "contract": task["contract"],
    })
}

/// Subcommand names listed under `Commands:` in a clap help screen, minus the
/// generated `help` entry.
fn advertised_subcommands(help: &str) -> BTreeSet<String> {
    help.lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| line.starts_with("  ") || line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_owned)
        .collect()
}

/// AC3 (structural): the `pio orch` group exposes **exactly** the seven-verb
/// source of truth — no verb missing, and no eighth verb that the MCP router
/// would not also advertise. The set equality is what matters: the MCP side
/// asserts the same equality against `Verb::ALL`, so neither surface can grow
/// or lose a verb alone.
#[test]
fn cli_orch_verbs_match_the_source_of_truth() {
    let home = fresh_home("verbs");
    fs::create_dir_all(&home).expect("home");
    for verb in Verb::ALL {
        let output = pio(&home, &["orch", verb.cli_name(), "--help"]);
        assert!(
            output.status.success(),
            "pio orch {} --help failed: {}",
            verb.cli_name(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let help = pio(&home, &["orch", "--help"]);
    let advertised = advertised_subcommands(&String::from_utf8_lossy(&help.stdout));
    let expected: BTreeSet<String> = Verb::ALL
        .iter()
        .map(|verb| verb.cli_name().to_owned())
        .collect();
    assert_eq!(
        advertised, expected,
        "pio orch must expose exactly the seven verbs, no more and no fewer"
    );
    let _ = fs::remove_dir_all(&home);
}

/// AC3 (behavioral): `pio orch delegate` and the shared `orch::delegate` core op
/// produce the same durable task on twin fixtures. Since the MCP tool is proven
/// (in orc-mcp) to return that same core outcome, the CLI and MCP surfaces match.
#[test]
fn cli_delegate_matches_core_delegate() {
    let cli_home = fresh_home("parity-cli");
    let core_home = fresh_home("parity-core");
    let contract = json!({
        "objective": "A parity artifact exists.",
        "acceptance_checks": ["both surfaces agree"]
    });

    let _guard = lock();
    // Build the CLI fixture, then the core fixture; the CLI child reads its own
    // ORC_HOME via `.env`, so it is unaffected by this process's env pointer.
    // SAFETY: env mutation is serialized by `lock()` and only this test thread
    // reads it (the core delegate below) before the guard drops.
    unsafe { std::env::set_var("ORC_HOME", &cli_home) };
    write_fixture_registry(&cli_home);
    let cli_session = write_fixture_session(&cli_home);

    unsafe { std::env::set_var("ORC_HOME", &core_home) };
    write_fixture_registry(&core_home);
    let core_session = write_fixture_session(&core_home);

    // Core surface (ORC_HOME still points at core_home).
    let core_outcome = orch::delegate(DelegateRequest {
        session: core_session.clone(),
        harness: "fake-worker".to_owned(),
        title: Some("parity task".to_owned()),
        contract: serde_json::from_value(contract.clone()).unwrap(),
        actor: OrchActor::Brain,
        ..Default::default()
    })
    .expect("core delegate");
    let core_task_id = core_outcome.tasks[0].id.clone();
    let core_terminal = orch::await_delegation(AwaitRequest {
        session: core_session,
        task: core_task_id,
        timeout_sec: Some(10),
        poll_interval_ms: Some(10),
    })
    .expect("await core delegate");
    let core_task = task_projection(&serde_json::to_value(&core_terminal.tasks[0]).unwrap());

    // CLI surface (child bound to cli_home).
    let cli_outcome = pio_ok(
        &cli_home,
        &[
            "orch",
            "delegate",
            "fake-worker",
            "--session",
            &cli_session,
            "--title",
            "parity task",
            "--objective",
            "A parity artifact exists.",
            "--check",
            "both surfaces agree",
            "--json",
        ],
    );
    let cli_task_id = cli_outcome["tasks"][0]["id"].as_str().expect("CLI task id");
    let cli_terminal = pio_ok(
        &cli_home,
        &[
            "orch",
            "await",
            cli_task_id,
            "--session",
            &cli_session,
            "--timeout",
            "10",
            "--json",
        ],
    );
    let cli_task = task_projection(&cli_terminal["tasks"][0]);
    drop(_guard);

    assert_eq!(
        cli_task, core_task,
        "CLI and core delegate produced different task state"
    );
    assert_eq!(cli_outcome["dispatches"][0]["status"], "confirmed");
    let _ = fs::remove_dir_all(&cli_home);
    let _ = fs::remove_dir_all(&core_home);
}

/// The CLI verbs drive the real lifecycle end to end: plan → delegate → await →
/// status → review → finish, then a dropped task via cancel.
#[test]
fn cli_full_lifecycle_over_the_verbs() {
    let home = fresh_home("lifecycle");
    let session;
    {
        let _guard = lock();
        // SAFETY: serialized by `lock()`; used only to write the fixture.
        unsafe { std::env::set_var("ORC_HOME", &home) };
        write_fixture_registry(&home);
        session = write_fixture_session(&home);
    }

    // plan a task, then delegate the planned task by id.
    let planned = pio_ok(
        &home,
        &[
            "orch",
            "plan",
            "ship it",
            "--session",
            &session,
            "--objective",
            "The widget ships.",
            "--check",
            "it builds",
            "--json",
        ],
    );
    let task_id = planned["tasks"][0]["id"]
        .as_str()
        .expect("task id")
        .to_owned();
    assert_eq!(planned["tasks"][0]["status"], "backlog");

    let delegated = pio_ok(
        &home,
        &[
            "orch",
            "delegate",
            "fake-worker",
            "--session",
            &session,
            "--task",
            &task_id,
            "--json",
        ],
    );
    assert_eq!(delegated["tasks"][0]["status"], "running");
    assert_eq!(delegated["dispatches"][0]["status"], "confirmed");

    let awaited = pio_ok(
        &home,
        &[
            "orch",
            "await",
            &task_id,
            "--session",
            &session,
            "--timeout",
            "10",
            "--json",
        ],
    );
    assert!(awaited["note"].is_null());

    let status = pio_ok(
        &home,
        &["orch", "status", &task_id, "--session", &session, "--json"],
    );
    assert_eq!(status["tasks"][0]["id"], task_id);

    let reviewed = pio_ok(
        &home,
        &["orch", "review", &task_id, "--session", &session, "--json"],
    );
    assert_eq!(reviewed["tasks"][0]["status"], "review");
    assert_eq!(reviewed["dispatches"][0]["purpose"], "review");

    let review_awaited = pio_ok(
        &home,
        &[
            "orch",
            "await",
            &task_id,
            "--session",
            &session,
            "--timeout",
            "10",
            "--json",
        ],
    );
    assert_eq!(
        review_awaited["dispatches"][0]["execution_status"],
        "succeeded"
    );

    let finished = pio_ok(
        &home,
        &["orch", "finish", &task_id, "--session", &session, "--json"],
    );
    assert_eq!(finished["tasks"][0]["status"], "done");
    assert_eq!(
        finished["tasks"][0]["report"]["verdicts"][0]["verdict"],
        "pass"
    );
    assert_eq!(finished["tasks"][0]["report"]["review_mode"], "self_review");

    // A second task exercises cancel (drop).
    let second = pio_ok(
        &home,
        &["orch", "plan", "scrap it", "--session", &session, "--json"],
    );
    let second_id = second["tasks"][0]["id"].as_str().unwrap().to_owned();
    let cancelled = pio_ok(
        &home,
        &[
            "orch",
            "cancel",
            &second_id,
            "--session",
            &session,
            "--json",
        ],
    );
    assert_eq!(cancelled["tasks"][0]["status"], "dropped");

    let _ = fs::remove_dir_all(&home);
}

/// A failed acceptance verdict is a hard completion barrier: the real CLI
/// lifecycle persists the report but must leave the task in review.
#[test]
fn failed_review_verdict_blocks_finish_and_keeps_task_in_review() {
    let home = fresh_home("failed-review");
    let session;
    {
        let _guard = lock();
        // SAFETY: serialized by `lock()`; used only to write the fixture.
        unsafe { std::env::set_var("ORC_HOME", &home) };
        write_fixture_registry_with_review_verdict(&home, "fail");
        session = write_fixture_session(&home);
    }

    let planned = pio_ok(
        &home,
        &[
            "orch",
            "plan",
            "reject failed review",
            "--session",
            &session,
            "--objective",
            "Only accepted work may finish.",
            "--check",
            "it builds",
            "--json",
        ],
    );
    let task_id = planned["tasks"][0]["id"]
        .as_str()
        .expect("task id")
        .to_owned();

    let delegated = pio_ok(
        &home,
        &[
            "orch",
            "delegate",
            "fake-worker",
            "--session",
            &session,
            "--task",
            &task_id,
            "--json",
        ],
    );
    assert_eq!(delegated["tasks"][0]["status"], "running");
    pio_ok(
        &home,
        &[
            "orch",
            "await",
            &task_id,
            "--session",
            &session,
            "--timeout",
            "10",
            "--json",
        ],
    );

    let reviewed = pio_ok(
        &home,
        &["orch", "review", &task_id, "--session", &session, "--json"],
    );
    assert_eq!(reviewed["tasks"][0]["status"], "review");
    pio_ok(
        &home,
        &[
            "orch",
            "await",
            &task_id,
            "--session",
            &session,
            "--timeout",
            "10",
            "--json",
        ],
    );

    let finish = pio(
        &home,
        &["orch", "finish", &task_id, "--session", &session, "--json"],
    );
    assert!(
        !finish.status.success(),
        "finish must reject a failed acceptance verdict"
    );
    let error = String::from_utf8_lossy(&finish.stderr);
    assert!(
        error.contains("remains review") && error.contains("1 acceptance check(s) failed"),
        "finish must explain the failed completion barrier: {error}"
    );

    let status = pio_ok(
        &home,
        &["orch", "status", &task_id, "--session", &session, "--json"],
    );
    let task = &status["tasks"][0];
    assert_eq!(task["status"], "review");
    assert_eq!(task["report"]["verdicts"][0]["check"], "it builds");
    assert_eq!(task["report"]["verdicts"][0]["verdict"], "fail");
    assert!(
        task["history"]
            .as_array()
            .expect("task history")
            .iter()
            .all(|event| event["to"] != "done"),
        "failed review must never record a transition to done"
    );

    let report_path = task["report"]["path"].as_str().expect("report path");
    let report: Value =
        serde_json::from_slice(&fs::read(report_path).expect("persisted failed report"))
            .expect("failed report JSON");
    assert_eq!(report["verdicts"][0]["verdict"], "fail");

    let _ = fs::remove_dir_all(&home);
}

/// A delegation whose delivery does not confirm must say so in the outcome
/// itself, not only in the CLI's exit code — the MCP surface has no exit code,
/// so a silent `note` would let a conductor read a failed delegation as a
/// successful one (issue #8 review).
#[test]
fn failed_delegation_is_announced_in_the_outcome() {
    let home = fresh_home("failed-delivery");
    let session;
    {
        let _guard = lock();
        // SAFETY: serialized by `lock()`; used only to write the fixture.
        unsafe { std::env::set_var("ORC_HOME", &home) };
        write_fixture_registry(&home);
        session = write_fixture_session(&home);
    }

    // An unknown harness cannot deliver, so the dispatch fails.
    let output = pio(
        &home,
        &[
            "orch",
            "delegate",
            "no-such-harness",
            "--session",
            &session,
            "--title",
            "doomed",
            "--json",
        ],
    );
    assert!(
        !output.status.success(),
        "the CLI must exit non-zero on a failed delivery"
    );
    let outcome: Value = serde_json::from_slice(&output.stdout).expect("delegate emitted JSON");
    assert_eq!(outcome["dispatches"][0]["status"], "failed");
    let note = outcome["note"]
        .as_str()
        .expect("a non-confirmed delivery must carry a note for the MCP surface");
    assert!(
        note.contains("did not confirm") && note.contains("unknown_harness"),
        "note must name the failure: {note}"
    );
    assert!(
        note.contains(outcome["tasks"][0]["id"].as_str().unwrap()),
        "note must name the task left behind: {note}"
    );

    // A delivered background worker tells the conductor how to observe it.
    let confirmed = pio_ok(
        &home,
        &[
            "orch",
            "delegate",
            "fake-worker",
            "--session",
            &session,
            "--title",
            "fine",
            "--json",
        ],
    );
    assert_eq!(confirmed["dispatches"][0]["status"], "confirmed");
    let execution = confirmed["dispatches"][0]["execution_status"]
        .as_str()
        .expect("confirmed delivery execution status");
    assert!(matches!(execution, "running" | "succeeded"));
    if execution == "running" {
        let note = confirmed["note"].as_str().expect("running guidance note");
        assert!(
            note.contains("still running") && note.contains("orch_await"),
            "a running background delivery must explain the next step: {confirmed}"
        );
    } else {
        assert!(
            confirmed["note"].is_null(),
            "an already successful delivery should be quiet: {confirmed}"
        );
    }

    let _ = fs::remove_dir_all(&home);
}

/// AC4: `pio mcp print-config` emits a valid Claude Code JSON object and a valid
/// Codex TOML block, and writes nothing — protected config files are untouched.
#[test]
fn mcp_print_config_emits_valid_snippets_without_writing_files() {
    let home = fresh_home("print-config");
    fs::create_dir_all(&home).expect("home");

    // Claude Code: valid JSON registering the server under mcpServers.
    let claude = pio(&home, &["mcp", "print-config", "--format", "claude"]);
    assert!(claude.status.success());
    let parsed: Value =
        serde_json::from_slice(&claude.stdout).expect("claude config is valid JSON");
    let command = parsed["mcpServers"]["pi-orchestra"]["command"]
        .as_str()
        .expect("claude config names the server command");
    assert!(
        command.ends_with("pio-mcp"),
        "claude command should point at pio-mcp, got {command}"
    );
    assert!(parsed["mcpServers"]["pi-orchestra"]["args"].is_array());

    // Codex: valid TOML block for the server table.
    let codex = pio(&home, &["mcp", "print-config", "--format", "codex"]);
    assert!(codex.status.success());
    let toml = String::from_utf8_lossy(&codex.stdout);
    assert!(
        toml.contains("[mcp_servers.pi-orchestra]"),
        "codex block missing table header: {toml}"
    );
    assert!(
        toml.contains("command = \"") && toml.trim_end().ends_with("args = []"),
        "codex block malformed: {toml}"
    );

    // No --format prints both snippets under commented headers.
    let both = pio(&home, &["mcp", "print-config"]);
    let both_text = String::from_utf8_lossy(&both.stdout);
    assert!(both_text.contains("mcpServers") && both_text.contains("[mcp_servers.pi-orchestra]"));

    // Protected-config safety: print-config must not create any files, including
    // the well-known protected paths, under the isolated HOME.
    for protected in [
        ".claude.json",
        ".claude/settings.json",
        ".codex/config.toml",
    ] {
        assert!(
            !home.join("empty-home").join(protected).exists(),
            "print-config must not write {protected}"
        );
    }
    assert!(
        !home.join("empty-home").exists(),
        "print-config created a HOME tree; it must write nothing"
    );
    let _ = fs::remove_dir_all(&home);
}

/// `pio session create` / `list` provide a headless way to open a delegation
/// session (folded in per the issue #5 review note).
#[test]
fn session_create_and_list_headless() {
    let home = fresh_home("session");
    let cwd = home.join("work");
    fs::create_dir_all(&cwd).expect("cwd");
    {
        let _guard = lock();
        // SAFETY: serialized by `lock()`; used only to write the fixture registry
        // that `session create` validates its workers against.
        unsafe { std::env::set_var("ORC_HOME", &home) };
        write_fixture_registry(&home);
    }

    let created = pio_ok(
        &home,
        &[
            "session",
            "create",
            "--brain",
            "claude",
            "--worker",
            "fake-worker",
            "--cwd",
            cwd.to_str().unwrap(),
            "--json",
        ],
    );
    let id = created["id"].as_str().expect("session id").to_owned();
    assert_eq!(created["brain"], "claude");

    let listed = pio_ok(&home, &["session", "list", "--json"]);
    assert!(
        listed
            .as_array()
            .expect("session list array")
            .iter()
            .any(|session| session["id"] == id),
        "created session not listed"
    );
    let _ = fs::remove_dir_all(&home);
}

// --- The --json contract, and errors that name their own remedy (issue #45) --

/// Create a session whose cwd is a real directory but **not** a git work tree.
///
/// This is the reproduction's shape: issue #45 delegated from
/// `/Users/comreton/.local/bin`. A contracted task wants an isolated worktree,
/// `materialize_worktree` cannot make one outside a repository, and it records
/// that as a task state rather than a failure — so the refusal only arrives at
/// dispatch time, long after the task looked fine.
fn session_outside_a_git_repo(home: &Path) -> String {
    let cwd = home.join("not-a-repo");
    fs::create_dir_all(&cwd).expect("create non-git cwd");
    let session =
        create_session("claude", &["fake-worker".to_owned()], &cwd).expect("create session");
    session.id
}

#[test]
fn orch_delegate_json_answers_in_json_when_isolation_is_unavailable() {
    // Check 7. Verbatim from the issue: `--json` printed `ISOLATION REQUIRED`
    // on stderr, exited 1, and left stdout EMPTY. A caller promised JSON got
    // nothing to parse and could not tell failure from silence.
    let _guard = lock();
    let home = fresh_home("json-isolation");
    fs::create_dir_all(&home).expect("create home");
    unsafe { std::env::set_var("ORC_HOME", &home) };
    write_fixture_registry(&home);
    let session = session_outside_a_git_repo(&home);

    let output = pio(
        &home,
        &[
            "orch",
            "delegate",
            "fake-worker",
            "--session",
            &session,
            "--title",
            "say hello",
            "--objective",
            "hermes replies",
            "--check",
            "the reply is exact",
            "--json",
        ],
    );

    assert!(!output.status.success(), "the delegation must still fail");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "--json must not leave stdout empty"
    );
    let payload: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("stdout is not JSON ({error}): {stdout}"));
    assert_eq!(payload["ok"], Value::Bool(false));
    assert_eq!(payload["verb"], "orch_delegate");
    assert_eq!(
        payload["error"]["reason"], "isolation_unavailable",
        "the reason is machine-readable: {payload}"
    );
    let message = payload["error"]["message"].as_str().expect("a message");
    assert!(
        message.contains("ISOLATION REQUIRED"),
        "the human message survives: {message}"
    );
    assert!(
        message.contains("not a Git work tree"),
        "and names the actual cause, not just the symptom: {message}"
    );
    assert!(
        message.contains("uncontracted"),
        "and the path that needs no worktree: {message}"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn orch_delegate_json_leaves_parseable_json_on_stdout_however_it_fails() {
    // The contract is a property of `--json`, not of one error, so it is
    // checked across the two genuinely different shapes a failure takes:
    //
    //  - an *error*, which aborts before an outcome exists (unknown session)
    //    and gets the `{ok:false, error:{reason,message}}` envelope; and
    //  - a *recorded failure*, where dispatch persists a failed record and
    //    reports it inside a normal outcome (unknown harness).
    //
    // Both must exit non-zero and both must leave JSON on stdout. Only the
    // first was broken, but a test that pinned only the first would let the
    // second silently regress into a bare stderr line.
    let _guard = lock();
    let home = fresh_home("json-shapes");
    fs::create_dir_all(&home).expect("create home");
    unsafe { std::env::set_var("ORC_HOME", &home) };
    write_fixture_registry(&home);
    let session = write_fixture_session(&home);

    // 1. An error: there is no such session, so nothing can be recorded.
    let output = pio(
        &home,
        &[
            "orch",
            "delegate",
            "fake-worker",
            "--session",
            "no-such-session-at-all",
            "--title",
            "anything",
            "--json",
        ],
    );
    assert!(!output.status.success(), "an unknown session must fail");
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(payload["ok"], Value::Bool(false));
    assert!(
        payload["error"]["reason"].is_string(),
        "every error carries a machine-readable reason: {payload}"
    );

    // 2. A recorded failure: the harness is unknown, but the board keeps the
    //    receipt, so the answer is a normal outcome — still JSON, still
    //    non-zero, and the failure is legible inside it.
    let output = pio(
        &home,
        &[
            "orch",
            "delegate",
            "no-such-harness",
            "--session",
            &session,
            "--title",
            "anything",
            "--json",
        ],
    );
    assert!(!output.status.success(), "an unknown harness must fail");
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    let dispatch = &payload["dispatches"][0];
    assert_eq!(
        dispatch["status"], "failed",
        "the record says so: {payload}"
    );
    assert!(
        dispatch["failure_kind"].is_string(),
        "and names the kind, machine-readably: {payload}"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn dispatch_send_with_an_unknown_task_names_the_command_that_lists_real_ones() {
    // Check 9. The issue records a conductor guessing `T-hello`, then
    // `T0002`, because the refusal named neither a valid id nor a way to find
    // one. An error that ends the search is worth more than one that is
    // merely accurate.
    let _guard = lock();
    let home = fresh_home("unknown-task");
    fs::create_dir_all(&home).expect("create home");
    unsafe { std::env::set_var("ORC_HOME", &home) };
    write_fixture_registry(&home);
    let session = write_fixture_session(&home);

    for task in ["T9999", "T-hello"] {
        let output = pio(
            &home,
            &[
                "dispatch",
                "send",
                task,
                "fake-worker",
                "say hello",
                "--session",
                &session,
            ],
        );
        assert!(!output.status.success(), "{task} must be refused");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("pio task list --session"),
            "{task}: the refusal must name how to find a real id: {stderr}"
        );
        assert!(
            stderr.contains(&session),
            "{task}: and name this session so the command is copy-pasteable: {stderr}"
        );
    }

    let _ = fs::remove_dir_all(&home);
}
