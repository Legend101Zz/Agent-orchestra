//! Durable Bench harness and session records.
//!
//! These records are plain, additive JSON. Mutations use the registry's atomic
//! temp, flush, sync, and rename path; terminal clients must invoke daemon/core
//! commands instead of writing these files directly.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::registry::{atomic_write_json, home, make_slug, now_iso};

static SESSION_NONCE: AtomicU64 = AtomicU64::new(0);

/// One configured interactive harness command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessConfig {
    /// Executable name or absolute path.
    pub command: String,
    /// Arguments used for a fresh pane.
    #[serde(default)]
    pub args: Vec<String>,
    /// Arguments appended when recovering a dead conductor.
    #[serde(default)]
    pub resume_args: Vec<String>,
    /// Valid roles, such as `brain` and `worker`.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Adapter capability name.
    pub adapter: String,
    /// Non-interactive command template used by Phase 4A dispatches.
    ///
    /// When non-empty, the dispatcher spawns
    /// `<command> <dispatch_args...> <prompt>` (or pipes `<prompt>` on stdin
    /// when [`Self::dispatch_uses_stdin`] is true). The leading flag must be a
    /// demonstrated non-interactive capability such as Hermes' `--oneshot`
    /// (`-z`) flag.
    #[serde(default)]
    pub dispatch_args: Vec<String>,
    /// Whether the dispatcher should pipe the prompt on stdin instead of
    /// appending it as the final command-line argument.
    #[serde(default)]
    pub dispatch_uses_stdin: bool,
    /// Upper bound in seconds for one dispatch invocation.
    ///
    /// Defaults to 120 seconds when zero.
    #[serde(default)]
    pub dispatch_timeout_sec: u64,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Effective dispatch timeout in seconds for one configured harness.
#[must_use]
pub fn dispatch_timeout_for(config: &HarnessConfig) -> u64 {
    if config.dispatch_timeout_sec == 0 {
        120
    } else {
        config.dispatch_timeout_sec
    }
}

/// Client behavior stored alongside the harness registry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BenchAppConfig {
    /// Only supported leader key in v4.
    pub leader_key: String,
    /// Whether transient animation is collapsed to state changes.
    pub reduced_motion: bool,
    /// Selected theme, constrained by the client to ember or phosphor.
    pub theme: String,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for BenchAppConfig {
    fn default() -> Self {
        Self {
            leader_key: "ctrl-g".to_owned(),
            reduced_motion: false,
            theme: "ember".to_owned(),
            extra: BTreeMap::new(),
        }
    }
}

/// Per-user harness registry and default worker choices.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessRegistry {
    /// Harness records by stable key.
    pub harnesses: BTreeMap<String, HarnessConfig>,
    /// Preselected, user-editable worker pool.
    pub default_workers: Vec<String>,
    /// Upper bound for concurrently launched workers.
    pub max_parallel_workers: usize,
    /// Client behavior and theme.
    pub app: BenchAppConfig,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for HarnessRegistry {
    fn default() -> Self {
        let harness = |command: &str,
                       args: &[&str],
                       resume: &[&str],
                       roles: &[&str],
                       adapter: &str,
                       dispatch: &[&str]| {
            HarnessConfig {
                command: command.to_owned(),
                args: args.iter().map(|value| (*value).to_owned()).collect(),
                resume_args: resume.iter().map(|value| (*value).to_owned()).collect(),
                roles: roles.iter().map(|value| (*value).to_owned()).collect(),
                adapter: adapter.to_owned(),
                dispatch_args: dispatch.iter().map(|value| (*value).to_owned()).collect(),
                dispatch_uses_stdin: false,
                dispatch_timeout_sec: 120,
                extra: BTreeMap::new(),
            }
        };
        Self {
            harnesses: BTreeMap::from([
                (
                    "claude".to_owned(),
                    harness(
                        "claude",
                        &[],
                        &["--continue"],
                        &["brain", "worker"],
                        "claude",
                        &[],
                    ),
                ),
                (
                    "codex".to_owned(),
                    harness(
                        "codex",
                        &[],
                        &["resume"],
                        &["brain", "worker"],
                        "codex",
                        &[],
                    ),
                ),
                (
                    "hermes".to_owned(),
                    harness(
                        "hermes",
                        &["--tui"],
                        &[],
                        &["brain", "worker"],
                        "hermes",
                        &["-z"],
                    ),
                ),
                (
                    "pi-m3".to_owned(),
                    harness(
                        "pi",
                        &["--provider", "minimax", "--model", "MiniMax-M3"],
                        &[],
                        &["brain", "worker"],
                        "pi",
                        &[],
                    ),
                ),
            ]),
            default_workers: vec!["hermes".to_owned(), "pi-m3".to_owned()],
            max_parallel_workers: 3,
            app: BenchAppConfig::default(),
            extra: BTreeMap::new(),
        }
    }
}

/// Persisted rectangle for one STAGE card.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneLayout {
    /// Stable pane identifier.
    pub pane_id: String,
    /// Horizontal cell coordinate.
    pub x: u16,
    /// Vertical cell coordinate.
    pub y: u16,
    /// Card width in cells.
    pub width: u16,
    /// Card height in cells.
    pub height: u16,
    /// Stable ordering used by keyboard swap.
    pub order: usize,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Durable pane identity and recovery state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionPaneRecord {
    /// Stable pane identifier.
    pub id: String,
    /// Harness registry key.
    pub harness: String,
    /// `brain` or `worker`.
    pub role: String,
    /// Plain state word.
    pub state: String,
    /// Last recorded process identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Time the process was observed dead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down_at: Option<String>,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Durable session record rendered by HOME and mutated by daemon commands.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BenchSession {
    /// Stable session identifier.
    pub id: String,
    /// Brain harness key.
    pub brain: String,
    /// Worker harness keys.
    pub workers: Vec<String>,
    /// Working directory shared by fresh panes.
    pub cwd: String,
    /// Recorded Git repository root used for task isolation, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_repo: Option<String>,
    /// Recorded base branch used for explicit task merges, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// Recorded base commit used for isolated task branches, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    /// Optional session isolation default, currently `worktree` when selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Last mutation timestamp.
    pub updated_at: String,
    /// Pane identities and recovery state.
    #[serde(default)]
    pub panes: Vec<SessionPaneRecord>,
    /// Persisted STAGE compositor rectangles.
    #[serde(default)]
    pub layout: Vec<PaneLayout>,
    /// Durable re-orientation message for a resumed conductor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reorientation: Option<String>,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn harness_path() -> PathBuf {
    home().join("harnesses.json")
}

fn session_key(session: &str) -> String {
    session
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// Return the plain session record path for a stable session identifier.
#[must_use]
pub fn session_path(session: &str) -> PathBuf {
    home()
        .join("sessions")
        .join(session_key(session))
        .join("session.json")
}

/// Load the harness registry, creating the documented defaults when absent.
pub fn load_harness_registry() -> Result<HarnessRegistry> {
    let path = harness_path();
    match fs::read(&path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let registry = HarnessRegistry::default();
            atomic_write_json(&path, &registry)?;
            Ok(registry)
        }
        Err(error) => Err(error.into()),
    }
}

/// Atomically write the harness registry while preserving additive fields.
pub fn write_harness_registry(registry: &HarnessRegistry) -> Result<()> {
    atomic_write_json(&harness_path(), registry)
}

/// Create a durable session after validating its brain, worker, count, and cwd.
pub fn create_session(brain: &str, workers: &[String], cwd: &Path) -> Result<BenchSession> {
    let registry = load_harness_registry()?;
    let brain_config = registry
        .harnesses
        .get(brain)
        .with_context(|| format!("unknown brain harness: {brain}"))?;
    if !brain_config.roles.iter().any(|role| role == "brain") {
        bail!("harness {brain} cannot be a brain");
    }
    if workers.len() > registry.max_parallel_workers {
        bail!("worker pool exceeds max_parallel_workers");
    }
    for worker in workers {
        let config = registry
            .harnesses
            .get(worker)
            .with_context(|| format!("unknown worker harness: {worker}"))?;
        if !config.roles.iter().any(|role| role == "worker") {
            bail!("harness {worker} cannot be a worker");
        }
    }
    if !cwd.is_dir() {
        bail!("session cwd is not a directory: {}", cwd.display());
    }
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let id = format!(
        "{}-{}-{:04x}",
        make_slug(
            cwd.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("session")
        ),
        epoch,
        SESSION_NONCE.fetch_add(1, Ordering::Relaxed) & 0xffff
    );
    let (base_repo, base_branch, base_commit) = git_base(cwd);
    let now = now_iso();
    let session = BenchSession {
        id,
        brain: brain.to_owned(),
        workers: workers.to_vec(),
        cwd: cwd.to_string_lossy().into_owned(),
        base_repo,
        base_branch,
        base_commit,
        isolation: None,
        created_at: now.clone(),
        updated_at: now,
        panes: Vec::new(),
        layout: Vec::new(),
        reorientation: None,
        extra: BTreeMap::new(),
    };
    write_session(&session)?;
    Ok(session)
}

fn git_base(cwd: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let output = |args: &[&str]| {
        Command::new("git")
            .args(["-C", &cwd.to_string_lossy()])
            .args(args)
            .output()
            .ok()
            .filter(|result| result.status.success())
            .and_then(|result| String::from_utf8(result.stdout).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let repo = output(&["rev-parse", "--show-toplevel"]);
    let branch = output(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let commit = output(&["rev-parse", "HEAD"]);
    (repo, branch, commit)
}

/// Read one durable session record.
pub fn read_session(session: &str) -> Result<BenchSession> {
    let path = session_path(session);
    serde_json::from_slice(&fs::read(&path)?).with_context(|| format!("parse {}", path.display()))
}

/// Atomically write one durable session record.
pub fn write_session(session: &BenchSession) -> Result<()> {
    atomic_write_json(&session_path(&session.id), session)
}

/// List every parseable session newest first while tolerating corrupt siblings.
pub fn list_sessions() -> Result<Vec<BenchSession>> {
    let root = home().join("sessions");
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut sessions = entries
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path().join("session.json")).ok())
        .filter_map(|bytes| serde_json::from_slice::<BenchSession>(&bytes).ok())
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(sessions)
}
