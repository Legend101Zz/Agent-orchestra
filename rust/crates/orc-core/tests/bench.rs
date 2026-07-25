#![allow(unsafe_code)]

use std::fs;
use std::sync::{Mutex, OnceLock};

use orc_core::bench::{
    HarnessRegistry, create_session, list_sessions, load_harness_registry, read_session,
    write_harness_registry, write_session,
};
use serde_json::json;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[test]
fn harness_defaults_are_editable_additive_and_sessions_write_atomically() {
    let _guard = lock();
    let home = std::env::temp_dir().join(format!("orc-bench-core-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    // SAFETY: this test serializes process-wide environment mutation.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let mut registry = load_harness_registry().unwrap();
    assert_eq!(registry.default_workers, ["hermes", "pi-m3"]);
    assert_eq!(
        registry
            .harnesses
            .get("pi-m3")
            .map(|config| config.dispatch_args.as_slice()),
        Some(["-p".to_owned(), "--no-session".to_owned()].as_slice())
    );
    registry
        .extra
        .insert("future".to_owned(), json!({"kept": true}));
    write_harness_registry(&registry).unwrap();
    assert_eq!(
        load_harness_registry().unwrap().extra["future"],
        json!({"kept": true})
    );

    let mut session = create_session(
        "codex",
        &registry.default_workers,
        std::env::temp_dir().as_path(),
    )
    .unwrap();
    session.extra.insert("future_session".to_owned(), json!(9));
    write_session(&session).unwrap();
    assert_eq!(
        read_session(&session.id).unwrap().extra["future_session"],
        9
    );
    assert_eq!(list_sessions().unwrap().len(), 1);
    assert_eq!(
        fs::read_dir(home.join("sessions").join(&session.id))
            .unwrap()
            .count(),
        1
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn registry_default_has_only_ember_phosphor_compatible_theme() {
    let registry = HarnessRegistry::default();
    assert_eq!(registry.app.theme, "ember");
    assert_eq!(registry.app.leader_key, "ctrl-g");
}

#[test]
fn per_harness_concurrency_cap_is_additive_and_round_trips() {
    let _guard = lock();
    let home = std::env::temp_dir().join(format!("orc-bench-cap-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    // SAFETY: this test serializes process-wide environment mutation.
    unsafe { std::env::set_var("ORC_HOME", &home) };

    // A pre-#7 registry file has no `concurrency` key at all; it must still load
    // (empty map) and preserve any unknown sibling field on round-trip.
    let legacy = json!({
        "harnesses": {},
        "default_workers": ["hermes"],
        "max_parallel_workers": 3,
        "app": {"leader_key": "ctrl-g", "reduced_motion": false, "theme": "ember"},
        "future_top_level": {"kept": true}
    });
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join("harnesses.json"),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let mut registry = load_harness_registry().unwrap();
    assert!(
        registry.concurrency.is_empty(),
        "a registry without a concurrency key loads as an empty override map"
    );
    assert_eq!(registry.extra["future_top_level"], json!({"kept": true}));

    // Setting a per-harness cap round-trips through the additive JSON.
    registry.concurrency.insert("hermes".to_owned(), 2);
    write_harness_registry(&registry).unwrap();
    let reloaded = load_harness_registry().unwrap();
    assert_eq!(reloaded.concurrency.get("hermes"), Some(&2));
    // The unknown sibling field is still preserved alongside the new field.
    assert_eq!(reloaded.extra["future_top_level"], json!({"kept": true}));
    let _ = fs::remove_dir_all(home);
}
