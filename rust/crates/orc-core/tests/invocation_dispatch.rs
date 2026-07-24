#![allow(unsafe_code)]

//! Universal worker adapter integration tests (issue #6, V1-4).
//!
//! Proves the dispatcher drives **any** probed harness as a worker and that the
//! adapter chooses the invocation style from the capability probe results:
//!
//! * AC1 — a flag-prompt worker and a subcommand-prompt worker each receive a
//!   brief and return a confirmed receipt;
//! * the same probe record toggles optional flags (structured output, cwd),
//!   so a harness that only probed non-interactive gets the bare style;
//! * AC2 — a worker whose probe lacks the required capability is refused with a
//!   failed record naming that capability.
//!
//! Fixtures live in `tools/fixtures/harness-styles/` so the tests never spawn a
//! real model provider.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{
    BenchSession, DiscoveredHarness, HarnessConfig, HarnessRegistry, create_session,
    write_harness_registry,
};
use orc_core::dispatch::{self, DeliveryStatus, DispatchActor, DispatchRequest};
use orc_core::probe::{BinaryIdentity, Capability, CapabilityProbe};
use orc_core::tasks::{NewTask, TaskActor, TaskStatus, add_task, assign_task, start_task};

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
        "orc-invocation-{label}-{}-{nonce}",
        std::process::id()
    ))
}

/// Absolute path to one invocation-style fixture script.
fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tools/fixtures/harness-styles")
        .join(name);
    fs::canonicalize(&path)
        .unwrap_or_else(|error| panic!("canonicalize fixture {}: {error}", path.display()))
        .to_string_lossy()
        .into_owned()
}

/// A `/bin/sh <script>` worker whose adapter/probe drive the invocation style.
fn probe_worker(script: &str, adapter: &str) -> HarnessConfig {
    HarnessConfig {
        command: "/bin/sh".to_owned(),
        args: vec![fixture(script)],
        resume_args: Vec::new(),
        roles: vec!["worker".to_owned()],
        adapter: adapter.to_owned(),
        // Empty: forces the probe-driven path rather than an explicit override.
        dispatch_args: Vec::new(),
        dispatch_uses_stdin: false,
        dispatch_timeout_sec: 30,
        extra: BTreeMap::new(),
    }
}

/// A discovered-harness record carrying a synthetic probe (as `pio doctor` would).
fn probed(adapter: &str, capabilities: &[Capability]) -> DiscoveredHarness {
    let slugs: BTreeSet<String> = capabilities
        .iter()
        .map(|capability| capability.slug().to_owned())
        .collect();
    DiscoveredHarness {
        path: format!("/fake/bin/{adapter}"),
        version: None,
        first_seen: "2026-07-24T00:00:00+00:00".to_owned(),
        last_seen: "2026-07-24T00:00:00+00:00".to_owned(),
        probe: Some(CapabilityProbe {
            probed_at: "2026-07-24T00:00:00+00:00".to_owned(),
            binary: BinaryIdentity {
                path: format!("/fake/bin/{adapter}"),
                mtime_ns: 1,
                size: 1,
                extra: BTreeMap::new(),
            },
            capabilities: slugs,
            error: None,
            extra: BTreeMap::new(),
        }),
        extra: BTreeMap::new(),
    }
}

fn running_task(session: &str, title: &str, worker: &str) -> orc_core::tasks::Task {
    let task = add_task(
        session,
        TaskActor::Brain,
        NewTask {
            title: title.to_owned(),
            ..NewTask::default()
        },
    )
    .expect("add task");
    assign_task(
        session,
        &task.id,
        worker.to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign task");
    let running = start_task(session, &task.id, TaskActor::Brain).expect("start task");
    assert_eq!(running.status, TaskStatus::Running.as_str());
    running
}

/// Build a session with three probe-driven workers of differing capabilities.
fn setup(home: &Path) -> BenchSession {
    fs::create_dir_all(home).expect("create home");
    // SAFETY: ORC_HOME mutation is serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", home) };
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert(
        "flag-worker".to_owned(),
        probe_worker("flag-style.sh", "claude"),
    );
    registry.harnesses.insert(
        "sub-worker".to_owned(),
        probe_worker("subcommand-style.sh", "codex"),
    );
    registry.harnesses.insert(
        "bare-worker".to_owned(),
        probe_worker("subcommand-style.sh", "opencode"),
    );
    // Probe records exactly as `pio doctor` would persist them (issue #4).
    registry.discovered.insert(
        "claude".to_owned(),
        probed(
            "claude",
            &[Capability::NonInteractive, Capability::StructuredOutput],
        ),
    );
    registry.discovered.insert(
        "codex".to_owned(),
        probed(
            "codex",
            &[
                Capability::NonInteractive,
                Capability::StructuredOutput,
                Capability::WorkingDir,
            ],
        ),
    );
    registry.discovered.insert(
        "opencode".to_owned(),
        probed("opencode", &[Capability::NonInteractive]),
    );
    write_harness_registry(&registry).expect("write registry");

    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("create cwd");
    create_session(
        "codex",
        &[
            "flag-worker".to_owned(),
            "sub-worker".to_owned(),
            "bare-worker".to_owned(),
        ],
        &cwd,
    )
    .expect("create session")
}

#[test]
fn flag_prompt_worker_receives_brief_and_returns_a_confirmed_receipt() {
    let _guard = lock();
    let home = fresh_home("flag");
    let session = setup(&home);
    let task = running_task(&session.id, "flag style dispatch", "flag-worker");

    let record = dispatch::dispatch(&DispatchRequest {
        session: session.id.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "flag-worker".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "summarize the diff".to_owned(),
        timeout_sec: Some(30),
    })
    .expect("dispatch resolves");

    assert_eq!(record.status, DeliveryStatus::Confirmed.as_str());
    // The claude probe advertised structured output → the adapter added the
    // stream-json flags, with the brief as the final positional argument.
    assert!(
        record.command_line.contains("-p"),
        "{}",
        record.command_line
    );
    assert!(
        record.command_line.contains("stream-json"),
        "{}",
        record.command_line
    );
    assert!(record.command_line.contains("summarize the diff"));
    // The worker received the brief and reported a confirmed receipt.
    assert!(
        record
            .stdout
            .contains("flag-style receipt brief: summarize the diff"),
        "{}",
        record.stdout
    );
    // Orchestrator-provided cwd control: the record and the worker agree.
    assert_eq!(record.cwd.as_deref(), Some(session.cwd.as_str()));
    let _ = fs::remove_dir_all(home);
}

#[test]
fn subcommand_worker_receives_brief_and_uses_probed_cwd_and_json_flags() {
    let _guard = lock();
    let home = fresh_home("sub");
    let session = setup(&home);
    let task = running_task(&session.id, "subcommand style dispatch", "sub-worker");

    let record = dispatch::dispatch(&DispatchRequest {
        session: session.id.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "sub-worker".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "write the migration".to_owned(),
        timeout_sec: Some(30),
    })
    .expect("dispatch resolves");

    assert_eq!(record.status, DeliveryStatus::Confirmed.as_str());
    // codex probed non-interactive + structured + working-dir → exec --json -C.
    assert!(
        record.command_line.contains("exec"),
        "{}",
        record.command_line
    );
    assert!(
        record.command_line.contains("--json"),
        "{}",
        record.command_line
    );
    assert!(
        record.command_line.contains("-C"),
        "{}",
        record.command_line
    );
    assert!(
        record.stdout.contains("subcommand-style receipt sub: exec"),
        "{}",
        record.stdout
    );
    assert!(
        record
            .stdout
            .contains("subcommand-style receipt brief: write the migration"),
        "{}",
        record.stdout
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn bare_worker_gets_only_the_style_when_optional_capabilities_were_not_probed() {
    let _guard = lock();
    let home = fresh_home("bare");
    let session = setup(&home);
    let task = running_task(&session.id, "bare style dispatch", "bare-worker");

    let record = dispatch::dispatch(&DispatchRequest {
        session: session.id.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "bare-worker".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "list the files".to_owned(),
        timeout_sec: Some(30),
    })
    .expect("dispatch resolves");

    assert_eq!(record.status, DeliveryStatus::Confirmed.as_str());
    // opencode probed non-interactive only → bare `run`, no --format, no --dir.
    assert!(
        record.command_line.contains("run"),
        "{}",
        record.command_line
    );
    assert!(
        !record.command_line.contains("--format"),
        "structured flag must not appear without a probe: {}",
        record.command_line
    );
    assert!(
        !record.command_line.contains("--dir"),
        "cwd flag must not appear without a probe: {}",
        record.command_line
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn worker_without_probed_non_interactive_is_refused_naming_the_capability() {
    let _guard = lock();
    let home = fresh_home("refuse");
    let session = setup(&home);
    // Re-probe opencode as interactive-only (drops non_interactive).
    let mut registry = orc_core::bench::load_harness_registry().expect("load registry to re-probe");
    registry.discovered.insert(
        "opencode".to_owned(),
        probed("opencode", &[Capability::ModelSelect]),
    );
    write_harness_registry(&registry).expect("persist interactive-only probe");
    let task = running_task(&session.id, "capability refusal", "bare-worker");

    let record = dispatch::dispatch(&DispatchRequest {
        session: session.id.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "bare-worker".to_owned(),
        pane_id: None,
        run: None,
        prompt: "should be refused".to_owned(),
        timeout_sec: Some(30),
    })
    .expect("dispatch persists even on refusal");

    assert_eq!(record.status, DeliveryStatus::Failed.as_str());
    assert_eq!(
        record.failure_kind.as_deref(),
        Some("capability_unavailable")
    );
    let error = record.error.as_deref().unwrap_or_default();
    assert!(
        error.contains("CAPABILITY UNAVAILABLE") && error.contains("non_interactive"),
        "refusal must name the missing capability; got {error:?}"
    );
    let _ = fs::remove_dir_all(home);
}
