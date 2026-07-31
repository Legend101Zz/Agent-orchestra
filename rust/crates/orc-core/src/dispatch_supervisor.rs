//! Detached dispatch supervisor (issue #30).
//!
//! `pio orch delegate` persists a private supervisor spec, re-execs the hidden
//! `pio _dispatch_exec` command, and transfers its durable slot leases to that
//! child. The supervisor owns the leases for the worker's real lifetime,
//! drains both pipes to EOF, and writes terminal state back to the dispatch
//! record. The calling conductor only waits until the brief has been handed to
//! the worker.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dispatch::{
    DeliveryStatus, DispatchFailureKind, DispatchProgress, ExecutionStatus, MAX_CAPTURED_BYTES,
    PROGRESS_LOG_MAX_BYTES, ProgressPaths, TRUNCATION_MARKER, kind_label, progress_paths,
    read_dispatch_unreconciled, supervisor_spec_path, write_dispatch,
};
use crate::dispatch_progress::{CloseReason, ProgressJournal, ProgressLog, StreamCounters};
use crate::invocation::Invocation;
use crate::ratelimit::{self, BackoffPolicy};
use crate::registry::{atomic_write_json, now_iso};
use crate::runner::{Usage, extract_adapter_event};
use crate::spawn_guard::{self, SlotLease};
use crate::tasks::{
    TaskActor, record_delivery, record_execution, record_review_delivery, record_review_execution,
};

/// Serialized backoff policy passed from the caller to the detached process.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct PolicySpec {
    min_delay_ms: u64,
    max_delay_ms: u64,
    factor: f32,
    max_retries: usize,
    jitter: bool,
}

impl From<&BackoffPolicy> for PolicySpec {
    fn from(policy: &BackoffPolicy) -> Self {
        Self {
            min_delay_ms: u64::try_from(policy.min_delay.as_millis()).unwrap_or(u64::MAX),
            max_delay_ms: u64::try_from(policy.max_delay.as_millis()).unwrap_or(u64::MAX),
            factor: policy.factor,
            max_retries: policy.max_retries,
            jitter: policy.jitter,
        }
    }
}

impl From<PolicySpec> for BackoffPolicy {
    fn from(policy: PolicySpec) -> Self {
        Self {
            min_delay: Duration::from_millis(policy.min_delay_ms),
            max_delay: Duration::from_millis(policy.max_delay_ms),
            factor: policy.factor,
            max_retries: policy.max_retries,
            jitter: policy.jitter,
        }
    }
}

/// Everything the detached supervisor needs after the conductor returns.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct SupervisorSpec {
    session: String,
    dispatch: String,
    program: PathBuf,
    invocation: Invocation,
    prompt: String,
    cwd: Option<PathBuf>,
    timeout_sec: u64,
    adapter: String,
    policy: PolicySpec,
    lease_paths: Vec<PathBuf>,
    confirmed_link: String,
    actor: TaskActor,
}

/// Inputs used to build a private supervisor spec without exposing its wire
/// representation to the rest of core.
pub(crate) struct SupervisorRequest<'a> {
    pub session: &'a str,
    pub dispatch: &'a str,
    pub program: &'a Path,
    pub invocation: &'a Invocation,
    pub prompt: &'a str,
    pub cwd: Option<&'a Path>,
    pub timeout: Duration,
    pub adapter: &'a str,
    pub policy: &'a BackoffPolicy,
    pub lease_paths: Vec<PathBuf>,
    pub confirmed_link: &'a str,
    pub actor: TaskActor,
}

impl SupervisorSpec {
    fn new(request: &SupervisorRequest<'_>) -> Self {
        Self {
            session: request.session.to_owned(),
            dispatch: request.dispatch.to_owned(),
            program: request.program.to_path_buf(),
            invocation: request.invocation.clone(),
            prompt: request.prompt.to_owned(),
            cwd: request.cwd.map(Path::to_path_buf),
            timeout_sec: request.timeout.as_secs(),
            adapter: request.adapter.to_owned(),
            policy: request.policy.into(),
            lease_paths: request.lease_paths.clone(),
            confirmed_link: request.confirmed_link.to_owned(),
            actor: request.actor,
        }
    }
}

/// Locate the user-facing `pio` binary for the hidden supervisor re-exec.
///
/// Installed `pio`/`pio-mcp` binaries are siblings. Cargo integration tests
/// live under `target/debug/deps`, so the parent directory is also checked.
fn pio_executable() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ORC_PIO_BIN").map(PathBuf::from)
        && path.is_file()
    {
        return Ok(path);
    }
    let current = std::env::current_exe().context("locate current executable")?;
    if current.file_name().is_some_and(|name| name == "pio") {
        return Ok(current);
    }
    let sibling = current.with_file_name("pio");
    if sibling.is_file() {
        return Ok(sibling);
    }
    if current
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "deps")
        && let Some(debug_dir) = current.parent().and_then(Path::parent)
    {
        let cargo_sibling = debug_dir.join("pio");
        if cargo_sibling.is_file() {
            return Ok(cargo_sibling);
        }
    }
    crate::adapter::locate_executable("pio")
        .ok_or_else(|| anyhow!("pio executable not found for detached dispatch supervisor"))
}

/// Persist a spec, launch the hidden supervisor, and transfer every slot lease
/// to its pid. On any partial handoff failure the child is killed and matching
/// lease records are removed.
pub(crate) fn launch(request: SupervisorRequest<'_>, leases: Vec<SlotLease>) -> Result<u32> {
    let spec = SupervisorSpec::new(&request);
    let path = supervisor_spec_path(request.session, request.dispatch);
    atomic_write_json(&path, &spec)?;

    let mut command = Command::new(pio_executable()?);
    command
        .arg("_dispatch_exec")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .context("launch detached dispatch supervisor")?;
    let pid = child.id();
    for lease in leases {
        if let Err(error) = lease.handoff_to(pid) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = spawn_guard::release_dispatch_slots(request.dispatch);
            let _ = fs::remove_file(&path);
            return Err(error).context("transfer dispatch slot to supervisor");
        }
    }
    Ok(pid)
}

/// Bytes kept from the start of a raw stream for fallback diagnostics.
const HEAD_BYTES: usize = MAX_CAPTURED_BYTES / 4;
/// Bytes kept from the end, where an unstructured worker's answer usually is.
const TAIL_BYTES: usize = MAX_CAPTURED_BYTES - HEAD_BYTES;

#[derive(Default)]
struct Captured {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    dropped: usize,
    answer: String,
    usage: Option<Usage>,
    /// Every byte read from the pipe, including what the window later drops.
    /// Exact and monotone, unlike anything derivable from `head`/`tail`, which
    /// is why issue #49 phase 2's journal reports these rather than a length.
    bytes: u64,
    /// Complete newline-terminated lines read.
    ///
    /// Counted on the terminator, not on the read: `read_until` also returns
    /// the unterminated remainder at EOF, and counting that would report a
    /// worker that wrote `alpha\nbeta` as having produced two lines when only
    /// one is complete. The distinction is the point — a block-buffered worker
    /// mid-line is exactly the case the journal must not overstate.
    lines: u64,
}

impl Captured {
    fn push(&mut self, chunk: &[u8]) {
        self.bytes += chunk.len() as u64;
        if chunk.last() == Some(&b'\n') {
            self.lines += 1;
        }
        let mut chunk = chunk;
        if self.head.len() < HEAD_BYTES {
            let take = (HEAD_BYTES - self.head.len()).min(chunk.len());
            self.head.extend_from_slice(&chunk[..take]);
            chunk = &chunk[take..];
        }
        self.tail.extend(chunk.iter().copied());
        while self.tail.len() > TAIL_BYTES {
            self.tail.pop_front();
            self.dropped += 1;
        }
    }

    fn inspect_event(&mut self, adapter: Option<&str>, line: &[u8]) {
        let Some(adapter) = adapter else { return };
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let (text, usage) = extract_adapter_event(adapter, &value);
        if let Some(text) = text {
            self.answer.push_str(text);
        }
        if usage.is_some() {
            self.usage = usage;
        }
    }

    fn raw(&self) -> String {
        let mut rendered = String::from_utf8_lossy(&self.head).into_owned();
        if self.dropped > 0 {
            rendered.push_str(&format!("{TRUNCATION_MARKER} ({} bytes)", self.dropped));
        }
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        rendered.push_str(&String::from_utf8_lossy(&tail));
        rendered
    }

    fn result(&self) -> StreamResult {
        let raw = self.raw();
        StreamResult {
            persisted: if self.answer.is_empty() {
                raw.clone()
            } else {
                self.answer.clone()
            },
            raw,
            usage: self.usage.clone(),
        }
    }
}

struct Drain {
    captured: Arc<Mutex<Captured>>,
    handle: thread::JoinHandle<()>,
    /// Bytes this stream's progress log holds, published by the drain thread.
    ///
    /// The log lives inside the drain thread — it is written there and never
    /// shared — so its length has to come back out somehow, and an atomic is
    /// the cheapest way that does not put the log behind the capture's mutex.
    kept: Arc<AtomicU64>,
}

impl Drain {
    fn finish(self) -> StreamResult {
        let Self {
            captured, handle, ..
        } = self;
        let _ = handle.join();
        capture_result(&captured)
    }

    fn snapshot(&self) -> StreamResult {
        capture_result(&self.captured)
    }

    /// A non-consuming, non-joining read of five integers (issue #49 phase 2).
    ///
    /// Deliberately **not** [`Drain::snapshot`], which goes through
    /// `Captured::result` → `raw()` and so allocates a 16 KiB `String` and runs
    /// two `from_utf8_lossy` passes while holding the mutex the drain thread
    /// takes once per line. This is what the supervisor's wait loop calls
    /// several times a second; `snapshot` stays exactly as it is, for the
    /// timeout branch that needs the text.
    ///
    /// `kept` comes from the log itself rather than from the capture, because
    /// they diverge: the in-memory window slides while the log freezes at its
    /// cap.
    fn counters(&self) -> StreamCounters {
        let read = |slot: &Captured| (slot.bytes, slot.lines, slot.dropped as u64);
        let (bytes, lines, dropped) = self
            .captured
            .lock()
            .map_or_else(|poisoned| read(&poisoned.into_inner()), |slot| read(&slot));
        StreamCounters {
            bytes,
            lines,
            dropped,
            kept: self.kept.load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

fn capture_result(captured: &Arc<Mutex<Captured>>) -> StreamResult {
    captured.lock().map_or_else(
        |poisoned| poisoned.into_inner().result(),
        |slot| slot.result(),
    )
}

#[derive(Default)]
struct StreamResult {
    persisted: String,
    raw: String,
    usage: Option<Usage>,
}

/// Drain one stream to EOF on its own thread while retaining a bounded raw
/// head+tail window and, for adapters with a proven extractor, only the
/// assistant answer plus exact usage.
fn drain_to_eof<R: std::io::Read + Send + 'static>(
    reader: R,
    adapter: Option<String>,
    mut log: ProgressLog,
) -> Drain {
    let captured = Arc::new(Mutex::new(Captured::default()));
    let sink = Arc::clone(&captured);
    let kept = Arc::new(AtomicU64::new(0));
    let published = Arc::clone(&kept);
    let handle = thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        loop {
            let mut line = Vec::new();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    // Mirror before taking the capture's lock. Issue #49 phase
                    // 2: this is the whole of "durable before it exits", and it
                    // is the only work added to the thread whose job is to keep
                    // the pipe empty — measured free, 20k lines through a real
                    // pipe took the same time mirrored or not.
                    log.append(&line);
                    published.store(log.kept(), std::sync::atomic::Ordering::Relaxed);
                    let Ok(mut slot) = sink.lock() else { break };
                    slot.push(&line);
                    slot.inspect_event(adapter.as_deref(), &line);
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
    Drain {
        captured,
        handle,
        kept,
    }
}

struct Invoked {
    exit_code: Option<i32>,
    stdout: String,
    raw_stdout: String,
    stderr: String,
    usage: Option<Usage>,
    success: bool,
}

type InvokeFailure = Box<(DispatchFailureKind, Option<Invoked>)>;

/// Open this attempt's three progress artifacts (issue #49 phase 2).
///
/// Called immediately after `spawn` and **before** `on_started`, so that a
/// failed delivery handshake still leaves whatever the worker managed to say.
/// The logs are created empty here rather than on first write, which is what
/// makes "present and zero-length" mean *we looked and there was nothing yet*
/// and "absent" mean *this supervisor is not streaming*.
fn open_progress(
    spec: &SupervisorSpec,
    attempt: u32,
) -> (
    ProgressPaths,
    ProgressLog,
    ProgressLog,
    ProgressJournal,
    Vec<String>,
) {
    let paths = progress_paths(&spec.session, &spec.dispatch, attempt);
    let mut warnings = Vec::new();
    if let Some(parent) = paths.journal.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let (stdout_log, stdout_warning) = ProgressLog::create(&paths.stdout_log);
    let (stderr_log, stderr_warning) = ProgressLog::create(&paths.stderr_log);
    let extractable = crate::runner::has_extractor(&spec.adapter);
    let (journal, journal_warning) =
        ProgressJournal::open(&paths.journal, attempt, &spec.adapter, extractable);
    warnings.extend(
        [stdout_warning, stderr_warning, journal_warning]
            .into_iter()
            .flatten(),
    );
    (paths, stdout_log, stderr_log, journal, warnings)
}

/// The counters for both streams right now, for a journal frame.
fn counters_of(stdout: Option<&Drain>, stderr: Option<&Drain>) -> (StreamCounters, StreamCounters) {
    (
        stdout.map(Drain::counters).unwrap_or_default(),
        stderr.map(Drain::counters).unwrap_or_default(),
    )
}

fn invoke_harness<F>(
    spec: &SupervisorSpec,
    attempt: u32,
    warnings: &RefCell<Vec<String>>,
    mut on_started: F,
) -> std::result::Result<Invoked, InvokeFailure>
where
    F: FnMut(u32, DispatchProgress) -> Result<()>,
{
    let mut command = Command::new(&spec.program);
    command.args(&spec.invocation.args);
    if spec.invocation.uses_stdin() {
        command.stdin(Stdio::piped());
    } else {
        command.arg(&spec.prompt).stdin(Stdio::null());
    }
    if let Some(dir) = &spec.cwd {
        command.current_dir(dir);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| Box::new((DispatchFailureKind::MissingExecutable, None)))?;
    let (paths, stdout_log, stderr_log, mut journal, progress_warnings) =
        open_progress(spec, attempt);
    // Durable, on the record, where `pio dispatch list/show` prints them.
    // Without this the artifacts can silently not exist while `record.progress`
    // still names them — a third state nothing documents and nothing explains.
    warnings.borrow_mut().extend(progress_warnings);
    let progress = DispatchProgress {
        attempt,
        stdout_log: file_name_of(&paths.stdout_log),
        stderr_log: file_name_of(&paths.stderr_log),
        journal: file_name_of(&paths.journal),
        adapter: spec.adapter.clone(),
        extractable: crate::runner::has_extractor(&spec.adapter),
        log_max_bytes: PROGRESS_LOG_MAX_BYTES as u64,
        extra: Default::default(),
    };
    let stdout_drain = child
        .stdout
        .take()
        .map(|stdout| drain_to_eof(stdout, Some(spec.adapter.clone()), stdout_log));
    let stderr_drain = child
        .stderr
        .take()
        .map(|stderr| drain_to_eof(stderr, None, stderr_log));
    if spec.invocation.uses_stdin()
        && let Some(mut stdin) = child.stdin.take()
    {
        let _ = stdin.write_all(spec.prompt.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    }
    if on_started(child.id(), progress).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        // The handshake failed, but the worker may already have spoken. Close
        // the journal from this thread so the artifacts are complete on an
        // exit path that never reaches the wait loop.
        let (out, err) = counters_of(stdout_drain.as_ref(), stderr_drain.as_ref());
        journal.close(CloseReason::HandshakeFailed, out, err);
        return Err(Box::new((DispatchFailureKind::HarnessError, None)));
    }

    let timeout = Duration::from_secs(spec.timeout_sec);
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let (out, err) = counters_of(stdout_drain.as_ref(), stderr_drain.as_ref());
                journal.close(CloseReason::Eof, out, err);
                let stdout = stdout_drain.map(Drain::finish).unwrap_or_default();
                let stderr = stderr_drain.map(Drain::finish).unwrap_or_default();
                return Ok(Invoked {
                    exit_code: status.code(),
                    stdout: stdout.persisted,
                    raw_stdout: stdout.raw,
                    stderr: stderr.raw,
                    usage: stdout.usage,
                    success: status.success(),
                });
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    crate::runner::terminate_pid(child.id());
                    let _ = child.wait();
                    let (out, err) = counters_of(stdout_drain.as_ref(), stderr_drain.as_ref());
                    journal.close(CloseReason::Timeout, out, err);
                    let stdout = stdout_drain
                        .as_ref()
                        .map(Drain::snapshot)
                        .unwrap_or_default();
                    let stderr = stderr_drain
                        .as_ref()
                        .map(Drain::snapshot)
                        .unwrap_or_default();
                    return Err(Box::new((
                        DispatchFailureKind::Timeout,
                        Some(Invoked {
                            exit_code: Some(124),
                            stdout: stdout.persisted,
                            raw_stdout: stdout.raw,
                            stderr: stderr.raw,
                            usage: stdout.usage,
                            success: false,
                        }),
                    )));
                }
                // Gate A and gate B live inside `note`: it writes only if a
                // counter moved and only if the floor has elapsed. Calling it
                // on every 25 ms tick is therefore not a 40 Hz cadence — a
                // silent worker produces no write at all.
                let (out, err) = counters_of(stdout_drain.as_ref(), stderr_drain.as_ref());
                journal.note(out, err);
                thread::sleep(
                    Duration::from_millis(25).min(timeout.saturating_sub(started.elapsed())),
                );
            }
            Err(_) => {
                let (out, err) = counters_of(stdout_drain.as_ref(), stderr_drain.as_ref());
                journal.close(CloseReason::WaitError, out, err);
                return Err(Box::new((DispatchFailureKind::HarnessError, None)));
            }
        }
    }
}

enum AttemptError {
    RateLimited(Invoked),
    Terminal(DispatchFailureKind, Option<Invoked>),
}

struct BackedOff {
    result: std::result::Result<Invoked, InvokeFailure>,
    warnings: Vec<String>,
}

fn invoke_with_backoff<F>(
    spec: &SupervisorSpec,
    policy: &BackoffPolicy,
    mut on_started: F,
) -> BackedOff
where
    F: FnMut(u32, DispatchProgress) -> Result<()>,
{
    let warnings = RefCell::new(Vec::new());
    // `ratelimit::run_with_backoff` exposes no attempt ordinal, and issue #49
    // phase 2 needs one: every attempt gets its own progress files, so that a
    // rate-limited attempt's bytes are neither deleted nor silently spliced
    // onto the next attempt's. Which attempt produced which bytes becomes a
    // fact of the filesystem rather than a marker a reader has to notice.
    let attempts = std::cell::Cell::new(0_u32);
    let attempt = || -> std::result::Result<Invoked, AttemptError> {
        let ordinal = attempts.get() + 1;
        attempts.set(ordinal);
        match invoke_harness(spec, ordinal, &warnings, &mut on_started) {
            Ok(invoked) if invoked.success => Ok(invoked),
            Ok(invoked) => {
                let combined = format!("{}\n{}", invoked.raw_stdout, invoked.stderr);
                if ratelimit::is_rate_limited(&spec.adapter, &combined) {
                    Err(AttemptError::RateLimited(invoked))
                } else {
                    Err(AttemptError::Terminal(
                        DispatchFailureKind::HarnessError,
                        Some(invoked),
                    ))
                }
            }
            Err(failure) => {
                let (kind, invoked) = *failure;
                Err(AttemptError::Terminal(kind, invoked))
            }
        }
    };
    let is_retryable = |error: &AttemptError| matches!(error, AttemptError::RateLimited(_));
    let notify = |error: &AttemptError, delay: Duration| {
        if let AttemptError::RateLimited(invoked) = error {
            let combined = format!("{}\n{}", invoked.raw_stdout, invoked.stderr);
            let hint = ratelimit::detect(&spec.adapter, &combined)
                .and_then(|signal| signal.retry_after)
                .map_or_else(String::new, |seconds| {
                    format!(" (harness asked for ~{seconds}s)")
                });
            warnings.borrow_mut().push(format!(
                "ORC WARNING: {} worker rate-limited{hint}; backing off {:.1}s before retry",
                spec.adapter,
                delay.as_secs_f64()
            ));
        }
    };
    let result = ratelimit::run_with_backoff(policy, attempt, is_retryable, notify);
    let result = match result {
        Ok(invoked) => Ok(invoked),
        Err(AttemptError::RateLimited(invoked)) => {
            Err(Box::new((DispatchFailureKind::RateLimited, Some(invoked))))
        }
        Err(AttemptError::Terminal(kind, invoked)) => Err(Box::new((kind, invoked))),
    };
    BackedOff {
        result,
        warnings: warnings.into_inner(),
    }
}

/// The file's own name, for the durable record's relative pointer.
fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn mark_started(spec: &SupervisorSpec, worker_pid: u32, progress: DispatchProgress) -> Result<()> {
    let mut record = read_dispatch_unreconciled(&spec.session, &spec.dispatch)?;
    let first_delivery = !record.is_confirmed();
    record.status = DeliveryStatus::Confirmed.as_str().to_owned();
    record.execution_status = Some(ExecutionStatus::Running.as_str().to_owned());
    record.supervisor_pid = Some(std::process::id());
    record.worker_pid = Some(worker_pid);
    // Rides the write that already happens here, so the pointer costs no extra
    // fsync. It is discoverability only — `dispatch::progress_paths` derives
    // the same paths without reading anything, which is what a reader should
    // rely on, because `write_dispatch` takes no lock.
    record.progress = Some(progress);
    record.started_at.get_or_insert_with(now_iso);
    record.updated_at = now_iso();
    write_dispatch(&record)?;
    if first_delivery {
        let detail = format!(
            "dispatch {} delivered to {}; worker running",
            record.id, record.harness
        );
        // Both halves take the same link, computed the same way by
        // `dispatch::deliver` — the selected pane, else the requested run,
        // else the dispatch id. They just write it to different fields.
        if record.is_review() {
            record_review_delivery(
                &record.session,
                &record.task,
                spec.actor,
                Some(spec.confirmed_link.clone()),
                detail,
            )?;
        } else {
            record_delivery(
                &record.session,
                &record.task,
                spec.actor,
                Some(spec.confirmed_link.clone()),
                detail,
            )?;
        }
    }
    Ok(())
}

fn persist_terminal(
    spec: &SupervisorSpec,
    result: std::result::Result<Invoked, InvokeFailure>,
    warnings: Vec<String>,
) -> Result<i32> {
    let mut record = read_dispatch_unreconciled(&spec.session, &spec.dispatch)?;
    record.warnings = warnings;
    record.worker_pid = None;
    record.ended_at = Some(now_iso());
    record.updated_at = now_iso();
    // Whether a worker actually ran, and how it ended. `None` means it never
    // got as far as running, so the `delivery_failed` written below is the
    // whole story and there is no execution to report.
    let (code, executed) = match result {
        Ok(invoked) => {
            record.execution_status = Some(ExecutionStatus::Succeeded.as_str().to_owned());
            record.exit_code = invoked.exit_code;
            record.stdout = invoked.stdout;
            record.stderr = invoked.stderr;
            record.usage = invoked.usage;
            // `invoke_with_backoff` routes every non-success exit into
            // `AttemptError`, so reaching here means `status.success()` was
            // true — and `invoke_harness` only returns it after `Drain::finish`
            // has joined both reader threads, so the answer is EOF-complete.
            (invoked.exit_code.unwrap_or(0), Some(true))
        }
        Err(failure) => {
            let (kind, invoked) = *failure;
            let delivered = record.is_confirmed();
            if !delivered {
                record.status = DeliveryStatus::Failed.as_str().to_owned();
                let detail = format!(
                    "dispatch {} failed before delivery: {}",
                    record.id,
                    super::dispatch::failure_message(&kind, &record.harness)
                );
                if record.is_review() {
                    record_review_delivery(
                        &record.session,
                        &record.task,
                        spec.actor,
                        None,
                        detail,
                    )?;
                } else {
                    record_delivery(&record.session, &record.task, spec.actor, None, detail)?;
                }
            }
            record.execution_status = Some(ExecutionStatus::Failed.as_str().to_owned());
            record.failure_kind = Some(kind_label(&kind).to_owned());
            record.error = Some(super::dispatch::failure_message(&kind, &record.harness));
            if let Some(invoked) = invoked {
                record.exit_code = invoked.exit_code;
                record.stdout = invoked.stdout;
                record.stderr = invoked.stderr;
                record.usage = invoked.usage;
            }
            (record.exit_code.unwrap_or(1), delivered.then_some(false))
        }
    };
    if let Some(succeeded) = executed {
        append_execution(spec, &mut record, succeeded);
    }
    write_dispatch(&record)?;
    Ok(code)
}

/// Append the completion event to the task board, before the dispatch record
/// is written.
///
/// This is issue #49's defect 1: the success path used to write the whole
/// terminal record — status, exit code, answer, usage — and append **nothing**
/// to the board, so the last durable word about any dispatch was
/// `delivery_confirmed`, which means "the process started". Nothing ever said
/// the answer had arrived.
///
/// Ordered before `write_dispatch` deliberately. It makes "the dispatch is
/// terminal" imply "the board has been told", which is what lets a test wait
/// on one and assert the other instead of racing two writers; and it means the
/// board can never be left permanently silent about a dispatch that has
/// visibly finished.
///
/// Best-effort, and the miss is durable rather than swallowed. Propagating
/// here would abort `execute` before it removes the supervisor spec and calls
/// `drain_queued`, so one contended board would leak a spec file and stall
/// every queued dispatch in the session — a worse failure than a missing
/// animation. A refusal is recorded on the dispatch's own warnings, where
/// `pio dispatch status` shows it.
fn append_execution(
    spec: &SupervisorSpec,
    record: &mut crate::dispatch::DispatchRecord,
    succeeded: bool,
) {
    let detail = format!(
        "dispatch {} {} on {} (exit {})",
        record.id,
        if succeeded { "answered" } else { "failed" },
        record.harness,
        record
            .exit_code
            .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
    );
    let appended = if record.is_review() {
        record_review_execution(&record.session, &record.task, spec.actor, succeeded, detail)
    } else {
        record_execution(&record.session, &record.task, spec.actor, succeeded, detail)
    };
    if let Err(error) = appended {
        record.warnings.push(format!(
            "ORC WARNING: dispatch {} finished but the task board refused its completion event: {error}",
            record.id
        ));
    }
}

/// Run one hidden supervisor spec. Called only by `pio _dispatch_exec`.
pub fn execute(path: &Path) -> Result<i32> {
    let spec: SupervisorSpec = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("parse dispatch supervisor spec {}", path.display()))?;
    let mut leases = Vec::with_capacity(spec.lease_paths.len());
    for lease_path in &spec.lease_paths {
        leases.push(spawn_guard::adopt_slot(lease_path)?);
    }

    let mut record = read_dispatch_unreconciled(&spec.session, &spec.dispatch)?;
    record.supervisor_pid = Some(std::process::id());
    record.execution_status = Some(ExecutionStatus::Starting.as_str().to_owned());
    record.updated_at = now_iso();
    write_dispatch(&record)?;

    let policy: BackoffPolicy = spec.policy.clone().into();
    let backed_off = invoke_with_backoff(&spec, &policy, |worker_pid, progress| {
        mark_started(&spec, worker_pid, progress)
    });
    let code = persist_terminal(&spec, backed_off.result, backed_off.warnings)?;

    drop(leases);
    let _ = fs::remove_file(path);
    // A worker finishing is the event that frees real capacity. Drain only
    // after both session and harness leases are gone.
    let _ = crate::dispatch::drain_queued(&spec.session);
    Ok(code)
}
