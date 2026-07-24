//! Bounded durable brain-to-worker command path.
//!
//! Phase 4A dispatches are explicit, recorded, and bounded. Every dispatch
//! carries a known actor (brain or human), an owning session, a worker pane
//! or harness key, a prompt body, and a delivery state machine that moves
//! from `pending` through either `confirmed` (exit code 0) or `failed`
//! (missing executable, capability unavailable, non-zero exit, bounded
//! timeout, or unparseable response).
//!
//! Dispatch is layered above the daemon/core registry and never injects
//! keystrokes into a PTY. It uses a configured non-interactive command
//! template such as Hermes' demonstrated `--oneshot` (`-z`) flag and pipes
//! the prompt through the standard input or trailing argument of that
//! command. The harness record declares whether stdin or argv should carry
//! the prompt and the bounded timeout for one invocation.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
use crate::invocation::{Invocation, resolve_worker_invocation};
use crate::probe::probed_from;
use crate::registry::{atomic_write_json, home, now_iso};
use crate::tasks::{Task, TaskActor, TaskStatus, read_task, record_delivery};

/// Maximum bytes of stdout captured from one dispatch invocation.
pub const MAX_CAPTURED_BYTES: usize = 16 * 1024;

/// Outcome of one recorded dispatch invocation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    /// The dispatch was recorded but the harness invocation has not completed.
    Pending,
    /// The harness exited successfully and produced parseable output.
    Confirmed,
    /// The dispatch could not be delivered or did not return success.
    Failed,
}

impl DeliveryStatus {
    /// Return the durable lowercase delivery word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
        }
    }

    /// Parse the durable delivery word.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
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
    /// The harness exited non-zero or returned a malformed response.
    HarnessError,
}

/// Plain additive durable dispatch record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
    /// Delivery state after the bounded invocation.
    pub status: String,
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
    /// Optional bounded timeout override in seconds.
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
fn effective_cwd(session: &BenchSession, task: &Task) -> Option<String> {
    let worktree = task
        .worktree
        .as_ref()
        .and_then(|worktree| worktree.path.clone());
    [worktree, Some(session.cwd.clone())]
        .into_iter()
        .flatten()
        .find(|dir| Path::new(dir).is_dir())
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

fn bounded_capture<R: Read>(mut reader: R, max: usize) -> Result<String> {
    let mut buffer = Vec::with_capacity(max.min(1024));
    let mut chunk = [0_u8; 1024];
    loop {
        let taken = reader.read(&mut chunk)?;
        if taken == 0 {
            break;
        }
        if buffer.len() + taken > max {
            let remaining = max.saturating_sub(buffer.len());
            buffer.extend_from_slice(&chunk[..remaining]);
            break;
        }
        buffer.extend_from_slice(&chunk[..taken]);
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
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

fn invoke_harness(
    program: &Path,
    invocation: &Invocation,
    prompt: &str,
    cwd: Option<&Path>,
    timeout: Duration,
) -> std::result::Result<(Option<i32>, String, String), DispatchFailureKind> {
    let mut command = Command::new(program);
    for arg in &invocation.args {
        command.arg(arg);
    }
    if invocation.uses_stdin() {
        command.stdin(Stdio::piped());
    } else {
        command.arg(prompt).stdin(Stdio::null());
    }
    // Orchestrator-provided cwd control: the worker runs in the task's effective
    // working directory (issue #6), independent of any per-adapter --dir flag.
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|_| DispatchFailureKind::MissingExecutable)?;
    if invocation.uses_stdin()
        && let Some(mut stdin) = child.stdin.take()
    {
        let _ = stdin.write_all(prompt.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout_handle = child.stdout.take();
                let mut stderr_handle = child.stderr.take();
                let stdout = stdout_handle
                    .as_mut()
                    .map(|handle| bounded_capture(handle, MAX_CAPTURED_BYTES))
                    .transpose()
                    .map_err(|_| DispatchFailureKind::HarnessError)?
                    .unwrap_or_default();
                let stderr = stderr_handle
                    .as_mut()
                    .map(|handle| bounded_capture(handle, MAX_CAPTURED_BYTES))
                    .transpose()
                    .map_err(|_| DispatchFailureKind::HarnessError)?
                    .unwrap_or_default();
                let code = status.code();
                return if status.success() {
                    Ok((code, stdout, stderr))
                } else {
                    Err(DispatchFailureKind::HarnessError)
                };
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(DispatchFailureKind::Timeout);
                }
                thread::sleep(
                    Duration::from_millis(25).min(timeout.saturating_sub(started.elapsed())),
                );
            }
            Err(_) => return Err(DispatchFailureKind::HarnessError),
        }
    }
}

fn failure_message(kind: &DispatchFailureKind, harness: &str) -> String {
    match kind {
        DispatchFailureKind::UnknownHarness => format!("UNKNOWN HARNESS: {harness}"),
        DispatchFailureKind::CapabilityUnavailable => {
            format!("CAPABILITY UNAVAILABLE: {harness} has no non-interactive dispatch_args")
        }
        DispatchFailureKind::MissingExecutable => {
            format!("MISSING EXECUTABLE: {harness} command not found on PATH")
        }
        DispatchFailureKind::Timeout => "DISPATCH TIMEOUT".to_owned(),
        DispatchFailureKind::HarnessError => "HARNESS ERROR".to_owned(),
    }
}

/// Record and dispatch one bounded command through the configured worker harness.
///
/// Returns the durable [`DispatchRecord`] describing the delivery state.
pub fn dispatch(request: &DispatchRequest) -> Result<DispatchRecord> {
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
    let session = read_session(&request.session)
        .with_context(|| format!("missing dispatch session {}", request.session))?;
    let task = read_task(&session.id, &request.task)
        .with_context(|| format!("missing dispatch task {}", request.task))?;
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
    let cwd = effective_cwd(&session, &task);
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
                kind,
                detail,
                placeholder_command(&registry, &resolved_key),
                cwd,
                "worker capability unavailable",
            );
        }
    };

    // The adapter chooses the invocation style from the probe results (issue #6);
    // an honest refusal names the missing capability instead of guessing one.
    let probed = probed_from(&registry, &config.adapter);
    let invocation = match resolve_worker_invocation(config, &probed, cwd.as_deref().map(Path::new))
    {
        Ok(invocation) => invocation,
        Err(error) => {
            return persist_failure(
                request,
                &resolved_key,
                DispatchFailureKind::CapabilityUnavailable,
                error.message(&resolved_key),
                placeholder_command(&registry, &resolved_key),
                cwd,
                "worker capability unavailable",
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
                DispatchFailureKind::MissingExecutable,
                failure_message(&DispatchFailureKind::MissingExecutable, &resolved_key),
                command_line,
                cwd,
                "worker executable unavailable",
            );
        }
    };
    let timeout = Duration::from_secs(
        request
            .timeout_sec
            .unwrap_or_else(|| dispatch_timeout_for(config)),
    );
    let mut record = new_record(request, &resolved_key, &command_line, cwd.clone());
    match invoke_harness(
        &program,
        &invocation,
        &request.prompt,
        cwd.as_deref().map(Path::new),
        timeout,
    ) {
        Ok((exit_code, stdout, stderr)) => {
            record.status = DeliveryStatus::Confirmed.as_str().to_owned();
            record.exit_code = exit_code;
            record.stdout = stdout;
            record.stderr = stderr;
            record.updated_at = now_iso();
        }
        Err(kind) => {
            record.status = DeliveryStatus::Failed.as_str().to_owned();
            record.failure_kind = Some(kind_label(&kind).to_owned());
            record.error = Some(failure_message(&kind, &resolved_key));
            record.updated_at = now_iso();
        }
    }
    write_dispatch(&record)?;
    let task_actor = TaskActor::from(request.actor);
    if record.is_confirmed() {
        let link = selected_pane
            .clone()
            .or_else(|| request.run.clone())
            .unwrap_or_else(|| record.id.clone());
        record_delivery(
            &request.session,
            &request.task,
            task_actor,
            Some(link),
            format!("dispatch {} confirmed by {}", record.id, record.harness),
        )?;
    } else {
        record_delivery(
            &request.session,
            &request.task,
            task_actor,
            None,
            format!(
                "dispatch {} failed: {}",
                record.id,
                record
                    .error
                    .as_deref()
                    .unwrap_or("worker did not confirm delivery")
            ),
        )?;
    }
    Ok(record)
}

fn kind_label(kind: &DispatchFailureKind) -> &'static str {
    match kind {
        DispatchFailureKind::UnknownHarness => "unknown_harness",
        DispatchFailureKind::CapabilityUnavailable => "capability_unavailable",
        DispatchFailureKind::MissingExecutable => "missing_executable",
        DispatchFailureKind::Timeout => "timeout",
        DispatchFailureKind::HarnessError => "harness_error",
    }
}

fn new_record(
    request: &DispatchRequest,
    harness: &str,
    command_line: &str,
    cwd: Option<String>,
) -> DispatchRecord {
    let now = now_iso();
    DispatchRecord {
        id: dispatch_id(harness, &request.session),
        session: request.session.clone(),
        task: request.task.clone(),
        actor: request.actor.as_str().to_owned(),
        harness: harness.to_owned(),
        pane_id: request.pane_id.clone(),
        run: request.run.clone(),
        command_line: command_line.to_owned(),
        cwd,
        prompt: request.prompt.clone(),
        status: DeliveryStatus::Pending.as_str().to_owned(),
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        failure_kind: None,
        error: None,
        created_at: now.clone(),
        updated_at: now,
        extra: BTreeMap::new(),
    }
}

/// Build, persist, and record delivery for one pre-invocation failure.
///
/// Consolidates the unknown-harness, capability-unavailable, and
/// missing-executable failure paths so every refusal is durable and shows up on
/// the task's delivery history identically.
fn persist_failure(
    request: &DispatchRequest,
    harness: &str,
    kind: DispatchFailureKind,
    detail: String,
    command_line: String,
    cwd: Option<String>,
    default_reason: &str,
) -> Result<DispatchRecord> {
    let mut record = new_record(request, harness, &command_line, cwd);
    record.status = DeliveryStatus::Failed.as_str().to_owned();
    record.failure_kind = Some(kind_label(&kind).to_owned());
    record.error = Some(detail);
    record.updated_at = now_iso();
    write_dispatch(&record)?;
    record_delivery(
        &request.session,
        &request.task,
        TaskActor::from(request.actor),
        None,
        format!(
            "dispatch {} failed: {}",
            record.id,
            record.error.as_deref().unwrap_or(default_reason)
        ),
    )?;
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
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<DispatchRecord>(&bytes).ok())
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
    if session.trim().is_empty() {
        bail!("dispatch session is required")
    }
    validate_id(id)?;
    let path = dispatch_path(session, id);
    serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("parse dispatch {}", path.display()))
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
}
