#![allow(unsafe_code)]

//! CLI tests for the normalized `orch_*` control verbs, `pio mcp print-config`,
//! and `pio session` (issue #8).
//!
//! The behavioral parity test proves the CLI verbs and the MCP tools drive the
//! *same* underlying operation: both are thin adapters over `orc_core::orch`.
//! The MCP suite (`orc-mcp/tests/tools.rs`) asserts each tool returns the core
//! `orch::*` outcome; here we assert the CLI verb returns that same outcome, so
//! by the shared core the two surfaces cannot diverge (AC3).

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
use orc_core::orch::{self, DelegateRequest, OrchActor, Verb};
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
    let bin = home.join("bin");
    fs::create_dir_all(&bin).expect("create bin");
    let script = bin.join("fake-worker.sh");
    fs::write(
        &script,
        "#!/bin/sh\necho \"fake-worker-stdout ${@: -1}\"\nexit 0\n",
    )
    .expect("write fake worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod fake worker");
    let mut registry = HarnessRegistry::default();
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

/// AC3 (structural): every `orch` verb is a real `pio orch` subcommand and the
/// group exposes exactly the seven-verb source of truth — the same set the MCP
/// router advertises (asserted against `Verb::ALL` there too).
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
    let text = String::from_utf8_lossy(&help.stdout);
    for verb in Verb::ALL {
        assert!(
            text.contains(verb.cli_name()),
            "orch --help omits {}",
            verb.cli_name()
        );
    }
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
        session: core_session,
        harness: "fake-worker".to_owned(),
        title: Some("parity task".to_owned()),
        contract: serde_json::from_value(contract.clone()).unwrap(),
        actor: OrchActor::Brain,
        ..Default::default()
    })
    .expect("core delegate");
    let core_task = task_projection(&serde_json::to_value(&core_outcome.tasks[0]).unwrap());

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
    let cli_task = task_projection(&cli_outcome["tasks"][0]);
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

    let finished = pio_ok(
        &home,
        &["orch", "finish", &task_id, "--session", &session, "--json"],
    );
    assert_eq!(finished["tasks"][0]["status"], "done");

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
