#![allow(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use orc_core::registry::{list_runs, read_meta, write_meta};
use serde_json::{Value, json};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-v3")
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[test]
fn every_python_fixture_is_consumed_and_legacy_data_survives() {
    let _guard = env_lock();
    let target = std::env::temp_dir().join(format!("orc-python-oracle-{}", std::process::id()));
    let _ = fs::remove_dir_all(&target);
    copy_tree(&fixture_root().join("home"), &target);
    // SAFETY: this integration test serializes the process-wide environment mutation.
    unsafe { std::env::set_var("ORC_HOME", &target) };

    let oracle: Value =
        serde_json::from_slice(&fs::read(fixture_root().join("oracle.json")).unwrap()).unwrap();
    let commands = oracle["python"].as_array().unwrap();
    assert_eq!(commands.len(), 4);
    assert!(commands.iter().all(|capture| capture["exit"] == 0));
    assert_eq!(commands[0]["args"], json!(["list", "--json"]));
    assert_eq!(
        commands[1]["args"],
        json!(["show", "exact-usage", "--tail", "2"])
    );
    assert_eq!(commands[2]["args"], json!(["stats", "--json"]));
    assert_eq!(commands[3]["args"], json!(["quota", "--json"]));

    let mut fixture_ids = BTreeSet::new();
    for entry in fs::read_dir(target.join("runs")).unwrap() {
        let entry = entry.unwrap();
        let bytes = fs::read(entry.path().join("meta.json")).unwrap();
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => {
                fixture_ids.insert(value["id"].as_str().unwrap().to_owned());
            }
            Err(_) => assert!(matches!(
                entry.file_name().to_str(),
                Some("corrupt" | "truncated")
            )),
        }
    }
    assert_eq!(fixture_ids.len(), 10);

    let runs = list_runs(false).unwrap();
    assert_eq!(runs.len(), fixture_ids.len());
    let found = runs
        .iter()
        .map(|run| run.id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(found, fixture_ids);
    let unicode = runs.iter().find(|run| run.id == "unicode-wide").unwrap();
    assert_eq!(unicode.task, "調査 世界 e\u{301}lan");
    assert_eq!(unicode.session.as_deref(), Some("セッション-界"));
    let legacy = runs.iter().find(|run| run.id == "legacy").unwrap();
    assert_eq!(legacy.extra["legacy_unknown"], 9);

    let current_dir = target.join("runs/current");
    let mut current = read_meta(&current_dir).unwrap();
    current.status = "running".to_owned();
    write_meta(&current_dir, &current).unwrap();
    let rewritten: Value =
        serde_json::from_slice(&fs::read(current_dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(rewritten["future"], json!({"kept": true}));

    let exact_dir = target.join("runs/exact-usage");
    let exact = read_meta(&exact_dir).unwrap();
    write_meta(&exact_dir, &exact).unwrap();
    let rewritten: Value =
        serde_json::from_slice(&fs::read(exact_dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(rewritten["tokens"]["token_future"], "preserve");
    fs::remove_dir_all(target).unwrap();
}
