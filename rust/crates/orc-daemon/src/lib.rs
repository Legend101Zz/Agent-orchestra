#![warn(missing_docs)]
//! Production Unix-socket daemon for the Bench workspace.
//!
//! The daemon owns hosted PTYs and screen replay. It must never render UI or
//! make orchestration policy decisions.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use orc_core::bench::{
    PaneLayout, SessionPaneRecord, create_session as create_bench_session, list_sessions,
    load_harness_registry, read_session, write_session,
};
use orc_core::tasks::{TaskActor, TaskStatus, diff_task, list_tasks, move_task};
use orc_proto::{
    ClientRequest, DaemonMetrics, HarnessSummary, LayoutRect, PROTOCOL_VERSION, PaneSequence,
    PaneSnapshot, ServerResponse, SessionSummary, TaskHistorySummary, TaskSummary,
};
use orc_pty::{HostedPane, UpdateSignal};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

const MAX_MESSAGE_BYTES: u64 = 1024 * 1024;
const MAX_CLIENTS: usize = 16;
static WRITE_NONCE: AtomicU64 = AtomicU64::new(0);
type PaneSize = (u16, u16);
type ClientSizes = HashMap<u64, HashMap<String, PaneSize>>;

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Errors emitted by the Bench daemon.
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Socket or stream I/O failed.
    #[error("daemon I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// A protocol message was invalid.
    #[error("invalid protocol message: {0}")]
    Json(#[from] serde_json::Error),
    /// A hosted pane failed.
    #[error("pane operation failed: {0}")]
    Pane(#[from] orc_pty::PtyError),
    /// A shared pane lock was poisoned.
    #[error("pane state is unavailable")]
    Poisoned,
    /// Another live daemon already owns the configured socket.
    #[error("orcd is already listening at {0}")]
    AlreadyRunning(PathBuf),
    /// An existing socket path is not safe for this user to replace.
    #[error("unsafe stale socket at {0}")]
    UnsafeSocket(PathBuf),
    /// A recorded process could not be inspected or reaped safely.
    #[error("process identity check failed: {0}")]
    Process(String),
    /// A core registry or session mutation failed.
    #[error("daemon core mutation failed: {0}")]
    Core(#[from] anyhow::Error),
}

/// Result type returned by daemon operations.
pub type Result<T> = std::result::Result<T, DaemonError>;

/// Canonical pane state shared by every attached client.
struct PaneMetadata {
    session_id: Option<String>,
    harness: Option<String>,
    role: Option<String>,
    command: String,
    args: Vec<String>,
    resume_args: Vec<String>,
    cwd: PathBuf,
    environment: Vec<(String, String)>,
    down_at: Option<u64>,
}

struct PaneEntry {
    pane: Mutex<HostedPane>,
    metadata: Mutex<PaneMetadata>,
}

/// Canonical pane and session state shared by every attached client.
pub struct Daemon {
    panes: RwLock<Vec<PaneEntry>>,
    signal: UpdateSignal,
    home: Option<PathBuf>,
    clients: AtomicUsize,
    next_client_id: AtomicU64,
    requested_sizes: Mutex<ClientSizes>,
}

impl Daemon {
    /// Construct a daemon whose panes share one output signal.
    #[must_use]
    pub fn new(panes: Vec<HostedPane>, signal: UpdateSignal) -> Self {
        Self {
            panes: RwLock::new(
                panes
                    .into_iter()
                    .map(|pane| PaneEntry {
                        metadata: Mutex::new(PaneMetadata {
                            session_id: None,
                            harness: None,
                            role: None,
                            command: String::new(),
                            args: Vec::new(),
                            resume_args: Vec::new(),
                            cwd: PathBuf::new(),
                            environment: Vec::new(),
                            down_at: None,
                        }),
                        pane: Mutex::new(pane),
                    })
                    .collect(),
            ),
            signal,
            home: None,
            clients: AtomicUsize::new(0),
            next_client_id: AtomicU64::new(1),
            requested_sizes: Mutex::new(HashMap::new()),
        }
    }

    /// Construct an empty production daemon that persists process records under `home`.
    #[must_use]
    pub fn production(home: PathBuf, signal: UpdateSignal) -> Self {
        let mut daemon = Self::new(Vec::new(), signal);
        daemon.home = Some(home);
        daemon
    }

    /// Return aggregate bounded-output and attachment counters.
    pub fn metrics(&self) -> Result<DaemonMetrics> {
        let panes = self
            .panes
            .read()
            .map_err(|_| DaemonError::Poisoned)?
            .iter()
            .map(|entry| {
                entry
                    .pane
                    .lock()
                    .map(|pane| pane.metrics())
                    .map_err(|_| DaemonError::Poisoned)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(DaemonMetrics {
            panes,
            attached_clients: self.clients.load(Ordering::Acquire),
        })
    }

    fn acquire_client(&self) -> bool {
        self.clients
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |clients| {
                (clients < MAX_CLIENTS).then_some(clients + 1)
            })
            .is_ok()
    }

    fn release_client(&self) {
        self.clients.fetch_sub(1, Ordering::AcqRel);
    }

    fn resize_for_client(
        &self,
        client_id: u64,
        pane_id: &str,
        rows: u16,
        cols: u16,
    ) -> Result<ServerResponse> {
        let mut requests = self
            .requested_sizes
            .lock()
            .map_err(|_| DaemonError::Poisoned)?;
        requests
            .entry(client_id)
            .or_default()
            .insert(pane_id.to_owned(), (rows, cols));
        let target = requests
            .values()
            .filter_map(|client| client.get(pane_id))
            .fold((rows, cols), |current, requested| {
                (current.0.max(requested.0), current.1.max(requested.1))
            });
        drop(requests);
        self.with_pane(pane_id, |pane| {
            pane.resize(target.0, target.1)?;
            Ok(())
        })?
        .map_or_else(
            || {
                Ok(ServerResponse::Error {
                    message: format!("unknown pane: {pane_id}"),
                })
            },
            |_| Ok(ServerResponse::Ack),
        )
    }

    fn forget_client_sizes(&self, client_id: u64) -> Result<()> {
        let mut requests = self
            .requested_sizes
            .lock()
            .map_err(|_| DaemonError::Poisoned)?;
        let removed = requests.remove(&client_id).unwrap_or_default();
        let mut remaining = Vec::new();
        for pane_id in removed.keys() {
            let target = requests
                .values()
                .filter_map(|client| client.get(pane_id))
                .fold(None, |current: Option<(u16, u16)>, requested| {
                    Some(current.map_or(*requested, |size| {
                        (size.0.max(requested.0), size.1.max(requested.1))
                    }))
                });
            if let Some(target) = target {
                remaining.push((pane_id.clone(), target));
            }
        }
        drop(requests);
        for (pane_id, (rows, cols)) in remaining {
            let _ = self.with_pane(&pane_id, |pane| {
                pane.resize(rows, cols)?;
                Ok(())
            })?;
        }
        Ok(())
    }

    fn sequences(&self) -> Result<Vec<PaneSequence>> {
        self.panes
            .read()
            .map_err(|_| DaemonError::Poisoned)?
            .iter()
            .map(|entry| {
                let pane = entry.pane.lock().map_err(|_| DaemonError::Poisoned)?;
                Ok(PaneSequence {
                    id: pane.id().to_owned(),
                    sequence: pane.sequence(),
                })
            })
            .collect()
    }

    fn wait_for_change(
        &self,
        previous: &[PaneSequence],
        timeout: Duration,
    ) -> Result<Vec<PaneSequence>> {
        let deadline = Instant::now() + timeout.min(Duration::from_secs(30));
        loop {
            let (epoch, changed) = &*self.signal;
            let guard = epoch.lock().map_err(|_| DaemonError::Poisoned)?;
            let current = self.sequences()?;
            if current != previous || Instant::now() >= deadline {
                return Ok(current);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let _ = changed
                .wait_timeout(guard, remaining)
                .map_err(|_| DaemonError::Poisoned)?;
        }
    }

    fn snapshots(&self) -> Result<Vec<PaneSnapshot>> {
        self.snapshots_for(None)
    }

    fn snapshots_for(&self, session_id: Option<&str>) -> Result<Vec<PaneSnapshot>> {
        let entries = self.panes.read().map_err(|_| DaemonError::Poisoned)?;
        let mut snapshots = Vec::new();
        for entry in entries.iter() {
            let mut metadata = entry.metadata.lock().map_err(|_| DaemonError::Poisoned)?;
            if session_id.is_some() && metadata.session_id.as_deref() != session_id {
                continue;
            }
            let mut pane = entry.pane.lock().map_err(|_| DaemonError::Poisoned)?;
            let exited = pane.has_exited()?;
            if exited && metadata.down_at.is_none() {
                metadata.down_at = Some(epoch_seconds());
                self.persist_down_state(pane.id(), &metadata)?;
            }
            let mut snapshot = pane.snapshot()?;
            snapshot.session_id.clone_from(&metadata.session_id);
            snapshot.harness.clone_from(&metadata.harness);
            snapshot.role.clone_from(&metadata.role);
            snapshot.state = Some(if exited && metadata.role.as_deref() == Some("brain") {
                "conductor_down".to_owned()
            } else if exited {
                "stopped".to_owned()
            } else {
                "running".to_owned()
            });
            snapshot.down_at = metadata.down_at;
            snapshots.push(snapshot);
        }
        Ok(snapshots)
    }

    fn with_pane<T>(
        &self,
        pane_id: &str,
        action: impl FnOnce(&mut HostedPane) -> Result<T>,
    ) -> Result<Option<T>> {
        let panes = self.panes.read().map_err(|_| DaemonError::Poisoned)?;
        for entry in panes.iter() {
            let mut pane = entry.pane.lock().map_err(|_| DaemonError::Poisoned)?;
            if pane.id() == pane_id {
                return action(&mut pane).map(Some);
            }
        }
        Ok(None)
    }

    fn persist_down_state(&self, pane_id: &str, metadata: &PaneMetadata) -> Result<()> {
        let Some(session_id) = &metadata.session_id else {
            return Ok(());
        };
        let mut session = read_session(session_id)?;
        if let Some(record) = session.panes.iter_mut().find(|record| record.id == pane_id) {
            record.state = if metadata.role.as_deref() == Some("brain") {
                "conductor_down".to_owned()
            } else {
                "stopped".to_owned()
            };
            record.down_at = Some(orc_core::registry::now_iso());
        }
        if metadata.role.as_deref() == Some("brain") {
            session.reorientation = Some(
                "Conductor resumed after a crash. Re-orient with the durable session record, inbox, and orc list before dispatching new work."
                    .to_owned(),
            );
        }
        session.updated_at = orc_core::registry::now_iso();
        write_session(&session)?;
        Ok(())
    }

    fn home_response(&self) -> Result<ServerResponse> {
        let registry = load_harness_registry()?;
        let sessions = list_sessions()?
            .into_iter()
            .map(|session| SessionSummary {
                attention: session
                    .panes
                    .iter()
                    .filter(|pane| pane.state == "conductor_down")
                    .count(),
                id: session.id,
                brain: session.brain,
                workers: session.workers,
                cwd: session.cwd,
                updated_at: session.updated_at,
            })
            .collect();
        let harnesses = registry
            .harnesses
            .iter()
            .map(|(id, harness)| HarnessSummary {
                id: id.clone(),
                roles: harness.roles.clone(),
                resumable: !harness.resume_args.is_empty(),
            })
            .collect();
        Ok(ServerResponse::Home {
            sessions,
            harnesses,
            default_workers: registry.default_workers,
            max_parallel_workers: registry.max_parallel_workers,
            theme: registry.app.theme,
            reduced_motion: registry.app.reduced_motion,
        })
    }

    fn launch_session(&self, brain: &str, workers: &[String], cwd: &Path) -> Result<String> {
        let registry = load_harness_registry()?;
        let mut session = create_bench_session(brain, workers, cwd)?;
        let mut launches = Vec::with_capacity(workers.len() + 1);
        launches.push((brain.to_owned(), "brain".to_owned(), 0_usize));
        launches.extend(
            workers
                .iter()
                .enumerate()
                .map(|(index, worker)| (worker.clone(), "worker".to_owned(), index + 1)),
        );
        let mut entries = Vec::with_capacity(launches.len());
        for (harness_id, role, index) in launches {
            let harness = registry.harnesses.get(&harness_id).ok_or_else(|| {
                DaemonError::Core(anyhow::anyhow!("unknown harness: {harness_id}"))
            })?;
            let pane_id = if role == "brain" {
                format!("{}-brain", session.id)
            } else {
                format!("{}-worker-{index}", session.id)
            };
            let environment = vec![
                ("ORC_SESSION".to_owned(), session.id.clone()),
                ("ORC_PANE_ID".to_owned(), pane_id.clone()),
            ];
            let pane = HostedPane::spawn_with_signal_and_env(
                &pane_id,
                &harness_id,
                &harness.command,
                &harness.args,
                cwd,
                30,
                90,
                self.signal.clone(),
                &environment,
            )?;
            session.panes.push(SessionPaneRecord {
                id: pane_id,
                harness: harness_id.clone(),
                role: role.clone(),
                state: "running".to_owned(),
                pid: pane.process_id(),
                down_at: None,
                extra: Default::default(),
            });
            entries.push(PaneEntry {
                pane: Mutex::new(pane),
                metadata: Mutex::new(PaneMetadata {
                    session_id: Some(session.id.clone()),
                    harness: Some(harness_id),
                    role: Some(role),
                    command: harness.command.clone(),
                    args: harness.args.clone(),
                    resume_args: harness.resume_args.clone(),
                    cwd: cwd.to_owned(),
                    environment,
                    down_at: None,
                }),
            });
        }
        session.updated_at = orc_core::registry::now_iso();
        write_session(&session)?;
        self.panes
            .write()
            .map_err(|_| DaemonError::Poisoned)?
            .extend(entries);
        self.persist_process_records()?;
        Ok(session.id)
    }

    fn persist_process_records(&self) -> Result<()> {
        let Some(home) = &self.home else {
            return Ok(());
        };
        let entries = self.panes.read().map_err(|_| DaemonError::Poisoned)?;
        let mut records = Vec::new();
        for entry in entries.iter() {
            let metadata = entry.metadata.lock().map_err(|_| DaemonError::Poisoned)?;
            let pane = entry.pane.lock().map_err(|_| DaemonError::Poisoned)?;
            if let (Some(session_id), Some(pid)) = (&metadata.session_id, pane.process_id())
                && let Ok(process) = process_identity(pid)
            {
                records.push(PaneProcessRecord {
                    pane_id: pane.id().to_owned(),
                    session_id: session_id.clone(),
                    process,
                });
            }
        }
        write_daemon_record(
            &home.join("daemon.json"),
            &DaemonRecord {
                version: 1,
                panes: records,
                extra: Default::default(),
            },
        )
    }

    fn update_layout(&self, session_id: &str, layout: Vec<LayoutRect>) -> Result<ServerResponse> {
        if layout.len() > 16 {
            return Ok(ServerResponse::Error {
                message: "layout exceeds the 16-pane bound".to_owned(),
            });
        }
        let mut session = read_session(session_id)?;
        session.layout = layout
            .into_iter()
            .map(|rect| PaneLayout {
                pane_id: rect.pane_id,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                order: rect.order,
                extra: Default::default(),
            })
            .collect();
        session.updated_at = orc_core::registry::now_iso();
        write_session(&session)?;
        Ok(ServerResponse::Ack)
    }

    fn respawn_conductor(&self, pane_id: &str) -> Result<ServerResponse> {
        let entries = self.panes.read().map_err(|_| DaemonError::Poisoned)?;
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.pane.lock().is_ok_and(|pane| pane.id() == pane_id))
        else {
            return Ok(ServerResponse::Error {
                message: format!("unknown pane: {pane_id}"),
            });
        };
        let mut metadata = entry.metadata.lock().map_err(|_| DaemonError::Poisoned)?;
        if metadata.role.as_deref() != Some("brain") {
            return Ok(ServerResponse::Error {
                message: "only the conductor pane can be respawned".to_owned(),
            });
        }
        if metadata.resume_args.is_empty() {
            return Ok(ServerResponse::Error {
                message: "RESUME NOT SUPPORTED by this harness".to_owned(),
            });
        }
        let mut pane = entry.pane.lock().map_err(|_| DaemonError::Poisoned)?;
        if !pane.has_exited()? {
            return Ok(ServerResponse::Error {
                message: "conductor is still running".to_owned(),
            });
        }
        let prior = pane.snapshot()?;
        let mut args = metadata.args.clone();
        args.extend(metadata.resume_args.clone());
        let replacement = HostedPane::spawn_with_signal_and_env(
            pane_id,
            metadata.harness.as_deref().unwrap_or("brain"),
            &metadata.command,
            &args,
            &metadata.cwd,
            prior.rows,
            prior.cols,
            self.signal.clone(),
            &metadata.environment,
        )?;
        *pane = replacement;
        metadata.down_at = None;
        if let Some(session_id) = &metadata.session_id {
            let mut session = read_session(session_id)?;
            if let Some(record) = session.panes.iter_mut().find(|record| record.id == pane_id) {
                record.state = "running".to_owned();
                record.down_at = None;
                record.pid = pane.process_id();
            }
            session.updated_at = orc_core::registry::now_iso();
            write_session(&session)?;
        }
        drop(pane);
        drop(metadata);
        drop(entries);
        self.persist_process_records()?;
        Ok(ServerResponse::Ack)
    }

    fn task_board(&self, session_id: &str) -> Result<ServerResponse> {
        let tasks = list_tasks(session_id)?;
        let cards = tasks
            .iter()
            .take(256)
            .map(|task| {
                let blocked = task.depends_on.iter().any(|dependency| {
                    tasks
                        .iter()
                        .find(|candidate| candidate.id == *dependency)
                        .is_none_or(|candidate| candidate.status != "done")
                });
                let diff = if task.status == "review" {
                    diff_task(session_id, &task.id).ok().map(|diff| {
                        format!(
                            "+{} -{} · {} files",
                            diff.insertions, diff.deletions, diff.files
                        )
                    })
                } else {
                    None
                };
                TaskSummary {
                    id: task.id.clone(),
                    title: task.title.clone(),
                    status: task.status.clone(),
                    assignee: task.assignee.clone(),
                    assignee_run: task.assignee_run.clone(),
                    isolated: task.worktree.is_some(),
                    isolation: task.worktree.as_ref().map(|worktree| {
                        worktree
                            .reason
                            .clone()
                            .unwrap_or_else(|| worktree.state.clone())
                    }),
                    blocked,
                    tokens: None,
                    diff,
                    history: task
                        .history
                        .iter()
                        .rev()
                        .take(8)
                        .rev()
                        .map(|entry| TaskHistorySummary {
                            at: entry.at.clone(),
                            actor: entry.actor.clone(),
                            action: entry.action.clone(),
                            to: entry.to.clone(),
                        })
                        .collect(),
                }
            })
            .collect();
        Ok(ServerResponse::TaskBoard {
            session_id: session_id.to_owned(),
            tasks: cards,
        })
    }

    fn respond(&self, request: ClientRequest) -> Result<ServerResponse> {
        match request {
            ClientRequest::Hello { version } if version == PROTOCOL_VERSION => {
                Ok(ServerResponse::Welcome {
                    version: PROTOCOL_VERSION,
                })
            }
            ClientRequest::Hello { version } => Ok(ServerResponse::Error {
                message: format!("protocol mismatch: client {version}, daemon {PROTOCOL_VERSION}"),
            }),
            ClientRequest::Snapshot => Ok(ServerResponse::Snapshot {
                panes: self.snapshots()?,
            }),
            ClientRequest::Wait {
                sequences,
                timeout_ms,
            } => Ok(ServerResponse::Changed {
                sequences: self.wait_for_change(&sequences, Duration::from_millis(timeout_ms))?,
            }),
            ClientRequest::Input { pane_id, bytes } => self
                .with_pane(&pane_id, |pane| {
                    pane.write_input(&bytes)?;
                    Ok(())
                })?
                .map_or_else(
                    || {
                        Ok(ServerResponse::Error {
                            message: format!("unknown pane: {pane_id}"),
                        })
                    },
                    |_| Ok(ServerResponse::Ack),
                ),
            ClientRequest::Resize {
                pane_id,
                rows,
                cols,
            } => self
                .with_pane(&pane_id, |pane| {
                    pane.resize(rows, cols)?;
                    Ok(())
                })?
                .map_or_else(
                    || {
                        Ok(ServerResponse::Error {
                            message: format!("unknown pane: {pane_id}"),
                        })
                    },
                    |_| Ok(ServerResponse::Ack),
                ),
            ClientRequest::Ping { nonce } => Ok(ServerResponse::Pong { nonce }),
            ClientRequest::Metrics => Ok(ServerResponse::Metrics {
                metrics: self.metrics()?,
            }),
            ClientRequest::Home => self.home_response(),
            ClientRequest::CreateSession {
                brain,
                workers,
                cwd,
            } => Ok(ServerResponse::SessionCreated {
                session_id: self.launch_session(&brain, &workers, Path::new(&cwd))?,
            }),
            ClientRequest::AttachSession { session_id } => {
                let session = read_session(&session_id)?;
                Ok(ServerResponse::SessionAttached {
                    panes: self.snapshots_for(Some(&session_id))?,
                    layout: session
                        .layout
                        .into_iter()
                        .map(|rect| LayoutRect {
                            pane_id: rect.pane_id,
                            x: rect.x,
                            y: rect.y,
                            width: rect.width,
                            height: rect.height,
                            order: rect.order,
                        })
                        .collect(),
                })
            }
            ClientRequest::UpdateLayout { session_id, layout } => {
                self.update_layout(&session_id, layout)
            }
            ClientRequest::RespawnConductor { pane_id } => self.respawn_conductor(&pane_id),
            ClientRequest::TaskBoard { session_id } => self.task_board(&session_id),
            ClientRequest::MoveTask {
                session_id,
                task_id,
                status,
            } => {
                let status = TaskStatus::parse(&status).map_err(anyhow::Error::from)?;
                move_task(&session_id, &task_id, status, TaskActor::Human)?;
                self.task_board(&session_id)
            }
        }
    }
}

/// Exact process identity persisted across daemon restarts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcessIdentity {
    /// Child process identifier.
    pub pid: u32,
    /// Child process group identifier.
    pub process_group: u32,
    /// Platform start-time marker used to reject PID reuse.
    pub started: String,
    /// Full command string reported by the operating system.
    pub command: String,
}

/// Persistent record for one daemon-owned pane process.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneProcessRecord {
    /// Stable pane identifier.
    pub pane_id: String,
    /// Session that owns the pane.
    pub session_id: String,
    /// Validated operating-system identity.
    pub process: ProcessIdentity,
}

/// Plain additive daemon restart record.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DaemonRecord {
    /// Version of this persistence shape.
    pub version: u16,
    /// Pane process records.
    #[serde(default)]
    pub panes: Vec<PaneProcessRecord>,
    /// Unknown future fields preserved by callers that round-trip as JSON values.
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

fn ps_field(pid: u32, field: &str) -> Result<String> {
    let output = Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()?;
    if !output.status.success() {
        return Err(DaemonError::Process(format!(
            "ps could not inspect pid {pid}"
        )));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if value.is_empty() {
        return Err(DaemonError::Process(format!("pid {pid} is not running")));
    }
    Ok(value)
}

/// Inspect the identity fields required to distinguish a live child from PID reuse.
pub fn process_identity(pid: u32) -> Result<ProcessIdentity> {
    let process_group = ps_field(pid, "pgid")?
        .parse::<u32>()
        .map_err(|error| DaemonError::Process(error.to_string()))?;
    Ok(ProcessIdentity {
        pid,
        process_group,
        started: ps_field(pid, "lstart")?,
        command: ps_field(pid, "command")?,
    })
}

/// Atomically persist daemon process records with flush, sync, and rename.
pub fn write_daemon_record(path: &Path, record: &DaemonRecord) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        DaemonError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "daemon record has no parent",
        ))
    })?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".orcd-record-{}-{}",
        std::process::id(),
        WRITE_NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, record)?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Reap only recorded process groups whose PID, group, start marker, and command still match.
pub fn reap_recorded_children(path: &Path) -> Result<usize> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let record: DaemonRecord = serde_json::from_slice(&bytes)?;
    let mut reaped = 0;
    for pane in record.panes {
        let Ok(current) = process_identity(pane.process.pid) else {
            continue;
        };
        if current != pane.process || current.process_group != current.pid {
            continue;
        }
        #[allow(unsafe_code)]
        let result = unsafe { libc::kill(-(current.process_group as i32), libc::SIGTERM) };
        if result == 0 {
            reaped += 1;
        }
    }
    Ok(reaped)
}

fn prepare_socket(socket_path: &Path) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let metadata = match fs::symlink_metadata(socket_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    #[allow(unsafe_code)]
    let user = unsafe { libc::geteuid() };
    if !metadata.file_type().is_socket() || metadata.uid() != user {
        return Err(DaemonError::UnsafeSocket(socket_path.to_owned()));
    }
    match UnixStream::connect(socket_path) {
        Ok(_) => Err(DaemonError::AlreadyRunning(socket_path.to_owned())),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            fs::remove_file(socket_path)?;
            Ok(())
        }
        Err(_) => Err(DaemonError::UnsafeSocket(socket_path.to_owned())),
    }
}

fn bind_socket(socket_path: &Path) -> Result<UnixListener> {
    prepare_socket(socket_path)?;
    let listener = UnixListener::bind(socket_path)?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Serve clients until the process is terminated.
pub fn serve(socket_path: &Path, daemon: Arc<Daemon>) -> Result<()> {
    let listener = bind_socket(socket_path)?;
    info!(socket = %socket_path.display(), "orcd listening");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if !daemon.acquire_client() {
                    let mut stream = stream;
                    let _ = write_response(
                        &mut stream,
                        &ServerResponse::Error {
                            message: "too many attached clients".to_owned(),
                        },
                    );
                    continue;
                }
                let daemon = Arc::clone(&daemon);
                thread::spawn(move || {
                    match handle_client(stream, &daemon) {
                        Ok(()) => {}
                        Err(DaemonError::Io(error))
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::BrokenPipe
                                    | std::io::ErrorKind::ConnectionReset
                                    | std::io::ErrorKind::NotConnected
                            ) =>
                        {
                            info!("client detached");
                        }
                        Err(error) => warn!(%error, "client disconnected with an error"),
                    }
                    daemon.release_client();
                });
            }
            Err(error) => warn!(%error, "client accept failed"),
        }
    }
    Ok(())
}

fn handle_client(mut stream: UnixStream, daemon: &Daemon) -> Result<()> {
    let reader_stream = stream.try_clone()?;
    let mut reader = BufReader::new(reader_stream);
    let mut negotiated = false;
    let client_id = daemon.next_client_id.fetch_add(1, Ordering::Relaxed);
    let result = (|| -> Result<()> {
        loop {
            let mut bytes = Vec::new();
            let read = reader
                .by_ref()
                .take(MAX_MESSAGE_BYTES + 1)
                .read_until(b'\n', &mut bytes)?;
            if read == 0 {
                break Ok(());
            }
            if read as u64 > MAX_MESSAGE_BYTES || !bytes.ends_with(b"\n") {
                write_response(
                    &mut stream,
                    &ServerResponse::Error {
                        message: "protocol message exceeds 1 MiB".to_owned(),
                    },
                )?;
                break Ok(());
            }
            let request = match serde_json::from_slice::<ClientRequest>(&bytes) {
                Ok(request) => request,
                Err(error) => {
                    write_response(
                        &mut stream,
                        &ServerResponse::Error {
                            message: format!("malformed protocol message: {error}"),
                        },
                    )?;
                    continue;
                }
            };
            if !negotiated && !matches!(request, ClientRequest::Hello { .. }) {
                write_response(
                    &mut stream,
                    &ServerResponse::Error {
                        message: "protocol hello required before commands".to_owned(),
                    },
                )?;
                continue;
            }
            let response = match request {
                ClientRequest::Resize {
                    pane_id,
                    rows,
                    cols,
                } => daemon.resize_for_client(client_id, &pane_id, rows, cols)?,
                request => daemon.respond(request)?,
            };
            negotiated |= matches!(response, ServerResponse::Welcome { .. });
            write_response(&mut stream, &response)?;
        }
    })();
    let cleanup = daemon.forget_client_sizes(client_id);
    result.and(cleanup)
}

fn write_response(stream: &mut UnixStream, response: &ServerResponse) -> Result<()> {
    serde_json::to_writer(&mut *stream, response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use orc_core::bench::{HarnessConfig, HarnessRegistry, read_session, write_harness_registry};
    use orc_proto::{ClientRequest, LayoutRect, PROTOCOL_VERSION, ServerResponse};
    use orc_pty::{HostedPane, update_signal};

    use super::{
        Daemon, DaemonRecord, PaneProcessRecord, bind_socket, handle_client, process_identity,
        reap_recorded_children, write_daemon_record,
    };

    #[test]
    fn client_can_detach_and_replay_the_same_screen() {
        let args = vec!["-c".to_owned(), "printf replay-ready; sleep 1".to_owned()];
        let signal = update_signal();
        let pane = HostedPane::spawn_with_signal(
            "brain",
            "fixture",
            "sh",
            &args,
            Path::new("/tmp"),
            8,
            40,
            signal.clone(),
        )
        .expect("spawn pane");
        let daemon = Daemon::new(vec![pane], signal);
        thread::sleep(std::time::Duration::from_millis(50));

        for _ in 0..2 {
            let (mut client, server) = UnixStream::pair().expect("socket pair");
            thread::scope(|scope| {
                scope.spawn(|| handle_client(server, &daemon).expect("serve client"));
                serde_json::to_writer(
                    &mut client,
                    &ClientRequest::Hello {
                        version: PROTOCOL_VERSION,
                    },
                )
                .expect("write hello");
                client.write_all(b"\n").expect("finish hello");
                serde_json::to_writer(&mut client, &ClientRequest::Snapshot)
                    .expect("write snapshot");
                client.write_all(b"\n").expect("finish snapshot");
                let mut reader = BufReader::new(client.try_clone().expect("clone client"));
                let mut line = String::new();
                reader.read_line(&mut line).expect("read welcome");
                let welcome: ServerResponse = serde_json::from_str(&line).expect("parse welcome");
                assert!(matches!(welcome, ServerResponse::Welcome { .. }));
                line.clear();
                reader.read_line(&mut line).expect("read snapshot");
                let snapshot: ServerResponse = serde_json::from_str(&line).expect("parse snapshot");
                let ServerResponse::Snapshot { panes } = snapshot else {
                    panic!("expected snapshot");
                };
                let text = panes[0]
                    .cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>();
                assert!(text.contains("replay-ready"));
                drop(client);
            });
        }
    }

    #[test]
    fn output_signal_wakes_waiter_without_polling() {
        let signal = update_signal();
        let pane = HostedPane::spawn_with_signal(
            "brain",
            "fixture",
            "cat",
            &[],
            Path::new("/tmp"),
            8,
            40,
            signal.clone(),
        )
        .expect("spawn pane");
        let daemon = Daemon::new(vec![pane], signal);
        let previous = daemon.sequences().expect("initial sequences");
        thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                let started = Instant::now();
                let next = daemon
                    .wait_for_change(&previous, Duration::from_secs(1))
                    .expect("wait for output");
                (next, started.elapsed())
            });
            thread::sleep(Duration::from_millis(20));
            daemon
                .respond(ClientRequest::Input {
                    pane_id: "brain".to_owned(),
                    bytes: b"wake\r".to_vec(),
                })
                .expect("write input");
            let (next, elapsed) = waiter.join().expect("join waiter");
            assert_ne!(next, previous);
            assert!(elapsed < Duration::from_millis(500));
        });
    }

    #[test]
    fn socket_parent_and_endpoint_are_private_and_symlinks_are_rejected() {
        let root = std::env::temp_dir().join(format!("orcd-socket-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let socket = root.join("private/orcd.sock");
        let listener = bind_socket(&socket).expect("bind private socket");
        assert_eq!(
            fs::metadata(socket.parent().expect("socket parent"))
                .expect("parent metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&socket)
                .expect("socket metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        drop(listener);
        fs::remove_file(&socket).expect("remove socket");
        symlink("elsewhere", &socket).expect("create unsafe symlink");
        assert!(bind_socket(&socket).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_and_oversized_messages_receive_explicit_errors() {
        let signal = update_signal();
        let daemon = Daemon::new(Vec::new(), signal);
        let (mut client, server) = UnixStream::pair().expect("socket pair");
        thread::scope(|scope| {
            scope.spawn(|| handle_client(server, &daemon).expect("serve malformed fixture"));
            client.write_all(b"not-json\n").expect("write malformed");
            client
                .write_all(b"{\"type\":\"hello\",\"version\":1}\n")
                .expect("write hello");
            client
                .write_all(b"{\"type\":\"ping\",\"nonce\":7}\n")
                .expect("write ping");
            let mut reader = BufReader::new(client.try_clone().expect("clone client"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("malformed error");
            assert!(line.contains("malformed protocol message"));
            line.clear();
            reader.read_line(&mut line).expect("welcome");
            assert!(line.contains("welcome"));
            line.clear();
            reader.read_line(&mut line).expect("pong");
            assert!(line.contains("pong"));
            drop(client);
        });

        let (mut client, server) = UnixStream::pair().expect("oversized pair");
        thread::scope(|scope| {
            scope.spawn(|| handle_client(server, &daemon).expect("serve oversized fixture"));
            client
                .write_all(&vec![b'x'; 1024 * 1024 + 1])
                .expect("write oversized body");
            client.write_all(b"\n").expect("finish oversized body");
            let mut line = String::new();
            BufReader::new(client.try_clone().expect("clone oversized client"))
                .read_line(&mut line)
                .expect("read oversized error");
            assert!(line.contains("exceeds 1 MiB"));
            drop(client);
        });
    }

    #[test]
    fn restart_reaps_only_an_exact_process_group_identity() {
        use std::os::unix::process::CommandExt;

        let root = std::env::temp_dir().join(format!("orcd-reap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create reap root");
        let mut child = Command::new("sh");
        child
            .args(["-c", "exec sleep 30"])
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = child.spawn().expect("spawn recorded child");
        let identity = process_identity(child.id()).expect("inspect child");
        assert_eq!(identity.pid, identity.process_group);
        let path = root.join("daemon.json");
        let mut mismatched = identity.clone();
        mismatched.command.push_str(" reused");
        write_daemon_record(
            &path,
            &DaemonRecord {
                version: 1,
                panes: vec![PaneProcessRecord {
                    pane_id: "brain".to_owned(),
                    session_id: "s".to_owned(),
                    process: mismatched,
                }],
                extra: Default::default(),
            },
        )
        .expect("write mismatched record");
        assert_eq!(reap_recorded_children(&path).expect("reject mismatch"), 0);
        assert!(child.try_wait().expect("poll child").is_none());
        write_daemon_record(
            &path,
            &DaemonRecord {
                version: 1,
                panes: vec![PaneProcessRecord {
                    pane_id: "brain".to_owned(),
                    session_id: "s".to_owned(),
                    process: identity,
                }],
                extra: Default::default(),
            },
        )
        .expect("write exact record");
        assert_eq!(reap_recorded_children(&path).expect("reap exact child"), 1);
        let deadline = Instant::now() + Duration::from_secs(2);
        while child.try_wait().expect("poll reaped child").is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(child.try_wait().expect("final child poll").is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn flood_metrics_record_coalescing_with_bounded_screen_state() {
        let args = vec![
            "-c".to_owned(),
            "i=0; while [ $i -lt 20000 ]; do printf 'flood-%06d\\r' $i; i=$((i+1)); done"
                .to_owned(),
        ];
        let signal = update_signal();
        let pane = HostedPane::spawn_with_signal(
            "flood",
            "fixture",
            "sh",
            &args,
            Path::new("/tmp"),
            20,
            80,
            signal.clone(),
        )
        .expect("spawn flood pane");
        let daemon = Daemon::new(vec![pane], signal);
        thread::sleep(Duration::from_millis(150));
        let first = daemon.snapshots().expect("first flood snapshot");
        assert_eq!(first[0].cells.len(), 20 * 80);
        thread::sleep(Duration::from_millis(150));
        let _ = daemon.snapshots().expect("second flood snapshot");
        let metrics = daemon.metrics().expect("flood metrics");
        assert!(metrics.panes[0].bytes_read > 0);
        assert!(metrics.panes[0].output_chunks > 1);
        assert!(metrics.panes[0].coalesced_updates > 0);
    }

    #[test]
    fn client_limit_is_exact_and_releases_capacity() {
        let daemon = Daemon::new(Vec::new(), update_signal());
        for _ in 0..16 {
            assert!(daemon.acquire_client());
        }
        assert!(!daemon.acquire_client());
        assert_eq!(
            daemon.metrics().expect("client metrics").attached_clients,
            16
        );
        daemon.release_client();
        assert!(daemon.acquire_client());
    }

    #[test]
    fn clients_at_different_sizes_use_largest_grid_then_restore_remaining_size() {
        let signal = update_signal();
        let pane = HostedPane::spawn_with_signal(
            "brain",
            "fixture",
            "cat",
            &[],
            Path::new("/tmp"),
            8,
            40,
            signal.clone(),
        )
        .expect("spawn resize pane");
        let daemon = Daemon::new(vec![pane], signal);
        daemon
            .resize_for_client(1, "brain", 20, 80)
            .expect("resize first client");
        daemon
            .resize_for_client(2, "brain", 40, 120)
            .expect("resize second client");
        let largest = daemon.snapshots().expect("largest snapshot");
        assert_eq!((largest[0].rows, largest[0].cols), (40, 120));
        daemon.forget_client_sizes(2).expect("detach large client");
        let remaining = daemon.snapshots().expect("remaining snapshot");
        assert_eq!((remaining[0].rows, remaining[0].cols), (20, 80));
    }

    #[test]
    #[allow(unsafe_code)]
    fn session_launch_attribution_layout_and_repeated_conductor_recovery_are_durable() {
        let root = std::env::temp_dir().join(format!("orcd-session-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create session root");
        // SAFETY: daemon crate tests that mutate ORC_HOME are serialized by this single test.
        unsafe { std::env::set_var("ORC_HOME", &root) };
        let brain_script = root.join("brain.sh");
        fs::write(
            &brain_script,
            format!(
                "#!/bin/sh\necho \"$ORC_SESSION|$ORC_PANE_ID|$*\" >> {}\necho last-screen-token\nsleep 0.1\n",
                root.join("launches.txt").display()
            ),
        )
        .expect("write brain fixture");
        let worker_script = root.join("worker.sh");
        fs::write(
            &worker_script,
            format!(
                "#!/bin/sh\necho \"$ORC_SESSION|$ORC_PANE_ID|worker\" >> {}\nsleep 30\n",
                root.join("launches.txt").display()
            ),
        )
        .expect("write worker fixture");
        let harness = |script: &Path, resume: &[&str], roles: &[&str]| HarnessConfig {
            command: "/bin/sh".to_owned(),
            args: vec![script.to_string_lossy().into_owned()],
            resume_args: resume.iter().map(|value| (*value).to_owned()).collect(),
            roles: roles.iter().map(|value| (*value).to_owned()).collect(),
            adapter: "fixture".to_owned(),
            extra: BTreeMap::new(),
        };
        let mut registry = HarnessRegistry {
            harnesses: BTreeMap::from([
                (
                    "brain-fixture".to_owned(),
                    harness(&brain_script, &["--resume"], &["brain"]),
                ),
                (
                    "worker-fixture".to_owned(),
                    harness(&worker_script, &[], &["worker"]),
                ),
            ]),
            default_workers: vec!["worker-fixture".to_owned()],
            ..HarnessRegistry::default()
        };
        write_harness_registry(&registry).expect("write fixture harness registry");

        let daemon = Daemon::production(root.clone(), update_signal());
        let session_id = daemon
            .launch_session(
                "brain-fixture",
                &["worker-fixture".to_owned()],
                Path::new("/tmp"),
            )
            .expect("launch fixture session");
        thread::sleep(Duration::from_millis(180));
        let panes = daemon
            .snapshots_for(Some(&session_id))
            .expect("snapshot dead conductor");
        let brain = panes
            .iter()
            .find(|pane| pane.role.as_deref() == Some("brain"))
            .expect("brain pane");
        let worker = panes
            .iter()
            .find(|pane| pane.role.as_deref() == Some("worker"))
            .expect("worker pane");
        assert_eq!(brain.state.as_deref(), Some("conductor_down"));
        assert_eq!(worker.state.as_deref(), Some("running"));
        let last_screen = brain
            .cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        assert!(last_screen.contains("last-screen-token"));
        let worker_pid = read_session(&session_id)
            .expect("read launched session")
            .panes
            .into_iter()
            .find(|pane| pane.role == "worker")
            .and_then(|pane| pane.pid)
            .expect("worker pid");

        let brain_id = brain.id.clone();
        assert!(matches!(
            daemon.respawn_conductor(&brain_id).expect("first respawn"),
            ServerResponse::Ack
        ));
        thread::sleep(Duration::from_millis(180));
        let repeated = daemon
            .snapshots_for(Some(&session_id))
            .expect("snapshot repeated crash");
        assert_eq!(
            repeated
                .iter()
                .find(|pane| pane.id == brain_id)
                .and_then(|pane| pane.state.as_deref()),
            Some("conductor_down")
        );
        assert!(matches!(
            daemon.respawn_conductor(&brain_id).expect("second respawn"),
            ServerResponse::Ack
        ));
        assert!(orc_core::registry::pid_alive(Some(worker_pid)));
        thread::sleep(Duration::from_millis(180));
        let _ = daemon
            .snapshots_for(Some(&session_id))
            .expect("persist second conductor crash");

        daemon
            .update_layout(
                &session_id,
                vec![LayoutRect {
                    pane_id: brain_id,
                    x: 2,
                    y: 1,
                    width: 60,
                    height: 28,
                    order: 0,
                }],
            )
            .expect("persist layout");
        let durable = read_session(&session_id).expect("read durable recovery session");
        assert_eq!(durable.layout[0].x, 2);
        assert!(durable.reorientation.is_some());
        let launches = fs::read_to_string(root.join("launches.txt")).expect("read attribution");
        assert!(launches.contains(&format!("{session_id}|{session_id}-brain|")));
        assert!(launches.contains(&format!("{session_id}|{session_id}-worker-1|worker")));
        assert!(launches.contains("--resume"));

        let restarted = Daemon::production(root.clone(), update_signal());
        let ServerResponse::Home { sessions, .. } =
            restarted.home_response().expect("restart HOME")
        else {
            panic!("expected HOME response");
        };
        assert_eq!(sessions[0].attention, 1);

        registry
            .harnesses
            .get_mut("brain-fixture")
            .expect("fixture brain config")
            .resume_args
            .clear();
        write_harness_registry(&registry).expect("write unsupported registry");
        let unsupported = daemon
            .launch_session("brain-fixture", &[], Path::new("/tmp"))
            .expect("launch unsupported session");
        thread::sleep(Duration::from_millis(180));
        let unsupported_id = format!("{unsupported}-brain");
        assert!(matches!(
            daemon
                .respawn_conductor(&unsupported_id)
                .expect("unsupported response"),
            ServerResponse::Error { message } if message.contains("RESUME NOT SUPPORTED")
        ));
        let _ = fs::remove_dir_all(root);
    }
}
