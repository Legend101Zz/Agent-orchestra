//! Bounded durable brain-to-worker command path.
//!
//! Phase 4A dispatches are explicit, recorded, and bounded. Every dispatch
//! carries a known actor (brain or human), an owning session, a worker pane
//! or harness key, a prompt body, and two honest state dimensions: delivery
//! moves from `pending`/`queued` to `confirmed` once the brief is handed over,
//! while the additive execution state moves from `starting`/`running` to a
//! terminal result written by a detached supervisor.
//!
//! Dispatch is layered above the daemon/core registry and never injects
//! keystrokes into a PTY. It uses a configured non-interactive command
//! template such as Hermes' demonstrated `--oneshot` (`-z`) flag and pipes
//! the prompt through the standard input or trailing argument of that
//! command. The harness record declares whether stdin or argv should carry
//! the prompt and the bounded timeout for one invocation.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::locate_executable;
use crate::bench::{
    BenchSession, HarnessConfig, HarnessRegistry, dispatch_timeout_for, load_harness_registry,
    read_session,
};
use crate::invocation::resolve_worker_invocation;
use crate::probe::probed_from;
use crate::ratelimit::BackoffPolicy;
use crate::registry::{atomic_write_json, home, now_iso, pid_alive};
use crate::runner::Usage;
use crate::spawn_guard;
use crate::tasks::{
    Task, TaskActor, TaskStatus, read_task, record_delivery, record_queued, record_review_delivery,
};

/// Maximum bytes of stdout captured from one dispatch invocation.
pub const MAX_CAPTURED_BYTES: usize = 16 * 1024;

/// Appended to a capture that hit [`MAX_CAPTURED_BYTES`] so a truncated record
/// never reads as a complete one.
pub const TRUNCATION_MARKER: &str = "\n… [truncated by pi-orchestra]";

/// Outcome of one recorded dispatch invocation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    /// The dispatch was recorded but the harness invocation has not completed.
    Pending,
    /// The dispatch is recorded but was held back because the harness is at its
    /// concurrency cap; no worker was spawned (issue #7). It is drained later by
    /// [`drain_queued`] when a slot frees.
    Queued,
    /// The worker process received the brief; it may still be running.
    Confirmed,
    /// The brief could not be delivered to a worker process.
    Failed,
}

/// Runtime state of a dispatch after the brief is delivered.
///
/// This additive field separates delivery (`status = confirmed`) from worker
/// completion. Old records omit it and retain their pre-#30 terminal meaning.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    /// The detached supervisor owns the slot but has not started the worker yet.
    Starting,
    /// The brief was handed to a live worker.
    Running,
    /// The worker exited successfully.
    Succeeded,
    /// The worker exited unsuccessfully or exceeded its execution bound.
    Failed,
    /// The detached supervisor died and reconciliation terminated the worker.
    Orphaned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DispatchPurpose {
    Execute,
    Review,
}

impl DispatchPurpose {
    const fn persisted(self) -> Option<&'static str> {
        match self {
            Self::Execute => None,
            Self::Review => Some("review"),
        }
    }

    fn from_record(record: &DispatchRecord) -> Self {
        if record.purpose.as_deref() == Some("review") {
            Self::Review
        } else {
            Self::Execute
        }
    }
}

impl ExecutionStatus {
    /// Return the durable lowercase execution word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Orphaned => "orphaned",
        }
    }

    /// Whether this runtime state is terminal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Orphaned)
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "starting" => Some(Self::Starting),
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "orphaned" => Some(Self::Orphaned),
            _ => None,
        }
    }
}

impl DeliveryStatus {
    /// Return the durable lowercase delivery word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Queued => "queued",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
        }
    }

    /// Parse the durable delivery word.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "queued" => Ok(Self::Queued),
            "confirmed" => Ok(Self::Confirmed),
            "failed" => Ok(Self::Failed),
            _ => Err(anyhow!("invalid delivery status '{value}'")),
        }
    }
}

/// Actor that originated a dispatch request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DispatchActor {
    /// A brain invoked the dispatch path.
    Brain,
    /// A human invoked the dispatch path.
    Human,
}

impl DispatchActor {
    /// Parse the actor contract word.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "brain" => Ok(Self::Brain),
            "human" => Ok(Self::Human),
            _ => Err(anyhow!(
                "invalid dispatch actor '{value}'; expected brain or human"
            )),
        }
    }

    /// Return the durable actor word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Brain => "brain",
            Self::Human => "human",
        }
    }
}

impl From<TaskActor> for DispatchActor {
    fn from(value: TaskActor) -> Self {
        match value {
            TaskActor::Brain => Self::Brain,
            TaskActor::Human => Self::Human,
        }
    }
}

impl From<DispatchActor> for TaskActor {
    fn from(value: DispatchActor) -> Self {
        match value {
            DispatchActor::Brain => Self::Brain,
            DispatchActor::Human => Self::Human,
        }
    }
}

/// Reason returned by the dispatcher when a delivery fails.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchFailureKind {
    /// The chosen harness key was not present in the registry.
    UnknownHarness,
    /// The harness exists but does not declare a non-interactive capability.
    CapabilityUnavailable,
    /// The harness's configured executable was missing on disk.
    MissingExecutable,
    /// The harness invocation exceeded its bounded timeout.
    Timeout,
    /// The harness kept emitting rate-limit signals after the backoff budget was
    /// exhausted (issue #7).
    RateLimited,
    /// The harness exited non-zero or returned a malformed response.
    HarnessError,
}

/// Plain additive durable dispatch record.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DispatchRecord {
    /// Stable `D`-prefixed dispatch identifier.
    pub id: String,
    /// Owning Bench session identifier.
    pub session: String,
    /// Stable task identifier in the same session.
    pub task: String,
    /// Originating actor word: `brain` or `human`.
    pub actor: String,
    /// Harness registry key used to deliver the dispatch.
    pub harness: String,
    /// Linked pane identifier, when one is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// Linked run or worker identifier, when one is supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// Effective command line that was launched.
    pub command_line: String,
    /// Worker working directory the dispatch was spawned in, when one was set.
    ///
    /// Evidence of orchestrator-provided cwd control (issue #6): the worktree
    /// path for an isolated task, else the session cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Persisted prompt body that was delivered.
    pub prompt: String,
    /// Additive dispatch purpose; absent means executor work, `review` means
    /// independent acceptance review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// Delivery state: pending, queued, confirmed receipt, or pre-delivery failure.
    pub status: String,
    /// Worker execution state after delivery (additive in issue #30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_status: Option<String>,
    /// Detached supervisor pid while the execution is live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_pid: Option<u32>,
    /// Current worker process-group leader while the execution is live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_pid: Option<u32>,
    /// Exit code reported by the harness, when one is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Captured bounded stdout excerpt.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stdout: String,
    /// Captured bounded stderr excerpt.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr: String,
    /// Failure reason when the delivery did not succeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
    /// Plain human-readable failure detail when one is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// User-visible ORC WARNING lines emitted during delivery, e.g. rate-limit
    /// backoff notices (issue #7). Additive; empty for a clean delivery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Exact structured usage extracted from a supported adapter transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Timestamp when the first worker received the brief.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Timestamp when the worker reached a terminal state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Last mutation timestamp.
    pub updated_at: String,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl DispatchRecord {
    /// Whether this record represents a successful delivery.
    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.status == DeliveryStatus::Confirmed.as_str()
    }

    /// Whether this record is queued behind a full concurrency cap (issue #7).
    #[must_use]
    pub fn is_queued(&self) -> bool {
        self.status == DeliveryStatus::Queued.as_str()
    }

    /// Whether this dispatch asks a reviewer to judge acceptance checks.
    #[must_use]
    pub fn is_review(&self) -> bool {
        self.purpose.as_deref() == Some("review")
    }

    /// Whether the worker execution is still live or starting.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.execution_status.as_deref().is_some_and(|status| {
            matches!(
                ExecutionStatus::parse(status),
                Some(ExecutionStatus::Starting | ExecutionStatus::Running)
            )
        })
    }

    /// Whether this record has reached a durable terminal state.
    ///
    /// Pre-#30 records have no execution state, so their confirmed/failed
    /// delivery status remains terminal for backward compatibility.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        match self
            .execution_status
            .as_deref()
            .and_then(ExecutionStatus::parse)
        {
            Some(status) => status.is_terminal(),
            None => self.is_confirmed() || self.status == DeliveryStatus::Failed.as_str(),
        }
    }
}

/// Inputs the caller supplies when recording one dispatch.
#[derive(Clone, Debug)]
pub struct DispatchRequest {
    /// Owning Bench session identifier.
    pub session: String,
    /// Stable task identifier in the same session.
    pub task: String,
    /// Originating actor for the dispatch.
    pub actor: DispatchActor,
    /// Harness registry key to dispatch through.
    pub harness: String,
    /// Optional explicit pane linkage.
    pub pane_id: Option<String>,
    /// Optional explicit run linkage.
    pub run: Option<String>,
    /// Prompt body that will be delivered to the harness.
    pub prompt: String,
    /// Optional background worker execution timeout override in seconds.
    pub timeout_sec: Option<u64>,
}

fn dispatch_nonce() -> u64 {
    static NONCE: AtomicU64 = AtomicU64::new(0);
    NONCE.fetch_add(1, Ordering::Relaxed)
}

fn dispatch_id(prefix: &str, session: &str) -> String {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let nonce = dispatch_nonce();
    let slug = crate::registry::make_slug(session);
    format!("D-{prefix}-{epoch}-{slug}-{nonce:04x}")
}

fn session_key(session: &str) -> String {
    session
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn dispatch_dir(session: &str) -> PathBuf {
    home().join("dispatches").join(session_key(session))
}

/// Return the dispatch JSON path for one stable dispatch.
#[must_use]
pub fn dispatch_path(session: &str, id: &str) -> PathBuf {
    dispatch_dir(session).join(format!("{id}.json"))
}

/// Private detached-supervisor input beside the durable dispatch record.
pub(crate) fn supervisor_spec_path(session: &str, id: &str) -> PathBuf {
    dispatch_dir(session).join(format!("{id}.supervisor.json"))
}

fn dispatch_id_is_valid(id: &str) -> bool {
    id.starts_with('D') && id.len() > 1 && !id.contains('/') && !id.contains('\\')
}

fn validate_id(id: &str) -> Result<()> {
    if !dispatch_id_is_valid(id) {
        bail!("invalid dispatch id '{id}'; expected a D-prefixed identifier")
    }
    Ok(())
}

/// Look up the requested worker harness, requiring only the `worker` role.
///
/// Whether it can actually run non-interactively is decided downstream by
/// [`resolve_worker_invocation`] from the probe results, so an honest refusal
/// can name the missing capability instead of a generic "no dispatch_args".
fn select_worker<'a>(registry: &'a HarnessRegistry, key: &str) -> Result<&'a HarnessConfig> {
    let config = registry
        .harnesses
        .get(key)
        .ok_or_else(|| anyhow!("unknown harness: {key}"))?;
    if !config.roles.iter().any(|role| role == "worker") {
        bail!("harness {key} cannot be a worker");
    }
    Ok(config)
}

/// The task's effective working directory: the materialized worktree when the
/// task is isolated, otherwise the session cwd. Returns the first that exists on
/// disk so a stale path never masquerades as a missing executable at spawn.
fn effective_cwd(session: &BenchSession, task: &Task) -> Result<Option<String>> {
    let worktree = task
        .worktree
        .as_ref()
        .and_then(|worktree| worktree.path.clone());
    if task.contract.is_some() {
        let metadata = task.worktree.as_ref().ok_or_else(|| {
            anyhow!(
                "ISOLATION REQUIRED: contracted task {} has no owned worktree",
                task.id
            )
        })?;
        if metadata.state != "ready" {
            // `materialize_worktree` records *why* it gave up and leaves the
            // task creatable, so the refusal only surfaces here — at which
            // point "worktree is unavailable" on its own tells the caller
            // nothing it can act on. The stored reason is usually "session
            // cwd is not a Git work tree", which is a one-line fix the
            // conductor can make itself (issue #45 defect 4).
            bail!(
                "ISOLATION REQUIRED: contracted task {} worktree is {}{}. A contracted \
                 task takes an isolated worktree, so it needs a Git work tree; delegate \
                 from inside a repository, or use an uncontracted task (no --objective \
                 /--check), which needs none.",
                task.id,
                metadata.state,
                metadata
                    .reason
                    .as_deref()
                    .map(|reason| format!(" ({reason})"))
                    .unwrap_or_default()
            )
        }
        let path = worktree
            .filter(|dir| Path::new(dir).is_dir())
            .ok_or_else(|| {
                anyhow!(
                    "ISOLATION REQUIRED: contracted task {} worktree is missing",
                    task.id
                )
            })?;
        return Ok(Some(path));
    }
    Ok([worktree, Some(session.cwd.clone())]
        .into_iter()
        .flatten()
        .find(|dir| Path::new(dir).is_dir()))
}

/// Placeholder command line for a pre-invocation failure record.
fn placeholder_command(registry: &HarnessRegistry, key: &str) -> String {
    match registry.harnesses.get(key) {
        Some(config) => {
            let mut parts = vec![config.command.clone()];
            parts.extend(config.args.iter().cloned());
            parts.extend(config.dispatch_args.iter().cloned());
            parts.join(" ")
        }
        None => key.to_owned(),
    }
}

fn render_command_line(program: &str, args: &[String], prompt: &str, stdin: bool) -> String {
    let mut parts = Vec::with_capacity(args.len() + 2);
    parts.push(shell_escape(program));
    for arg in args {
        parts.push(shell_escape(arg));
    }
    if stdin {
        parts.push("<prompt-on-stdin>".to_owned());
    } else {
        parts.push(shell_escape(prompt));
    }
    parts.join(" ")
}

fn shell_escape(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '=' | ':'))
        && !value.is_empty()
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn record_pre_delivery_failure(
    record: &DispatchRecord,
    actor: TaskActor,
    detail: String,
) -> Result<()> {
    if record.is_review() {
        record_review_delivery(&record.session, &record.task, actor, false, detail)?;
    } else {
        record_delivery(&record.session, &record.task, actor, None, detail)?;
    }
    Ok(())
}

pub(crate) fn failure_message(kind: &DispatchFailureKind, harness: &str) -> String {
    match kind {
        DispatchFailureKind::UnknownHarness => format!("UNKNOWN HARNESS: {harness}"),
        DispatchFailureKind::CapabilityUnavailable => {
            format!("CAPABILITY UNAVAILABLE: {harness} has no non-interactive dispatch_args")
        }
        DispatchFailureKind::MissingExecutable => {
            format!("MISSING EXECUTABLE: {harness} command not found on PATH")
        }
        DispatchFailureKind::Timeout => "DISPATCH TIMEOUT".to_owned(),
        DispatchFailureKind::RateLimited => {
            format!("RATE LIMITED: {harness} kept signaling rate limits after backoff")
        }
        DispatchFailureKind::HarnessError => "HARNESS ERROR".to_owned(),
    }
}

/// Identity to reuse when re-delivering a previously queued record (issue #7).
struct Reuse {
    id: String,
    created_at: String,
}

/// Record and dispatch one bounded command through the configured worker harness.
///
/// Uses the production backoff schedule. Returns the durable [`DispatchRecord`]
/// describing the delivery state — which may be `queued` when the harness is at
/// its concurrency cap (issue #7), in which case no worker was spawned.
pub fn dispatch(request: &DispatchRequest) -> Result<DispatchRecord> {
    deliver(
        request,
        &BackoffPolicy::production(),
        None,
        DispatchPurpose::Execute,
    )
}

/// Dispatch a structured acceptance review from the same isolated worktree.
pub fn dispatch_review(request: &DispatchRequest) -> Result<DispatchRecord> {
    deliver(
        request,
        &BackoffPolicy::production(),
        None,
        DispatchPurpose::Review,
    )
}

/// Dispatch with an explicit backoff schedule (tests inject a fast policy).
pub fn dispatch_with_policy(
    request: &DispatchRequest,
    policy: &BackoffPolicy,
) -> Result<DispatchRecord> {
    deliver(request, policy, None, DispatchPurpose::Execute)
}

fn deliver(
    request: &DispatchRequest,
    policy: &BackoffPolicy,
    reuse: Option<Reuse>,
    purpose: DispatchPurpose,
) -> Result<DispatchRecord> {
    if request.session.trim().is_empty() {
        bail!("dispatch session is required")
    }
    if request.task.trim().is_empty() {
        bail!("dispatch task is required")
    }
    if request.prompt.is_empty() {
        bail!("dispatch prompt cannot be empty")
    }
    if request.prompt.len() > MAX_CAPTURED_BYTES {
        bail!("dispatch prompt exceeds {MAX_CAPTURED_BYTES} bytes; refactor into a smaller prompt")
    }
    let session = read_session(&request.session).with_context(|| {
        format!(
            "missing dispatch session {}; list the ones that exist with `pio session list`",
            request.session
        )
    })?;
    // Naming the listing command is the difference between a typo and a dead
    // end: `pio dispatch send` needs a task id that already exists, and issue
    // #45 records a conductor guessing `T-hello`, then `T0002`, because
    // nothing told it where the real ids live.
    let task = read_task(&session.id, &request.task).with_context(|| {
        format!(
            "missing dispatch task {}; list this session's tasks with \
             `pio task list --session {}`",
            request.task, session.id
        )
    })?;
    let task_status = TaskStatus::parse(&task.status).map_err(anyhow::Error::from)?;
    if task_status != TaskStatus::Running {
        bail!("dispatch task {} must be running before dispatch", task.id);
    }
    if task.assignee.is_none() {
        bail!("dispatch task {} has no recorded assignee", task.id);
    }
    let selected_pane = if session.panes.is_empty() {
        request.pane_id.clone()
    } else if let Some(pane_id) = request.pane_id.as_deref() {
        let pane = session
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)
            .ok_or_else(|| anyhow!("WORKER UNAVAILABLE: pane {pane_id} is not in this session"))?;
        if pane.role != "worker" || pane.harness != request.harness || pane.state != "running" {
            bail!("WORKER UNAVAILABLE: pane {pane_id} cannot receive this task")
        }
        Some(pane.id.clone())
    } else {
        session
            .panes
            .iter()
            .find(|pane| {
                pane.role == "worker" && pane.harness == request.harness && pane.state == "running"
            })
            .map(|pane| pane.id.clone())
    };
    let registry = load_harness_registry()?;
    let resolved_key = request.harness.clone();
    let cwd = effective_cwd(&session, &task)?;
    let config = match select_worker(&registry, &resolved_key) {
        Ok(config) => config,
        Err(error) => {
            let known = registry.harnesses.contains_key(&resolved_key);
            let (kind, detail) = if known {
                (
                    DispatchFailureKind::CapabilityUnavailable,
                    error.to_string(),
                )
            } else {
                (
                    DispatchFailureKind::UnknownHarness,
                    format!("UNKNOWN HARNESS: {resolved_key}"),
                )
            };
            return persist_failure(
                request,
                &resolved_key,
                FailureSpec {
                    kind,
                    detail,
                    command_line: placeholder_command(&registry, &resolved_key),
                    default_reason: "worker capability unavailable",
                },
                cwd,
                reuse.as_ref(),
                purpose,
            );
        }
    };

    // The adapter chooses the invocation style from the probe results (issue #6);
    // an honest refusal names the missing capability instead of guessing one.
    let adapter = config.adapter.clone();
    let harness_cap = spawn_guard::effective_cap(&registry, &resolved_key);
    let session_cap = registry.max_parallel_workers.max(1);
    let probed = probed_from(&registry, &config.adapter);
    let invocation = match resolve_worker_invocation(config, &probed, cwd.as_deref().map(Path::new))
    {
        Ok(invocation) => invocation,
        Err(error) => {
            return persist_failure(
                request,
                &resolved_key,
                FailureSpec {
                    kind: DispatchFailureKind::CapabilityUnavailable,
                    detail: error.message(&resolved_key),
                    command_line: placeholder_command(&registry, &resolved_key),
                    default_reason: "worker capability unavailable",
                },
                cwd,
                reuse.as_ref(),
                purpose,
            );
        }
    };
    let command_line = render_command_line(
        &config.command,
        &invocation.args,
        &request.prompt,
        invocation.uses_stdin(),
    );
    let program = match locate_executable(&config.command) {
        Some(program) => program,
        None => {
            return persist_failure(
                request,
                &resolved_key,
                FailureSpec {
                    kind: DispatchFailureKind::MissingExecutable,
                    detail: failure_message(&DispatchFailureKind::MissingExecutable, &resolved_key),
                    command_line,
                    default_reason: "worker executable unavailable",
                },
                cwd,
                reuse.as_ref(),
                purpose,
            );
        }
    };
    let timeout = Duration::from_secs(
        request
            .timeout_sec
            .unwrap_or_else(|| dispatch_timeout_for(config)),
    );

    let mut record = new_record(
        request,
        &resolved_key,
        &command_line,
        cwd.clone(),
        reuse.as_ref(),
        purpose,
    );
    // Record the pane that was *chosen*, not merely the one that was asked
    // for. Selecting a seated worker without being told which one is the
    // whole affordance issue #45 rests on, and until now the durable receipt
    // stayed empty in exactly that case — so "which pane got this brief?"
    // could only be answered when the caller already knew.
    record.pane_id = selected_pane.clone();

    // Hold both the provider-facing harness cap and the session-wide
    // max_parallel_workers cap for the real worker lifetime. The detached
    // supervisor adopts both leases before this conductor returns.
    let attempts = u32::try_from(policy.max_retries)
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let lease_ttl = timeout
        .saturating_mul(attempts)
        .saturating_add(policy.max_delay.saturating_mul(attempts))
        .max(spawn_guard::DEFAULT_LEASE_TTL);
    let session_slot = format!("session:{}", request.session);
    let Some(session_lease) =
        spawn_guard::acquire_slot(&session_slot, session_cap, lease_ttl, Some(&record.id))?
    else {
        let note = format!(
            "ORC WARNING: session {} at max_parallel_workers {session_cap}; queued dispatch {} (no worker spawned)",
            request.session, record.id
        );
        return persist_queued(record, note);
    };
    let Some(harness_lease) =
        spawn_guard::acquire_slot(&resolved_key, harness_cap, lease_ttl, Some(&record.id))?
    else {
        drop(session_lease);
        let note = format!(
            "ORC WARNING: {resolved_key} at concurrency cap {harness_cap}; queued dispatch {} (no worker spawned)",
            record.id
        );
        return persist_queued(record, note);
    };
    let leases = vec![session_lease, harness_lease];
    record.execution_status = Some(ExecutionStatus::Starting.as_str().to_owned());
    write_dispatch(&record)?;
    let confirmed_link = selected_pane
        .or_else(|| request.run.clone())
        .unwrap_or_else(|| record.id.clone());
    let lease_paths = leases
        .iter()
        .map(|lease| lease.path().to_path_buf())
        .collect();
    let supervisor = crate::dispatch_supervisor::launch(
        crate::dispatch_supervisor::SupervisorRequest {
            session: &request.session,
            dispatch: &record.id,
            program: &program,
            invocation: &invocation,
            prompt: &request.prompt,
            cwd: cwd.as_deref().map(Path::new),
            timeout,
            adapter: &adapter,
            policy,
            lease_paths,
            confirmed_link: &confirmed_link,
            actor: TaskActor::from(request.actor),
        },
        leases,
    );
    let supervisor_pid = match supervisor {
        Ok(pid) => pid,
        Err(error) => {
            record.status = DeliveryStatus::Failed.as_str().to_owned();
            record.execution_status = Some(ExecutionStatus::Failed.as_str().to_owned());
            record.failure_kind = Some("supervisor_error".to_owned());
            record.error = Some(format!("SUPERVISOR ERROR: {error:#}"));
            record.ended_at = Some(now_iso());
            record.updated_at = now_iso();
            write_dispatch(&record)?;
            record_pre_delivery_failure(
                &record,
                TaskActor::from(request.actor),
                format!(
                    "dispatch {} failed before delivery: {}",
                    record.id,
                    record.error.as_deref().unwrap_or("supervisor error")
                ),
            )?;
            return Ok(record);
        }
    };

    // Wait only for the delivery handshake. The worker's execution bound is
    // enforced inside the detached supervisor; orch_await performs the long
    // user-facing wait.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let observed = read_dispatch_unreconciled(&request.session, &record.id)?;
        if observed.is_confirmed() || observed.status == DeliveryStatus::Failed.as_str() {
            return Ok(observed);
        }
        if !pid_alive(Some(supervisor_pid)) || Instant::now() >= deadline {
            crate::runner::terminate_pid(supervisor_pid);
            let _ = spawn_guard::release_dispatch_slots(&record.id);
            record = observed;
            record.status = DeliveryStatus::Failed.as_str().to_owned();
            record.execution_status = Some(ExecutionStatus::Orphaned.as_str().to_owned());
            record.supervisor_pid = Some(supervisor_pid);
            record.failure_kind = Some("supervisor_lost".to_owned());
            record.error =
                Some("SUPERVISOR LOST before the worker confirmed receipt of the brief".to_owned());
            record.ended_at = Some(now_iso());
            record.updated_at = now_iso();
            write_dispatch(&record)?;
            let _ = fs::remove_file(supervisor_spec_path(&request.session, &record.id));
            record_pre_delivery_failure(
                &record,
                TaskActor::from(request.actor),
                format!(
                    "dispatch {} failed before delivery: supervisor lost",
                    record.id
                ),
            )?;
            return Ok(record);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn kind_label(kind: &DispatchFailureKind) -> &'static str {
    match kind {
        DispatchFailureKind::UnknownHarness => "unknown_harness",
        DispatchFailureKind::CapabilityUnavailable => "capability_unavailable",
        DispatchFailureKind::MissingExecutable => "missing_executable",
        DispatchFailureKind::Timeout => "timeout",
        DispatchFailureKind::RateLimited => "rate_limited",
        DispatchFailureKind::HarnessError => "harness_error",
    }
}

fn new_record(
    request: &DispatchRequest,
    harness: &str,
    command_line: &str,
    cwd: Option<String>,
    reuse: Option<&Reuse>,
    purpose: DispatchPurpose,
) -> DispatchRecord {
    let now = now_iso();
    // Re-delivering a queued record keeps its stable id and original creation
    // time so the queue-then-run history is one record, not two (issue #7).
    let (id, created_at) = match reuse {
        Some(reuse) => (reuse.id.clone(), reuse.created_at.clone()),
        None => (dispatch_id(harness, &request.session), now.clone()),
    };
    DispatchRecord {
        id,
        session: request.session.clone(),
        task: request.task.clone(),
        actor: request.actor.as_str().to_owned(),
        harness: harness.to_owned(),
        pane_id: request.pane_id.clone(),
        run: request.run.clone(),
        command_line: command_line.to_owned(),
        cwd,
        prompt: request.prompt.clone(),
        purpose: purpose.persisted().map(str::to_owned),
        status: DeliveryStatus::Pending.as_str().to_owned(),
        execution_status: None,
        supervisor_pid: None,
        worker_pid: None,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        failure_kind: None,
        error: None,
        warnings: Vec::new(),
        usage: None,
        started_at: None,
        ended_at: None,
        created_at,
        updated_at: now,
        extra: BTreeMap::new(),
    }
}

/// The four things a pre-invocation refusal needs to record, bundled so the
/// failure path stays a single small call (issue #6 consolidation, kept DRY as
/// the reuse arg was added in #7).
struct FailureSpec {
    kind: DispatchFailureKind,
    detail: String,
    command_line: String,
    default_reason: &'static str,
}

/// Build, persist, and record delivery for one pre-invocation failure.
///
/// Consolidates the unknown-harness, capability-unavailable, and
/// missing-executable failure paths so every refusal is durable and shows up on
/// the task's delivery history identically.
fn persist_failure(
    request: &DispatchRequest,
    harness: &str,
    spec: FailureSpec,
    cwd: Option<String>,
    reuse: Option<&Reuse>,
    purpose: DispatchPurpose,
) -> Result<DispatchRecord> {
    let mut record = new_record(request, harness, &spec.command_line, cwd, reuse, purpose);
    record.status = DeliveryStatus::Failed.as_str().to_owned();
    record.execution_status = Some(ExecutionStatus::Failed.as_str().to_owned());
    record.failure_kind = Some(kind_label(&spec.kind).to_owned());
    record.error = Some(spec.detail);
    record.ended_at = Some(now_iso());
    record.updated_at = now_iso();
    write_dispatch(&record)?;
    record_pre_delivery_failure(
        &record,
        TaskActor::from(request.actor),
        format!(
            "dispatch {} failed: {}",
            record.id,
            record.error.as_deref().unwrap_or(spec.default_reason)
        ),
    )?;
    Ok(record)
}

/// Build and persist one *queued* dispatch: the harness is at its concurrency
/// cap, so no worker was spawned (issue #7, AC1). The record is durable and
/// visible via `pio dispatch list`; the task history gains a `delivery_queued`
/// event without changing the task's status or claiming a worker received it.
fn persist_queued(mut record: DispatchRecord, note: String) -> Result<DispatchRecord> {
    record.status = DeliveryStatus::Queued.as_str().to_owned();
    record.execution_status = None;
    record.supervisor_pid = None;
    record.worker_pid = None;
    record.warnings.push(note.clone());
    record.updated_at = now_iso();
    write_dispatch(&record)?;
    let actor = DispatchActor::parse(&record.actor).unwrap_or(DispatchActor::Brain);
    record_queued(&record.session, &record.task, TaskActor::from(actor), note)?;
    Ok(record)
}

/// Persist a single dispatch record atomically.
pub fn write_dispatch(record: &DispatchRecord) -> Result<()> {
    validate_id(&record.id)?;
    let path = dispatch_path(&record.session, &record.id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    atomic_write_json(&path, record)
}

/// Read every parseable dispatch for one session newest first, tolerating corrupt siblings.
pub fn list_dispatches(session: &str) -> Result<Vec<DispatchRecord>> {
    if session.trim().is_empty() {
        bail!("dispatch session is required")
    }
    let dir = dispatch_dir(session);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut records = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<DispatchRecord>(&bytes).ok())
        .filter_map(|record| reconcile_record(record).ok())
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(records)
}

/// Read one durable dispatch record.
pub fn read_dispatch(session: &str, id: &str) -> Result<DispatchRecord> {
    reconcile_record(read_dispatch_unreconciled(session, id)?)
}

/// Read one record without performing dead-supervisor reconciliation.
pub(crate) fn read_dispatch_unreconciled(session: &str, id: &str) -> Result<DispatchRecord> {
    if session.trim().is_empty() {
        bail!("dispatch session is required")
    }
    validate_id(id)?;
    let path = dispatch_path(session, id);
    serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("parse dispatch {}", path.display()))
}

/// Reconcile a dispatch whose detached supervisor no longer exists.
///
/// The worker is a separate process group, so a killed supervisor can leave it
/// alive. Reconciliation terminates that group, marks the record terminal, and
/// releases only this dispatch's durable leases.
fn reconcile_record(mut record: DispatchRecord) -> Result<DispatchRecord> {
    if !record.is_running() {
        return Ok(record);
    }
    let Some(supervisor_pid) = record.supervisor_pid else {
        return Ok(record);
    };
    if pid_alive(Some(supervisor_pid)) {
        return Ok(record);
    }
    if let Some(worker_pid) = record.worker_pid {
        crate::runner::terminate_pid(worker_pid);
    }
    let _ = spawn_guard::release_dispatch_slots(&record.id);
    record.execution_status = Some(ExecutionStatus::Orphaned.as_str().to_owned());
    record.worker_pid = None;
    record.failure_kind = Some("supervisor_lost".to_owned());
    record.error = Some("SUPERVISOR LOST: worker terminated during reconciliation".to_owned());
    record.ended_at = Some(now_iso());
    record.updated_at = now_iso();
    write_dispatch(&record)?;
    let _ = fs::remove_file(supervisor_spec_path(&record.session, &record.id));
    Ok(record)
}

/// Re-attempt every queued dispatch for a session, oldest first (issue #7).
///
/// Uses the production backoff schedule. A queued record that now wins a
/// concurrency slot is delivered and the *same* record becomes `confirmed` or
/// `failed`; one still at the cap stays `queued`. Per-record errors (e.g. the
/// task moved out of `running`) leave that record queued and do not abort the
/// sweep. Returns the records that were re-attempted (no longer queued).
pub fn drain_queued(session: &str) -> Result<Vec<DispatchRecord>> {
    drain_queued_with_policy(session, &BackoffPolicy::production())
}

/// [`drain_queued`] with an explicit backoff schedule (tests inject a fast one).
pub fn drain_queued_with_policy(
    session: &str,
    policy: &BackoffPolicy,
) -> Result<Vec<DispatchRecord>> {
    if session.trim().is_empty() {
        bail!("dispatch session is required")
    }
    let mut queued = list_dispatches(session)?
        .into_iter()
        .filter(DispatchRecord::is_queued)
        .collect::<Vec<_>>();
    // Oldest first: honor arrival order as slots free.
    queued.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut drained = Vec::new();
    for record in queued {
        let request = DispatchRequest {
            session: record.session.clone(),
            task: record.task.clone(),
            actor: DispatchActor::parse(&record.actor).unwrap_or(DispatchActor::Brain),
            harness: record.harness.clone(),
            pane_id: record.pane_id.clone(),
            run: record.run.clone(),
            prompt: record.prompt.clone(),
            timeout_sec: None,
        };
        let reuse = Reuse {
            id: record.id.clone(),
            created_at: record.created_at.clone(),
        };
        // A per-record failure (task no longer running, missing session, …)
        // leaves the record queued and visible rather than aborting the sweep.
        let purpose = DispatchPurpose::from_record(&record);
        if let Ok(updated) = deliver(&request, policy, Some(reuse), purpose)
            && !updated.is_queued()
        {
            drained.push(updated);
        }
    }
    Ok(drained)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_escape_handles_alphanumeric_and_paths() {
        assert_eq!(shell_escape("hermes"), "hermes");
        assert_eq!(
            shell_escape("/usr/local/bin/hermes"),
            "/usr/local/bin/hermes"
        );
        assert_eq!(shell_escape("with space"), "'with space'");
        assert_eq!(shell_escape("o'clock"), "'o'\\''clock'");
    }

    #[test]
    fn render_command_line_marks_stdin_prompt() {
        let rendered = render_command_line("hermes", &["-z".to_owned()], "hello", false);
        assert_eq!(rendered, "hermes -z hello");
        let stdin_rendered = render_command_line("pi", &["--stdin".to_owned()], "hi", true);
        assert_eq!(stdin_rendered, "pi --stdin <prompt-on-stdin>");
    }

    #[test]
    fn delivery_status_round_trips_via_parse() {
        for value in ["pending", "confirmed", "failed"] {
            assert_eq!(DeliveryStatus::parse(value).unwrap().as_str(), value);
        }
        assert!(DeliveryStatus::parse("not-a-state").is_err());
    }

    #[test]
    fn real_pre_issue_30_record_loads_without_runtime_fields() {
        // Shape copied from the durable record written by issue #28: no
        // execution_status/pids/usage/timestamps, plus an unknown future field.
        let record: DispatchRecord = serde_json::from_value(serde_json::json!({
            "id": "D-pi-m3-1785136766-session-0001",
            "session": "agent-orchestra-1785136766-0000",
            "task": "T0001",
            "actor": "brain",
            "harness": "pi-m3",
            "command_line": "pi -p --mode json brief",
            "prompt": "brief",
            "status": "confirmed",
            "exit_code": 0,
            "stdout": "answer",
            "stderr": "",
            "warnings": [],
            "created_at": "2026-07-27T09:00:00+00:00",
            "updated_at": "2026-07-27T09:00:24+00:00",
            "pre_change_receipt": "preserve me"
        }))
        .expect("pre-change record loads");
        assert!(record.execution_status.is_none());
        assert!(record.supervisor_pid.is_none());
        assert!(record.worker_pid.is_none());
        assert!(record.usage.is_none());
        assert!(
            record.is_terminal(),
            "old confirmed semantics stay terminal"
        );
        assert_eq!(
            record.extra.get("pre_change_receipt"),
            Some(&serde_json::json!("preserve me"))
        );
    }
}
