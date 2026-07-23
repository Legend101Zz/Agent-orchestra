//! PATH auto-discovery of known coding harnesses.
//!
//! Discovery scans `PATH` for a small, extensible set of harness executables
//! and records each hit in the additive `~/.orchestra/harnesses.json` registry
//! under [`HarnessRegistry::discovered`](crate::bench::HarnessRegistry). The
//! record keeps the resolved path, a cheap version string when one is
//! obtainable, and honest first/last-seen timestamps.
//!
//! Two honesty rules hold. Discovery never invents a capability: a version is
//! recorded only when the executable actually printed one within a bounded
//! probe. And discovery never hides a known harness: [`present`] always returns
//! every entry in [`KNOWN_HARNESSES`], marking the missing ones unavailable
//! rather than dropping them.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::adapter::locate_executable;
use crate::bench::{
    DiscoveredHarness, HarnessRegistry, load_harness_registry, read_harness_registry,
    write_harness_registry,
};
use crate::quota::command_output_with_timeout;
use crate::registry::now_iso;

/// Extensible list of harness executables auto-discovery scans for.
pub const KNOWN_HARNESSES: &[&str] = &["claude", "codex", "hermes", "pi", "opencode"];

/// Bounded upper limit for one `--version` probe.
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Cap on a recorded version string, in characters.
const VERSION_MAX_CHARS: usize = 80;

/// One presented harness row: current availability plus recorded history.
///
/// This is the read-only view surfaced by `pio harness list` and the HOME
/// availability strip. `available` and `path` reflect the current `PATH`
/// resolution; `version`, `first_seen`, and `last_seen` come from the persisted
/// discovery record when one exists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessDiscovery {
    /// Harness executable name from [`KNOWN_HARNESSES`].
    pub name: String,
    /// Whether the executable currently resolves on `PATH`.
    pub available: bool,
    /// Currently resolved absolute path when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Cheap version string recorded by the last successful probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// First time this harness was ever discovered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<String>,
    /// Most recent time this harness was discovered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

/// Scan `PATH`, persist an additive discovery record per hit, and present all.
///
/// When `probe_versions` is true, each resolved executable is invoked once with
/// `--version` under a bounded timeout; failures fall back to any previously
/// recorded version. Records for harnesses no longer on `PATH` are preserved,
/// never deleted. Unknown fields in the existing registry survive the write.
pub fn discover(probe_versions: bool) -> Result<Vec<HarnessDiscovery>> {
    let mut registry = load_harness_registry()?;
    let now = now_iso();
    for name in KNOWN_HARNESSES {
        let Some(path) = locate_executable(name) else {
            continue;
        };
        let path_str = path.to_string_lossy().into_owned();
        let stored_version = registry
            .discovered
            .get(*name)
            .and_then(|record| record.version.clone());
        let version = if probe_versions {
            probe_version(&path).or(stored_version)
        } else {
            stored_version
        };
        record_discovery(&mut registry.discovered, name, path_str, version, &now);
    }
    write_harness_registry(&registry)?;
    Ok(present(&registry))
}

/// Present every known harness with current availability and recorded history.
#[must_use]
pub fn present(registry: &HarnessRegistry) -> Vec<HarnessDiscovery> {
    KNOWN_HARNESSES
        .iter()
        .map(|name| {
            let stored = registry.discovered.get(*name);
            let current = locate_executable(name);
            HarnessDiscovery {
                name: (*name).to_owned(),
                available: current.is_some(),
                path: current.map(|path| path.to_string_lossy().into_owned()),
                version: stored.and_then(|record| record.version.clone()),
                first_seen: stored.map(|record| record.first_seen.clone()),
                last_seen: stored.map(|record| record.last_seen.clone()),
            }
        })
        .collect()
}

/// Present known harnesses from the persisted registry without side effects.
///
/// Read-only: when `harnesses.json` is absent it presents current `PATH`
/// availability against an empty history rather than writing the defaults file.
#[must_use]
pub fn present_current() -> Vec<HarnessDiscovery> {
    let registry = read_harness_registry().ok().flatten().unwrap_or_default();
    present(&registry)
}

/// Upsert one discovery record: set `first_seen` once, refresh path/last_seen.
fn record_discovery(
    discovered: &mut BTreeMap<String, DiscoveredHarness>,
    name: &str,
    path: String,
    version: Option<String>,
    now: &str,
) {
    match discovered.get_mut(name) {
        Some(existing) => {
            existing.path = path;
            existing.version = version;
            existing.last_seen = now.to_owned();
        }
        None => {
            discovered.insert(
                name.to_owned(),
                DiscoveredHarness {
                    path,
                    version,
                    first_seen: now.to_owned(),
                    last_seen: now.to_owned(),
                    probe: None,
                    extra: BTreeMap::new(),
                },
            );
        }
    }
}

/// Best-effort cheap version string from one bounded `--version` invocation.
///
/// Returns `None` unless the command exits successfully, so a harness that
/// rejects `--version` (non-zero exit) never has its stderr error text recorded
/// as a "version". This upholds the module contract ("a version is recorded
/// only when the executable actually printed one") and AGENTS.md's invariant
/// ("never claim a capability that wasn't probed"). On `None`, [`discover`]
/// falls back to any previously stored version rather than overwriting it.
fn probe_version(path: &Path) -> Option<String> {
    let mut command = Command::new(path);
    command.arg("--version");
    let output = command_output_with_timeout(&mut command, VERSION_PROBE_TIMEOUT).ok()??;
    if !output.status.success() {
        return None;
    }
    let raw = if output.stdout.iter().any(|byte| !byte.is_ascii_whitespace()) {
        output.stdout
    } else {
        output.stderr
    };
    let text = String::from_utf8_lossy(&raw);
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(line.chars().take(VERSION_MAX_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::{HarnessRegistry, present, record_discovery};
    use crate::bench::DiscoveredHarness;
    use std::collections::BTreeMap;

    #[test]
    fn record_discovery_sets_first_seen_once_and_refreshes_last_seen() {
        let mut discovered = BTreeMap::new();
        record_discovery(
            &mut discovered,
            "pi",
            "/old/bin/pi".to_owned(),
            Some("pi 1.0".to_owned()),
            "2026-07-01T00:00:00+00:00",
        );
        record_discovery(
            &mut discovered,
            "pi",
            "/new/bin/pi".to_owned(),
            Some("pi 2.0".to_owned()),
            "2026-07-23T00:00:00+00:00",
        );
        let record = &discovered["pi"];
        assert_eq!(record.first_seen, "2026-07-01T00:00:00+00:00");
        assert_eq!(record.last_seen, "2026-07-23T00:00:00+00:00");
        assert_eq!(record.path, "/new/bin/pi");
        assert_eq!(record.version.as_deref(), Some("pi 2.0"));
    }

    #[test]
    fn registry_round_trip_preserves_unknown_fields() {
        // A pre-existing registry with unknown fields at every additive layer.
        let json = r#"{
            "harnesses": {},
            "default_workers": [],
            "max_parallel_workers": 3,
            "app": { "leader_key": "ctrl-g", "reduced_motion": false, "theme": "ember", "future_app": 7 },
            "discovered": {
                "pi": {
                    "path": "/usr/bin/pi",
                    "version": "pi 9.9",
                    "first_seen": "2026-01-01T00:00:00+00:00",
                    "last_seen": "2026-01-02T00:00:00+00:00",
                    "capabilities": ["steer", "exact_usage"]
                }
            },
            "future_top_level": {"kept": true}
        }"#;
        let registry: HarnessRegistry = serde_json::from_str(json).expect("parse registry");
        // Unknown fields survived into the typed model.
        assert!(registry.extra.contains_key("future_top_level"));
        assert!(registry.app.extra.contains_key("future_app"));
        let pi = &registry.discovered["pi"];
        assert!(pi.extra.contains_key("capabilities"));
        assert_eq!(pi.first_seen, "2026-01-01T00:00:00+00:00");

        // And they round-trip back out on re-serialization.
        let reserialized = serde_json::to_string(&registry).expect("serialize registry");
        assert!(reserialized.contains("future_top_level"));
        assert!(reserialized.contains("future_app"));
        assert!(reserialized.contains("capabilities"));
    }

    #[test]
    fn present_returns_every_known_harness_and_merges_history() {
        // History for one harness plus the default (empty) discovery map.
        let registry = HarnessRegistry {
            discovered: BTreeMap::from([(
                "pi".to_owned(),
                DiscoveredHarness {
                    path: "/usr/bin/pi".to_owned(),
                    version: Some("pi 9.9".to_owned()),
                    first_seen: "2026-01-01T00:00:00+00:00".to_owned(),
                    last_seen: "2026-01-02T00:00:00+00:00".to_owned(),
                    probe: None,
                    extra: BTreeMap::new(),
                },
            )]),
            ..HarnessRegistry::default()
        };
        let rows = present(&registry);
        // Every known harness is present, in order, never hidden.
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, super::KNOWN_HARNESSES);
        // Recorded history is surfaced regardless of current PATH availability.
        let pi = rows.iter().find(|row| row.name == "pi").expect("pi row");
        assert_eq!(pi.version.as_deref(), Some("pi 9.9"));
        assert_eq!(pi.first_seen.as_deref(), Some("2026-01-01T00:00:00+00:00"));
        assert_eq!(pi.last_seen.as_deref(), Some("2026-01-02T00:00:00+00:00"));
        // A harness with no record still appears, with empty history.
        let opencode = rows
            .iter()
            .find(|row| row.name == "opencode")
            .expect("opencode row");
        assert!(opencode.first_seen.is_none());
    }
}
