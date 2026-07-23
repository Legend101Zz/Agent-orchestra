//! Downstream capability API coverage for `pio doctor` (issue #4).
//!
//! Acceptance check 4: a harness that fails a probe (or was never probed) is
//! never offered a capability anywhere downstream. These tests exercise the
//! persisted-read API [`orc_core::probe::probed_capabilities`] /
//! [`orc_core::probe::has_capability`] against a seeded `harnesses.json`.

#![allow(unsafe_code)]

use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use orc_core::probe::{Capability, has_capability, probed_capabilities};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn with_home(registry_json: &str, test: impl FnOnce()) {
    let _guard = env_lock();
    let dir = std::env::temp_dir().join(format!(
        "orc-core-probe-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("harnesses.json"), registry_json).unwrap();
    // SAFETY: env mutation is serialized by env_lock.
    unsafe { std::env::set_var("ORC_HOME", &dir) };
    test();
    let _ = fs::remove_dir_all(&dir);
}

/// A registry where `pi` probed two known caps plus one unknown slug, `claude`
/// probed but failed (empty set + error), and `codex` was never probed.
fn seeded() -> String {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/fixtures/probed-harnesses.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
    let discovered = value["discovered"].as_object_mut().unwrap();
    // claude: probed, but the probe failed — empty capabilities plus an error.
    discovered.insert(
        "claude".to_owned(),
        serde_json::json!({
            "path": "/bin/claude",
            "first_seen": "2026-01-01T00:00:00+00:00",
            "last_seen": "2026-01-01T00:00:00+00:00",
            "probe": {
                "probed_at": "2026-01-01T00:00:00+00:00",
                "binary": { "path": "/bin/claude", "mtime_ns": 1, "size": 2 },
                "capabilities": [],
                "error": "no --help output within probe timeout"
            }
        }),
    );
    // codex: discovered but NEVER probed (no probe field at all).
    discovered.insert(
        "codex".to_owned(),
        serde_json::json!({
            "path": "/bin/codex",
            "first_seen": "2026-01-01T00:00:00+00:00",
            "last_seen": "2026-01-01T00:00:00+00:00"
        }),
    );
    serde_json::to_string_pretty(&value).unwrap()
}

#[test]
fn probed_capabilities_returns_only_known_probed_capabilities() {
    with_home(&seeded(), || {
        let pi = probed_capabilities("pi");
        // The two known probed capabilities are offered.
        assert!(pi.contains(&Capability::NonInteractive));
        assert!(pi.contains(&Capability::Resume));
        // The unknown "telepathy" slug is dropped, not offered as anything.
        assert_eq!(pi.len(), 2);
        // Capabilities pi did not probe are never offered.
        assert!(!has_capability("pi", Capability::Tools));
        assert!(has_capability("pi", Capability::Resume));
    });
}

#[test]
fn failed_probe_offers_no_capability_downstream() {
    with_home(&seeded(), || {
        // claude's probe failed: it must offer nothing, for every capability.
        assert!(probed_capabilities("claude").is_empty());
        for capability in Capability::ALL {
            assert!(
                !has_capability("claude", *capability),
                "failed probe must not offer {}",
                capability.slug()
            );
        }
    });
}

#[test]
fn never_probed_or_unknown_harness_offers_nothing() {
    with_home(&seeded(), || {
        // codex was discovered but never probed.
        assert!(probed_capabilities("codex").is_empty());
        // opencode is not in the registry at all.
        assert!(probed_capabilities("opencode").is_empty());
        assert!(!has_capability("codex", Capability::NonInteractive));
    });
}
