#![allow(unsafe_code)]

//! Contract v2 durability (issue #5, acceptance check 1).
//!
//! These tests prove the additive-JSON guarantee from the #16 decision
//! record: a pre-v2 task record (no `contract` key) still loads, and a record
//! carrying unknown future fields survives a read→write cycle unchanged.

use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessRegistry, create_session, write_harness_registry};
use orc_core::contract::{TaskBudget, TaskContract, TaskLimits};
use orc_core::tasks::{NewTask, TaskActor, add_task, read_task, task_path};
use serde_json::{Value, json};

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fresh_home() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("orc-contract-{}-{nonce}", std::process::id()))
}

fn seed_session() -> (std::path::PathBuf, String) {
    let home = fresh_home();
    // SAFETY: the shared lock serializes the process-wide registry root.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    write_harness_registry(&HarnessRegistry::default()).unwrap();
    let session = create_session(
        "codex",
        &["pi-m3".to_owned()],
        std::env::temp_dir().as_path(),
    )
    .unwrap();
    (home, session.id)
}

#[test]
fn pre_v2_task_records_still_load_and_gain_no_spurious_contract_key() {
    let _guard = lock();
    let (home, session) = seed_session();

    // A durable record written before contract v2 has no `contract` field.
    let pre_v2 = json!({
        "id": "T0001",
        "session": session,
        "title": "legacy task",
        "status": "backlog",
        "depends_on": [],
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "history": [],
    });
    let path = task_path(&session, "T0001");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&pre_v2).unwrap()).unwrap();

    let loaded = read_task(&session, "T0001").unwrap();
    assert!(loaded.contract.is_none(), "pre-v2 record must load");

    // Re-serializing an uncontracted task must not invent a `contract` key.
    let round_tripped: Value = serde_json::to_value(&loaded).unwrap();
    assert!(
        round_tripped.get("contract").is_none(),
        "additive serialization leaked a contract key: {round_tripped}"
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn contract_and_unknown_future_fields_survive_read_then_write() {
    let _guard = lock();
    let (home, session) = seed_session();

    let contract = TaskContract {
        objective: "ship the thing".to_owned(),
        allowed_paths: vec!["src/thing.rs".to_owned()],
        forbidden: vec!["no new deps".to_owned()],
        expected_artifact: Some("branch + tests".to_owned()),
        acceptance_checks: vec!["it works".to_owned()],
        reviewer: Some("claude".to_owned()),
        limits: TaskLimits {
            timeout_sec: Some(300),
            max_retries: Some(1),
            ..TaskLimits::default()
        },
        budget: TaskBudget {
            max_tokens: Some(1000),
            max_usd_cents: Some(199),
            ..TaskBudget::default()
        },
        ..TaskContract::default()
    };
    let created = add_task(
        &session,
        TaskActor::Human,
        NewTask {
            title: "contracted task".to_owned(),
            contract: Some(contract.clone()),
            ..NewTask::default()
        },
    )
    .unwrap();
    assert_eq!(
        created.contract.as_ref().unwrap().objective,
        "ship the thing"
    );

    // Inject unknown future fields at both the top level and inside the
    // nested contract, then confirm they survive a read→write cycle.
    let path = task_path(&session, &created.id);
    let mut raw: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    raw["future_top"] = json!({"kept": true});
    raw["contract"]["future_contract"] = json!("v3-field");
    raw["contract"]["limits"]["future_limit"] = json!(42);
    fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

    let reloaded = read_task(&session, &created.id).unwrap();
    assert_eq!(reloaded.extra["future_top"], json!({"kept": true}));
    let reloaded_contract = reloaded.contract.as_ref().unwrap();
    assert_eq!(
        reloaded_contract.extra["future_contract"],
        json!("v3-field")
    );
    assert_eq!(reloaded_contract.limits.extra["future_limit"], json!(42));

    // Write it back out and confirm the unknown fields are still present.
    orc_core::registry::atomic_write_json(&path, &reloaded).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(after["future_top"], json!({"kept": true}));
    assert_eq!(after["contract"]["future_contract"], json!("v3-field"));
    assert_eq!(after["contract"]["limits"]["future_limit"], json!(42));

    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_contract_with_an_objective_but_no_check_is_rejected() {
    let _guard = lock();
    let (home, session) = seed_session();

    let result = add_task(
        &session,
        TaskActor::Human,
        NewTask {
            title: "half a contract".to_owned(),
            contract: Some(TaskContract {
                objective: "do a thing".to_owned(),
                ..TaskContract::default()
            }),
            ..NewTask::default()
        },
    );
    assert!(
        result.is_err(),
        "objective without a check must be rejected"
    );

    let _ = fs::remove_dir_all(Path::new(&home));
}
