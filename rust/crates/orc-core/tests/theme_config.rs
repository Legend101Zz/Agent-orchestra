#![allow(unsafe_code)]

//! One theme, one authoritative record.
//!
//! Before issue #37 `theme` lived in two files that nothing kept in step:
//! `pio config set theme` wrote `config.json` while the client rendered
//! `harnesses.json`'s `app.theme`, so the CLI could report a palette the TUI
//! had never heard of. These tests pin the settled shape: the registry is
//! authoritative, `config.json` keeps a derived copy for readers that predate
//! the decision, and no reader can be handed a stale value.

use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use orc_core::bench::{load_harness_registry, read_harness_registry, write_harness_registry};
use orc_core::control::{read_config_value, resolve_theme, set_config, set_theme, theme};
use serde_json::{Value, json};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn with_home(name: &str, test: impl FnOnce(&Path)) {
    let _guard = env_lock();
    let dir = std::env::temp_dir().join(format!("orc-theme-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test home");
    // SAFETY: every test in this file serializes environment mutation on
    // `env_lock`, and each one owns its own directory.
    unsafe { std::env::set_var("ORC_HOME", &dir) };
    test(&dir);
    let _ = fs::remove_dir_all(&dir);
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read json")).expect("parse json")
}

#[test]
fn an_unknown_or_malformed_name_resolves_to_the_flagship() {
    for name in ["nocturne", "NOCTURNE", " Nocturne "] {
        assert_eq!(resolve_theme(name), "nocturne", "{name:?}");
    }
    assert_eq!(resolve_theme("Ember"), "ember");
    assert_eq!(resolve_theme("phosphor"), "phosphor");
    // Anything else is decoration nobody configured on purpose: answer the
    // flagship rather than failing to render.
    for name in ["", "   ", "teal", "ember-dark", "nocturn", "🎨"] {
        assert_eq!(resolve_theme(name), "nocturne", "{name:?}");
    }
}

#[test]
fn set_theme_writes_the_registry_and_derives_the_config_copy() {
    with_home("write", |home| {
        let written = set_theme("phosphor").expect("persist theme");
        assert_eq!(written, "phosphor");

        let registry = read_harness_registry()
            .expect("read registry")
            .expect("registry exists");
        assert_eq!(
            registry.app.theme, "phosphor",
            "the registry is the authoritative record"
        );
        assert_eq!(
            read_json(&home.join("config.json"))["theme"],
            json!("phosphor"),
            "config.json keeps a derived copy so old readers agree"
        );
        assert_eq!(theme(), "phosphor");
        assert_eq!(read_config_value()["theme"], json!("phosphor"));

        // Atomic write: nothing is left behind in the home directory.
        let leftovers: Vec<_> = fs::read_dir(home)
            .expect("scan home")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files survived: {leftovers:?}");
    });
}

#[test]
fn the_registry_wins_when_the_two_files_disagree() {
    with_home("disagree", |home| {
        set_theme("phosphor").expect("persist theme");
        // Simulate the pre-#37 world: something wrote config.json directly.
        let mut config = read_json(&home.join("config.json"));
        config["theme"] = json!("ember");
        fs::write(
            home.join("config.json"),
            serde_json::to_vec(&config).expect("encode config"),
        )
        .expect("write stale config");

        assert_eq!(theme(), "phosphor", "the registry is authoritative");
        assert_eq!(
            read_config_value()["theme"],
            json!("phosphor"),
            "no reader of config.json may be handed the stale copy"
        );
    });
}

#[test]
fn a_config_that_predates_this_change_still_loads_and_keeps_its_fields() {
    with_home("legacy", |home| {
        // A machine installed before #37: config.json only, carrying a theme
        // and a field this build has never heard of.
        fs::write(
            home.join("config.json"),
            serde_json::to_vec_pretty(&json!({
                "warn_pct": 25,
                "block_pct": 10,
                "theme": "ember",
                "some_future_key": {"nested": true},
            }))
            .expect("encode legacy config"),
        )
        .expect("write legacy config");
        assert!(
            !home.join("harnesses.json").exists(),
            "fixture must start without a registry"
        );

        assert_eq!(
            theme(),
            "ember",
            "an install with no registry keeps the choice it already had"
        );

        set_theme("nocturne").expect("persist theme");
        let config = read_json(&home.join("config.json"));
        assert_eq!(config["theme"], json!("nocturne"));
        assert_eq!(
            config["some_future_key"],
            json!({"nested": true}),
            "unknown fields survive the write"
        );
        assert_eq!(config["warn_pct"], json!(25));
    });
}

#[test]
fn unknown_registry_fields_survive_a_theme_write() {
    with_home("registry-additive", |home| {
        let mut registry = load_harness_registry().expect("seed registry");
        registry
            .extra
            .insert("future_top_level".to_owned(), json!(["keep", "me"]));
        registry
            .app
            .extra
            .insert("future_app_key".to_owned(), json!(7));
        write_harness_registry(&registry).expect("write seeded registry");

        set_theme("ember").expect("persist theme");

        let raw = read_json(&home.join("harnesses.json"));
        assert_eq!(raw["app"]["theme"], json!("ember"));
        assert_eq!(raw["future_top_level"], json!(["keep", "me"]));
        assert_eq!(raw["app"]["future_app_key"], json!(7));
    });
}

#[test]
fn set_config_theme_routes_to_the_authoritative_record() {
    with_home("set-config", |home| {
        // This is the path `pio config set theme <x>` takes. Before #37 it
        // wrote config.json and the client never saw it.
        let reported = set_config("theme", "phosphor").expect("set theme through config");
        assert_eq!(reported["theme"], json!("phosphor"));
        assert_eq!(
            read_harness_registry()
                .expect("read registry")
                .expect("registry exists")
                .app
                .theme,
            "phosphor",
            "`config set theme` must reach what the client renders"
        );

        // An unknown name is resolved on the way *in*, not just on the way
        // out: a durable record must never hold a name nothing can render.
        set_config("theme", "chartreuse").expect("set unknown theme");
        assert_eq!(theme(), "nocturne");
        assert_eq!(
            read_json(&home.join("harnesses.json"))["app"]["theme"],
            json!("nocturne"),
            "an unknown name must not be stored verbatim"
        );

        // Setting an unrelated key leaves the theme alone and keeps the
        // derived copy fresh.
        set_config("warn_pct", "40").expect("set warn_pct");
        let config = read_json(&home.join("config.json"));
        assert_eq!(config["warn_pct"], json!(40));
        assert_eq!(config["theme"], json!("nocturne"));
    });
}
