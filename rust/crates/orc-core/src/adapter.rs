//! Honest capability declarations for supported harness adapters.
//!
//! The registry remains user-editable, but an adapter name alone must not imply
//! a capability. This module records the small set of interfaces that have
//! been verified against local command help and exposes their degradations to
//! the CLI and documentation. Runtime receipt is still recorded only by the
//! bounded dispatch and runner paths.

use std::path::PathBuf;

use serde::Serialize;

use crate::bench::{HarnessConfig, HarnessRegistry};

/// Static capabilities of one known harness adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AdapterCapabilities {
    /// Whether the harness is suitable for an interactive Bench pane.
    pub interactive_pane: bool,
    /// Whether the harness has a verified non-interactive prompt interface.
    pub headless_delivery: bool,
    /// Whether the orchestration runner can deliver follow-up prompts.
    pub steerable: bool,
    /// Whether a completed run can report exact provider usage.
    pub exact_usage: bool,
    /// Plain-language constraint that callers must preserve.
    pub degradation: String,
}

/// Availability and declared capabilities for one configured harness.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AdapterSummary {
    /// User-editable harness key from `harnesses.json`.
    pub harness: String,
    /// Adapter identifier declared by that harness.
    pub adapter: String,
    /// Configured executable.
    pub command: String,
    /// Resolved executable when it is present on `PATH`.
    pub executable: Option<String>,
    /// Whether the harness can be launched in an interactive Bench pane.
    pub interactive_pane: bool,
    /// Whether the adapter and configuration together permit bounded dispatch.
    pub headless_delivery: bool,
    /// Whether `orc rpc` supports durable follow-up prompts for this adapter.
    pub steerable: bool,
    /// Whether a successful run may record exact usage if the provider emits it.
    pub exact_usage: bool,
    /// Plain-language capability/degradation explanation.
    pub degradation: String,
}

/// Return static capability declarations for a known adapter.
#[must_use]
pub fn capabilities(adapter: &str) -> AdapterCapabilities {
    match adapter {
        "hermes" => AdapterCapabilities {
            interactive_pane: true,
            headless_delivery: true,
            steerable: false,
            exact_usage: false,
            degradation: "Hermes can receive a bounded --oneshot brief, but has no verified durable steering or exact-usage event.".to_owned(),
        },
        "pi" => AdapterCapabilities {
            interactive_pane: true,
            headless_delivery: true,
            steerable: true,
            exact_usage: true,
            degradation: "Pi steering is available only through a live RPC run; exact usage is recorded only when its completed event contains usage.".to_owned(),
        },
        _ => AdapterCapabilities {
            interactive_pane: true,
            headless_delivery: false,
            steerable: false,
            exact_usage: false,
            degradation: "No verified non-interactive, steering, or exact-usage adapter is installed for this harness.".to_owned(),
        },
    }
}

/// Resolve a configured executable without invoking it.
#[must_use]
pub fn locate_executable(command: &str) -> Option<PathBuf> {
    if command.contains('/') {
        let path = PathBuf::from(command);
        return path.is_file().then_some(path);
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|directory| directory.join(command))
        .find(|candidate| candidate.is_file())
}

fn command_matches_adapter(adapter: &str, command: &str) -> bool {
    let expected = match adapter {
        "hermes" => "hermes",
        "pi" => "pi",
        _ => return true,
    };
    PathBuf::from(command)
        .file_name()
        .is_some_and(|name| name == expected)
}

/// Summarize one configured harness without executing it.
#[must_use]
pub fn summarize_harness(harness: &str, config: &HarnessConfig) -> AdapterSummary {
    let declared = capabilities(&config.adapter);
    let executable = locate_executable(&config.command);
    let configured_dispatch = !config.dispatch_args.is_empty();
    let verified_command = command_matches_adapter(&config.adapter, &config.command);
    let headless_delivery = declared.headless_delivery
        && configured_dispatch
        && executable.is_some()
        && verified_command;
    let mut degradation = declared.degradation;
    if executable.is_none() {
        degradation.push_str(" Executable is unavailable on PATH.");
    } else if !verified_command {
        degradation.push_str(
            " The configured command does not match this adapter's verified executable name.",
        );
    } else if declared.headless_delivery && !configured_dispatch {
        degradation
            .push_str(" The registry has no dispatch_args, so bounded delivery is unavailable.");
    }
    AdapterSummary {
        harness: harness.to_owned(),
        adapter: config.adapter.clone(),
        command: config.command.clone(),
        executable: executable
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        interactive_pane: declared.interactive_pane && executable.is_some() && verified_command,
        headless_delivery,
        steerable: declared.steerable && executable.is_some() && verified_command,
        exact_usage: declared.exact_usage && executable.is_some() && verified_command,
        degradation,
    }
}

/// Summarize every configured harness in stable key order.
#[must_use]
pub fn summarize_registry(registry: &HarnessRegistry) -> Vec<AdapterSummary> {
    registry
        .harnesses
        .iter()
        .map(|(harness, config)| summarize_harness(harness, config))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::capabilities;

    #[test]
    fn verified_adapters_degrade_without_inventing_support() {
        let hermes = capabilities("hermes");
        assert!(hermes.headless_delivery);
        assert!(!hermes.steerable);
        assert!(!hermes.exact_usage);

        let pi = capabilities("pi");
        assert!(pi.headless_delivery);
        assert!(pi.steerable);
        assert!(pi.exact_usage);

        let unknown = capabilities("unknown");
        assert!(!unknown.headless_delivery);
        assert!(!unknown.steerable);
        assert!(!unknown.exact_usage);
    }
}
