#![allow(unsafe_code)]

//! MCP-surface tests for the `orch_*` control tools (issue #8).
//!
//! Every test spawns the real `pio-mcp` binary and drives it as an MCP client
//! over stdio (`TokioChildProcess`), so `tools/list` and `tools/call` exercise
//! the genuine JSON-RPC-over-stdio path an assistant would use. Each child gets
//! its own `ORC_HOME`, so the tests are isolated and parallel-safe.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{
    HarnessConfig, HarnessRegistry, SessionPaneRecord, create_session, write_harness_registry,
    write_session,
};
use orc_core::orch::Verb;
use orc_mcp::OrchServer;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientInfo};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{Value, json};

/// Serialize env-mutating fixture writes; the child reads its own `.env` copy.
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
    std::env::temp_dir().join(format!("orc-mcp-{label}-{}-{nonce}", std::process::id()))
}

/// Write a fixture harness + session under `home`, returning the session id.
///
/// The fake worker is a shell script that echoes its last argument, so a
/// dispatch confirms without reaching a real provider.
fn setup_fixture(label: &str) -> (PathBuf, String) {
    let home = fresh_home(label);
    let bin = home.join("bin");
    fs::create_dir_all(&bin).expect("create bin");
    let script = bin.join("fake-worker.sh");
    fs::write(
        &script,
        "#!/bin/sh\necho \"fake-worker-stdout ${@: -1}\"\necho fake-worker-stderr 1>&2\nexit 0\n",
    )
    .expect("write fake worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod fake worker");

    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("create cwd");

    let _guard = lock();
    // SAFETY: fixture writes serialize through `lock()`; the spawned child reads
    // its own ORC_HOME via `.env`, so it never races this process's env.
    unsafe { std::env::set_var("ORC_HOME", &home) };
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
    (home, session.id)
}

#[derive(Clone)]
struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }
}

/// Connect to a freshly spawned `pio-mcp` child bound to `home`.
async fn connect(home: &Path) -> rmcp::service::RunningService<rmcp::RoleClient, TestClient> {
    let home = home.to_path_buf();
    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_pio-mcp")).configure(|command| {
            command.env("ORC_HOME", &home);
            command.env("HOME", home.join("empty-home"));
        }),
    )
    .expect("spawn pio-mcp");
    TestClient
        .serve(transport)
        .await
        .expect("initialize client")
}

fn structured(result: &CallToolResult) -> Value {
    result
        .structured_content
        .clone()
        .expect("tool returned structured content")
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    name: &'static str,
    body: Value,
) -> Value {
    let request = CallToolRequestParams::new(name)
        .with_arguments(body.as_object().expect("object args").clone());
    let result = client
        .call_tool(request)
        .await
        .unwrap_or_else(|error| panic!("call {name}: {error}"));
    assert_ne!(
        result.is_error,
        Some(true),
        "tool {name} returned an error result: {:?}",
        result.content
    );
    structured(&result)
}

/// AC3 (structural): the MCP router advertises exactly the seven `orch_*` tools,
/// and each tool's advertised description is the `Verb` source-of-truth literal.
#[test]
fn router_advertises_exactly_the_seven_verbs() {
    let server = OrchServer::new();
    let mut expected: Vec<String> = Verb::ALL.iter().map(|v| v.tool_name().to_owned()).collect();
    expected.sort();
    assert_eq!(server.tool_names(), expected);

    for tool in server.advertised_tools() {
        let verb = Verb::ALL
            .iter()
            .find(|v| v.tool_name() == tool.name.as_ref())
            .unwrap_or_else(|| panic!("router exposed unknown tool {}", tool.name));
        assert_eq!(
            tool.description.as_deref(),
            Some(verb.description()),
            "description drift for {}",
            tool.name
        );
    }
}

/// AC1: `tools/list` over stdio returns exactly the seven tools, each carrying a
/// non-empty JSON input schema.
#[tokio::test]
async fn tools_list_over_stdio_returns_seven_tools_with_schemas() {
    let home = fresh_home("list");
    let client = connect(&home).await;
    let tools = client.list_all_tools().await.expect("list tools");

    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "orch_await",
            "orch_cancel",
            "orch_delegate",
            "orch_finish",
            "orch_plan",
            "orch_review",
            "orch_status",
        ]
    );
    for tool in &tools {
        let schema = tool.input_schema.as_ref();
        assert_eq!(
            schema.get("type"),
            Some(&json!("object")),
            "{} input schema is not an object",
            tool.name
        );
        assert!(
            schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some(),
            "{} input schema has no properties",
            tool.name
        );
    }
    client.cancel().await.expect("shutdown");
    let _ = fs::remove_dir_all(&home);
}

/// AC2: end-to-end over stdio — `orch_delegate` against the fixture worker
/// creates a contracted task and confirms receipt while execution is running;
/// `orch_await` and `orch_status` then observe the terminal result.
#[tokio::test]
async fn delegate_await_status_end_to_end_over_stdio() {
    let (home, session) = setup_fixture("e2e");
    let client = connect(&home).await;

    let delegated = call(
        &client,
        "orch_delegate",
        json!({
            "session": session,
            "harness": "fake-worker",
            "title": "summarize the diff",
            "contract": {
                "objective": "A summary of the diff exists.",
                "acceptance_checks": ["the summary names each changed file"]
            }
        }),
    )
    .await;
    assert_eq!(delegated["verb"], "orch_delegate");
    let task_id = delegated["tasks"][0]["id"]
        .as_str()
        .expect("delegated task id")
        .to_owned();
    assert_eq!(delegated["tasks"][0]["status"], "running");
    assert_eq!(delegated["dispatches"][0]["status"], "confirmed");
    let immediate_execution = delegated["dispatches"][0]["execution_status"]
        .as_str()
        .expect("delegate reports execution status");
    assert!(
        matches!(immediate_execution, "running" | "succeeded"),
        "confirmed receipt must be running or already succeeded: {delegated}"
    );
    assert_eq!(delegated["dispatches"][0]["harness"], "fake-worker");
    // The worker really received the rendered acceptance brief as its prompt —
    // assert on the delivered prompt, not merely on the worker having run.
    let prompt = delegated["dispatches"][0]["prompt"]
        .as_str()
        .expect("dispatch records the delivered prompt");
    assert!(
        prompt.contains("## Objective")
            && prompt.contains("A summary of the diff exists.")
            && prompt.contains("## Acceptance checks")
            && prompt.contains("the summary names each changed file"),
        "delivered prompt is not the rendered contract brief: {prompt}"
    );
    // Receipt is not completion: while still running, the note tells conductors
    // how to observe it. A tiny worker may already be terminal by serialization.
    if immediate_execution == "running" {
        assert!(
            delegated["note"]
                .as_str()
                .is_some_and(|note| note.contains("still running") && note.contains("orch_await")),
            "a running delegation must explain how to observe completion: {delegated}"
        );
    } else {
        assert!(
            delegated["note"].is_null(),
            "a terminal successful delegation should be quiet: {delegated}"
        );
    }

    let awaited = call(
        &client,
        "orch_await",
        json!({ "session": session, "task": task_id, "timeout_sec": 10 }),
    )
    .await;
    assert_eq!(awaited["verb"], "orch_await");
    assert_eq!(awaited["dispatches"][0]["status"], "confirmed");
    assert_eq!(awaited["dispatches"][0]["execution_status"], "succeeded");
    assert_eq!(awaited["dispatches"][0]["exit_code"], 0);
    assert!(
        awaited["dispatches"][0]["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("fake-worker-stdout"),
        "worker stdout was not captured by await: {awaited}"
    );
    assert!(awaited["note"].is_null(), "await should not have timed out");

    let observed = call(
        &client,
        "orch_status",
        json!({ "session": session, "task": task_id }),
    )
    .await;
    assert_eq!(observed["tasks"][0]["id"], task_id);
    assert_eq!(observed["tasks"][0]["status"], "running");
    assert_eq!(observed["dispatches"][0]["status"], "confirmed");
    // Confirmed delivery linked a run to the task (honest completion record).
    assert!(
        observed["tasks"][0]["assignee_run"].is_string(),
        "confirmed delivery must link a run: {observed}"
    );

    client.cancel().await.expect("shutdown");
    let _ = fs::remove_dir_all(&home);
}

/// A delegation that cannot be delivered must announce itself over MCP too.
///
/// The CLI signals this with a non-zero exit code; MCP has no such channel, so
/// the outcome's `note` is the only thing standing between a conductor and
/// reading a failed delegation as a successful one (issue #8 review).
#[tokio::test]
async fn failed_delegation_over_stdio_carries_a_note() {
    let (home, session) = setup_fixture("failed");
    let client = connect(&home).await;

    let delegated = call(
        &client,
        "orch_delegate",
        json!({
            "session": session,
            "harness": "no-such-harness",
            "title": "doomed",
        }),
    )
    .await;
    assert_eq!(delegated["dispatches"][0]["status"], "failed");
    let note = delegated["note"]
        .as_str()
        .expect("a non-confirmed delivery must carry a note");
    assert!(
        note.contains("did not confirm") && note.contains("unknown_harness"),
        "note must name the failure: {note}"
    );
    assert!(
        note.contains(delegated["tasks"][0]["id"].as_str().unwrap()),
        "note must name the task left behind: {note}"
    );

    client.cancel().await.expect("shutdown");
    let _ = fs::remove_dir_all(&home);
}
