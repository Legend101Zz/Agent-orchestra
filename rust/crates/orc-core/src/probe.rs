//! Capability probes and the honest `pio doctor` report (issue #4).
//!
//! Executable presence (issue #3) is not the same as capability. This module
//! probes each discovered harness for the eight capabilities the V1 spec names
//! and records the verified set — with a probe timestamp and the probed binary's
//! identity — under `discovered.<name>.probe` in the additive
//! `~/.orchestra/harnesses.json` registry.
//!
//! ## How a capability is probed
//!
//! Probing inspects what the *binary itself advertises*, never a version pin
//! (the binding decision from issue #16, §2): each harness is invoked with a
//! bounded `--help` corpus and the output is scanned for the flags/subcommands
//! that prove a capability. This is cheap, deterministic, spends no provider
//! tokens, and — per AGENTS.md — never claims a capability that was not probed.
//! Cancellation is orchestrator-provided (pi-orchestra spawns, bounds, and
//! kills the child), so it is derived: available exactly when the harness can be
//! driven non-interactively. The exact per-harness signal strings are the
//! documented follow-up of the decision record's open question 1 (feeding #7);
//! this module is the mechanism that captures them.
//!
//! ## Honesty rules
//!
//! * A harness whose probe fails records an empty capability set plus an
//!   `error`, so [`probed_capabilities`] offers it nothing downstream.
//! * The persisted set is stored as string slugs, so a reader from a future or
//!   past pio tolerates capability names it does not know ([`CapabilityProbe::typed`]
//!   simply drops the unknown ones) — the same additive contract the rest of the
//!   registry follows.
//! * The cache is keyed on binary identity (path + mtime + size); a reinstalled
//!   or upgraded binary invalidates it and forces a fresh probe.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::locate_executable;
use crate::bench::{
    DiscoveredHarness, load_harness_registry, read_harness_registry, write_harness_registry,
};
use crate::discovery::KNOWN_HARNESSES;
use crate::quota::command_output_with_timeout;
use crate::registry::now_iso;

mod profiles;
use profiles::{ProbeProfile, profile_for};

/// Bounded upper limit for one `--help` capability probe invocation.
const HELP_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// One probed harness capability, in the spec's declared order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    /// Non-interactive (one-shot) invocation.
    NonInteractive,
    /// Continuation / resume of a prior session.
    Resume,
    /// Tool access / permission control for unattended runs.
    Tools,
    /// Model selection.
    ModelSelect,
    /// Structured (machine-readable) output.
    StructuredOutput,
    /// Usage / cost reporting.
    UsageReporting,
    /// Timeout / cancellation (orchestrator-provided; derived).
    Cancellation,
    /// Working-directory control.
    WorkingDir,
}

impl Capability {
    /// Every capability in the spec's declared order.
    pub const ALL: &'static [Self] = &[
        Self::NonInteractive,
        Self::Resume,
        Self::Tools,
        Self::ModelSelect,
        Self::StructuredOutput,
        Self::UsageReporting,
        Self::Cancellation,
        Self::WorkingDir,
    ];

    /// Durable snake_case slug persisted in the registry.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::NonInteractive => "non_interactive",
            Self::Resume => "resume",
            Self::Tools => "tools",
            Self::ModelSelect => "model_select",
            Self::StructuredOutput => "structured_output",
            Self::UsageReporting => "usage_reporting",
            Self::Cancellation => "cancellation",
            Self::WorkingDir => "working_dir",
        }
    }

    /// Short column label for the `pio doctor` capability matrix.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NonInteractive => "1shot",
            Self::Resume => "resume",
            Self::Tools => "tools",
            Self::ModelSelect => "model",
            Self::StructuredOutput => "json",
            Self::UsageReporting => "usage",
            Self::Cancellation => "cancel",
            Self::WorkingDir => "cwd",
        }
    }

    /// Parse a durable slug, returning `None` for slugs this build does not know.
    #[must_use]
    pub fn parse(slug: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|cap| cap.slug() == slug)
    }
}

/// Identity of a probed binary, used to invalidate the capability cache.
///
/// A reinstall, upgrade, or move changes at least one of these fields, which is
/// enough to force a fresh probe on the next `pio doctor` run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryIdentity {
    /// Absolute executable path at probe time.
    pub path: String,
    /// Modification time in nanoseconds since the Unix epoch (0 when unknown).
    pub mtime_ns: u128,
    /// Executable size in bytes.
    pub size: u64,
    /// Unknown future fields, preserved verbatim across round-trips.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl BinaryIdentity {
    /// Read the current identity of the executable at `path`.
    #[must_use]
    pub fn current(path: &Path) -> Self {
        let metadata = std::fs::metadata(path).ok();
        let mtime_ns = metadata
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        Self {
            path: path.to_string_lossy().into_owned(),
            mtime_ns,
            size: metadata.map_or(0, |meta| meta.len()),
            extra: BTreeMap::new(),
        }
    }

    /// Whether two identities describe the same binary (ignoring unknown fields).
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        self.path == other.path && self.mtime_ns == other.mtime_ns && self.size == other.size
    }
}

/// One cached capability probe result for a discovered harness.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityProbe {
    /// ISO-8601 timestamp of when this probe ran.
    pub probed_at: String,
    /// Identity of the binary that was probed.
    pub binary: BinaryIdentity,
    /// Verified capability slugs (stored as strings for forward tolerance).
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
    /// Why the probe found nothing, when it failed to run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Unknown future fields, preserved verbatim across round-trips.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl CapabilityProbe {
    /// Typed capabilities, silently dropping any slug this build does not know.
    #[must_use]
    pub fn typed(&self) -> BTreeSet<Capability> {
        self.capabilities
            .iter()
            .filter_map(|slug| Capability::parse(slug))
            .collect()
    }
}

/// Split a help corpus into case-sensitive words for flag membership tests.
fn token_set(corpus: &str) -> HashSet<&str> {
    corpus
        .split(|ch: char| ch.is_whitespace() || ",=<>[](){}|\"'`:".contains(ch))
        .filter(|word| !word.is_empty())
        .collect()
}

/// Detect capabilities from a help corpus using one harness's proof tokens.
///
/// Pure and side-effect free so the token tables can be unit-tested against
/// synthetic help text without spawning anything.
fn detect(corpus: &str, profile: &ProbeProfile) -> BTreeSet<Capability> {
    let words = token_set(corpus);
    let mut capabilities: BTreeSet<Capability> = profile
        .flags
        .iter()
        .filter(|(_, tokens)| tokens.iter().any(|token| words.contains(*token)))
        .map(|(capability, _)| *capability)
        .collect();
    // Cancellation is orchestrator-provided: we can bound and kill exactly those
    // harnesses we can drive non-interactively.
    if capabilities.contains(&Capability::NonInteractive) {
        capabilities.insert(Capability::Cancellation);
    }
    capabilities
}

/// Probe one resolved executable, spawning its bounded `--help` corpus.
fn probe_binary(path: &Path, profile: &ProbeProfile) -> CapabilityProbe {
    let binary = BinaryIdentity::current(path);
    let mut corpus = String::new();
    let mut any_help = false;
    for argv in profile.help_argvs {
        let mut command = Command::new(path);
        command.args(*argv);
        if let Ok(Some(output)) = command_output_with_timeout(&mut command, HELP_PROBE_TIMEOUT) {
            any_help = true;
            corpus.push_str(&String::from_utf8_lossy(&output.stdout));
            corpus.push('\n');
            corpus.push_str(&String::from_utf8_lossy(&output.stderr));
            corpus.push('\n');
        }
    }
    if !any_help {
        return CapabilityProbe {
            probed_at: now_iso(),
            binary,
            capabilities: BTreeSet::new(),
            error: Some("no --help output within probe timeout".to_owned()),
            extra: BTreeMap::new(),
        };
    }
    CapabilityProbe {
        probed_at: now_iso(),
        binary,
        capabilities: detect(&corpus, profile)
            .iter()
            .map(|capability| capability.slug().to_owned())
            .collect(),
        error: None,
        extra: BTreeMap::new(),
    }
}

/// One presented harness row for `pio doctor`: probed capabilities and roles.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessReport {
    /// Harness executable name from [`KNOWN_HARNESSES`].
    pub name: String,
    /// Human-facing display name.
    pub display: String,
    /// Whether the executable currently resolves on `PATH`.
    pub available: bool,
    /// Resolved absolute path when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Recorded version string from discovery, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Highest applicable role: `conductor`, `worker`, `limited`, or `unavailable`.
    pub role: String,
    /// Every role this harness's probed capabilities support.
    pub roles: Vec<String>,
    /// One-line plain-language capability summary.
    pub summary: String,
    /// Every known capability mapped to whether this harness has it.
    pub capabilities: BTreeMap<String, bool>,
    /// Why the probe found nothing, when it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
    /// When this harness was last probed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probed_at: Option<String>,
}

/// Options controlling a `pio doctor` run.
#[derive(Clone, Copy, Debug, Default)]
pub struct DoctorOptions {
    /// Re-probe every available harness even when its binary identity matches.
    pub refresh: bool,
}

/// Probe every known harness and present an honest capability report.
///
/// Uses the cached probe when the binary identity still matches (unless
/// [`DoctorOptions::refresh`]); otherwise probes afresh and persists the result
/// additively. Harnesses no longer on `PATH` are presented as unavailable,
/// never hidden.
pub fn doctor(options: &DoctorOptions) -> Result<Vec<HarnessReport>> {
    let mut registry = load_harness_registry()?;
    let mut dirty = false;
    let mut reports = Vec::new();
    for name in KNOWN_HARNESSES {
        let profile = profile_for(name);
        let current = locate_executable(name);
        let stored = registry.discovered.get(*name);
        let version = stored.and_then(|record| record.version.clone());
        // Clone the stored probe out so the immutable borrow ends before the
        // mutable `upsert_probe` below (also lets us surface last-known
        // capabilities for a harness that has since left PATH).
        let stored_probe = stored.and_then(|record| record.probe.clone());
        let probe = match &current {
            Some(path) => {
                let identity = BinaryIdentity::current(path);
                let fresh = match &stored_probe {
                    Some(probe) if !options.refresh && probe.binary.matches(&identity) => {
                        probe.clone()
                    }
                    _ => {
                        let probe = probe_binary(path, &profile);
                        upsert_probe(&mut registry.discovered, name, path, probe.clone());
                        dirty = true;
                        probe
                    }
                };
                Some(fresh)
            }
            // Unavailable now, but keep last-known probe as honest history.
            None => stored_probe,
        };
        reports.push(build_report(
            name,
            profile.display,
            current,
            version,
            probe.as_ref(),
        ));
    }
    if dirty {
        write_harness_registry(&registry)?;
    }
    Ok(reports)
}

/// Upsert one probe record: refresh path/last_seen, set the probe, keep history.
fn upsert_probe(
    discovered: &mut BTreeMap<String, DiscoveredHarness>,
    name: &str,
    path: &Path,
    probe: CapabilityProbe,
) {
    let now = now_iso();
    match discovered.get_mut(name) {
        Some(existing) => {
            existing.path = path.to_string_lossy().into_owned();
            existing.last_seen = now;
            existing.probe = Some(probe);
        }
        None => {
            discovered.insert(
                name.to_owned(),
                DiscoveredHarness {
                    path: path.to_string_lossy().into_owned(),
                    version: None,
                    first_seen: now.clone(),
                    last_seen: now,
                    probe: Some(probe),
                    extra: BTreeMap::new(),
                },
            );
        }
    }
}

/// Assemble one presented report row from a resolved harness and its probe.
fn build_report(
    name: &str,
    display: &str,
    path: Option<PathBuf>,
    version: Option<String>,
    probe: Option<&CapabilityProbe>,
) -> HarnessReport {
    let available = path.is_some();
    let typed = probe.map(CapabilityProbe::typed).unwrap_or_default();
    let capabilities = Capability::ALL
        .iter()
        .map(|capability| (capability.slug().to_owned(), typed.contains(capability)))
        .collect();
    let roles = roles_for(&typed);
    HarnessReport {
        name: name.to_owned(),
        display: display.to_owned(),
        available,
        path: path.map(|path| path.to_string_lossy().into_owned()),
        version,
        role: primary_role(available, &roles),
        roles,
        summary: summary_for(available, probe, &typed),
        capabilities,
        probe_error: probe.and_then(|probe| probe.error.clone()),
        probed_at: probe.map(|probe| probe.probed_at.clone()),
    }
}

/// Every role a capability set supports, in precedence order.
///
/// A conductor must both hold a durable session ([`Capability::Resume`]) and
/// read machine-readable worker results ([`Capability::StructuredOutput`]) to
/// orchestrate; tool access alone describes a worker that acts, not one that
/// coordinates. A worker only needs non-interactive dispatch.
fn roles_for(capabilities: &BTreeSet<Capability>) -> Vec<String> {
    let mut roles = Vec::new();
    let conductor = capabilities.contains(&Capability::Resume)
        && capabilities.contains(&Capability::StructuredOutput);
    if conductor {
        roles.push("conductor".to_owned());
    }
    if capabilities.contains(&Capability::NonInteractive) {
        roles.push("worker".to_owned());
    }
    roles
}

/// The single role shown in the summary table's role column.
fn primary_role(available: bool, roles: &[String]) -> String {
    if !available {
        return "unavailable".to_owned();
    }
    roles
        .first()
        .cloned()
        .unwrap_or_else(|| "limited".to_owned())
}

/// One-line plain-language capability summary for the report.
fn summary_for(
    available: bool,
    probe: Option<&CapabilityProbe>,
    capabilities: &BTreeSet<Capability>,
) -> String {
    if !available {
        return "not on PATH".to_owned();
    }
    if let Some(error) = probe.and_then(|probe| probe.error.as_ref()) {
        return format!("probe failed: {error}");
    }
    if capabilities.is_empty() {
        return "no capabilities detected".to_owned();
    }
    let mut parts = Vec::new();
    parts.push(if capabilities.contains(&Capability::NonInteractive) {
        "headless dispatch"
    } else {
        "interactive only"
    });
    if capabilities.contains(&Capability::Resume) {
        parts.push("resumable");
    }
    if capabilities.contains(&Capability::StructuredOutput) {
        parts.push("structured output");
    }
    if capabilities.contains(&Capability::UsageReporting) {
        parts.push("usage");
    }
    if capabilities.contains(&Capability::ModelSelect) {
        parts.push("model select");
    }
    parts.join(" \u{b7} ")
}

/// Verified capabilities for one harness, read from the persisted registry.
///
/// The downstream honesty guarantee (acceptance check 4): a harness whose probe
/// failed, or that was never probed, returns an empty set — so no capability it
/// did not demonstrate is ever offered anywhere.
#[must_use]
pub fn probed_capabilities(name: &str) -> BTreeSet<Capability> {
    read_harness_registry()
        .ok()
        .flatten()
        .and_then(|registry| registry.discovered.get(name).and_then(|r| r.probe.clone()))
        .map(|probe| probe.typed())
        .unwrap_or_default()
}

/// Whether one harness has verifiably probed one capability.
#[must_use]
pub fn has_capability(name: &str, capability: Capability) -> bool {
    probed_capabilities(name).contains(&capability)
}

#[cfg(test)]
mod tests {
    use super::{
        BinaryIdentity, Capability, CapabilityProbe, detect, primary_role, profile_for, roles_for,
    };
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn capability_slug_round_trips_and_drops_unknown() {
        for capability in Capability::ALL {
            assert_eq!(Capability::parse(capability.slug()), Some(*capability));
        }
        assert_eq!(Capability::parse("telepathy"), None);
    }

    #[test]
    fn detect_reads_advertised_flags_and_derives_cancellation() {
        // A realistic Claude-shaped help slice: has print, resume, model, and
        // structured output, but no --add-dir (no working-directory flag).
        let corpus = "\
Usage: claude [options]
  -p, --print              Print response and exit
  -c, --continue           Continue the most recent conversation
      --resume [id]        Resume a conversation
      --model <name>       Model to use
      --output-format <f>  Output format (text|json|stream-json)
      --permission-mode <m>  Permission mode";
        let capabilities = detect(corpus, &profile_for("claude"));
        assert!(capabilities.contains(&Capability::NonInteractive));
        assert!(capabilities.contains(&Capability::Resume));
        assert!(capabilities.contains(&Capability::ModelSelect));
        assert!(capabilities.contains(&Capability::StructuredOutput));
        assert!(capabilities.contains(&Capability::UsageReporting));
        assert!(capabilities.contains(&Capability::Tools));
        // Cancellation is derived from non-interactive support.
        assert!(capabilities.contains(&Capability::Cancellation));
        // No --add-dir word in the corpus, so no working-directory capability.
        assert!(!capabilities.contains(&Capability::WorkingDir));
    }

    #[test]
    fn short_flag_never_matches_a_longer_substring() {
        // `-p` must not be found inside `--provider`; only a standalone word wins.
        let corpus = "Usage: pi [--provider <name>] [--model <m>]";
        let capabilities = detect(corpus, &profile_for("pi"));
        assert!(!capabilities.contains(&Capability::NonInteractive));
        assert!(capabilities.contains(&Capability::ModelSelect));
        // Without non-interactive, cancellation is not derived either.
        assert!(!capabilities.contains(&Capability::Cancellation));
    }

    #[test]
    fn empty_help_detects_nothing() {
        assert!(detect("", &profile_for("hermes")).is_empty());
    }

    #[test]
    fn roles_derive_from_capabilities() {
        let conductor = BTreeSet::from([
            Capability::NonInteractive,
            Capability::Resume,
            Capability::StructuredOutput,
        ]);
        assert_eq!(roles_for(&conductor), ["conductor", "worker"]);
        assert_eq!(primary_role(true, &roles_for(&conductor)), "conductor");

        let worker = BTreeSet::from([Capability::NonInteractive]);
        assert_eq!(roles_for(&worker), ["worker"]);
        assert_eq!(primary_role(true, &roles_for(&worker)), "worker");

        let limited = BTreeSet::from([Capability::ModelSelect]);
        assert!(roles_for(&limited).is_empty());
        assert_eq!(primary_role(true, &roles_for(&limited)), "limited");
        assert_eq!(primary_role(false, &[]), "unavailable");
    }

    #[test]
    fn typed_drops_unknown_slugs_but_keeps_known() {
        let probe = CapabilityProbe {
            probed_at: "2026-07-23T00:00:00+00:00".to_owned(),
            binary: BinaryIdentity {
                path: "/bin/pi".to_owned(),
                mtime_ns: 1,
                size: 2,
                extra: BTreeMap::new(),
            },
            capabilities: BTreeSet::from([
                "resume".to_owned(),
                "telepathy".to_owned(),
                "non_interactive".to_owned(),
            ]),
            error: None,
            extra: BTreeMap::new(),
        };
        let typed = probe.typed();
        assert!(typed.contains(&Capability::Resume));
        assert!(typed.contains(&Capability::NonInteractive));
        assert_eq!(typed.len(), 2);
    }

    #[test]
    fn binary_identity_matches_only_when_all_fields_agree() {
        let base = BinaryIdentity {
            path: "/bin/pi".to_owned(),
            mtime_ns: 10,
            size: 20,
            extra: BTreeMap::new(),
        };
        assert!(base.matches(&base.clone()));
        let mut newer = base.clone();
        newer.mtime_ns = 11;
        assert!(!base.matches(&newer));
        let mut resized = base.clone();
        resized.size = 21;
        assert!(!base.matches(&resized));
    }
}
