#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use orc_core::model::RunMeta;
use orc_core::registry::{
    NewRunOptions, atomic_write_json, list_runs, make_slug, new_run, read_meta, write_meta,
};
use serde_json::json;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn with_home(test: impl FnOnce(&Path)) {
    let _guard = env_lock();
    let dir = std::env::temp_dir().join(format!(
        "orc-rust-registry-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // SAFETY: tests serialize environment mutation with env_lock.
    unsafe { std::env::set_var("ORC_HOME", &dir) };
    test(&dir);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn slug_matches_compatibility_rules() {
    assert_eq!(make_slug("Hello, WORLD / later"), "hello-world-later");
    assert_eq!(make_slug("---"), "task");
    assert_eq!(
        make_slug("123456789012345678901234Z"),
        "123456789012345678901234"
    );
}

#[test]
fn new_run_uses_compatible_shape_and_omits_optional_fields() {
    with_home(|_| {
        let run = new_run("hello", &NewRunOptions::compatibility_defaults()).unwrap();
        assert!(run.join("inbox").is_dir());
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(run.join("meta.json")).unwrap()).unwrap();
        assert_eq!(value["status"], "starting");
        assert!(value.get("session").is_none());
        assert!(value.get("run_dir").is_none());
        assert_eq!(value["tokens"]["estimated_total"], 0);
    });
}

#[test]
fn unknown_fields_survive_update() {
    with_home(|_| {
        let run = new_run("future", &NewRunOptions::compatibility_defaults()).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(run.join("meta.json")).unwrap()).unwrap();
        value["future_field"] = json!({"kept": true});
        atomic_write_json(&run.join("meta.json"), &value).unwrap();
        let mut meta: RunMeta = read_meta(&run).unwrap();
        meta.status = "running".to_owned();
        write_meta(&run, &meta).unwrap();
        let reread: serde_json::Value =
            serde_json::from_slice(&fs::read(run.join("meta.json")).unwrap()).unwrap();
        assert_eq!(reread["future_field"], json!({"kept": true}));
    });
}

#[test]
fn atomic_write_leaves_no_temp_files() {
    with_home(|home| {
        let path = home.join("value.json");
        atomic_write_json(&path, &json!({"ok": true})).unwrap();
        assert_eq!(fs::read_dir(home).unwrap().count(), 1);
    });
}

#[test]
fn list_tolerates_legacy_meta_and_corrupt_sibling() {
    with_home(|home| {
        let runs = home.join("runs");
        fs::create_dir_all(runs.join("legacy")).unwrap();
        fs::write(
            runs.join("legacy/meta.json"),
            r#"{"id":"legacy","task":"old","brain":"human","cwd":"/tmp","provider":"minimax","model":"MiniMax-M3","status":"done","started_at":"2020-01-01T00:00:00+00:00","tokens":{"estimated_total":4},"unknown":1}"#,
        )
        .unwrap();
        fs::create_dir_all(runs.join("bad")).unwrap();
        fs::write(runs.join("bad/meta.json"), "{").unwrap();
        let found = list_runs(false).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "legacy");
        assert_eq!(
            found[0].extra,
            BTreeMap::from([("unknown".to_owned(), json!(1))])
        );
    });
}
