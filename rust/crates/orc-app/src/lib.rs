#![warn(missing_docs)]
//! Ratatui HOME and STAGE client for the Bench workspace.
//!
//! This crate owns rendering and input forwarding. It must never write
//! registry/session/task files or outlive the daemon-owned PTYs.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

use crossterm::SynchronizedUpdate;
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, KeyCode, KeyEvent, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use orc_core::discovery::{self, HarnessDiscovery};
use orc_core::single_harness::{self, SINGLE_HARNESS_MESSAGE, SingleHarnessPlan};
use orc_proto::{
    ClientRequest, DaemonMetrics, HarnessSummary, LayoutRect, PROTOCOL_VERSION, PaneSequence,
    PaneSnapshot, ServerResponse, SessionSummary, TaskSummary,
};
use orc_pty::trigger::{Trigger, scan_line};
use ratatui::Frame;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tachyonfx::{EffectTimer, Interpolation};
use thiserror::Error;

pub mod baton;
pub mod glyph;
#[cfg(test)]
mod snapshot;
pub mod theme;

use crate::glyph::{Glyph, GlyphTier, Glyphs};
pub use crate::theme::ThemeName;
use crate::theme::{ColorTier, Slot, TRIGGER_RAINBOW, Theme};

const MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;

/// Errors produced by the Bench client.
#[derive(Debug, Error)]
pub enum AppError {
    /// Socket or terminal I/O failed.
    #[error("client I/O failed: {0}")]
    Io(#[from] io::Error),
    /// A daemon response was malformed.
    #[error("invalid daemon response: {0}")]
    Json(#[from] serde_json::Error),
    /// The daemon rejected a request.
    #[error("daemon rejected request: {0}")]
    Daemon(String),
    /// The daemon connection closed or desynchronized mid-request.
    #[error("{0}")]
    Connection(String),
    /// The daemon and client are running different builds.
    #[error("{0}")]
    BuildMismatch(String),
    /// A background event source stopped unexpectedly.
    #[error("client event source disconnected")]
    EventSource,
}

/// Result type returned by client operations.
pub type Result<T> = std::result::Result<T, AppError>;

/// A version-negotiated connection used for command and benchmark traffic.
pub struct BenchClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

/// HOME shelf data returned by the daemon.
#[derive(Clone, Debug)]
pub struct HomeData {
    /// Durable sessions newest first.
    pub sessions: Vec<SessionSummary>,
    /// Configured brain and worker choices.
    pub harnesses: Vec<HarnessSummary>,
    /// PATH-discovered known harnesses (read-only view of the registry).
    pub discovered: Vec<HarnessDiscovery>,
    /// Preselected but editable worker choices.
    pub default_workers: Vec<String>,
    /// Configured worker bound.
    pub max_parallel_workers: usize,
    /// Honest sequential fallback when exactly one adapter family is capable.
    pub single_harness: Option<SingleHarnessPlan>,
    /// The configured theme name: nocturne, ember, or phosphor.
    pub theme: String,
    /// Reduced-motion preference.
    pub reduced_motion: bool,
    /// Configured leader chord label, e.g. `ctrl-g`.
    pub leader_key: String,
}

/// The verified leader chord: the raw control byte plus its display label.
#[derive(Clone, Debug, Eq, PartialEq)]
struct LeaderKey {
    byte: u8,
    label: String,
}

impl LeaderKey {
    /// Parse a `ctrl-<letter>` label, refusing bytes that collide with
    /// enter, tab, escape, backspace, or flow control; anything unusable
    /// falls back to the default `ctrl-g`.
    fn parse(label: &str) -> Self {
        let fallback = Self {
            byte: 0x07,
            label: "ctrl-g".to_owned(),
        };
        let Some(letter) = label
            .strip_prefix("ctrl-")
            .and_then(|rest| {
                let mut chars = rest.chars();
                chars.next().filter(|_| chars.next().is_none())
            })
            .filter(char::is_ascii_lowercase)
        else {
            return fallback;
        };
        // ctrl-i tab, ctrl-j newline, ctrl-m enter, ctrl-h backspace,
        // ctrl-q/ctrl-s XON/XOFF, ctrl-c/ctrl-d conventional interrupts.
        if matches!(letter, 'i' | 'j' | 'm' | 'h' | 'q' | 's' | 'c' | 'd') {
            return fallback;
        }
        Self {
            byte: (letter as u8) & 0x1f,
            label: format!("ctrl-{letter}"),
        }
    }
}

/// Session replay returned on attach.
#[derive(Clone, Debug)]
pub struct SessionData {
    /// Canonical pane screens.
    pub panes: Vec<PaneSnapshot>,
    /// Durable card layout.
    pub layout: Vec<LayoutRect>,
}

impl BenchClient {
    /// Connect to a daemon, verifying its protocol version and build identity.
    ///
    /// A daemon running a different build than this client — including a
    /// daemon that predates the build handshake — is refused with one
    /// actionable message instead of failing obscurely on a later request.
    pub fn connect(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket)?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut client = Self { stream, reader };
        match client.request(&ClientRequest::Hello {
            version: PROTOCOL_VERSION,
        })? {
            ServerResponse::Welcome { version, build } if version == PROTOCOL_VERSION => {
                if build == orc_proto::BUILD_IDENTIFIER {
                    Ok(client)
                } else if build.is_empty() {
                    Err(AppError::BuildMismatch(format!(
                        "the running daemon predates this client (client build {}) — detach other clients, then run `orc daemon restart`",
                        orc_proto::BUILD_IDENTIFIER
                    )))
                } else {
                    Err(AppError::BuildMismatch(format!(
                        "daemon build {build} does not match client build {} — detach other clients, then run `orc daemon restart`",
                        orc_proto::BUILD_IDENTIFIER
                    )))
                }
            }
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected hello response: {response:?}"
            ))),
        }
    }

    /// Fetch complete replayable screens, optionally for one session only.
    ///
    /// Session-bound callers must pass the session so unrelated sessions
    /// cannot inflate the response toward the wire cap.
    pub fn snapshot(&mut self, session_id: Option<String>) -> Result<Vec<PaneSnapshot>> {
        match self.request(&ClientRequest::Snapshot { session_id })? {
            ServerResponse::Snapshot { panes } => Ok(panes),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected snapshot response: {response:?}"
            ))),
        }
    }

    /// Block until output changes or the daemon's timeout expires.
    pub fn wait(
        &mut self,
        sequences: Vec<PaneSequence>,
        timeout: Duration,
    ) -> Result<Vec<PaneSequence>> {
        match self.request(&ClientRequest::Wait {
            sequences,
            timeout_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
        })? {
            ServerResponse::Changed { sequences } => Ok(sequences),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected wait response: {response:?}"
            ))),
        }
    }

    /// Forward bytes to one pane.
    pub fn input(&mut self, pane_id: String, bytes: Vec<u8>) -> Result<()> {
        match self.request(&ClientRequest::Input { pane_id, bytes })? {
            ServerResponse::Ack => Ok(()),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected input response: {response:?}"
            ))),
        }
    }

    /// Resize one pane.
    pub fn resize(&mut self, pane_id: String, rows: u16, cols: u16) -> Result<()> {
        match self.request(&ClientRequest::Resize {
            pane_id,
            rows,
            cols,
        })? {
            ServerResponse::Ack => Ok(()),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected resize response: {response:?}"
            ))),
        }
    }

    /// Measure one protocol round trip without writing to a harness.
    pub fn ping(&mut self, nonce: u64) -> Result<()> {
        match self.request(&ClientRequest::Ping { nonce })? {
            ServerResponse::Pong { nonce: returned } if returned == nonce => Ok(()),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected ping response: {response:?}"
            ))),
        }
    }

    /// Fetch daemon backpressure and attachment counters.
    pub fn metrics(&mut self) -> Result<DaemonMetrics> {
        match self.request(&ClientRequest::Metrics)? {
            ServerResponse::Metrics { metrics } => Ok(metrics),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected metrics response: {response:?}"
            ))),
        }
    }

    /// Fetch HOME session and harness choices.
    pub fn home(&mut self) -> Result<HomeData> {
        match self.request(&ClientRequest::Home)? {
            ServerResponse::Home {
                sessions,
                harnesses,
                default_workers,
                max_parallel_workers,
                theme,
                reduced_motion,
                leader_key,
            } => Ok(HomeData {
                sessions,
                harnesses,
                discovered: discovery::present_current(),
                default_workers,
                max_parallel_workers,
                single_harness: orc_core::bench::read_harness_registry()
                    .ok()
                    .flatten()
                    .and_then(|registry| single_harness::detect(&registry, None)),
                theme,
                reduced_motion,
                leader_key,
            }),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected HOME response: {response:?}"
            ))),
        }
    }

    /// Persist the chosen theme through the daemon/core writer.
    ///
    /// This crate never writes `~/.orchestra` itself; the daemon owns the
    /// registry. The reply carries the name that was actually written, which
    /// is the flagship when the daemon did not recognise the request.
    pub fn set_theme(&mut self, theme: String) -> Result<String> {
        match self.request(&ClientRequest::SetTheme { theme })? {
            ServerResponse::ThemeSet { theme } => Ok(theme),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected theme response: {response:?}"
            ))),
        }
    }

    /// Fetch SCORE cards through the daemon-owned task command path.
    pub fn task_board(&mut self, session_id: String) -> Result<Vec<TaskSummary>> {
        match self.request(&ClientRequest::TaskBoard { session_id })? {
            ServerResponse::TaskBoard { tasks, .. } => Ok(tasks),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected task response: {response:?}"
            ))),
        }
    }

    /// Move a SCORE card as a human through the daemon/core writer.
    pub fn move_task(
        &mut self,
        session_id: String,
        task_id: String,
        status: String,
    ) -> Result<Vec<TaskSummary>> {
        match self.request(&ClientRequest::MoveTask {
            session_id,
            task_id,
            status,
        })? {
            ServerResponse::TaskBoard { tasks, .. } => Ok(tasks),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected task move response: {response:?}"
            ))),
        }
    }

    /// Create and launch a session through the daemon/core writer.
    pub fn create_session(
        &mut self,
        brain: String,
        workers: Vec<String>,
        cwd: String,
    ) -> Result<String> {
        match self.request(&ClientRequest::CreateSession {
            brain,
            workers,
            cwd,
        })? {
            ServerResponse::SessionCreated { session_id } => Ok(session_id),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected create-session response: {response:?}"
            ))),
        }
    }

    /// Attach to one durable session and fetch its replay and layout.
    pub fn attach_session(&mut self, session_id: String) -> Result<SessionData> {
        match self.request(&ClientRequest::AttachSession { session_id })? {
            ServerResponse::SessionAttached { panes, layout } => Ok(SessionData { panes, layout }),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected attach response: {response:?}"
            ))),
        }
    }

    /// Persist the complete STAGE card layout through the daemon/core writer.
    pub fn update_layout(&mut self, session_id: String, layout: Vec<LayoutRect>) -> Result<()> {
        match self.request(&ClientRequest::UpdateLayout { session_id, layout })? {
            ServerResponse::Ack => Ok(()),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected layout response: {response:?}"
            ))),
        }
    }

    /// Recover a dead conductor through its configured resume arguments.
    pub fn respawn_conductor(&mut self, pane_id: String) -> Result<()> {
        match self.request(&ClientRequest::RespawnConductor { pane_id })? {
            ServerResponse::Ack => Ok(()),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected respawn response: {response:?}"
            ))),
        }
    }

    fn request(&mut self, request: &ClientRequest) -> Result<ServerResponse> {
        serde_json::to_writer(&mut self.stream, request)?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()?;
        let mut bytes = Vec::new();
        let read = self
            .reader
            .by_ref()
            .take(MAX_RESPONSE_BYTES + 1)
            .read_until(b'\n', &mut bytes)?;
        if read == 0 {
            return Err(AppError::Connection(
                "the daemon closed the connection — it may have exited or restarted; run `orc daemon status`, then reattach".to_owned(),
            ));
        }
        if read as u64 > MAX_RESPONSE_BYTES {
            return Err(AppError::Connection(format!(
                "daemon response exceeded the {} MiB cap (stopped after {read} bytes)",
                MAX_RESPONSE_BYTES / (1024 * 1024)
            )));
        }
        if !bytes.ends_with(b"\n") {
            return Err(AppError::Connection(format!(
                "malformed daemon response: {read} bytes arrived without a trailing newline"
            )));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

/// Latency summary emitted by the spike benchmark.
#[derive(Clone, Copy, Debug)]
pub struct LatencySummary {
    /// Median round-trip latency in microseconds.
    pub p50_us: u128,
    /// 95th-percentile latency in microseconds.
    pub p95_us: u128,
    /// 99th-percentile latency in microseconds.
    pub p99_us: u128,
    /// Maximum latency in microseconds.
    pub max_us: u128,
}

/// Measure daemon round-trip latency for a fixed number of samples.
pub fn benchmark(socket: &Path, iterations: usize) -> Result<LatencySummary> {
    let mut client = BenchClient::connect(socket)?;
    let mut values = Vec::with_capacity(iterations);
    for nonce in 0..iterations {
        let started = Instant::now();
        client.ping(nonce as u64)?;
        values.push(started.elapsed().as_micros());
    }
    values.sort_unstable();
    let percentile = |pct: usize| {
        let index = values.len().saturating_sub(1) * pct / 100;
        values.get(index).copied().unwrap_or_default()
    };
    Ok(LatencySummary {
        p50_us: percentile(50),
        p95_us: percentile(95),
        p99_us: percentile(99),
        max_us: values.last().copied().unwrap_or_default(),
    })
}

/// Measure focused-pane input through PTY output and visible snapshot replay.
pub fn visible_input_benchmark(
    socket: &Path,
    pane_id: &str,
    iterations: usize,
) -> Result<LatencySummary> {
    let mut client = BenchClient::connect(socket)?;
    let mut sequences = client
        .snapshot(None)?
        .into_iter()
        .map(|pane| PaneSequence {
            id: pane.id,
            sequence: pane.sequence,
        })
        .collect::<Vec<_>>();
    let mut values = Vec::with_capacity(iterations);
    for index in 0..iterations {
        let started = Instant::now();
        client.input(pane_id.to_owned(), vec![b'a' + (index % 26) as u8])?;
        let next = client.wait(sequences, Duration::from_secs(1))?;
        let panes = client.snapshot(None)?;
        if !panes.iter().any(|pane| pane.id == pane_id) {
            return Err(AppError::Daemon(format!(
                "unknown benchmark pane: {pane_id}"
            )));
        }
        values.push(started.elapsed().as_micros());
        sequences = next;
    }
    values.sort_unstable();
    let percentile = |pct: usize| {
        let index = values.len().saturating_sub(1) * pct / 100;
        values.get(index).copied().unwrap_or_default()
    };
    Ok(LatencySummary {
        p50_us: percentile(50),
        p95_us: percentile(95),
        p99_us: percentile(99),
        max_us: values.last().copied().unwrap_or_default(),
    })
}

struct StageState {
    panes: Vec<PaneSnapshot>,
    focus: usize,
    pane_areas: Vec<Rect>,
    /// The baton's decay timer. It is reset on every output tick, so
    /// `done()` means "no output for [`baton::DECAY`]" — the spec's trigger
    /// for falling back to the idle rail — and the shell's repaint loop reads
    /// the same signal to stop animating.
    pulse: EffectTimer,
    last_tick: Instant,
    /// When the current sweep began, so the packet's frame is a function of
    /// wall-clock rather than of how often the shell happened to repaint.
    sweep_start: Instant,
    theme: Theme,
    glyphs: Glyphs,
    session_id: Option<String>,
    layout: Vec<LayoutRect>,
    zoomed: bool,
    dragging: Option<(usize, u16, u16)>,
    raw_router: RawRouter,
    confirmed_panes: std::collections::HashSet<String>,
    leader_label: String,
    /// Recoverable command failure shown on the legend line instead of
    /// exiting the client.
    message: String,
}

impl StageState {
    fn new(panes: Vec<PaneSnapshot>, theme: Theme, glyphs: Glyphs) -> Self {
        let now = Instant::now();
        Self {
            panes,
            focus: 0,
            pane_areas: Vec::new(),
            pulse: EffectTimer::from_ms(
                u32::try_from(baton::DECAY.as_millis()).unwrap_or(u32::MAX),
                Interpolation::Linear,
            ),
            last_tick: now,
            sweep_start: now,
            theme,
            glyphs,
            session_id: None,
            layout: Vec::new(),
            zoomed: false,
            dragging: None,
            raw_router: RawRouter::default(),
            confirmed_panes: std::collections::HashSet::new(),
            leader_label: "ctrl-g".to_owned(),
            message: String::new(),
        }
    }

    fn for_session(
        session_id: String,
        panes: Vec<PaneSnapshot>,
        layout: Vec<LayoutRect>,
        theme: Theme,
        glyphs: Glyphs,
    ) -> Self {
        let mut state = Self::new(panes, theme, glyphs);
        state.session_id = Some(session_id);
        state.layout = layout;
        state
    }

    fn apply_snapshot(&mut self, panes: Vec<PaneSnapshot>) {
        let changed = panes
            .iter()
            .zip(&self.panes)
            .any(|(next, prior)| next.id != prior.id || next.sequence != prior.sequence)
            || panes.len() != self.panes.len();
        self.panes = panes;
        if changed {
            self.mark_output();
        }
        self.focus = self.focus.min(self.panes.len().saturating_sub(1));
    }

    /// A pane produced output, or a task event landed: restart the decay
    /// timer, and begin a sweep if the rail had gone idle.
    fn mark_output(&mut self) {
        if self.pulse.done() {
            self.sweep_start = Instant::now();
        }
        self.pulse.reset();
    }

    fn advance(&mut self) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        let _ = self.pulse.process(elapsed);
    }

    /// The baton's state right now, given the client's motion preference.
    ///
    /// `pulse.done()` is the 400 ms silence the spec decays on; the sweep
    /// frame comes from `sweep_start` so the packet advances at a fixed
    /// 110 ms/frame regardless of repaint rate.
    fn baton_state(&self, reduced_motion: bool) -> baton::State {
        let since_output = if self.pulse.done() {
            baton::DECAY
        } else {
            Duration::ZERO
        };
        baton::State::resolve(reduced_motion, since_output, self.sweep_start.elapsed())
    }

    /// Whether any conductor pane currently shows a trigger. The shell keeps
    /// repainting while this holds so the trigger rainbow keeps flowing after
    /// the baton pulse has settled.
    fn has_live_trigger(&self) -> bool {
        self.panes
            .iter()
            .any(|pane| !conductor_triggers(pane).0.is_empty())
    }
}

enum UiEvent {
    Raw(Vec<u8>),
    Resize,
    Snapshot(Vec<PaneSnapshot>),
    WatchFailed(String),
    RunsChanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeaderAction {
    Quit,
    Next,
    Previous,
    Zoom,
    Swap,
    Grow,
    Shrink,
    Home,
    Score,
    Views,
    Help,
    Theme,
}

struct RawRouter {
    leader: bool,
    paste: bool,
    recent: VecDeque<u8>,
    leader_byte: u8,
}

impl Default for RawRouter {
    fn default() -> Self {
        Self {
            leader: false,
            paste: false,
            recent: VecDeque::new(),
            leader_byte: 0x07,
        }
    }
}

impl RawRouter {
    fn route(&mut self, bytes: &[u8]) -> (Vec<u8>, Vec<LeaderAction>) {
        let mut forwarded = Vec::with_capacity(bytes.len());
        let mut actions = Vec::new();
        for &byte in bytes {
            if self.leader && !self.paste {
                self.leader = false;
                let action = match byte {
                    byte if byte == self.leader_byte => {
                        forwarded.push(byte);
                        None
                    }
                    b'q' => Some(LeaderAction::Quit),
                    b'n' | b'\t' => Some(LeaderAction::Next),
                    b'p' => Some(LeaderAction::Previous),
                    b'z' => Some(LeaderAction::Zoom),
                    b's' => Some(LeaderAction::Swap),
                    b'+' | b'=' => Some(LeaderAction::Grow),
                    b'-' => Some(LeaderAction::Shrink),
                    b'h' => Some(LeaderAction::Home),
                    b'b' => Some(LeaderAction::Score),
                    b'v' => Some(LeaderAction::Views),
                    b'?' => Some(LeaderAction::Help),
                    b't' => Some(LeaderAction::Theme),
                    _ => {
                        forwarded.push(byte);
                        None
                    }
                };
                if let Some(action) = action {
                    actions.push(action);
                }
            } else if byte == self.leader_byte && !self.paste {
                self.leader = true;
            } else {
                forwarded.push(byte);
            }
            self.recent.push_back(byte);
            while self.recent.len() > 6 {
                self.recent.pop_front();
            }
            let recent = self.recent.iter().copied().collect::<Vec<_>>();
            if recent.ends_with(b"\x1b[200~") {
                self.paste = true;
            } else if recent.ends_with(b"\x1b[201~") {
                self.paste = false;
            }
        }
        (forwarded, actions)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellView {
    Home,
    Stage,
    Score,
    Runs,
}

struct ScoreState {
    session_id: String,
    tasks: Vec<TaskSummary>,
    reports: HashMap<String, orc_core::report::FinalReport>,
    selected: usize,
    message: String,
    dragging: Option<String>,
    width: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FlowStep {
    Brain,
    Workers,
    Cwd,
}

struct NewSessionFlow {
    step: FlowStep,
    brain_choices: Vec<String>,
    brain_index: usize,
    worker_choices: Vec<String>,
    selected_workers: Vec<String>,
    worker_index: usize,
    cwd: String,
}

impl NewSessionFlow {
    fn new(home: &HomeData) -> Self {
        let allowed_brains = home
            .single_harness
            .as_ref()
            .map(|plan| &plan.brain_profiles);
        let allowed_workers = home
            .single_harness
            .as_ref()
            .map(|plan| &plan.worker_profiles);
        let brain_choices = home
            .harnesses
            .iter()
            .filter(|harness| harness.roles.iter().any(|role| role == "brain"))
            .filter(|harness| allowed_brains.is_none_or(|profiles| profiles.contains(&harness.id)))
            .map(|harness| harness.id.clone())
            .collect();
        let worker_choices = home
            .harnesses
            .iter()
            .filter(|harness| harness.roles.iter().any(|role| role == "worker"))
            .filter(|harness| allowed_workers.is_none_or(|profiles| profiles.contains(&harness.id)))
            .map(|harness| harness.id.clone())
            .collect::<Vec<_>>();
        let mut selected_workers = home
            .default_workers
            .iter()
            .filter(|worker| worker_choices.contains(*worker))
            .cloned()
            .collect::<Vec<_>>();
        if selected_workers.is_empty()
            && let Some(worker) = worker_choices.first()
        {
            selected_workers.push(worker.clone());
        }
        Self {
            step: FlowStep::Brain,
            brain_choices,
            brain_index: 0,
            worker_choices,
            selected_workers,
            worker_index: 0,
            cwd: std::env::current_dir().map_or_else(
                |_| ".".to_owned(),
                |path| path.to_string_lossy().into_owned(),
            ),
        }
    }
}

struct HomeState {
    data: HomeData,
    selected: usize,
    flow: Option<NewSessionFlow>,
    message: String,
}

struct ShellState {
    view: ShellView,
    home: HomeState,
    stage: Option<StageState>,
    score: Option<ScoreState>,
    theme: Theme,
    glyphs: Glyphs,
    runs: orc_tui::App,
    reports: Vec<orc_core::report::FinalReport>,
    help: bool,
    reduced_motion: bool,
    /// Wall-clock origin for the ambient HOME animation.
    epoch: Instant,
    /// Parsed leader chord, shared by every screen.
    leader: LeaderKey,
    /// A leader press is pending its follow-up key on HOME, SCORE, or RUNS.
    ///
    /// STAGE keeps its own pending bit inside [`RawRouter`], which has to work
    /// a byte at a time so a chord can be re-sent literally and a bracketed
    /// paste can suppress it; the other three screens consume whole keys and
    /// share this one flag rather than growing a copy each.
    leader_pending: bool,
    /// Session filter shared with the screen-watch thread so snapshots stay
    /// bounded to the attached session.
    watch_session: Arc<Mutex<Option<String>>>,
}

/// The glyph and semantic slot one card carries.
///
/// A blocked card is the exception that outranks its column, because a task
/// nobody can start is not the same news as a task nobody has started. The
/// glyph is what survives `NO_COLOR`; the slot is decoration on top.
fn task_state(task: &TaskSummary) -> (Glyph, Slot) {
    if task.blocked {
        return (Glyph::Failed, Slot::Failed);
    }
    match task.status.as_str() {
        "done" => (Glyph::Confirmed, Slot::Confirmed),
        "running" => (Glyph::InProgress, Slot::Worker),
        "review" => (Glyph::InProgress, Slot::Brain),
        _ => (Glyph::Pending, Slot::Pending),
    }
}

fn render_score(
    frame: &mut Frame<'_>,
    score: &mut ScoreState,
    theme: Theme,
    glyphs: Glyphs,
    leader_label: &str,
) {
    let area = frame.area();
    score.width = area.width.max(1);
    frame.render_widget(Block::new().style(Style::default().bg(theme.bg())), area);
    let columns = ["backlog", "assigned", "running", "review", "done"];
    let width = (area.width / columns.len() as u16).max(1);
    let body = Style::default().fg(theme.fg());
    let meta = theme.state(Slot::Muted);
    for (index, status) in columns.iter().enumerate() {
        let x = area.x.saturating_add(width.saturating_mul(index as u16));
        let column = Rect::new(
            x,
            area.y,
            if index + 1 == columns.len() {
                area.right().saturating_sub(x)
            } else {
                width
            },
            area.height,
        );
        let mut lines = vec![(
            format!(" {}", status.to_ascii_uppercase()),
            Style::default().fg(theme.fg()).add_modifier(Modifier::BOLD),
        )];
        for task in score.tasks.iter().filter(|task| task.status == *status) {
            let selected = score
                .tasks
                .get(score.selected)
                .is_some_and(|chosen| chosen.id == task.id);
            let (glyph, slot) = task_state(task);
            lines.push((
                format!(
                    "{} {} {} {}",
                    if selected { "›" } else { " " },
                    glyphs.get(glyph),
                    task.id,
                    task.title
                ),
                if selected {
                    theme.selection()
                } else {
                    theme.state(slot)
                },
            ));
            lines.push((
                format!(
                    "  {} · {}",
                    task.assignee.as_deref().unwrap_or("unassigned"),
                    if task.isolated { "isolate" } else { "shared" }
                ),
                meta,
            ));
            if let Some(diff) = &task.diff {
                lines.push((format!("  {diff}"), meta));
            }
            if let Some(tokens) = &task.tokens {
                lines.push((format!("  {tokens} tokens"), meta));
            }
            if task.blocked {
                lines.push((
                    format!("  {} BLOCKED: dependencies", glyphs.get(Glyph::Failed)),
                    theme.state(Slot::Failed),
                ));
            }
            if let Some(report) = score.reports.get(&task.id) {
                let passed = report
                    .verdicts
                    .iter()
                    .filter(|verdict| verdict.verdict == "pass")
                    .count();
                let all_passed = passed == report.verdicts.len();
                lines.push((
                    format!(
                        "  {} {passed}/{} · {} {}",
                        glyphs.get(if all_passed {
                            Glyph::Confirmed
                        } else {
                            Glyph::Failed
                        }),
                        report.verdicts.len(),
                        report.reviewer,
                        report.review_mode
                    ),
                    theme.state(if all_passed {
                        Slot::Confirmed
                    } else {
                        Slot::Failed
                    }),
                ));
                if selected {
                    for verdict in &report.verdicts {
                        let passed = verdict.verdict == "pass";
                        lines.push((
                            format!(
                                "  {} {}",
                                glyphs.get(if passed {
                                    Glyph::Confirmed
                                } else {
                                    Glyph::Failed
                                }),
                                verdict.check
                            ),
                            theme.state(if passed {
                                Slot::Confirmed
                            } else {
                                Slot::Failed
                            }),
                        ));
                    }
                }
            }
            if selected {
                if let Some(history) = task.history.last() {
                    lines.push((format!("  {} {}", history.actor, history.action), meta));
                }
                if !score.message.is_empty() {
                    lines.push((
                        format!("  {} ERROR: {}", glyphs.get(Glyph::Failed), score.message),
                        theme.state(Slot::Failed),
                    ));
                }
            }
        }
        if lines.len() == 1 {
            lines.push(("  no tasks".to_owned(), meta));
        }
        // Truncate with an ellipsis inside a one-cell right gutter so the
        // last column no longer clips mid-word at the screen edge (bug B2).
        let usable = usize::from(column.width.saturating_sub(1));
        let clipped = lines
            .into_iter()
            .map(|(line, style)| Line::styled(clip_ellipsis(&line, usable), style))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(clipped).style(body), column);
    }
    frame.render_widget(
        Paragraph::new(format!(
            " SCORE / {} · j/k select · h/l move · drag column · g stage · V RUNS · {leader} h HOME",
            score.session_id,
            leader = leader_label
        ))
        .style(theme.state(Slot::Muted)),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
}

/// Truncate to `width` characters, marking the cut with an ellipsis.
fn clip_ellipsis(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut clipped = text
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    clipped.push('…');
    clipped
}

/// The braille spinner the design gives the conductor while it is thinking.
/// Under reduced motion it settles to the register's `⠿` (see [`avatar`]).
const AVATAR_FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
/// The same spinner for a terminal that cannot draw braille.
const AVATAR_FRAMES_ASCII: [&str; 8] = ["|", "/", "-", "\\", "|", "/", "-", "\\"];
const HOME_TITLE: &str = "PI ORCHESTRA";
const HOME_TAGLINE: &str = "one conductor · a bench of workers · sessions survive detach";
const SINGLE_HARNESS_TAGLINE: &str = "one harness · sequential roles · sessions survive detach";

/// The conductor's ambient spinner frame, or its settled glyph when motion is
/// reduced. Never animation-only: the settled form is a real register glyph,
/// so a still frame still says "conductor".
fn avatar(glyphs: Glyphs, motion: Option<usize>) -> &'static str {
    match motion {
        Some(tick) => match glyphs.tier() {
            GlyphTier::Unicode => AVATAR_FRAMES[tick % AVATAR_FRAMES.len()],
            GlyphTier::Ascii => AVATAR_FRAMES_ASCII[tick % AVATAR_FRAMES_ASCII.len()],
        },
        None => glyphs.get(Glyph::Pulse),
    }
}

/// Render the animated masthead card and return the row below it.
fn render_home_masthead(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: Theme,
    glyphs: Glyphs,
    motion: Option<usize>,
    single_harness: bool,
) -> u16 {
    let card_width = area.width.saturating_sub(4).clamp(24, 68);
    let card = Rect::new(
        area.x + 2,
        area.y + 1,
        card_width,
        4.min(area.height.saturating_sub(1)),
    );
    let avatar_color = match motion {
        Some(tick) if tick % 2 == 0 => theme.glow(),
        _ => theme.brain(),
    };
    let avatar = format!(
        "{} {}",
        glyphs.get(Glyph::Conductor),
        avatar(glyphs, motion)
    );
    let sweep = motion.map(|tick| tick % (HOME_TITLE.len() + 8));
    let mut title = vec![Span::styled(
        format!(" {avatar}  "),
        Style::default()
            .fg(avatar_color)
            .add_modifier(Modifier::BOLD),
    )];
    for (index, glyph) in HOME_TITLE.chars().enumerate() {
        let lit = sweep == Some(index);
        title.push(Span::styled(
            glyph.to_string(),
            Style::default()
                .fg(if lit { theme.glow() } else { theme.brain() })
                .add_modifier(Modifier::BOLD),
        ));
    }
    let masthead = Paragraph::new(vec![
        Line::from(title),
        Line::from(Span::styled(
            format!(
                "     {}",
                if single_harness {
                    SINGLE_HARNESS_TAGLINE
                } else {
                    HOME_TAGLINE
                }
            ),
            Style::default().fg(theme.muted()),
        )),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(theme.border()))
            .style(Style::default().bg(theme.surface())),
    );
    frame.render_widget(masthead, card);
    card.bottom().saturating_add(1)
}

/// One line per configured harness: PATH resolution and verified dispatch.
///
/// The daemon computed the configured summaries from the adapter summary;
/// nothing here contacts a provider. The DISCOVERED block is a read-only,
/// local PATH probe of the additive registry (never written from the client).
fn availability_lines(
    harnesses: &[HarnessSummary],
    discovered: &[HarnessDiscovery],
    theme: Theme,
    glyphs: Glyphs,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(
        "  BENCH AVAILABILITY",
        Style::default().fg(theme.fg()).add_modifier(Modifier::BOLD),
    )];
    let id_width = harnesses
        .iter()
        .map(|harness| harness.id.chars().count())
        .max()
        .unwrap_or(0)
        .max(6);
    for harness in harnesses {
        let (glyph, status, style) = if !harness.available {
            (
                Glyph::Unavailable,
                "NOT ON PATH · unavailable",
                theme.state(Slot::Unavail),
            )
        } else if harness.dispatch_verified {
            (
                Glyph::Available,
                "on PATH · dispatch verified",
                theme.state(Slot::Avail),
            )
        } else {
            (
                Glyph::Available,
                "on PATH · interactive pane only",
                theme.state(Slot::Muted),
            )
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", glyphs.get(glyph)), style),
            Span::styled(
                format!("{:<id_width$}  ", harness.id),
                Style::default().fg(theme.fg()),
            ),
            Span::styled(status.to_owned(), style),
        ]));
    }
    if !discovered.is_empty() {
        lines.push(Line::styled(
            "  DISCOVERED ON PATH",
            Style::default().fg(theme.fg()).add_modifier(Modifier::BOLD),
        ));
        let name_width = discovered
            .iter()
            .map(|harness| harness.name.chars().count())
            .max()
            .unwrap_or(0)
            .max(6);
        for harness in discovered {
            let (glyph, status, style) = if harness.available {
                let label = harness.version.as_deref().map_or_else(
                    || "on PATH · available".to_owned(),
                    |version| format!("on PATH · {version}"),
                );
                (Glyph::Available, label, theme.state(Slot::Avail))
            } else {
                (
                    Glyph::Unavailable,
                    "NOT ON PATH · unavailable".to_owned(),
                    theme.state(Slot::Unavail),
                )
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", glyphs.get(glyph)), style),
                Span::styled(
                    format!("{:<name_width$}  ", harness.name),
                    Style::default().fg(theme.fg()),
                ),
                Span::styled(status, style),
            ]));
        }
    }
    lines
}

/// Plain-language pane health for one shelf card, paired with the glyph and
/// slot that say the same thing without words or colour.
fn session_health(session: &SessionSummary) -> (Glyph, Slot, String) {
    match session.conductor.as_str() {
        // The design sheet gives `⏻` to both "durable session" and "conductor
        // down". On the shelf those are the two states a reader most needs to
        // tell apart, so a healthy card takes the bench's `●` instead —
        // distinguishability outranks a literal reading of the register.
        "live" => (
            Glyph::WorkerSeated,
            Slot::Avail,
            format!(
                "{}/{} workers live · READY",
                session.workers_live, session.workers_total
            ),
        ),
        "down" => (
            Glyph::ConductorDown,
            Slot::Failed,
            format!(
                "{}/{} workers · CONDUCTOR DOWN · R recovers",
                session.workers_live, session.workers_total
            ),
        ),
        "dead" if session.workers_live == 0 => (
            Glyph::Failed,
            Slot::Failed,
            "ALL PANES DEAD · daemon restarted".to_owned(),
        ),
        "dead" => (
            Glyph::Failed,
            Slot::Failed,
            format!(
                "{}/{} workers · CONDUCTOR DEAD",
                session.workers_live, session.workers_total
            ),
        ),
        // A daemon predating pane-health reporting.
        _ if session.attention > 0 => (
            Glyph::Pending,
            Slot::Pending,
            format!("ATTENTION {}", session.attention),
        ),
        _ => (
            Glyph::Detached,
            Slot::Muted,
            format!("{} workers", session.workers.len()),
        ),
    }
}

/// Expand a leading `~` or `~/` using `$HOME`.
fn expand_tilde(path: &str) -> String {
    if (path == "~" || path.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME")
    {
        let home = home.to_string_lossy();
        return if path == "~" {
            home.into_owned()
        } else {
            format!("{home}{}", &path[1..])
        };
    }
    path.to_owned()
}

/// Complete the last segment of a directory path against the filesystem.
///
/// A single match completes fully with a trailing slash; several matches
/// extend to their longest common prefix. Only directories are offered,
/// because a session cwd must be a directory. Hidden directories are offered
/// only when the partial segment already starts with a dot.
fn complete_cwd(input: &str) -> Option<String> {
    let expanded = expand_tilde(input);
    let (parent, partial) = match expanded.rfind('/') {
        Some(index) => (
            expanded[..=index].to_owned(),
            expanded[index + 1..].to_owned(),
        ),
        None => (String::new(), expanded.clone()),
    };
    let read_root = if parent.is_empty() {
        "."
    } else {
        parent.as_str()
    };
    let mut matches = std::fs::read_dir(read_root)
        .ok()?
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name.starts_with(partial.as_str())
                && (!name.starts_with('.') || partial.starts_with('.'))
        })
        .collect::<Vec<_>>();
    matches.sort();
    match matches.as_slice() {
        [] => None,
        [only] => Some(format!("{parent}{only}/")),
        multiple => {
            let mut prefix = multiple[0].clone();
            for name in &multiple[1..] {
                let common = prefix
                    .chars()
                    .zip(name.chars())
                    .take_while(|(left, right)| left == right)
                    .count();
                let end = prefix
                    .char_indices()
                    .nth(common)
                    .map_or(prefix.len(), |(index, _)| index);
                prefix.truncate(end);
            }
            (prefix.chars().count() > partial.chars().count()).then(|| format!("{parent}{prefix}"))
        }
    }
}

/// Wrap a notice on word boundaries without changing its words or punctuation.
fn wrap_words(message: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in message.split_whitespace() {
        let next_len = current
            .chars()
            .count()
            .saturating_add(usize::from(!current.is_empty()))
            .saturating_add(word.chars().count());
        if !current.is_empty() && next_len > width {
            lines.push(current);
            current = String::new();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn render_home(
    frame: &mut Frame<'_>,
    state: &HomeState,
    theme: Theme,
    glyphs: Glyphs,
    motion: Option<usize>,
    leader: &str,
) {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::default().bg(theme.bg())), area);
    let single_harness = state.data.single_harness.is_some();
    let body_top = render_home_masthead(frame, area, theme, glyphs, motion, single_harness);
    let text = Style::default().fg(theme.fg());
    let dim = theme.state(Slot::Muted);
    let focus = theme.state(Slot::Brain);
    let mut lines: Vec<Line<'_>> = Vec::new();
    if let Some(flow) = &state.flow {
        lines.push(Line::styled(
            if single_harness {
                "  NEW SESSION   1 brain  →  2 worker profile  →  3 cwd"
            } else {
                "  NEW SESSION   1 brain  →  2 worker pool  →  3 cwd"
            },
            dim,
        ));
        lines.push(Line::default());
        if single_harness {
            for line in wrap_words(
                SINGLE_HARNESS_MESSAGE,
                usize::from(area.width.saturating_sub(4)).max(1),
            ) {
                lines.push(Line::styled(
                    format!("  {line}"),
                    Style::default()
                        .fg(theme.brain())
                        .add_modifier(Modifier::BOLD),
                ));
            }
            lines.push(Line::default());
        }
        match flow.step {
            FlowStep::Brain => {
                lines.push(Line::styled("  STEP 1 / 3   CHOOSE BRAIN", text));
                for (index, brain) in flow.brain_choices.iter().enumerate() {
                    let chosen = index == flow.brain_index;
                    lines.push(Line::styled(
                        format!(
                            "  {} {}  {brain}",
                            if chosen { "›" } else { " " },
                            glyphs.get(Glyph::Conductor)
                        ),
                        if chosen { theme.selection() } else { text },
                    ));
                }
                lines.push(Line::styled(
                    "  ↑/↓ choose · enter continue · esc cancel",
                    dim,
                ));
            }
            FlowStep::Workers => {
                lines.push(Line::styled(
                    if single_harness {
                        "  STEP 2 / 3   CHOOSE SEQUENTIAL WORKER PROFILE"
                    } else {
                        "  STEP 2 / 3   CHOOSE WORKER POOL"
                    },
                    text,
                ));
                for (index, worker) in flow.worker_choices.iter().enumerate() {
                    let selected = flow.selected_workers.contains(worker);
                    let chosen = index == flow.worker_index;
                    // ● seated, ○ an empty seat: the bench metaphor, and the
                    // only cue that survives a colourless terminal.
                    lines.push(Line::styled(
                        format!(
                            "  {} {}  [{}] {worker}",
                            if chosen { "›" } else { " " },
                            glyphs.get(if selected {
                                Glyph::WorkerSeated
                            } else {
                                Glyph::WorkerIdle
                            }),
                            if selected { "PRESELECTED" } else { "EDITABLE" }
                        ),
                        if chosen { theme.selection() } else { text },
                    ));
                }
                lines.push(Line::styled(
                    if single_harness {
                        "  space edits profile selection · enter continue"
                    } else {
                        "  space edits selection · enter continue"
                    },
                    dim,
                ));
            }
            FlowStep::Cwd => {
                lines.push(Line::styled(
                    "  STEP 3 / 3   CHOOSE WORKING DIRECTORY",
                    text,
                ));
                let brain = flow
                    .brain_choices
                    .get(flow.brain_index)
                    .map_or("none", String::as_str);
                let workers = if flow.selected_workers.is_empty() {
                    "none".to_owned()
                } else {
                    flow.selected_workers.join(", ")
                };
                lines.push(Line::styled(
                    if single_harness {
                        format!("  launching brain {brain} · sequential profiles {workers}")
                    } else {
                        format!("  launching brain {brain} · workers {workers}")
                    },
                    dim,
                ));
                lines.push(Line::styled(format!("  > {}", flow.cwd), focus));
                if Path::new(&expand_tilde(&flow.cwd)).is_dir() {
                    lines.push(Line::styled(
                        "  directory exists — every pane starts here",
                        dim,
                    ));
                } else {
                    lines.push(Line::styled(
                        "  NOT A DIRECTORY — fix the path before launch",
                        Style::default().fg(theme.failed()),
                    ));
                }
                lines.push(Line::styled(
                    "  type · tab complete · ctrl-u clear · enter launch · esc back",
                    dim,
                ));
            }
        }
    } else if state.data.sessions.is_empty() {
        lines.extend([
            Line::styled(
                "  WELCOME TO THE BENCH",
                Style::default().fg(theme.fg()).add_modifier(Modifier::BOLD),
            ),
            Line::default(),
            Line::styled(
                if single_harness {
                    "  One capable HARNESS plans, implements, then reviews in"
                } else {
                    "  One expensive BRAIN plans and delegates; a bench of cheap"
                },
                text,
            ),
            Line::styled(
                if single_harness {
                    "  sequence with evidence. Panes live in the orcd daemon,"
                } else {
                    "  WORKERS executes bounded briefs. Panes live in the orcd"
                },
                text,
            ),
            Line::styled(
                if single_harness {
                    "  so sessions survive detach — close this client any time"
                } else {
                    "  daemon, so sessions survive detach — close this client any"
                },
                text,
            ),
            Line::styled(
                if single_harness {
                    "  and reattach later."
                } else {
                    "  time and reattach later."
                },
                text,
            ),
            Line::default(),
            Line::styled(
                "  FIRST KEYS",
                Style::default().fg(theme.fg()).add_modifier(Modifier::BOLD),
            ),
            Line::from(vec![
                Span::styled("  n      ", focus),
                Span::styled(
                    if single_harness {
                        "new session — brain, sequential profile, directory"
                    } else {
                        "new session — brain, worker pool, directory"
                    },
                    text,
                ),
            ]),
            Line::from(vec![
                Span::styled("  enter  ", focus),
                Span::styled(
                    format!("attach a session · {leader} q detaches, panes keep running"),
                    text,
                ),
            ]),
            Line::from(vec![
                Span::styled("  ?      ", focus),
                Span::styled("help — every key, delegation, recovery", text),
            ]),
            Line::default(),
        ]);
        lines.extend(availability_lines(
            &state.data.harnesses,
            &state.data.discovered,
            theme,
            glyphs,
        ));
    } else {
        lines.push(Line::styled(
            "  SESSION SHELF",
            Style::default().fg(theme.fg()).add_modifier(Modifier::BOLD),
        ));
        for (index, session) in state.data.sessions.iter().enumerate() {
            let chosen = index == state.selected;
            let (glyph, slot, health) = session_health(session);
            lines.push(Line::styled(
                format!(
                    "  {} {} ╭ {} · {health}",
                    if chosen { "›" } else { " " },
                    glyphs.get(glyph),
                    session.id
                ),
                if chosen {
                    theme.selection()
                } else {
                    theme.state(slot)
                },
            ));
            lines.push(Line::styled(
                format!(
                    "      ╰ {}  ·  {}  ·  {}",
                    session.brain, session.cwd, session.updated_at
                ),
                if chosen { focus } else { dim },
            ));
        }
        lines.push(Line::default());
        lines.push(Line::styled("  enter attach · n new session · V RUNS", dim));
        lines.push(Line::default());
        lines.extend(availability_lines(
            &state.data.harnesses,
            &state.data.discovered,
            theme,
            glyphs,
        ));
    }
    if !state.message.is_empty() {
        lines.push(Line::default());
        lines.push(Line::styled(format!("  {}", state.message), text));
    }
    let body = Rect::new(
        area.x,
        body_top.min(area.bottom().saturating_sub(1)),
        area.width,
        area.bottom()
            .saturating_sub(1)
            .saturating_sub(body_top.min(area.bottom().saturating_sub(1))),
    );
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.fg()).bg(theme.bg())),
        body,
    );
    render_legend(
        frame,
        area,
        "n new · enter attach · V views · ? help · q quit",
        theme,
    );
}

fn render_legend(frame: &mut Frame<'_>, area: Rect, text: &str, theme: Theme) {
    frame.render_widget(
        Paragraph::new(format!(" {text}")).style(theme.state(Slot::Muted).bg(theme.bg())),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
}

fn render_help(frame: &mut Frame<'_>, theme: Theme, leader: &str) {
    let area = frame.area();
    // Help floats over the stage, so it takes the overlay fill.
    frame.render_widget(
        Block::new().style(Style::default().bg(theme.overlay())),
        area,
    );
    frame.render_widget(
        Paragraph::new(format!(
            "  PI ORCHESTRA / HELP\n\n  FIRST USE\n  n creates a session: choose a brain, edit worker offers, choose a cwd.\n  The brain plans; available workers receive explicit durable task briefs.\n\n  CONTROL\n  In STAGE everything you type goes to the focused pane. Commands need\n  the leader first: press {leader}, release, then one key.\n  {leader} n/p focus · {leader} z zoom · {leader} s swap · {leader} b SCORE\n  {leader} h HOME · {leader} v views · {leader} ? help · {leader} q detach\n  {leader} twice sends the literal chord to the pane.\n  Outside STAGE, bare V cycles HOME, SCORE, RUNS and ? opens help.\n\n  THEME\n  {leader} t cycles nocturne, ember, phosphor on every screen, and\n  asks the daemon to remember it: the next launch opens the same.\n  pio config set theme <name> does it from a shell; pio config get\n  theme reports what is stored. No file to edit.\n  Set the leader with app.leader_key in ~/.orchestra/harnesses.json.\n\n  DURABILITY AND RECOVERY\n  Closing the client detaches; pi-orchestra attach replays the session.\n  SCORE is the durable task board. Delivery is shown only after confirmation.\n  Missing executables are UNAVAILABLE. R recovers a supported dead brain.\n  If recovery fails, reattach and inspect SCORE, orc task list, and orc list.\n\n  Esc or ? closes help.",
        ))
        .style(Style::default().fg(theme.fg()).bg(theme.overlay())),
        area,
    );
    render_legend(frame, area, "Esc / ? close help", theme);
}

fn render_shell(frame: &mut Frame<'_>, shell: &mut ShellState) {
    if shell.help {
        render_help(frame, shell.theme, &shell.leader.label);
        return;
    }
    match shell.view {
        ShellView::Home => {
            let motion =
                (!shell.reduced_motion).then(|| (shell.epoch.elapsed().as_millis() / 120) as usize);
            render_home(
                frame,
                &shell.home,
                shell.theme,
                shell.glyphs,
                motion,
                &shell.leader.label,
            );
        }
        ShellView::Stage => {
            let motion =
                (!shell.reduced_motion).then(|| (shell.epoch.elapsed().as_millis() / 120) as usize);
            let baton = shell.stage.as_ref().map_or(baton::State::Idle, |stage| {
                stage.baton_state(shell.reduced_motion)
            });
            if let Some(stage) = shell.stage.as_mut() {
                render_stage(frame, stage, motion, baton);
            }
        }
        ShellView::Score => {
            if let Some(score) = shell.score.as_mut() {
                render_score(frame, score, shell.theme, shell.glyphs, &shell.leader.label);
            }
        }
        ShellView::Runs => {
            orc_tui::draw(frame, &mut shell.runs);
            render_runs_reports(frame, &shell.reports, shell.theme, shell.glyphs);
            // One line, consistent with what the embedded App actually
            // answers in its current view.
            let legend = match shell.runs.view {
                orc_tui::View::Dashboard => {
                    "RUNS · j/k select · enter open · / search · V/h HOME · ? help · q quit"
                }
                orc_tui::View::Session => {
                    "RUNS · tab tabs · s send · r retry · h handoff · Esc back · q quit"
                }
                orc_tui::View::Settings => {
                    "RUNS settings · t theme · n notifications · Esc back · q quit"
                }
            };
            render_legend(frame, frame.area(), legend, shell.theme);
        }
    }
}

fn render_runs_reports(
    frame: &mut Frame<'_>,
    reports: &[orc_core::report::FinalReport],
    theme: Theme,
    glyphs: Glyphs,
) {
    let Some(report) = reports.first() else {
        return;
    };
    let passed = report
        .verdicts
        .iter()
        .filter(|verdict| verdict.verdict == "pass")
        .count();
    let all_passed = passed == report.verdicts.len();
    let glyph = glyphs.get(if all_passed {
        Glyph::Confirmed
    } else {
        Glyph::Failed
    });
    let line = format!(
        " REPORTS {glyph} {} {passed}/{} · {} {} · {}",
        report.task,
        report.verdicts.len(),
        report.reviewer,
        report.review_mode,
        report.title
    );
    let area = frame.area();
    frame.render_widget(
        Paragraph::new(clip_ellipsis(
            &line,
            usize::from(area.width.saturating_sub(1)),
        ))
        .style(theme.state(if all_passed {
            Slot::Confirmed
        } else {
            Slot::Failed
        })),
        Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 1),
    );
}

/// Pick the theme a launch opens in.
///
/// The daemon's configured theme wins — that is where `<leader> t` and `pio
/// config set theme` both land, so it is what makes a choice survive a
/// relaunch. The CLI flag is the fallback for a daemon that has no opinion
/// yet, and an unrecognised name resolves to the flagship rather than
/// refusing to start. One theme for every screen, so STAGE and HOME can never
/// disagree about which palette is live.
fn resolve_initial_theme(configured: &str, fallback: ThemeName) -> ThemeName {
    if configured.trim().is_empty() {
        fallback
    } else {
        ThemeName::named(configured.trim())
    }
}

/// Run the interactive HOME/STAGE shell until the leader-key detach command.
pub fn run(socket: PathBuf, theme: ThemeName) -> Result<()> {
    run_initial(socket, theme, None, false)
}

/// Run the client with an optional initial session or the honest RUNS placeholder.
pub fn run_initial(
    socket: PathBuf,
    theme: ThemeName,
    initial_session: Option<String>,
    runs: bool,
) -> Result<()> {
    let mut commands = BenchClient::connect(&socket)?;
    let home = commands.home()?;
    let selected_theme = resolve_initial_theme(&home.theme, theme);
    // Probed, never assumed: what this terminal can actually render.
    let resolved = Theme::new(selected_theme, ColorTier::detect());
    let glyphs = Glyphs::new(GlyphTier::detect());
    let reduced_motion = home.reduced_motion;
    let leader = LeaderKey::parse(&home.leader_key);
    let mut runs_app = orc_tui::App::new(Some(selected_theme.as_str()))
        .map_err(|error| AppError::Daemon(format!("RUNS ledger unavailable: {error}")))?;
    // The embedded ledger borrows this crate's theme map rather than its own
    // two-theme set, so RUNS is nocturne when everything else is.
    runs_app.theme = resolved.runs_theme();
    let mut shell = ShellState {
        view: if runs {
            ShellView::Runs
        } else {
            ShellView::Home
        },
        home: HomeState {
            data: home,
            selected: 0,
            flow: None,
            message: String::new(),
        },
        stage: None,
        score: None,
        theme: resolved,
        glyphs,
        runs: runs_app,
        reports: orc_core::report::list_reports(None).unwrap_or_default(),
        help: false,
        reduced_motion,
        epoch: Instant::now(),
        leader,
        leader_pending: false,
        watch_session: Arc::new(Mutex::new(None)),
    };
    if let Some(session_id) = initial_session {
        attach_stage(&mut commands, &mut shell, session_id)?;
    }
    let (events_tx, events_rx) = mpsc::sync_channel(64);
    spawn_screen_watch(socket, Arc::clone(&shell.watch_session), events_tx.clone());
    spawn_runs_watch(events_tx.clone());
    spawn_reports_watch(events_tx.clone());

    let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES;
    execute!(
        io::stdout(),
        EnableMouseCapture,
        EnableBracketedPaste,
        EnableFocusChange,
        PushKeyboardEnhancementFlags(flags)
    )?;
    let mut terminal = ratatui::init();
    spawn_raw_terminal_events(events_tx.clone());
    spawn_resize_events(events_tx);
    let result = run_shell_loop(&mut terminal, &mut commands, &mut shell, &events_rx);
    ratatui::restore();
    execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableFocusChange,
        DisableBracketedPaste,
        DisableMouseCapture
    )?;
    result
}

fn attach_stage(
    commands: &mut BenchClient,
    shell: &mut ShellState,
    session_id: String,
) -> Result<()> {
    let session = commands.attach_session(session_id.clone())?;
    let tasks = commands.task_board(session_id.clone())?;
    if let Ok(mut watched) = shell.watch_session.lock() {
        *watched = Some(session_id.clone());
    }
    let mut stage = StageState::for_session(
        session_id.clone(),
        session.panes,
        session.layout,
        shell.theme,
        shell.glyphs,
    );
    stage.raw_router.leader_byte = shell.leader.byte;
    stage.leader_label = shell.leader.label.clone();
    stage.confirmed_panes = tasks
        .iter()
        .filter_map(|task| {
            task.history
                .last()
                .filter(|history| history.action == "delivery_confirmed")
                .and(task.assignee_run.clone())
        })
        .collect();
    if tasks.iter().any(|task| {
        task.history
            .last()
            .is_some_and(|history| history.action == "delivery_confirmed")
    }) {
        stage.mark_output();
    }
    shell.stage = Some(stage);
    shell.score = Some(ScoreState {
        reports: shell
            .reports
            .iter()
            .filter(|report| report.session == session_id)
            .map(|report| (report.task.clone(), report.clone()))
            .collect(),
        tasks,
        session_id,
        selected: 0,
        message: String::new(),
        dragging: None,
        width: 1,
    });
    shell.view = ShellView::Stage;
    Ok(())
}

fn run_shell_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    commands: &mut BenchClient,
    shell: &mut ShellState,
    events: &Receiver<UiEvent>,
) -> Result<()> {
    let mut redraw = true;
    let mut requested_sizes = HashMap::new();
    loop {
        // The decay timer runs under reduced motion too: it is state (is the
        // baton live or idle?), not animation, and the static rails still have
        // to switch between the two.
        if let Some(stage) = shell.stage.as_mut() {
            stage.advance();
        }
        let stage_live = shell.view == ShellView::Stage
            && shell
                .stage
                .as_ref()
                .is_some_and(|stage| !stage.pulse.done());
        let animating = !shell.reduced_motion
            && shell.stage.as_ref().is_some_and(|stage| {
                shell.view == ShellView::Stage && (!stage.pulse.done() || stage.has_live_trigger())
            });
        let home_ambient = !shell.reduced_motion && !shell.help && shell.view == ShellView::Home;
        // The RUNS embed repaints on a modest tick so quota/history updates
        // arriving on the App's internal channel become visible without a
        // keypress. This is data refresh, not animation, so it is kept under
        // reduced_motion; App::refresh is internally rate-limited to 500 ms.
        let runs_ambient = !shell.help && shell.view == ShellView::Runs;
        if runs_ambient {
            let _ = shell.runs.refresh();
        }
        if redraw || animating || home_ambient || runs_ambient || stage_live {
            let mut stdout = io::stdout();
            stdout.sync_update(|_| terminal.draw(|frame| render_shell(frame, shell)))??;
            if shell.view == ShellView::Stage
                && let Some(stage) = shell.stage.as_mut()
            {
                resize_to_cards(commands, stage, &mut requested_sizes)?;
                persist_stage_layout(commands, stage)?;
            }
            redraw = false;
        }
        let wait = if animating {
            Duration::from_millis(16)
        } else if stage_live {
            // Reduced motion: the rail is static, so this cadence only exists
            // so the decay to the idle rail is noticed promptly.
            Duration::from_millis(500)
        } else if home_ambient {
            Duration::from_millis(120)
        } else if runs_ambient {
            Duration::from_millis(500)
        } else {
            Duration::from_secs(30)
        };
        let event = match events.recv_timeout(wait) {
            Ok(event) => Some(event),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(AppError::EventSource),
        };
        match event {
            Some(UiEvent::Snapshot(panes)) => {
                if let Some(stage) = shell.stage.as_mut() {
                    let panes = if let Some(session_id) = &stage.session_id {
                        panes
                            .into_iter()
                            .filter(|pane| pane.session_id.as_ref() == Some(session_id))
                            .collect()
                    } else {
                        panes
                    };
                    stage.apply_snapshot(panes);
                }
                if let Some(score) = shell.score.as_mut()
                    && let Ok(tasks) = commands.task_board(score.session_id.clone())
                {
                    score.tasks = tasks;
                    if let Some(stage) = shell.stage.as_mut() {
                        stage.confirmed_panes = score
                            .tasks
                            .iter()
                            .filter_map(|task| {
                                task.history
                                    .last()
                                    .filter(|history| history.action == "delivery_confirmed")
                                    .and(task.assignee_run.clone())
                            })
                            .collect();
                        // A task event is traffic on the filament too, so it
                        // pulses the baton exactly as a stdout tick does.
                        if score
                            .tasks
                            .iter()
                            .filter_map(|task| task.history.last())
                            .any(|history| {
                                matches!(
                                    history.action.as_str(),
                                    "delivery_confirmed" | "delivery_failed" | "done"
                                )
                            })
                        {
                            stage.mark_output();
                        }
                    }
                }
                let _ = shell.runs.refresh();
                redraw = true;
            }
            Some(UiEvent::Raw(bytes)) => {
                if handle_raw_event(&bytes, commands, shell)? {
                    return Ok(());
                }
                redraw = true;
            }
            Some(UiEvent::Resize) => {
                requested_sizes.clear();
                redraw = true;
            }
            Some(UiEvent::WatchFailed(message)) => return Err(AppError::Connection(message)),
            Some(UiEvent::RunsChanged) => {
                let _ = shell.runs.refresh_now();
                shell.reports = orc_core::report::list_reports(None).unwrap_or_default();
                if let Some(score) = shell.score.as_mut() {
                    score.reports = shell
                        .reports
                        .iter()
                        .filter(|report| report.session == score.session_id)
                        .map(|report| (report.task.clone(), report.clone()))
                        .collect();
                }
                redraw = true;
            }
            None => {}
        }
    }
}

fn persist_stage_layout(commands: &mut BenchClient, state: &mut StageState) -> Result<()> {
    let Some(session_id) = state.session_id.clone() else {
        return Ok(());
    };
    if state.zoomed || state.pane_areas.len() != state.panes.len() {
        return Ok(());
    }
    let layout = state
        .panes
        .iter()
        .zip(&state.pane_areas)
        .enumerate()
        .map(|(order, (pane, area))| LayoutRect {
            pane_id: pane.id.clone(),
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
            order,
        })
        .collect::<Vec<_>>();
    if layout != state.layout {
        commands.update_layout(session_id, layout.clone())?;
        state.layout = layout;
    }
    Ok(())
}

fn resize_to_cards(
    commands: &mut BenchClient,
    state: &StageState,
    requested_sizes: &mut HashMap<String, (u16, u16)>,
) -> Result<()> {
    for (pane, area) in state.panes.iter().zip(&state.pane_areas) {
        let size = (
            area.height.saturating_sub(2).max(1),
            area.width.saturating_sub(2).max(1),
        );
        if requested_sizes.get(&pane.id) != Some(&size) {
            commands.resize(pane.id.clone(), size.0, size.1)?;
            requested_sizes.insert(pane.id.clone(), size);
        }
    }
    Ok(())
}

fn spawn_raw_terminal_events(sender: SyncSender<UiEvent>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut stdin = stdin.lock();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            if sender.send(UiEvent::Raw(buffer[..read].to_vec())).is_err() {
                break;
            }
        }
    });
}

fn spawn_resize_events(sender: SyncSender<UiEvent>) {
    thread::spawn(move || {
        let Ok(mut signals) = signal_hook::iterator::Signals::new([signal_hook::consts::SIGWINCH])
        else {
            return;
        };
        for _ in signals.forever() {
            if sender.send(UiEvent::Resize).is_err() {
                break;
            }
        }
    });
}

fn spawn_screen_watch(
    socket: PathBuf,
    watch_session: Arc<Mutex<Option<String>>>,
    sender: SyncSender<UiEvent>,
) {
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let mut client = BenchClient::connect(&socket)?;
            let mut sequences = Vec::new();
            loop {
                let next = client.wait(sequences.clone(), Duration::from_secs(30))?;
                if next != sequences {
                    sequences = next;
                    let session = watch_session
                        .lock()
                        .ok()
                        .and_then(|watched| watched.clone());
                    let panes = client.snapshot(session)?;
                    if sender.send(UiEvent::Snapshot(panes)).is_err() {
                        return Ok(());
                    }
                }
            }
        })();
        if let Err(error) = result {
            let _ = sender.send(UiEvent::WatchFailed(format!(
                "screen watch failed: {error}"
            )));
        }
    });
}

fn spawn_runs_watch(sender: SyncSender<UiEvent>) {
    spawn_runs_watch_path(orc_core::registry::home().join("runs"), sender);
}

fn spawn_reports_watch(sender: SyncSender<UiEvent>) {
    spawn_runs_watch_path(orc_core::registry::home().join("reports"), sender);
}

fn spawn_runs_watch_path(path: PathBuf, sender: SyncSender<UiEvent>) {
    thread::spawn(move || {
        if std::fs::create_dir_all(&path).is_err() {
            let _ = sender.send(UiEvent::WatchFailed(
                "runs watcher could not create the runs directory".to_owned(),
            ));
            return;
        }
        let (events, changes) = mpsc::sync_channel(16);
        let Ok(mut watcher) =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok() {
                    let _ = events.try_send(());
                }
            })
        else {
            let _ = sender.send(UiEvent::WatchFailed(
                "runs watcher could not start".to_owned(),
            ));
            return;
        };
        if watcher.watch(&path, RecursiveMode::Recursive).is_err() {
            let _ = sender.send(UiEvent::WatchFailed(
                "runs watcher could not watch the runs directory".to_owned(),
            ));
            return;
        }
        while changes.recv().is_ok() {
            if sender.send(UiEvent::RunsChanged).is_err() {
                break;
            }
        }
    });
}

/// Remove terminal FocusIn/FocusOut reports (`ESC [ I`, `ESC [ O`).
///
/// The client enables `EnableFocusChange`, so terminals emit these reports on
/// focus moves. Outside STAGE they would otherwise be decoded as junk keys
/// (`Esc`, `[`, `I`) and typed into flow fields.
fn strip_focus_reports(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b
            && bytes.get(index + 1) == Some(&b'[')
            && matches!(bytes.get(index + 2), Some(b'I' | b'O'))
        {
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    output
}

fn handle_raw_event(
    bytes: &[u8],
    commands: &mut BenchClient,
    shell: &mut ShellState,
) -> Result<bool> {
    // STAGE forwards raw bytes verbatim to the focused pane; every other view
    // consumes focus reports so they can never masquerade as typed keys.
    let stripped;
    let bytes = if shell.view == ShellView::Stage {
        bytes
    } else {
        stripped = strip_focus_reports(bytes);
        if stripped.is_empty() {
            return Ok(false);
        }
        &stripped
    };
    if shell.help {
        if matches!(bytes, b"?" | b"\x1b") {
            shell.help = false;
        }
        return Ok(false);
    }
    if let Some(quit) = route_leader(bytes, Some(commands), shell) {
        return Ok(quit);
    }
    // Bare `?` and `V` are view keys only where no raw input is expected:
    // STAGE forwards every unprefixed byte to the focused pane, the launch
    // flow needs literal `V` and `?` for paths and titles, and an active
    // RUNS text input (search, brief) must accept them as characters.
    let raw_input_view = shell.view == ShellView::Stage
        || (shell.view == ShellView::Home && shell.home.flow.is_some())
        || (shell.view == ShellView::Runs && shell.runs.input_mode != orc_tui::InputMode::None);
    if !raw_input_view {
        if bytes == b"?" {
            shell.help = true;
            return Ok(false);
        }
        if bytes == b"V" {
            shell.view = match shell.view {
                ShellView::Home => {
                    if shell.score.is_some() {
                        ShellView::Score
                    } else {
                        ShellView::Runs
                    }
                }
                ShellView::Score => ShellView::Runs,
                ShellView::Runs | ShellView::Stage => ShellView::Home,
            };
            return Ok(false);
        }
    }
    match shell.view {
        ShellView::Runs => {
            for key in raw_home_keys(bytes) {
                if route_runs_key(shell, key, Some(commands)) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ShellView::Home => {
            for key in raw_home_keys(bytes) {
                if handle_home_key(key, commands, shell)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ShellView::Score => {
            let Some(score) = shell.score.as_mut() else {
                return Ok(false);
            };
            if bytes == b"g" {
                if let (Some(stage), Some(task)) =
                    (shell.stage.as_mut(), score.tasks.get(score.selected))
                    && let Some(pane_id) = &task.assignee_run
                    && let Some(index) = stage.panes.iter().position(|pane| &pane.id == pane_id)
                {
                    stage.focus = index;
                }
                shell.view = ShellView::Stage;
                return Ok(false);
            }
            if let Some((button, column, _row, suffix)) = score_mouse(bytes) {
                let statuses = ["backlog", "assigned", "running", "review", "done"];
                let index = usize::from(column.saturating_sub(1)).saturating_mul(statuses.len())
                    / usize::from(score.width.max(1));
                let target = statuses[index.min(statuses.len().saturating_sub(1))];
                if button == 0 && suffix == 'M' {
                    score.dragging = score
                        .tasks
                        .iter()
                        .find(|task| task.status == target)
                        .map(|task| task.id.clone());
                    return Ok(false);
                }
                if suffix == 'm' {
                    if let Some(task_id) = score.dragging.take() {
                        match commands.move_task(
                            score.session_id.clone(),
                            task_id,
                            target.to_owned(),
                        ) {
                            Ok(tasks) => {
                                score.tasks = tasks;
                                score.message.clear();
                            }
                            Err(error) => score.message = error.to_string(),
                        }
                    }
                    return Ok(false);
                }
            }
            if bytes == b"j" && !score.tasks.is_empty() {
                score.selected = (score.selected + 1) % score.tasks.len();
            }
            if bytes == b"k" && !score.tasks.is_empty() {
                score.selected = score
                    .selected
                    .checked_sub(1)
                    .unwrap_or_else(|| score.tasks.len().saturating_sub(1));
            }
            let target = score
                .tasks
                .get(score.selected)
                .and_then(|task| match bytes {
                    b"h" => match task.status.as_str() {
                        "assigned" => Some("backlog"),
                        "running" => Some("assigned"),
                        "review" => Some("running"),
                        _ => None,
                    },
                    b"l" => match task.status.as_str() {
                        "backlog" => Some("assigned"),
                        "assigned" => Some("running"),
                        "running" => Some("review"),
                        "review" => Some("done"),
                        _ => None,
                    },
                    _ => None,
                });
            if let (Some(status), Some(task)) = (target, score.tasks.get(score.selected)) {
                match commands.move_task(
                    score.session_id.clone(),
                    task.id.clone(),
                    status.to_owned(),
                ) {
                    Ok(tasks) => {
                        score.tasks = tasks;
                        score.message.clear();
                    }
                    Err(error) => score.message = error.to_string(),
                }
            }
            Ok(false)
        }
        ShellView::Stage => {
            let Some(stage) = shell.stage.as_mut() else {
                return Ok(false);
            };
            if bytes == b"R"
                && let Some(pane) = stage.panes.get(stage.focus)
                && pane.state.as_deref() == Some("conductor_down")
            {
                match commands.respawn_conductor(pane.id.clone()) {
                    Ok(()) => stage.message.clear(),
                    // A refused recovery (for example RESUME NOT SUPPORTED)
                    // is shown in place instead of exiting the client.
                    Err(AppError::Daemon(message)) => stage.message = message,
                    Err(error) => return Err(error),
                }
                return Ok(false);
            }
            if let Some(mouse) = route_raw_mouse(bytes, stage) {
                if let Some(mouse) = mouse {
                    send_focused(commands, stage, mouse)?;
                }
                return Ok(false);
            }
            let (forwarded, actions) = stage.raw_router.route(bytes);
            for action in actions {
                // The theme is shell-wide state — every screen's copy has to
                // move together — so it takes `shell` rather than the
                // STAGE-local borrow the other actions use.
                if action == LeaderAction::Theme {
                    cycle_theme(shell, Some(commands));
                    continue;
                }
                let Some(stage) = shell.stage.as_mut() else {
                    continue;
                };
                match action {
                    LeaderAction::Theme => {}
                    LeaderAction::Quit => return Ok(true),
                    LeaderAction::Next => {
                        if !stage.panes.is_empty() {
                            stage.focus = (stage.focus + 1) % stage.panes.len();
                        }
                    }
                    LeaderAction::Previous => {
                        if !stage.panes.is_empty() {
                            stage.focus = stage
                                .focus
                                .checked_sub(1)
                                .unwrap_or_else(|| stage.panes.len().saturating_sub(1));
                        }
                    }
                    LeaderAction::Zoom => stage.zoomed = !stage.zoomed,
                    LeaderAction::Swap => {
                        if stage.panes.len() > 1 {
                            let next = (stage.focus + 1) % stage.panes.len();
                            stage.panes.swap(stage.focus, next);
                            stage.focus = next;
                        }
                    }
                    LeaderAction::Grow | LeaderAction::Shrink => {
                        ensure_layout(stage);
                        if let Some(area) = stage.layout.get_mut(stage.focus) {
                            let grow = action == LeaderAction::Grow;
                            area.width = if grow {
                                area.width.saturating_add(2)
                            } else {
                                area.width.saturating_sub(2).max(10)
                            };
                            area.height = if grow {
                                area.height.saturating_add(1)
                            } else {
                                area.height.saturating_sub(1).max(5)
                            };
                        }
                    }
                    LeaderAction::Home => shell.view = ShellView::Home,
                    LeaderAction::Score => {
                        if shell.score.is_some() {
                            shell.view = ShellView::Score;
                        }
                    }
                    LeaderAction::Views => shell.view = ShellView::Home,
                    LeaderAction::Help => shell.help = true,
                }
            }
            if !forwarded.is_empty()
                && let Some(stage) = shell.stage.as_ref()
            {
                send_focused(commands, stage, forwarded)?;
            }
            Ok(false)
        }
    }
}

/// Apply one theme to every screen at once.
///
/// The live theme is held in three places — the shell (HOME, SCORE, help),
/// the embedded RUNS ledger, and STAGE's own `StageState` — so a switcher that
/// updates fewer than all three leaves a screen rendering the previous
/// palette. RUNS borrows this crate's map via [`Theme::runs_theme`] rather
/// than `orc-tui`'s own set, so the ledger can never fall back to a colour the
/// seventeen-slot map does not contain.
fn apply_theme(shell: &mut ShellState, name: ThemeName) {
    shell.theme = Theme::new(name, shell.theme.tier());
    shell.runs.theme = shell.theme.runs_theme();
    if let Some(stage) = shell.stage.as_mut() {
        stage.theme = shell.theme;
    }
}

/// Advance the theme one step and ask the daemon to remember it.
///
/// The switch itself is local and always succeeds; only the *persistence*
/// needs the daemon, so a failed round trip degrades to a session-only change
/// with the reason on the message line instead of refusing to switch.
fn cycle_theme(shell: &mut ShellState, commands: Option<&mut BenchClient>) {
    let next = shell.theme.name().next();
    apply_theme(shell, next);
    let Some(commands) = commands else {
        return;
    };
    let message = match commands.set_theme(next.as_str().to_owned()) {
        Ok(_) => String::new(),
        Err(error) => format!("theme not saved: {error}"),
    };
    set_message(shell, message);
}

/// Put one recoverable message on whichever screen the user is looking at.
fn set_message(shell: &mut ShellState, message: String) {
    match shell.view {
        ShellView::Home => shell.home.message = message,
        ShellView::Stage => {
            if let Some(stage) = shell.stage.as_mut() {
                stage.message = message;
            }
        }
        ShellView::Score => {
            if let Some(score) = shell.score.as_mut() {
                score.message = message;
            }
        }
        ShellView::Runs => shell.runs.message = message,
    }
}

/// Arm and consume the leader chord on HOME, SCORE, and RUNS.
///
/// Returns `None` when the bytes are not part of a chord and belong to the
/// screen underneath, or `Some(quit)` when the chord swallowed them. STAGE is
/// excluded: it arms its own inside [`RawRouter`], which works a byte at a
/// time so the chord can be re-sent literally and suppressed inside a
/// bracketed paste.
fn route_leader(
    bytes: &[u8],
    commands: Option<&mut BenchClient>,
    shell: &mut ShellState,
) -> Option<bool> {
    if shell.view == ShellView::Stage {
        return None;
    }
    if shell.leader_pending {
        shell.leader_pending = false;
        return Some(handle_leader_chord(bytes, commands, shell));
    }
    if bytes == [shell.leader.byte] {
        shell.leader_pending = true;
        return Some(false);
    }
    None
}

/// Act on the key that followed the leader chord on HOME, SCORE, or RUNS.
///
/// One table for all three: the chord used to exist only on STAGE and SCORE,
/// which left HOME — the launch screen, and the first thing a new user sees —
/// with no way to reach it. Returns true when the client should quit.
fn handle_leader_chord(
    bytes: &[u8],
    commands: Option<&mut BenchClient>,
    shell: &mut ShellState,
) -> bool {
    match bytes {
        b"q" => return true,
        b"h" => shell.view = ShellView::Home,
        b"b" => {
            if shell.score.is_some() {
                shell.view = ShellView::Score;
            }
        }
        b"v" => shell.view = ShellView::Runs,
        b"?" => shell.help = true,
        b"t" => cycle_theme(shell, commands),
        _ => {}
    }
    false
}

/// Route one decoded key into the embedded RUNS control plane.
///
/// Returns true when the whole client should quit. The documented exits act
/// only at the App's top-level dashboard while no text input is active;
/// deeper views and active inputs receive every key, so the embedded
/// legends describe what actually happens.
fn route_runs_key(
    shell: &mut ShellState,
    key: KeyEvent,
    commands: Option<&mut BenchClient>,
) -> bool {
    let busy = shell.runs.input_mode != orc_tui::InputMode::None || shell.runs.help;
    if !busy {
        if key.code == KeyCode::Char('q') {
            return true;
        }
        // `t` is the theme switcher on every screen, so it takes the shell's
        // path here. Letting it reach `orc_tui::App::cycle_theme` would swap
        // in that crate's own two-theme set behind the map's back and shell
        // out to `current_exe()` — which in the embed is `pi-orchestra`, a
        // binary with no `config` subcommand.
        if key.code == KeyCode::Char('t') {
            cycle_theme(shell, commands);
            return false;
        }
        if matches!(
            key.code,
            KeyCode::Char('V') | KeyCode::Char('h') | KeyCode::Esc
        ) && shell.runs.view == orc_tui::View::Dashboard
        {
            shell.view = ShellView::Home;
            return false;
        }
    }
    if shell.runs.handle_key(key) {
        // The embedded App asked to quit; leave only the shell view.
        shell.view = ShellView::Home;
    }
    false
}

fn raw_home_keys(bytes: &[u8]) -> Vec<KeyEvent> {
    match bytes {
        b"\x1b[A" => return vec![KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)],
        b"\x1b[B" => return vec![KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)],
        b"\x1b[Z" => return vec![KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)],
        b"\x1b[5~" => return vec![KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)],
        b"\x1b[6~" => return vec![KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)],
        _ => {}
    }
    let mut keys = Vec::new();
    if let Ok(text) = std::str::from_utf8(bytes) {
        for character in text.chars() {
            let code = match character {
                '\r' | '\n' => KeyCode::Enter,
                '\t' => KeyCode::Tab,
                '\u{1b}' => KeyCode::Esc,
                '\u{7f}' | '\u{8}' => KeyCode::Backspace,
                character => KeyCode::Char(character),
            };
            keys.push(KeyEvent::new(code, KeyModifiers::NONE));
        }
    }
    keys
}

fn route_raw_mouse(bytes: &[u8], state: &mut StageState) -> Option<Option<Vec<u8>>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let body = text.strip_prefix("\x1b[<")?;
    let suffix = body.chars().last()?;
    if !matches!(suffix, 'M' | 'm') {
        return None;
    }
    let fields = body[..body.len().saturating_sub(1)]
        .split(';')
        .map(str::parse::<u16>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    let [code, terminal_x, terminal_y] = fields.as_slice() else {
        return None;
    };
    let column = terminal_x.saturating_sub(1);
    let row = terminal_y.saturating_sub(1);
    let pane_index = state
        .pane_areas
        .iter()
        .position(|area| area.contains((column, row).into()));
    if *code == 0
        && let Some(index) = pane_index
        && let Some(area) = state.pane_areas.get(index)
        && row == area.y
    {
        state.focus = index;
        state.dragging = Some((
            index,
            column.saturating_sub(area.x),
            row.saturating_sub(area.y),
        ));
        return Some(None);
    }
    if *code == 32
        && let Some((index, offset_x, offset_y)) = state.dragging
        && let Some(pane_id) = state.panes.get(index).map(|pane| pane.id.clone())
        && let Some(area) = state.pane_areas.get(index).copied()
    {
        ensure_layout(state);
        if let Some(rect) = state.layout.iter_mut().find(|rect| rect.pane_id == pane_id) {
            rect.x = column.saturating_sub(offset_x);
            rect.y = row.saturating_sub(offset_y);
            rect.width = area.width;
            rect.height = area.height;
        }
        return Some(None);
    }
    if *code == 3 || suffix == 'm' {
        state.dragging = None;
        return Some(None);
    }
    let area = *state.pane_areas.get(state.focus)?;
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if !inner.contains((column, row).into()) {
        return Some(None);
    }
    let x = column.saturating_sub(inner.x) + 1;
    let y = row.saturating_sub(inner.y) + 1;
    Some(Some(format!("\x1b[<{code};{x};{y}{suffix}").into_bytes()))
}

/// Parse the bounded SGR mouse sequence used for SCORE card dragging.
///
/// The client only consumes complete press/release events; every other byte
/// remains available to the focused STAGE pane through its raw router.
fn score_mouse(bytes: &[u8]) -> Option<(u16, u16, u16, char)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let body = text.strip_prefix("\u{1b}[<")?;
    let suffix = body.chars().last()?;
    if !matches!(suffix, 'M' | 'm') {
        return None;
    }
    let values = body.strip_suffix(suffix)?.split(';').collect::<Vec<_>>();
    if values.len() != 3 {
        return None;
    }
    Some((
        values[0].parse().ok()?,
        values[1].parse().ok()?,
        values[2].parse().ok()?,
        suffix,
    ))
}

fn handle_home_key(
    key: KeyEvent,
    commands: &mut BenchClient,
    shell: &mut ShellState,
) -> Result<bool> {
    let home = &mut shell.home;
    if home.flow.is_none() {
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('n') => {
                home.flow = Some(NewSessionFlow::new(&home.data));
                home.message.clear();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                home.selected = home.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                home.selected = (home.selected + 1).min(home.data.sessions.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(session_id) = home
                    .data
                    .sessions
                    .get(home.selected)
                    .map(|session| session.id.clone())
                {
                    match attach_stage(commands, shell, session_id) {
                        Ok(()) => shell.home.message.clear(),
                        // A refused attach stays on HOME with the reason in
                        // place instead of exiting the client.
                        Err(AppError::Daemon(message)) => {
                            shell.home.message = format!("attach failed: {message}");
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            _ => {}
        }
        return Ok(false);
    }
    let Some(flow) = home.flow.as_mut() else {
        return Ok(false);
    };
    if key.code == KeyCode::Esc {
        match flow.step {
            FlowStep::Brain => home.flow = None,
            FlowStep::Workers => flow.step = FlowStep::Brain,
            FlowStep::Cwd => flow.step = FlowStep::Workers,
        }
        return Ok(false);
    }
    match flow.step {
        FlowStep::Brain => match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                flow.brain_index = flow.brain_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                flow.brain_index =
                    (flow.brain_index + 1).min(flow.brain_choices.len().saturating_sub(1));
            }
            KeyCode::Enter if !flow.brain_choices.is_empty() => flow.step = FlowStep::Workers,
            _ => {}
        },
        FlowStep::Workers => match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                flow.worker_index = flow.worker_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                flow.worker_index =
                    (flow.worker_index + 1).min(flow.worker_choices.len().saturating_sub(1));
            }
            KeyCode::Char(' ') => {
                if let Some(worker) = flow.worker_choices.get(flow.worker_index).cloned() {
                    if let Some(index) = flow
                        .selected_workers
                        .iter()
                        .position(|selected| selected == &worker)
                    {
                        flow.selected_workers.remove(index);
                    } else if flow.selected_workers.len() < home.data.max_parallel_workers {
                        flow.selected_workers.push(worker);
                    }
                }
            }
            KeyCode::Enter => flow.step = FlowStep::Cwd,
            _ => {}
        },
        FlowStep::Cwd => match key.code {
            KeyCode::Backspace => {
                flow.cwd.pop();
            }
            KeyCode::Tab => {
                if let Some(completed) = complete_cwd(&flow.cwd) {
                    flow.cwd = completed;
                }
            }
            // ctrl-u arrives as a raw NAK byte through the raw stdin path.
            KeyCode::Char('\u{15}') => flow.cwd.clear(),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !character.is_control() =>
            {
                flow.cwd.push(character);
            }
            KeyCode::Enter => {
                let Some(brain) = flow.brain_choices.get(flow.brain_index).cloned() else {
                    home.message = "No brain harness is configured.".to_owned();
                    return Ok(false);
                };
                // Validate before launch so a typo cannot strand a session
                // in the wrong directory (bug B4).
                let cwd = expand_tilde(flow.cwd.trim());
                if !Path::new(&cwd).is_dir() {
                    home.message = format!("not a directory: {cwd}");
                    return Ok(false);
                }
                match commands.create_session(brain, flow.selected_workers.clone(), cwd) {
                    Ok(session_id) => {
                        home.flow = None;
                        home.message.clear();
                        match attach_stage(commands, shell, session_id) {
                            Ok(()) => {}
                            Err(AppError::Daemon(message)) => {
                                shell.home.message = format!("attach failed: {message}");
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    Err(error) => home.message = error.to_string(),
                }
            }
            _ => {}
        },
    }
    Ok(false)
}

fn ensure_layout(state: &mut StageState) {
    if state.layout.len() == state.panes.len() {
        return;
    }
    state.layout = state
        .panes
        .iter()
        .zip(&state.pane_areas)
        .enumerate()
        .map(|(order, (pane, area))| LayoutRect {
            pane_id: pane.id.clone(),
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
            order,
        })
        .collect();
}

fn send_focused(commands: &mut BenchClient, state: &StageState, bytes: Vec<u8>) -> Result<()> {
    if let Some(pane) = state.panes.get(state.focus) {
        commands.input(pane.id.clone(), bytes)?;
    }
    Ok(())
}

fn render_stage(
    frame: &mut Frame<'_>,
    state: &mut StageState,
    motion: Option<usize>,
    baton_state: baton::State,
) {
    let area = frame.area();
    // The trigger rainbow steps one colour per motion tick; `None` (reduced
    // motion) freezes the gradient at phase 0 so it stays colourful but still.
    let phase = motion.unwrap_or(0);
    frame.render_widget(
        Block::new().style(Style::default().bg(state.theme.bg())),
        area,
    );
    state.pane_areas = stage_areas(area, state);
    if area.width >= 100 && state.panes.len() >= 2 && !state.zoomed {
        // The rail spans the gap `stage_areas` leaves between the conductor
        // and the bench, one column clear of the conductor's drop shadow, and
        // centred on the stage's vertical middle.
        let rail = Rect::new(
            area.x
                .saturating_add(2)
                .saturating_add(conductor_width(area)),
            area.y.saturating_add(area.height / 2),
            BATON_RAIL,
            1,
        );
        render_baton(frame, rail, state, baton_state);
    }
    let areas = state.pane_areas.clone();
    if state.zoomed {
        if let (Some(pane), Some(pane_area)) =
            (state.panes.get(state.focus), areas.first().copied())
        {
            render_shadow(frame, pane_area, state.theme);
            render_pane(
                frame,
                pane_area,
                pane,
                PaneChrome {
                    focus: true,
                    confirmed: state.confirmed_panes.contains(&pane.id),
                    phase,
                },
                state.theme,
                state.glyphs,
            );
        }
    } else {
        for (index, (pane, pane_area)) in state.panes.iter().zip(areas).enumerate() {
            render_shadow(frame, pane_area, state.theme);
            render_pane(
                frame,
                pane_area,
                pane,
                PaneChrome {
                    focus: index == state.focus,
                    confirmed: state.confirmed_panes.contains(&pane.id),
                    phase,
                },
                state.theme,
                state.glyphs,
            );
        }
    }
    if state.message.is_empty() {
        let legend = format!(
            "typing goes to the pane — {leader} then: n/p focus · z zoom · s swap · b SCORE · h HOME · ? help · q detach",
            leader = state.leader_label
        );
        render_legend(frame, area, &legend, state.theme);
    } else {
        frame.render_widget(
            Paragraph::new(format!(" {}", state.message)).style(
                Style::default()
                    .fg(state.theme.failed())
                    .bg(state.theme.bg()),
            ),
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        );
    }
}

/// The baton rail's own width: twelve cells plus its `◆`/`●` endpoints and a
/// space either side.
const BATON_RAIL: u16 = baton::CELLS as u16 + 4;

/// Columns reserved between the conductor pane and the bench: the rail, plus
/// the one column the conductor's drop shadow claims. The rail and the layout
/// must agree about this, so both read these constants.
const BATON_GAP: u16 = BATON_RAIL + 1;

/// The conductor pane's width in the wide STAGE layout.
fn conductor_width(area: Rect) -> u16 {
    area.width.saturating_sub(3) * 53 / 100
}

fn stage_areas(area: Rect, state: &StageState) -> Vec<Rect> {
    if state.panes.is_empty() {
        return Vec::new();
    }
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(3),
        area.height.saturating_sub(3),
    );
    if state.zoomed {
        return vec![inner];
    }
    if state.layout.len() == state.panes.len() {
        let mut ordered = state.layout.clone();
        ordered.sort_by_key(|rect| rect.order);
        return ordered
            .into_iter()
            .map(|rect| {
                let x = rect.x.clamp(inner.x, inner.right().saturating_sub(10));
                let y = rect.y.clamp(inner.y, inner.bottom().saturating_sub(5));
                Rect::new(
                    x,
                    y,
                    rect.width.min(inner.right().saturating_sub(x)).max(10),
                    rect.height.min(inner.bottom().saturating_sub(y)).max(5),
                )
            })
            .collect();
    }
    if state.panes.len() == 1 {
        return vec![inner];
    }
    if area.width < 100 {
        let count = state.panes.len() as u16;
        let height = inner.height.saturating_sub(count.saturating_sub(1)) / count.max(1);
        return (0..count)
            .map(|index| {
                Rect::new(
                    inner.x,
                    inner.y + index * (height + 1),
                    inner.width,
                    if index + 1 == count {
                        inner
                            .bottom()
                            .saturating_sub(inner.y + index * (height + 1))
                    } else {
                        height
                    },
                )
            })
            .collect();
    }
    let brain_width = conductor_width(area);
    let worker_x = inner.x + brain_width + BATON_GAP;
    let worker_width = inner.right().saturating_sub(worker_x);
    let workers = state.panes.len().saturating_sub(1) as u16;
    let worker_height = inner.height.saturating_sub(workers.saturating_sub(1)) / workers.max(1);
    let mut areas = vec![Rect::new(
        inner.x,
        inner.y + inner.height / 10,
        brain_width,
        inner.height * 8 / 10,
    )];
    for index in 0..workers {
        let arc = if workers > 2 && (index == 0 || index + 1 == workers) {
            2
        } else {
            0
        };
        areas.push(Rect::new(
            worker_x + arc,
            inner.y + index * (worker_height + 1),
            worker_width.saturating_sub(arc),
            worker_height,
        ));
    }
    areas
}

/// The drop shadow that lifts a pane off the stage.
///
/// The palette's darkest slot *is* the stage, so there is no colour darker
/// than the backdrop to cast with. The shadow is drawn in the dim frame slot
/// instead, which reads as a soft edge and keeps the elevation cue without
/// inventing a hex outside the map.
fn render_shadow(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let style = Style::default()
        .fg(theme.border())
        .bg(theme.bg())
        .add_modifier(Modifier::DIM);
    let buffer = frame.buffer_mut();
    let right = area.right();
    for row in area.y.saturating_add(1)..area.bottom().saturating_add(1) {
        if let Some(cell) = buffer.cell_mut((right, row)) {
            cell.set_symbol("▐");
            cell.set_style(style);
        }
    }
    let bottom = area.bottom();
    for col in area.x.saturating_add(1)..area.right() {
        if let Some(cell) = buffer.cell_mut((col, bottom)) {
            cell.set_symbol("▄");
            cell.set_style(style);
        }
    }
}

/// Draw the baton: `◆` conductor, twelve rail cells, `●` bench.
///
/// One row, one direction, exactly the cells [`baton::cells`] returns — the
/// whole frame is decided by the passed state, so a snapshot pins a frame
/// instead of racing the clock.
fn render_baton(frame: &mut Frame<'_>, area: Rect, state: &StageState, baton_state: baton::State) {
    if area.width < BATON_RAIL || area.height == 0 {
        return;
    }
    let theme = state.theme;
    let glyphs = state.glyphs;
    let conductor = glyphs.get(Glyph::Conductor);
    let bench = glyphs.get(Glyph::WorkerSeated);
    // The ASCII column's endpoints are three cells wide, which will not fit
    // beside twelve rail cells in the gap the layout reserves. The rail is
    // what carries the meaning, so the endpoints are what gets dropped.
    let endpoints =
        baton::CELLS + conductor.chars().count() + bench.chars().count() + 2 <= area.width.into();
    let mut spans = Vec::with_capacity(baton::CELLS + 4);
    if endpoints {
        spans.push(Span::styled(conductor.to_owned(), theme.state(Slot::Brain)));
        spans.push(Span::raw(" "));
    }
    for cell in baton::cells(baton_state) {
        spans.push(Span::styled(
            cell.symbol(glyphs).to_owned(),
            theme.state(cell.slot()),
        ));
    }
    if endpoints {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(bench.to_owned(), theme.state(Slot::Worker)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.bg())),
        area,
    );
}

/// A trigger token located on the pane grid, in terminal columns.
struct TriggerSpan {
    row: u16,
    col: u16,
    len: u16,
}

/// Scan one grid row of a hosted pane for **every** trigger token, mapping the
/// grammar's character offsets back onto terminal columns.
fn scan_pane_row(pane: &PaneSnapshot, row: u16) -> Vec<(TriggerSpan, Trigger)> {
    let cols = pane.cols;
    let mut line = String::new();
    // char index -> source column, so a token can be located even when earlier
    // cells hold wide graphemes or blanks.
    let mut columns: Vec<u16> = Vec::new();
    for col in 0..cols {
        let index = usize::from(row) * usize::from(cols) + usize::from(col);
        let text = pane.cells.get(index).map_or("", |cell| cell.text.as_str());
        if text.is_empty() {
            line.push(' ');
            columns.push(col);
        } else {
            for ch in text.chars() {
                line.push(ch);
                columns.push(col);
            }
        }
    }
    scan_line(&line)
        .into_iter()
        .filter_map(|matched| {
            let start_col = *columns.get(matched.char_start)?;
            let end_col = *columns.get(matched.char_start + matched.char_len - 1)?;
            Some((
                TriggerSpan {
                    row,
                    col: start_col,
                    len: end_col - start_col + 1,
                },
                matched.trigger,
            ))
        })
        .collect()
}

/// Detect every trigger token in a conductor pane's current screen.
///
/// Detection is scoped to the conductor (`brain`) pane: the trigger grammar is
/// the conductor asserting intent, and workers must never light up because they
/// happened to echo the word. Returns the spans to highlight and the distinct
/// triggers present, in first-seen order, for the title badge.
fn conductor_triggers(pane: &PaneSnapshot) -> (Vec<TriggerSpan>, Vec<Trigger>) {
    if pane.role.as_deref() != Some("brain") {
        return (Vec::new(), Vec::new());
    }
    let mut spans = Vec::new();
    let mut seen: Vec<Trigger> = Vec::new();
    for row in 0..pane.rows {
        for (span, trigger) in scan_pane_row(pane, row) {
            if !seen.contains(&trigger) {
                seen.push(trigger);
            }
            spans.push(span);
        }
    }
    (spans, seen)
}

/// The marker that opens a trigger badge in a pane title.
///
/// The badge glyph is `◆`, which is also the conductor's anchor at the start
/// of every brain pane's title — so "a badge is present" is this whole marker,
/// never the bare glyph. Tests assert on this so the two can never be confused.
fn trigger_badge_mark() -> String {
    format!("· {} ", Trigger::GLYPH)
}

/// The `· ◆ DELEGATE` badge naming every spell detected in a conductor pane,
/// or the empty string when there is none. A glyph and a label, so the trigger
/// is legible with no colour at all.
fn trigger_badge(triggers: &[Trigger]) -> String {
    if triggers.is_empty() {
        return String::new();
    }
    let labels = triggers
        .iter()
        .map(|trigger| trigger.label())
        .collect::<Vec<_>>()
        .join(" ");
    format!(" {}{labels}", trigger_badge_mark())
}

/// The glyph and slot a pane's role and reported state earn in its title.
///
/// The conductor is `◆` and a worker is `●` whatever else is true; a pane the
/// daemon reports as down or detached takes the state glyph instead, because
/// that is the more urgent fact about it.
fn pane_state(pane: &PaneSnapshot) -> (Glyph, Slot) {
    match pane.state.as_deref() {
        Some("conductor_down") => (Glyph::ConductorDown, Slot::Failed),
        Some("dead") => (Glyph::Failed, Slot::Failed),
        Some("detached") => (Glyph::Detached, Slot::Muted),
        _ if pane.role.as_deref() == Some("brain") => (Glyph::Conductor, Slot::Brain),
        _ => (Glyph::WorkerSeated, Slot::Worker),
    }
}

/// How one pane is drawn this frame: whether it holds focus, whether its
/// task's delivery is confirmed, and where the trigger rainbow's gradient has
/// slid to.
#[derive(Clone, Copy)]
struct PaneChrome {
    focus: bool,
    confirmed: bool,
    phase: usize,
}

fn render_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    pane: &PaneSnapshot,
    chrome: PaneChrome,
    theme: Theme,
    glyphs: Glyphs,
) {
    let PaneChrome {
        focus,
        confirmed,
        phase,
    } = chrome;
    let (trigger_spans, triggers) = conductor_triggers(pane);
    let badge = trigger_badge(&triggers);
    let (state_glyph, state_slot) = pane_state(pane);
    let border_color = if focus {
        theme.border_hi()
    } else {
        theme.border()
    };
    let block = Block::default()
        .title(format!(
            " {} {}  {}{}{} ",
            glyphs.get(state_glyph),
            pane.title.to_uppercase(),
            pane.state.as_deref().unwrap_or("LIVE"),
            if confirmed {
                format!(" · {} TASK CONFIRMED", glyphs.get(Glyph::Confirmed))
            } else {
                String::new()
            },
            badge,
        ))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(border_color))
        .title_style(if confirmed {
            theme.state(Slot::Confirmed)
        } else if focus {
            theme.state(state_slot)
        } else {
            Style::default().fg(theme.fg()).add_modifier(Modifier::BOLD)
        })
        .style(Style::default().bg(theme.surface()).fg(theme.fg()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if pane.state.as_deref() == Some("conductor_down") {
        let elapsed = pane
            .down_at
            .map_or(0, |down| epoch_now().saturating_sub(down));
        let overlay = Rect::new(
            inner.x + inner.width.saturating_sub(38) / 2,
            inner.y + inner.height.saturating_sub(3) / 2,
            inner.width.min(38),
            3.min(inner.height),
        );
        // Calm and recoverable, never alarming: an overlay fill with the
        // failure slot as text, not a slab of alarm colour.
        frame.render_widget(
            Paragraph::new(format!(
                "{} CONDUCTOR DOWN\n{elapsed}s elapsed · {} R resume",
                glyphs.get(Glyph::ConductorDown),
                glyphs.get(Glyph::RecoveryHint)
            ))
            .style(
                theme
                    .state(Slot::Failed)
                    .bg(theme.overlay())
                    .add_modifier(Modifier::BOLD),
            ),
            overlay,
        );
    }
    let rows = inner.height.min(pane.rows);
    let cols = inner.width.min(pane.cols);
    let buffer = frame.buffer_mut();
    for row in 0..rows {
        for col in 0..cols {
            let index = usize::from(row) * usize::from(pane.cols) + usize::from(col);
            let Some(source) = pane.cells.get(index) else {
                continue;
            };
            let Some(target) = buffer.cell_mut((inner.x + col, inner.y + row)) else {
                continue;
            };
            let mut style = Style::default()
                .fg(theme.pane_color(source.foreground, Slot::Fg))
                .bg(theme.pane_color(source.background, Slot::Surface));
            if source.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            if source.dim {
                style = style.add_modifier(Modifier::DIM);
            }
            if source.italic {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if source.underline {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            if source.inverse {
                style = style.add_modifier(Modifier::REVERSED);
            }
            // A detected conductor trigger shimmers like `ultrathink`: each
            // column of the token takes the next colour in TRIGGER_RAINBOW, and
            // `phase` slides the gradient one stop per motion tick so it flows.
            // Kept BOLD so the span still reads when colour is stripped (the
            // `◆ LABEL` title badge names it too — never colour alone).
            if let Some(span) = trigger_spans
                .iter()
                .find(|span| span.row == row && col >= span.col && col < span.col + span.len)
            {
                let offset = usize::from(col - span.col);
                let colour = TRIGGER_RAINBOW[(offset + phase) % TRIGGER_RAINBOW.len()];
                style = style
                    .fg(colour)
                    .add_modifier(Modifier::BOLD)
                    .remove_modifier(Modifier::REVERSED);
            }
            target.set_symbol(if source.text.is_empty() {
                " "
            } else {
                &source.text
            });
            target.set_style(style);
        }
    }
}

fn epoch_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use orc_proto::{
        HarnessSummary, PaneSnapshot, SessionSummary, TaskHistorySummary, TaskSummary, TerminalCell,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;

    use super::{
        AVATAR_FRAMES, HarnessDiscovery, HashMap, HomeData, HomeState, LeaderAction, LeaderKey,
        NewSessionFlow, RawRouter, SINGLE_HARNESS_MESSAGE, ScoreState, ShellState, ShellView,
        SingleHarnessPlan, StageState, Theme, ThemeName, baton, cycle_theme, render_help,
        render_home, render_score, render_shell, render_stage, route_leader, route_raw_mouse,
        route_runs_key, score_mouse,
    };
    use crate::glyph::{Glyph, GlyphTier, Glyphs};
    use crate::theme::{ColorTier, Slot};

    /// Tests render with the Unicode register and a truecolor palette unless
    /// they are specifically exercising a degradation tier, so no snapshot
    /// depends on the machine's locale or `TERM`.
    const GLYPHS: Glyphs = Glyphs::new(GlyphTier::Unicode);
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use orc_pty::cells_from_stream;
    use orc_pty::trigger::Trigger;

    fn ledger_run(id: &str, status: &str, session: Option<&str>) -> orc_core::model::RunMeta {
        orc_core::model::RunMeta {
            id: id.to_owned(),
            task: "Audit the registry and report evidence".to_owned(),
            brain: "codex".to_owned(),
            cwd: "/tmp".to_owned(),
            provider: "minimax".to_owned(),
            model: "MiniMax-M3".to_owned(),
            pid: None,
            status: status.to_owned(),
            started_at: "2026-07-12T12:00:00+00:00".to_owned(),
            created_ts: 1.0,
            ended_at: None,
            exit_code: None,
            tokens: orc_core::model::Tokens {
                estimated_total: 42_000,
                ..orc_core::model::Tokens::default()
            },
            session: session.map(str::to_owned),
            name: None,
            mode: Some("rpc".to_owned()),
            retry_of: None,
            handoff_from: None,
            attention: None,
            failure_kind: None,
            brain_model: None,
            extra: std::collections::BTreeMap::new(),
            run_dir: None,
        }
    }

    fn final_report() -> orc_core::report::FinalReport {
        orc_core::report::FinalReport {
            version: 1,
            session: "score-session".to_owned(),
            task: "T0001".to_owned(),
            title: "review worktree".to_owned(),
            executor: "pi-m3".to_owned(),
            reviewer: "codex".to_owned(),
            review_mode: "independent".to_owned(),
            verdicts: vec![
                orc_core::report::AcceptanceVerdict {
                    check: "main clean".to_owned(),
                    verdict: "pass".to_owned(),
                    evidence: "git status empty".to_owned(),
                },
                orc_core::report::AcceptanceVerdict {
                    check: "gates green".to_owned(),
                    verdict: "pass".to_owned(),
                    evidence: "cargo test passed".to_owned(),
                },
            ],
            usage: orc_core::report::ReportUsage {
                total: Some(42),
                cost_usd: Some(0.001),
                ..orc_core::report::ReportUsage::default()
            },
            receipts: vec!["dispatch:D-review".to_owned()],
            created_at: "2026-07-28T12:00:00+00:00".to_owned(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn runs_shell(theme_name: ThemeName) -> ShellState {
        // The embed borrows this crate's map, exactly as `run_initial` wires
        // it, so a fixture can never assert against a palette the real client
        // would not render.
        let theme = Theme::from(theme_name).runs_theme();
        ShellState {
            view: ShellView::Runs,
            home: HomeState {
                data: HomeData {
                    sessions: Vec::new(),
                    harnesses: Vec::new(),
                    discovered: Vec::new(),
                    default_workers: Vec::new(),
                    max_parallel_workers: 3,
                    single_harness: None,
                    theme: "ember".to_owned(),
                    reduced_motion: false,
                    leader_key: "ctrl-g".to_owned(),
                },
                selected: 0,
                flow: None,
                message: String::new(),
            },
            stage: None,
            score: None,
            theme: theme_name.into(),
            glyphs: GLYPHS,
            runs: orc_tui::App::with_runs(
                vec![
                    ledger_run("worker-live", "running", Some("bench-session")),
                    ledger_run("worker-done", "done", Some("bench-session")),
                    ledger_run("worker-solo", "done", None),
                ],
                theme,
            ),
            reports: Vec::new(),
            help: false,
            reduced_motion: false,
            epoch: std::time::Instant::now(),
            leader: LeaderKey::parse("ctrl-g"),
            leader_pending: false,
            watch_session: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn embedded_runs_view_renders_the_control_plane_with_an_honest_legend() {
        for (width, height) in [(150, 44), (72, 30)] {
            for theme_name in [ThemeName::Ember, ThemeName::Phosphor] {
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).expect("test RUNS terminal");
                let mut shell = runs_shell(theme_name);
                terminal
                    .draw(|frame| render_shell(frame, &mut shell))
                    .expect("render embedded RUNS");
                let text = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("CONTROL PLANE"), "{width}x{height}");
                assert!(text.contains("bench-session"), "{width}x{height}");
                // The legend advertises only interactions that route.
                assert!(text.contains("j/k select"), "{width}x{height}");
                assert!(text.contains("V/h HOME"), "{width}x{height}");
                assert!(text.contains("q quit"), "{width}x{height}");
                assert!(!text.contains("read-only"), "{width}x{height}");
            }
        }
    }

    #[test]
    fn runs_surfaces_the_latest_final_report() {
        let backend = TestBackend::new(150, 44);
        let mut terminal = Terminal::new(backend).expect("test RUNS report terminal");
        let mut shell = runs_shell(ThemeName::Ember);
        shell.reports = vec![final_report()];
        terminal
            .draw(|frame| render_shell(frame, &mut shell))
            .expect("render RUNS report");
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("REPORTS ✓ T0001 2/2"));
        assert!(text.contains("codex independent"));
    }

    #[test]
    fn embedded_runs_keys_route_into_the_app_and_documented_exits_stay_reserved() {
        let mut shell = runs_shell(ThemeName::Ember);
        assert!(shell.runs.rows.len() > 1, "fixture must have rows");

        // j/k selection routes into the App.
        let before = shell.runs.selected_row;
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('j')), None));
        assert_ne!(shell.runs.selected_row, before, "j must move selection");
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('k')), None));
        assert_eq!(shell.runs.selected_row, before, "k must move back");

        // enter expands the selected session group.
        assert!(shell.runs.expanded.is_empty());
        assert!(!route_runs_key(&mut shell, key(KeyCode::Enter), None));
        assert!(!shell.runs.expanded.is_empty(), "enter must expand");

        // `/` begins search; literal V and Esc belong to the input.
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('/')), None));
        assert_eq!(shell.runs.input_mode, orc_tui::InputMode::Search);
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('V')), None));
        assert_eq!(shell.runs.input, "V", "V must type into the input");
        assert_eq!(shell.view, ShellView::Runs);
        assert!(!route_runs_key(&mut shell, key(KeyCode::Esc), None));
        assert_eq!(shell.runs.input_mode, orc_tui::InputMode::None);
        assert_eq!(shell.view, ShellView::Runs, "Esc must only cancel input");

        // Esc and h at the dashboard are documented exits to HOME.
        assert!(!route_runs_key(&mut shell, key(KeyCode::Esc), None));
        assert_eq!(shell.view, ShellView::Home);
        shell.view = ShellView::Runs;
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('h')), None));
        assert_eq!(shell.view, ShellView::Home);

        // q quits the client from the embed.
        shell.view = ShellView::Runs;
        assert!(route_runs_key(&mut shell, key(KeyCode::Char('q')), None));
    }

    #[test]
    fn embedded_runs_session_view_keeps_esc_for_the_app_and_updates_the_legend() {
        let mut shell = runs_shell(ThemeName::Ember);
        // Expand the session group, select a child run, open it.
        assert!(!route_runs_key(&mut shell, key(KeyCode::Enter), None));
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('j')), None));
        assert!(!route_runs_key(&mut shell, key(KeyCode::Enter), None));
        assert_eq!(shell.runs.view, orc_tui::View::Session);

        // The legend now describes the session workspace, not the dashboard.
        let backend = TestBackend::new(150, 44);
        let mut terminal = Terminal::new(backend).expect("session legend terminal");
        terminal
            .draw(|frame| render_shell(frame, &mut shell))
            .expect("render session embed");
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Esc back"));
        assert!(text.contains("tab tabs"));

        // tab cycles detail tabs inside the App.
        let tab_before = shell.runs.detail_tab;
        assert!(!route_runs_key(&mut shell, key(KeyCode::Tab), None));
        assert_ne!(shell.runs.detail_tab, tab_before);

        // Esc returns to the App dashboard, not to HOME.
        assert!(!route_runs_key(&mut shell, key(KeyCode::Esc), None));
        assert_eq!(shell.runs.view, orc_tui::View::Dashboard);
        assert_eq!(shell.view, ShellView::Runs);
    }

    /// A shell with all four screens live, in one theme, exactly as
    /// `run_initial` and `attach_stage` wire them.
    fn four_screen_shell(theme_name: ThemeName) -> ShellState {
        let mut shell = runs_shell(theme_name);
        let theme = Theme::from(theme_name);
        let mut stage = StageState::new(panes(), theme, GLYPHS);
        stage.session_id = Some("score-session".to_owned());
        shell.stage = Some(stage);
        shell.score = Some(ScoreState {
            session_id: "score-session".to_owned(),
            tasks: vec![TaskSummary {
                id: "T0001".to_owned(),
                title: "a brief".to_owned(),
                status: "backlog".to_owned(),
                assignee: None,
                assignee_run: None,
                isolated: false,
                isolation: None,
                blocked: false,
                tokens: None,
                diff: None,
                history: Vec::new(),
            }],
            reports: HashMap::new(),
            selected: 0,
            message: String::new(),
            dragging: None,
            width: 120,
        });
        shell
    }

    /// Every copy of the live theme, so a switcher that misses one is caught
    /// rather than merely looking right on whichever screen the test drew.
    fn theme_copies(shell: &ShellState) -> (ThemeName, orc_tui::Theme, Option<ThemeName>) {
        (
            shell.theme.name(),
            shell.runs.theme,
            shell.stage.as_ref().map(|stage| stage.theme.name()),
        )
    }

    #[test]
    fn leader_t_cycles_every_screen_together_from_every_screen() {
        for view in [
            ShellView::Home,
            ShellView::Stage,
            ShellView::Score,
            ShellView::Runs,
        ] {
            let mut shell = four_screen_shell(ThemeName::Nocturne);
            shell.view = view;
            // Nocturne, ember, phosphor, and back to the flagship.
            for expected in [
                ThemeName::Ember,
                ThemeName::Phosphor,
                ThemeName::Nocturne,
                ThemeName::Ember,
            ] {
                // The chord detector differs by screen — STAGE reads raw
                // bytes so a chord can be re-sent literally — but both must
                // reach the one switcher.
                if view == ShellView::Stage {
                    let stage = shell.stage.as_mut().expect("stage fixture");
                    let (forwarded, actions) = stage.raw_router.route(b"\x07t");
                    assert!(forwarded.is_empty(), "{view:?}: the chord must not leak");
                    assert_eq!(actions, vec![LeaderAction::Theme], "{view:?}");
                    cycle_theme(&mut shell, None);
                } else {
                    assert_eq!(
                        route_leader(&[shell.leader.byte], None, &mut shell),
                        Some(false),
                        "{view:?}: the leader must arm here"
                    );
                    assert!(shell.leader_pending, "{view:?}");
                    assert_eq!(
                        route_leader(b"t", None, &mut shell),
                        Some(false),
                        "{view:?}: t must be consumed as a command"
                    );
                }

                let (shell_theme, runs_theme, stage_theme) = theme_copies(&shell);
                assert_eq!(shell_theme, expected, "{view:?}: HOME/SCORE/help palette");
                assert_eq!(stage_theme, Some(expected), "{view:?}: STAGE holds its own");
                assert_eq!(
                    runs_theme,
                    Theme::from(expected).runs_theme(),
                    "{view:?}: RUNS must borrow the map"
                );
                // Never the ledger's own two-theme set: those colours are in
                // no row of the seventeen-slot map.
                assert_ne!(runs_theme, orc_tui::EMBER, "{view:?}");
                assert_ne!(runs_theme, orc_tui::PHOSPHOR, "{view:?}");
                // The switch is local and always succeeds; nothing shells out.
                for message in [
                    shell.home.message.as_str(),
                    shell.runs.message.as_str(),
                    shell.stage.as_ref().map_or("", |stage| &stage.message),
                    shell.score.as_ref().map_or("", |score| &score.message),
                ] {
                    assert!(
                        !message.contains("unrecognized subcommand"),
                        "{view:?}: {message:?} reached a message line"
                    );
                    assert!(
                        message.is_empty(),
                        "{view:?}: unexpected message {message:?}"
                    );
                }
                // The chord never changes which screen you are on.
                assert_eq!(shell.view, view, "{view:?}: theme must not navigate");
            }
        }
    }

    #[test]
    fn bare_t_in_runs_takes_the_shell_path_not_the_ledgers_own_switcher() {
        let mut shell = four_screen_shell(ThemeName::Nocturne);
        shell.view = ShellView::Runs;
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('t')), None));

        assert_eq!(shell.theme.name(), ThemeName::Ember, "the map advanced");
        assert_eq!(shell.runs.theme, Theme::from(ThemeName::Ember).runs_theme());
        assert_eq!(
            shell.stage.as_ref().map(|stage| stage.theme.name()),
            Some(ThemeName::Ember),
            "STAGE's own copy must move with it"
        );
        // `orc_tui::App::cycle_theme` would have swapped in its own palette
        // and shelled out to `current_exe()`, which in the embed is
        // `pi-orchestra` — a binary with no `config` subcommand.
        assert!(
            shell.runs.message.is_empty(),
            "the ledger's switcher ran: {:?}",
            shell.runs.message
        );
        assert_ne!(shell.runs.theme, orc_tui::EMBER);
        assert_ne!(shell.runs.theme, orc_tui::PHOSPHOR);
    }

    #[test]
    fn the_leader_chord_reaches_home_and_runs_not_only_stage_and_score() {
        // HOME is the launch screen; before #37 it had no chord at all.
        for view in [ShellView::Home, ShellView::Runs] {
            let mut shell = four_screen_shell(ThemeName::Nocturne);
            shell.view = view;

            // An ordinary key is not a chord and belongs to the screen.
            assert_eq!(route_leader(b"j", None, &mut shell), None, "{view:?}");
            assert!(!shell.leader_pending, "{view:?}");

            // The leader arms, and the follow-up key is consumed as a command
            // rather than reaching the screen underneath.
            let chord = [shell.leader.byte];
            assert_eq!(route_leader(&chord, None, &mut shell), Some(false));
            assert_eq!(route_leader(b"?", None, &mut shell), Some(false));
            assert!(shell.help, "{view:?}: leader ? must open help");
            shell.help = false;

            assert_eq!(route_leader(&chord, None, &mut shell), Some(false));
            assert_eq!(route_leader(b"b", None, &mut shell), Some(false));
            assert_eq!(shell.view, ShellView::Score, "{view:?}: leader b to SCORE");

            shell.view = view;
            assert_eq!(route_leader(&chord, None, &mut shell), Some(false));
            assert_eq!(
                route_leader(b"q", None, &mut shell),
                Some(true),
                "{view:?}: leader q must quit"
            );

            // STAGE keeps its own router; this path must not shadow it.
            shell.view = ShellView::Stage;
            assert_eq!(route_leader(&chord, None, &mut shell), None, "{view:?}");
        }
    }

    #[test]
    fn score_keeps_its_documented_chord_after_moving_to_the_shared_table() {
        let mut shell = four_screen_shell(ThemeName::Nocturne);
        shell.view = ShellView::Score;
        let chord = [shell.leader.byte];

        for (key, expected) in [
            (b"h", ShellView::Home),
            (b"v", ShellView::Runs),
            (b"b", ShellView::Score),
        ] {
            shell.view = ShellView::Score;
            assert_eq!(route_leader(&chord, None, &mut shell), Some(false));
            assert_eq!(route_leader(key, None, &mut shell), Some(false));
            assert_eq!(shell.view, expected, "leader {key:?}");
        }

        shell.view = ShellView::Score;
        assert_eq!(route_leader(&chord, None, &mut shell), Some(false));
        assert_eq!(route_leader(b"q", None, &mut shell), Some(true));
    }

    #[test]
    fn a_relaunch_opens_in_the_persisted_theme() {
        // What `run_initial` does with the daemon's `Home` answer: the stored
        // choice wins, the CLI flag is only the fallback, and a name nobody
        // recognises resolves rather than refusing to start.
        for name in ThemeName::ALL {
            assert_eq!(
                super::resolve_initial_theme(name.as_str(), ThemeName::Ember),
                name,
                "{name:?} must survive the relaunch"
            );
        }
        assert_eq!(
            super::resolve_initial_theme("  phosphor ", ThemeName::Ember),
            ThemeName::Phosphor
        );
        assert_eq!(
            super::resolve_initial_theme("", ThemeName::Phosphor),
            ThemeName::Phosphor,
            "an unconfigured daemon falls back to the flag"
        );
        assert_eq!(
            super::resolve_initial_theme("chartreuse", ThemeName::Ember),
            ThemeName::Nocturne,
            "an unknown stored name resolves to the flagship"
        );
    }

    fn panes() -> Vec<PaneSnapshot> {
        ["claude", "hermes"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| {
                let mut cells = vec![TerminalCell::default(); 30 * 90];
                cells[0].text = format!("{title} ready");
                PaneSnapshot {
                    id: format!("pane-{index}"),
                    title: title.to_owned(),
                    rows: 30,
                    cols: 90,
                    cursor: (0, 0),
                    sequence: 1,
                    cells,
                    session_id: None,
                    harness: None,
                    role: None,
                    state: None,
                    down_at: None,
                }
            })
            .collect()
    }

    #[test]
    fn stage_snapshots_cover_both_themes_and_sizes() {
        for (width, height) in [(150, 44), (72, 30)] {
            for theme in [ThemeName::Ember, ThemeName::Phosphor] {
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).expect("test terminal");
                let mut state = StageState::new(panes(), theme.into(), GLYPHS);
                state.confirmed_panes.insert("pane-1".to_owned());
                terminal
                    .draw(|frame| render_stage(frame, &mut state, None, baton::State::Sweeping(1)))
                    .expect("render stage");
                let text = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("CLAUDE"));
                assert!(text.contains("HERMES"));
                assert!(text.contains("TASK CONFIRMED"));
            }
        }
    }

    fn conductor_pane(stream: &[u8]) -> PaneSnapshot {
        let rows = 30;
        let cols = 90;
        PaneSnapshot {
            id: "brain".to_owned(),
            title: "claude".to_owned(),
            rows,
            cols,
            cursor: (0, 0),
            sequence: 1,
            cells: cells_from_stream(rows, cols, stream).expect("parse conductor stream"),
            session_id: None,
            harness: None,
            role: Some("brain".to_owned()),
            state: None,
            down_at: None,
        }
    }

    /// The concatenated symbols of every rainbow-highlighted trigger cell, in
    /// buffer (row-major) order. A highlighted cell is BOLD with a foreground
    /// drawn from `TRIGGER_RAINBOW`; for a single trigger this is exactly the
    /// token, so tests can assert the prompt glyph is never part of the span.
    fn highlighted_symbols(buffer: &ratatui::buffer::Buffer) -> String {
        buffer
            .content()
            .iter()
            .filter(|cell| {
                cell.modifier.contains(Modifier::BOLD) && super::TRIGGER_RAINBOW.contains(&cell.fg)
            })
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    fn rendered_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    // Prompt prefixes a real hosted pane renders before typed input. `\u{276f}`
    // is Claude Code; `\u{279c}` is oh-my-zsh; the bare case keeps back-compat.
    // These are the shapes the earlier bare-only fixtures failed to represent.
    const REAL_PROMPTS: [&str; 6] = ["", "\u{276f} ", "> ", "$ ", "% ", "\u{279c} "];

    #[test]
    fn conductor_trigger_grammar_highlights_each_spell_in_every_theme() {
        // AC1: each trigger streamed through the vt parser produces an accent
        // highlight span in every theme, behind every real prompt prefix, plus
        // a glyph + label badge. The highlighted span is exactly the
        // keyword+colon -- never the prompt glyph.
        for trigger in Trigger::ALL {
            for prompt in REAL_PROMPTS {
                let stream = format!("{prompt}{}: build the thing\r\n", trigger.keyword());
                for theme_name in ThemeName::ALL {
                    let backend = TestBackend::new(120, 40);
                    let mut terminal = Terminal::new(backend).expect("test terminal");
                    let mut state = StageState::new(
                        vec![conductor_pane(stream.as_bytes())],
                        theme_name.into(),
                        GLYPHS,
                    );
                    terminal
                        .draw(|frame| {
                            render_stage(frame, &mut state, None, baton::State::Sweeping(1))
                        })
                        .expect("render stage");
                    let buffer = terminal.backend().buffer();
                    assert_eq!(
                        highlighted_symbols(buffer),
                        format!("{}:", trigger.keyword()),
                        "{trigger:?} prompt={prompt:?} in {theme_name:?}: highlighted span"
                    );
                    let text = rendered_text(buffer);
                    assert!(
                        text.contains(&super::trigger_badge_mark()),
                        "{trigger:?} prompt={prompt:?} in {theme_name:?}: missing glyph badge"
                    );
                    assert!(
                        text.contains(trigger.label()),
                        "{trigger:?} prompt={prompt:?} in {theme_name:?}: missing label badge"
                    );
                }
            }
        }
    }

    #[test]
    fn conductor_highlights_every_occurrence_including_mid_line() {
        // Ultrathink-style: a spell lights up wherever it appears, so a line with
        // the trigger twice highlights both, and a trigger that starts mid-line
        // (after prose) still lights. `highlighted_symbols` concatenates every
        // accent cell in row-major order, so two spans read as two tokens.
        for (stream, expected) in [
            (
                b"delegate: a and delegate: b\r\n".as_slice(),
                "delegate:delegate:",
            ),
            (b"please delegate: this now\r\n".as_slice(), "delegate:"),
        ] {
            for theme_name in ThemeName::ALL {
                let backend = TestBackend::new(120, 40);
                let mut terminal = Terminal::new(backend).expect("test terminal");
                let mut state =
                    StageState::new(vec![conductor_pane(stream)], theme_name.into(), GLYPHS);
                terminal
                    .draw(|frame| render_stage(frame, &mut state, None, baton::State::Sweeping(1)))
                    .expect("render stage");
                let buffer = terminal.backend().buffer();
                assert_eq!(
                    highlighted_symbols(buffer),
                    expected,
                    "{stream:?} in {theme_name:?}: highlighted spans"
                );
                assert!(
                    rendered_text(buffer).contains(&super::trigger_badge_mark()),
                    "{stream:?} in {theme_name:?}: missing glyph badge"
                );
            }
        }
    }

    #[test]
    fn conductor_pane_does_not_highlight_non_triggers() {
        // AC2: `redelegate:` is a different word and a bare `delegate` without a
        // colon is prose; neither may highlight or badge, in any theme -- and
        // that must still hold with a real prompt prefix present.
        for stream in [
            "redelegate: not a trigger\r\n",
            "please delegate this work\r\n",
            "orchestrate the plan carefully\r\n",
            "\u{276f} redelegate: not a trigger\r\n",
            "\u{276f} please delegate this work\r\n",
            "\u{276f} Delegate: capitalized\r\n",
            "> delegated: past tense\r\n",
        ] {
            for theme_name in ThemeName::ALL {
                let backend = TestBackend::new(120, 40);
                let mut terminal = Terminal::new(backend).expect("test terminal");
                let mut state = StageState::new(
                    vec![conductor_pane(stream.as_bytes())],
                    theme_name.into(),
                    GLYPHS,
                );
                terminal
                    .draw(|frame| render_stage(frame, &mut state, None, baton::State::Sweeping(1)))
                    .expect("render stage");
                let buffer = terminal.backend().buffer();
                assert_eq!(
                    highlighted_symbols(buffer),
                    "",
                    "{stream:?} in {theme_name:?}: false-positive highlight"
                );
                assert!(
                    !rendered_text(buffer).contains(&super::trigger_badge_mark()),
                    "{stream:?} in {theme_name:?}: false-positive badge"
                );
            }
        }
    }

    #[test]
    fn worker_pane_never_highlights_a_trigger() {
        // The grammar is the conductor asserting intent; a worker echoing the
        // word must stay quiet even with an exact trigger on screen.
        let mut pane = conductor_pane(b"delegate: worker echo\r\n");
        pane.role = Some("worker".to_owned());
        for theme_name in ThemeName::ALL {
            let backend = TestBackend::new(120, 40);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            let mut state = StageState::new(vec![pane.clone()], theme_name.into(), GLYPHS);
            terminal
                .draw(|frame| render_stage(frame, &mut state, None, baton::State::Sweeping(1)))
                .expect("render stage");
            let buffer = terminal.backend().buffer();
            assert_eq!(
                highlighted_symbols(buffer),
                "",
                "{theme_name:?}: worker pane highlighted a trigger"
            );
        }
    }

    fn stage_shell(
        panes: Vec<PaneSnapshot>,
        theme_name: ThemeName,
        reduced_motion: bool,
    ) -> ShellState {
        let tui_theme = Theme::from(theme_name).runs_theme();
        ShellState {
            view: ShellView::Stage,
            home: HomeState {
                data: HomeData {
                    sessions: Vec::new(),
                    harnesses: Vec::new(),
                    discovered: Vec::new(),
                    default_workers: Vec::new(),
                    max_parallel_workers: 3,
                    single_harness: None,
                    theme: theme_name.as_str().to_owned(),
                    reduced_motion,
                    leader_key: "ctrl-g".to_owned(),
                },
                selected: 0,
                flow: None,
                message: String::new(),
            },
            stage: Some(StageState::new(panes, theme_name.into(), GLYPHS)),
            score: None,
            theme: theme_name.into(),
            glyphs: GLYPHS,
            runs: orc_tui::App::with_runs(Vec::new(), tui_theme),
            reports: Vec::new(),
            help: false,
            reduced_motion,
            epoch: std::time::Instant::now(),
            leader: LeaderKey::parse("ctrl-g"),
            leader_pending: false,
            watch_session: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    #[test]
    fn trigger_highlight_is_reduced_motion_and_color_safe() {
        // AC3: under reduced motion the rainbow is frozen at phase 0, so two
        // renders are byte-identical (no animation), and the affordance still
        // survives colour removal because the token stays BOLD and a glyph +
        // label badge names it. Uses a real Claude Code prompt prefix (U+276F,
        // as UTF-8 bytes) so the fixture matches a live pane, not a bare stream.
        let stream = b"\xe2\x9d\xaf delegate: add OAuth login\r\n";
        for theme_name in ThemeName::ALL {
            let mut frames = Vec::new();
            for _ in 0..2 {
                let backend = TestBackend::new(120, 40);
                let mut terminal = Terminal::new(backend).expect("test terminal");
                let mut shell = stage_shell(vec![conductor_pane(stream)], theme_name, true);
                terminal
                    .draw(|frame| render_shell(frame, &mut shell))
                    .expect("render shell");
                let buffer = terminal.backend().buffer().clone();
                assert_eq!(
                    highlighted_symbols(&buffer),
                    "delegate:",
                    "{theme_name:?} reduced motion: token span"
                );
                let text = rendered_text(&buffer);
                assert!(
                    text.contains(&super::trigger_badge_mark()),
                    "{theme_name:?} reduced motion: glyph badge"
                );
                assert!(
                    text.contains("DELEGATE"),
                    "{theme_name:?} reduced motion: label badge"
                );
                frames.push(buffer);
            }
            assert_eq!(
                frames[0], frames[1],
                "{theme_name:?}: reduced-motion rainbow is not static"
            );
        }
    }

    #[test]
    fn trigger_rainbow_animates_when_motion_is_on() {
        // With motion on, the gradient slides one colour stop per tick, so the
        // same token renders with different colours at different phases -- the
        // ultrathink shimmer -- while motion off (`None`) stays frozen.
        let render = |motion: Option<usize>| {
            let backend = TestBackend::new(120, 40);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            let mut state = StageState::new(
                vec![conductor_pane(b"delegate: go\r\n")],
                ThemeName::Ember.into(),
                GLYPHS,
            );
            terminal
                .draw(|frame| render_stage(frame, &mut state, motion, baton::State::Sweeping(1)))
                .expect("render stage");
            terminal.backend().buffer().clone()
        };
        // The highlighted token's per-cell colours, left to right.
        let token_colours = |buffer: &ratatui::buffer::Buffer| -> Vec<ratatui::style::Color> {
            buffer
                .content()
                .iter()
                .filter(|cell| {
                    cell.modifier.contains(Modifier::BOLD)
                        && super::TRIGGER_RAINBOW.contains(&cell.fg)
                })
                .map(|cell| cell.fg)
                .collect()
        };
        let phase0 = token_colours(&render(Some(0)));
        let phase1 = token_colours(&render(Some(1)));
        assert_eq!(phase0.len(), 9, "delegate: is nine cells"); // keyword + colon
        assert_eq!(phase1.len(), 9);
        assert_ne!(phase0, phase1, "rainbow did not move between phases");
        // A one-stop slide: colour at phase 1 position i == phase 0 position i+1.
        for i in 0..8 {
            assert_eq!(
                phase1[i],
                phase0[i + 1],
                "phase shift is not a one-stop slide at {i}"
            );
        }
        // Motion off is frozen: two `None` renders are identical.
        assert_eq!(render(None), render(None), "reduced-motion rainbow moved");
    }

    #[test]
    fn recorded_claude_code_prompt_stream_lights_up_the_typed_trigger() {
        // Evidence beyond hand-placed cells (the gap that hid the prompt bug):
        // a recorded Claude-Code-shaped byte stream -- ANSI color around the
        // U+276F prompt glyph, then the typed trigger -- run through the REAL
        // vt100 parser and the full render pipeline. This is the exact line the
        // reviewer typed live (`\u{276f} delegate: some web research ...`).
        let stream = b"\x1b[2K\x1b[38;5;213m\xe2\x9d\xaf\x1b[39m \
                       delegate: some web research to the workers\r\n";
        for theme_name in ThemeName::ALL {
            let backend = TestBackend::new(120, 40);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            let mut state =
                StageState::new(vec![conductor_pane(stream)], theme_name.into(), GLYPHS);
            terminal
                .draw(|frame| render_stage(frame, &mut state, None, baton::State::Sweeping(1)))
                .expect("render stage");
            let buffer = terminal.backend().buffer();
            // Only the keyword+colon lights up -- never the colored prompt glyph.
            assert_eq!(
                highlighted_symbols(buffer),
                "delegate:",
                "{theme_name:?}: recorded prompt stream did not light up the trigger"
            );
            let text = rendered_text(buffer);
            assert!(
                text.contains(&super::trigger_badge_mark()),
                "{theme_name:?}: missing glyph badge"
            );
            assert!(
                text.contains("DELEGATE"),
                "{theme_name:?}: missing label badge"
            );
        }
    }

    #[test]
    fn no_color_keeps_every_state_distinguishable_by_glyph_bold_and_reverse() {
        // AC3. `NO_COLOR` is a real probe of a real environment, so this drives
        // the tier the same way a user would, then proves the screens still
        // separate their states with no colour left to spend.
        let mono = super::theme::ColorTier::from_env(|key| match key {
            "NO_COLOR" => Some(String::new()),
            "TERM" => Some("xterm-256color".to_owned()),
            "COLORTERM" => Some("truecolor".to_owned()),
            _ => None,
        });
        assert_eq!(mono, ColorTier::Monochrome, "NO_COLOR must win");
        let theme = Theme::new(ThemeName::Nocturne, mono);
        for slot in Slot::ALL {
            assert_eq!(
                super::theme::describe(theme.slot(slot)),
                "reset",
                "{slot:?} still emits colour under NO_COLOR"
            );
        }

        // SCORE carries the five board states at once.
        let card = |id: &str, status: &str, blocked: bool| TaskSummary {
            id: id.to_owned(),
            title: format!("{status} brief"),
            status: status.to_owned(),
            assignee: Some("pi-m3".to_owned()),
            assignee_run: None,
            isolated: true,
            isolation: None,
            blocked,
            tokens: None,
            diff: None,
            history: Vec::new(),
        };
        let mut score = ScoreState {
            session_id: "mono".to_owned(),
            reports: std::collections::HashMap::new(),
            tasks: vec![
                card("T1", "backlog", false),
                card("T2", "assigned", true),
                card("T3", "running", false),
                card("T4", "review", false),
                card("T5", "done", false),
            ],
            selected: 0,
            message: String::new(),
            dragging: None,
            width: 100,
        };
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("mono SCORE terminal");
        terminal
            .draw(|frame| render_score(frame, &mut score, theme, GLYPHS, "ctrl-g"))
            .expect("render mono SCORE");
        let buffer = terminal.backend().buffer().clone();
        let text = rendered_text(&buffer);
        for glyph in [
            Glyph::Pending,
            Glyph::InProgress,
            Glyph::Confirmed,
            Glyph::Failed,
        ] {
            assert!(
                text.contains(GLYPHS.get(glyph)),
                "SCORE lost {glyph:?} under NO_COLOR"
            );
        }
        // Nothing on screen carries colour, and the selected card is the only
        // thing carrying reverse video.
        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| super::theme::describe(cell.fg) == "reset"
                    && super::theme::describe(cell.bg) == "reset"),
            "a NO_COLOR screen emitted colour"
        );
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.modifier.contains(Modifier::REVERSED)),
            "the selection is invisible under NO_COLOR"
        );
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.modifier.contains(Modifier::BOLD)),
            "emphasis is invisible under NO_COLOR"
        );
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.modifier.contains(Modifier::DIM)),
            "recessive metadata is indistinguishable under NO_COLOR"
        );

        // HOME carries availability and session health.
        let mut data = HomeData {
            sessions: vec![
                SessionSummary {
                    id: "live-one".to_owned(),
                    brain: "codex".to_owned(),
                    workers: vec!["pi-m3".to_owned()],
                    cwd: "/repo".to_owned(),
                    updated_at: "2026-07-29T09:00:00Z".to_owned(),
                    attention: 0,
                    workers_live: 1,
                    workers_total: 1,
                    conductor: "live".to_owned(),
                },
                SessionSummary {
                    id: "down-one".to_owned(),
                    brain: "codex".to_owned(),
                    workers: vec!["pi-m3".to_owned()],
                    cwd: "/repo".to_owned(),
                    updated_at: "2026-07-29T08:00:00Z".to_owned(),
                    attention: 0,
                    workers_live: 0,
                    workers_total: 1,
                    conductor: "down".to_owned(),
                },
            ],
            harnesses: vec![
                HarnessSummary {
                    id: "codex".to_owned(),
                    roles: vec!["brain".to_owned()],
                    resumable: true,
                    available: true,
                    dispatch_verified: true,
                },
                HarnessSummary {
                    id: "gone".to_owned(),
                    roles: vec!["worker".to_owned()],
                    resumable: false,
                    available: false,
                    dispatch_verified: false,
                },
            ],
            discovered: Vec::new(),
            default_workers: Vec::new(),
            max_parallel_workers: 3,
            single_harness: None,
            theme: "nocturne".to_owned(),
            reduced_motion: true,
            leader_key: "ctrl-g".to_owned(),
        };
        data.sessions[1].conductor = "down".to_owned();
        let home = HomeState {
            data,
            selected: 0,
            flow: None,
            message: String::new(),
        };
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("mono HOME terminal");
        terminal
            .draw(|frame| render_home(frame, &home, theme, GLYPHS, None, "ctrl-g"))
            .expect("render mono HOME");
        let text = rendered_text(terminal.backend().buffer());
        // Live vs down, and on-PATH vs not, both separate on the glyph alone.
        assert!(
            text.contains(GLYPHS.get(Glyph::WorkerSeated)),
            "live session"
        );
        assert!(
            text.contains(GLYPHS.get(Glyph::ConductorDown)),
            "down session"
        );
        assert!(text.contains(GLYPHS.get(Glyph::Available)), "on PATH");
        assert!(text.contains(GLYPHS.get(Glyph::Unavailable)), "not on PATH");
        assert_ne!(
            GLYPHS.get(Glyph::WorkerSeated),
            GLYPHS.get(Glyph::ConductorDown),
            "live and down must not share a glyph"
        );
    }

    /// The baton row STAGE actually painted, as text.
    fn baton_row(buffer: &ratatui::buffer::Buffer, width: u16) -> String {
        buffer
            .content()
            .chunks(usize::from(width))
            .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect())
            .find(|row: &String| row.contains('▓') || row.contains('·') || row.contains('━'))
            .unwrap_or_default()
    }

    #[test]
    fn stage_paints_the_spec_baton_between_the_conductor_and_the_bench() {
        // AC4: what STAGE draws is the design sheet's discrete frame, endpoints
        // and all — not an approximation of it.
        let width = 120;
        for (state, want) in [
            (baton::State::Sweeping(0), "◆ ▓▒░───────── ●"),
            (baton::State::Sweeping(3), "◆ ──────▓▒░─── ●"),
            (baton::State::Idle, "◆ ············ ●"),
            (baton::State::Steady, "◆ ━━━━━━━━━━━━ ●"),
        ] {
            let backend = TestBackend::new(width, 40);
            let mut terminal = Terminal::new(backend).expect("baton terminal");
            let mut stage = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
            terminal
                .draw(|frame| render_stage(frame, &mut stage, None, state))
                .expect("render baton");
            assert!(
                baton_row(terminal.backend().buffer(), width).contains(want),
                "{state:?}: STAGE did not paint {want:?}"
            );
        }
    }

    #[test]
    fn reduced_motion_freezes_the_baton_and_full_motion_moves_it() {
        // AC4: reduced motion gets static rails. The rail is a pure function of
        // the state value, so a reduced-motion client can only ever paint the
        // two static forms — never a packet.
        let width = 120;
        let render = |state: baton::State| {
            let backend = TestBackend::new(width, 40);
            let mut terminal = Terminal::new(backend).expect("baton terminal");
            let mut stage = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
            terminal
                .draw(|frame| render_stage(frame, &mut stage, None, state))
                .expect("render baton");
            baton_row(terminal.backend().buffer(), width)
        };
        let mut stage = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        stage.mark_output();
        assert_eq!(
            stage.baton_state(true),
            baton::State::Steady,
            "a live pane under reduced motion gets the solid rail"
        );
        assert_eq!(
            render(stage.baton_state(true)),
            render(baton::State::Steady)
        );
        // Full motion at the same instant is a travelling packet instead.
        assert!(matches!(
            stage.baton_state(false),
            baton::State::Sweeping(_)
        ));
        // Two different sweep frames really do paint differently.
        assert_ne!(
            render(baton::State::Sweeping(0)),
            render(baton::State::Sweeping(3))
        );
    }

    #[test]
    fn output_pulses_the_baton_and_silence_decays_it() {
        let mut stage = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        // A fresh stage has not seen output yet, so it starts live and decays.
        stage.advance();
        stage.mark_output();
        assert!(matches!(
            stage.baton_state(false),
            baton::State::Sweeping(_)
        ));
        // Advancing past the decay window with no further output idles it.
        stage
            .pulse
            .process(baton::DECAY + std::time::Duration::from_millis(1));
        assert_eq!(stage.baton_state(false), baton::State::Idle);
        assert_eq!(stage.baton_state(true), baton::State::Idle);
        // A new snapshot with a fresh sequence is an output tick.
        let mut next = panes();
        next[0].sequence = 2;
        stage.apply_snapshot(next);
        assert!(matches!(
            stage.baton_state(false),
            baton::State::Sweeping(_)
        ));
    }

    #[test]
    fn runs_watcher_wakes_on_registry_change_without_polling() {
        let root = std::env::temp_dir().join(format!("orc-app-runs-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (sender, receiver) = std::sync::mpsc::sync_channel(4);
        super::spawn_runs_watch_path(root.join("runs"), sender);
        let runs = root.join("runs").join("event-run");
        // Watcher registration is asynchronous, so under parallel test load
        // a single early write can land before the watch exists. Rewriting
        // until the event arrives removes the timing assumption; production
        // behavior is unchanged (any registry write wakes the watcher).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let woke = loop {
            std::fs::create_dir_all(&runs).expect("create watched run");
            std::fs::write(runs.join("meta.json"), b"{}\n").expect("write watched meta");
            match receiver.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(super::UiEvent::RunsChanged) => break true,
                Ok(other) => panic!(
                    "unexpected watcher event: {:?}",
                    std::mem::discriminant(&other)
                ),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if std::time::Instant::now() >= deadline {
                        break false;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("watcher thread stopped")
                }
            }
        };
        assert!(woke, "runs watcher never delivered RunsChanged within 10s");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn home_empty_flow_and_shelf_cover_both_themes_and_sizes() {
        for (width, height) in [(150, 44), (72, 30)] {
            for theme_name in [ThemeName::Ember, ThemeName::Phosphor] {
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).expect("test HOME terminal");
                let mut state = HomeState {
                    data: HomeData {
                        sessions: Vec::new(),
                        harnesses: vec![
                            HarnessSummary {
                                id: "codex".to_owned(),
                                roles: vec!["brain".to_owned()],
                                resumable: true,
                                available: true,
                                dispatch_verified: false,
                            },
                            HarnessSummary {
                                id: "hermes".to_owned(),
                                roles: vec!["worker".to_owned()],
                                resumable: false,
                                available: true,
                                dispatch_verified: true,
                            },
                            HarnessSummary {
                                id: "pi-m3".to_owned(),
                                roles: vec!["worker".to_owned()],
                                resumable: false,
                                available: false,
                                dispatch_verified: false,
                            },
                        ],
                        discovered: vec![
                            HarnessDiscovery {
                                name: "codex".to_owned(),
                                available: true,
                                path: Some("/usr/local/bin/codex".to_owned()),
                                version: Some("codex 1.2.3".to_owned()),
                                first_seen: Some("2026-07-01T00:00:00+00:00".to_owned()),
                                last_seen: Some("2026-07-23T00:00:00+00:00".to_owned()),
                            },
                            HarnessDiscovery {
                                name: "opencode".to_owned(),
                                available: false,
                                path: None,
                                version: None,
                                first_seen: None,
                                last_seen: None,
                            },
                        ],
                        default_workers: vec!["hermes".to_owned(), "pi-m3".to_owned()],
                        max_parallel_workers: 3,
                        single_harness: None,
                        theme: "ember".to_owned(),
                        reduced_motion: false,
                        leader_key: "ctrl-b".to_owned(),
                    },
                    selected: 0,
                    flow: None,
                    message: String::new(),
                };
                terminal
                    .draw(|frame| {
                        render_home(
                            frame,
                            &state,
                            Theme::from(theme_name),
                            GLYPHS,
                            Some(5),
                            "ctrl-b",
                        )
                    })
                    .expect("render empty HOME");
                let text = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("PI ORCHESTRA"));
                assert!(text.contains(AVATAR_FRAMES[5]));
                assert!(text.contains("WELCOME TO THE BENCH"));
                // Teaching: brain, workers, durability, and the first keys.
                assert!(text.contains("BRAIN plans and delegates"));
                assert!(text.contains("FIRST KEYS"));
                assert!(text.contains("new session"));
                // The configured leader chord, never a hardcoded ctrl-g.
                assert!(text.contains("ctrl-b q detaches"));
                assert!(!text.contains("ctrl-g"));
                // The availability strip states PATH and verified dispatch.
                assert!(text.contains("BENCH AVAILABILITY"));
                assert!(text.contains("dispatch verified"));
                assert!(text.contains("interactive pane only"));
                assert!(text.contains("NOT ON PATH"));
                // The DISCOVERED strip reflects the auto-discovery registry.
                // Guarded to the tall layout where the whole strip is on-screen.
                if height >= 44 {
                    assert!(text.contains("DISCOVERED ON PATH"));
                    assert!(text.contains("codex 1.2.3"));
                }
                terminal
                    .draw(|frame| {
                        render_home(
                            frame,
                            &state,
                            Theme::from(theme_name),
                            GLYPHS,
                            None,
                            "ctrl-b",
                        )
                    })
                    .expect("render reduced-motion HOME");
                let text = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains(GLYPHS.get(Glyph::Pulse)));
                assert!(text.contains(GLYPHS.get(Glyph::Conductor)));
                assert!(text.contains("PI ORCHESTRA"));
                state.data.sessions.push(SessionSummary {
                    id: "session-one".to_owned(),
                    brain: "codex".to_owned(),
                    workers: vec!["hermes".to_owned(), "pi-m3".to_owned()],
                    cwd: "/tmp".to_owned(),
                    updated_at: "2026-07-11T00:00:00Z".to_owned(),
                    attention: 1,
                    workers_live: 1,
                    workers_total: 2,
                    conductor: "down".to_owned(),
                });
                state.data.sessions.push(SessionSummary {
                    id: "session-two".to_owned(),
                    brain: "codex".to_owned(),
                    workers: vec!["hermes".to_owned()],
                    cwd: "/tmp".to_owned(),
                    updated_at: "2026-07-11T00:00:00Z".to_owned(),
                    attention: 0,
                    workers_live: 0,
                    workers_total: 1,
                    conductor: "dead".to_owned(),
                });
                terminal
                    .draw(|frame| {
                        render_home(
                            frame,
                            &state,
                            Theme::from(theme_name),
                            GLYPHS,
                            Some(0),
                            "ctrl-b",
                        )
                    })
                    .expect("render HOME shelf");
                let text = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("session-one"));
                // Pane health on the shelf: a dead session is no surprise.
                assert!(text.contains("1/2 workers"));
                assert!(text.contains("CONDUCTOR DOWN"));
                assert!(text.contains("R recovers"));
                assert!(text.contains("ALL PANES DEAD"));
            }
        }
    }

    #[test]
    fn single_harness_launch_message_snapshot_is_exact_and_sequential() {
        for (width, height) in [(150, 44), (72, 30)] {
            for theme_name in ThemeName::ALL {
                let data = HomeData {
                    sessions: Vec::new(),
                    harnesses: vec![HarnessSummary {
                        id: "solo".to_owned(),
                        roles: vec!["brain".to_owned(), "worker".to_owned()],
                        resumable: true,
                        available: true,
                        dispatch_verified: true,
                    }],
                    discovered: Vec::new(),
                    default_workers: vec!["solo".to_owned()],
                    max_parallel_workers: 3,
                    single_harness: Some(SingleHarnessPlan {
                        adapter: "solo".to_owned(),
                        brain_profiles: vec!["solo".to_owned()],
                        worker_profiles: vec!["solo".to_owned()],
                        models: Vec::new(),
                        accounts: Vec::new(),
                    }),
                    theme: theme_name.as_str().to_owned(),
                    reduced_motion: false,
                    leader_key: "ctrl-g".to_owned(),
                };
                let state = HomeState {
                    flow: Some(NewSessionFlow::new(&data)),
                    data,
                    selected: 0,
                    message: String::new(),
                };
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).expect("single-harness HOME terminal");
                terminal
                    .draw(|frame| {
                        render_home(
                            frame,
                            &state,
                            Theme::from(theme_name),
                            GLYPHS,
                            None,
                            "ctrl-g",
                        )
                    })
                    .expect("render single-harness launch");
                let rows = terminal
                    .backend()
                    .buffer()
                    .content()
                    .chunks(usize::from(width))
                    .map(|row| {
                        row.iter()
                            .map(|cell| cell.symbol())
                            .collect::<String>()
                            .trim()
                            .to_owned()
                    })
                    .collect::<Vec<_>>();
                let start = rows
                    .iter()
                    .position(|line| line.starts_with("One capable harness detected."))
                    .expect("exact notice start");
                let end = rows[start..]
                    .iter()
                    .position(|line| line.ends_with("self-review."))
                    .map(|offset| start + offset)
                    .expect("exact notice end");
                assert_eq!(
                    rows[start..=end].join(" "),
                    SINGLE_HARNESS_MESSAGE,
                    "{theme_name:?} {width}x{height}"
                );
                let screen = rows.join("\n");
                assert!(screen.contains("one harness · sequential roles"));
                assert!(screen.contains("worker profile"));
                assert!(!screen.contains("a bench of workers"));
                assert!(!screen.contains("independent review"));
            }
        }
    }

    #[test]
    fn availability_lines_render_discovered_section() {
        let harnesses = vec![HarnessSummary {
            id: "hermes".to_owned(),
            roles: vec!["worker".to_owned()],
            resumable: false,
            available: true,
            dispatch_verified: true,
        }];
        let discovered = vec![
            HarnessDiscovery {
                name: "pi".to_owned(),
                available: true,
                path: Some("/usr/local/bin/pi".to_owned()),
                version: Some("pi 0.9.1".to_owned()),
                first_seen: Some("2026-07-01T00:00:00+00:00".to_owned()),
                last_seen: Some("2026-07-23T00:00:00+00:00".to_owned()),
            },
            HarnessDiscovery {
                name: "opencode".to_owned(),
                available: false,
                path: None,
                version: None,
                first_seen: None,
                last_seen: None,
            },
        ];
        let text = super::availability_lines(
            &harnesses,
            &discovered,
            Theme::from(ThemeName::Ember),
            GLYPHS,
        )
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
        // The configured availability strip is still rendered first.
        assert!(text.contains("BENCH AVAILABILITY"));
        // The discovered section reflects the registry and hides nothing.
        assert!(text.contains("DISCOVERED ON PATH"));
        assert!(text.contains("on PATH · pi 0.9.1"));
        assert!(text.contains("opencode"));
        assert!(text.contains("NOT ON PATH · unavailable"));
    }

    #[test]
    fn cwd_step_shows_choices_and_validates_the_path_before_launch() {
        let data = HomeData {
            sessions: Vec::new(),
            harnesses: vec![
                HarnessSummary {
                    id: "claude".to_owned(),
                    roles: vec!["brain".to_owned()],
                    resumable: true,
                    available: true,
                    dispatch_verified: false,
                },
                HarnessSummary {
                    id: "hermes".to_owned(),
                    roles: vec!["worker".to_owned()],
                    resumable: false,
                    available: true,
                    dispatch_verified: true,
                },
            ],
            discovered: Vec::new(),
            default_workers: vec!["hermes".to_owned()],
            max_parallel_workers: 3,
            single_harness: None,
            theme: "ember".to_owned(),
            reduced_motion: false,
            leader_key: "ctrl-g".to_owned(),
        };
        let mut flow = super::NewSessionFlow::new(&data);
        flow.step = super::FlowStep::Cwd;
        flow.cwd = "/definitely/not/a/directory".to_owned();
        let state = HomeState {
            data,
            selected: 0,
            flow: Some(flow),
            message: String::new(),
        };
        for (width, height) in [(150, 44), (72, 30)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("cwd step terminal");
            terminal
                .draw(|frame| {
                    render_home(
                        frame,
                        &state,
                        Theme::from(ThemeName::Ember),
                        GLYPHS,
                        None,
                        "ctrl-g",
                    )
                })
                .expect("render cwd step");
            let text = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                text.contains("CHOOSE WORKING DIRECTORY"),
                "{width}x{height}"
            );
            // The confirmation line names the chosen brain and workers.
            assert!(text.contains("brain claude"), "{width}x{height}");
            assert!(text.contains("workers hermes"), "{width}x{height}");
            // The bad path is flagged before launch.
            assert!(text.contains("NOT A DIRECTORY"), "{width}x{height}");
            assert!(text.contains("tab complete"), "{width}x{height}");
            assert!(text.contains("ctrl-u clear"), "{width}x{height}");
        }
    }

    #[test]
    fn cwd_flow_defaults_to_the_client_directory_not_home() {
        let data = HomeData {
            sessions: Vec::new(),
            harnesses: Vec::new(),
            discovered: Vec::new(),
            default_workers: Vec::new(),
            max_parallel_workers: 3,
            single_harness: None,
            theme: "ember".to_owned(),
            reduced_motion: false,
            leader_key: "ctrl-g".to_owned(),
        };
        let flow = super::NewSessionFlow::new(&data);
        let current = std::env::current_dir()
            .expect("test cwd")
            .to_string_lossy()
            .into_owned();
        assert_eq!(flow.cwd, current);
    }

    #[test]
    fn tilde_expansion_and_directory_completion_work_on_real_paths() {
        let home = std::env::var("HOME").expect("HOME for tilde test");
        assert_eq!(super::expand_tilde("~"), home);
        assert_eq!(super::expand_tilde("~/x"), format!("{home}/x"));
        assert_eq!(super::expand_tilde("/plain"), "/plain");

        let root = std::env::temp_dir().join(format!("orc-app-complete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("projects")).expect("create projects");
        std::fs::create_dir_all(root.join("prototypes")).expect("create prototypes");
        std::fs::create_dir_all(root.join("unique")).expect("create unique");
        std::fs::write(root.join("profile.txt"), b"file").expect("create decoy file");
        let base = root.to_string_lossy();

        // A unique prefix completes fully with a trailing slash.
        assert_eq!(
            super::complete_cwd(&format!("{base}/u")),
            Some(format!("{base}/unique/"))
        );
        // An ambiguous prefix extends to the longest common prefix, and a
        // plain file never completes even though it shares the prefix.
        assert_eq!(
            super::complete_cwd(&format!("{base}/p")),
            Some(format!("{base}/pro"))
        );
        // No match completes nothing.
        assert_eq!(super::complete_cwd(&format!("{base}/zzz")), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn score_columns_keep_a_right_gutter_and_ellipsize_long_titles() {
        assert_eq!(super::clip_ellipsis("short", 10), "short");
        assert_eq!(super::clip_ellipsis("exactly-ten", 11), "exactly-ten");
        assert_eq!(super::clip_ellipsis("a very long title", 8), "a very …");
        assert_eq!(super::clip_ellipsis("anything", 0), "");

        let backend = TestBackend::new(72, 30);
        let mut terminal = Terminal::new(backend).expect("score gutter terminal");
        let mut state = ScoreState {
            session_id: "gutter-session".to_owned(),
            reports: std::collections::HashMap::new(),
            tasks: vec![TaskSummary {
                id: "T0001".to_owned(),
                title: "a title long enough to reach past its narrow column".to_owned(),
                status: "done".to_owned(),
                assignee: Some("hermes".to_owned()),
                assignee_run: None,
                isolated: false,
                isolation: None,
                blocked: false,
                tokens: None,
                diff: None,
                history: Vec::new(),
            }],
            selected: 0,
            message: String::new(),
            dragging: None,
            width: 1,
        };
        terminal
            .draw(|frame| {
                render_score(
                    frame,
                    &mut state,
                    Theme::from(ThemeName::Ember),
                    GLYPHS,
                    "ctrl-g",
                )
            })
            .expect("render score gutter");
        let buffer = terminal.backend().buffer();
        // The DONE column occupies the right fifth; its rows must end in a
        // blank gutter cell, with the ellipsis marking the truncation.
        let mut saw_ellipsis = false;
        for row in 0..29 {
            let last = buffer.cell((71, row)).expect("last cell").symbol();
            assert_eq!(last, " ", "row {row} must keep the right gutter clear");
            let line = (0..72)
                .map(|col| buffer.cell((col, row)).expect("cell").symbol())
                .collect::<String>();
            saw_ellipsis |= line.contains('…');
        }
        assert!(saw_ellipsis, "long titles must show an ellipsis");
    }

    #[test]
    fn help_snapshots_cover_first_use_recovery_and_required_sizes() {
        for (width, height) in [(150, 44), (72, 30)] {
            for theme_name in [ThemeName::Ember, ThemeName::Phosphor] {
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).expect("test help terminal");
                terminal
                    .draw(|frame| render_help(frame, Theme::from(theme_name), "ctrl-g"))
                    .expect("render help");
                let text = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("FIRST USE"));
                assert!(text.contains("ctrl-g"));
                assert!(text.contains("SCORE"));
                assert!(text.contains("UNAVAILABLE"));
                assert!(text.contains("reattach"));
                // The theme is a command now, not a file to hand-edit.
                assert!(text.contains("THEME"), "{width}x{height}");
                assert!(
                    text.contains("t cycles nocturne"),
                    "help must teach the switcher ({width}x{height})"
                );
                assert!(
                    text.contains("pio config set theme"),
                    "help must name the CLI path ({width}x{height})"
                );
            }
        }
    }

    #[test]
    fn raw_router_preserves_kitty_and_bracketed_paste_and_only_ctrl_g_is_leader() {
        let mut router = RawRouter::default();
        let kitty = b"\x1b[97;5u\x1b[57358;1u";
        assert_eq!(router.route(kitty).0, kitty);
        let paste = b"\x1b[200~paste\x07inside\x1b[201~";
        assert_eq!(router.route(&paste[..5]).0, &paste[..5]);
        assert_eq!(router.route(&paste[5..]).0, &paste[5..]);
        assert!(router.route(b"\x07").0.is_empty());
        let (literal, actions) = router.route(b"\x07");
        assert_eq!(literal, vec![0x07]);
        assert!(actions.is_empty());
        assert!(
            router
                .route(b"\x07z")
                .1
                .contains(&super::LeaderAction::Zoom)
        );
    }

    #[test]
    fn raw_mouse_is_forwarded_content_relative() {
        let mut state = StageState::new(panes(), ThemeName::Ember.into(), GLYPHS);
        state.pane_areas = vec![ratatui::layout::Rect::new(10, 5, 40, 20)];
        state.panes.truncate(1);
        let translated = route_raw_mouse(b"\x1b[<0;13;8M", &mut state)
            .expect("parse mouse")
            .expect("forward mouse");
        assert_eq!(translated, b"\x1b[<0;2;2M");
    }

    #[test]
    fn score_snapshots_and_drag_parser_cover_the_two_themes_and_required_sizes() {
        for (width, height) in [(150, 44), (72, 30)] {
            for theme_name in [ThemeName::Ember, ThemeName::Phosphor] {
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).expect("test SCORE terminal");
                let mut state = ScoreState {
                    session_id: "score-session".to_owned(),
                    reports: std::collections::HashMap::from([(
                        "T0001".to_owned(),
                        final_report(),
                    )]),
                    tasks: vec![TaskSummary {
                        id: "T0001".to_owned(),
                        title: "review worktree".to_owned(),
                        status: "review".to_owned(),
                        assignee: Some("pi-m3".to_owned()),
                        assignee_run: Some("pane-worker".to_owned()),
                        isolated: true,
                        isolation: Some("ready".to_owned()),
                        blocked: true,
                        tokens: Some("1.2k".to_owned()),
                        diff: Some("+4 -1 · 1 files".to_owned()),
                        history: vec![TaskHistorySummary {
                            at: "now".to_owned(),
                            actor: "human".to_owned(),
                            action: "moved".to_owned(),
                            to: Some("review".to_owned()),
                        }],
                    }],
                    selected: 0,
                    message: "dependency still open".to_owned(),
                    dragging: None,
                    width: 1,
                };
                terminal
                    .draw(|frame| {
                        render_score(frame, &mut state, Theme::from(theme_name), GLYPHS, "ctrl-g")
                    })
                    .expect("render SCORE");
                let text = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("SCORE"));
                assert!(text.contains("T0001"));
                assert!(text.contains("BLOCKED"));
                assert!(text.contains("+4 -1"));
                assert!(text.contains("2/2"));
                assert!(text.contains("✓ main"));
            }
        }
        assert_eq!(score_mouse(b"\x1b[<0;12;4M"), Some((0, 12, 4, 'M')));
        assert_eq!(score_mouse(b"\x1b[<0;70;9m"), Some((0, 70, 9, 'm')));
        assert_eq!(score_mouse(b"not-mouse"), None);
    }

    /// Bind a scripted one-connection daemon and return its socket path.
    fn scripted_daemon<F>(name: &str, script: F) -> std::path::PathBuf
    where
        F: FnOnce(std::os::unix::net::UnixStream) + Send + 'static,
    {
        let dir = std::env::temp_dir().join(format!("orc-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scripted daemon dir");
        let socket = dir.join("orcd.sock");
        let listener =
            std::os::unix::net::UnixListener::bind(&socket).expect("bind scripted daemon");
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                script(stream);
            }
        });
        socket
    }

    fn read_request_line(stream: &std::os::unix::net::UnixStream) -> String {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone stream"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read request");
        line
    }

    #[test]
    fn connect_refuses_a_daemon_predating_the_build_handshake_with_guidance() {
        let socket = scripted_daemon("legacy-welcome", |mut stream| {
            use std::io::Write;
            let _ = read_request_line(&stream);
            stream
                .write_all(b"{\"type\":\"welcome\",\"version\":1}\n")
                .expect("write legacy welcome");
        });
        let Err(error) = super::BenchClient::connect(&socket) else {
            panic!("legacy daemon must refuse");
        };
        let message = error.to_string();
        assert!(message.contains("predates this client"), "got: {message}");
        assert!(message.contains("orc daemon restart"), "got: {message}");
    }

    #[test]
    fn connect_refuses_a_daemon_on_a_different_build_with_both_builds_named() {
        let socket = scripted_daemon("mismatched-welcome", |mut stream| {
            use std::io::Write;
            let _ = read_request_line(&stream);
            stream
                .write_all(b"{\"type\":\"welcome\",\"version\":1,\"build\":\"0.0.1+deadbeef\"}\n")
                .expect("write mismatched welcome");
        });
        let Err(error) = super::BenchClient::connect(&socket) else {
            panic!("mismatch must refuse");
        };
        let message = error.to_string();
        assert!(message.contains("0.0.1+deadbeef"), "got: {message}");
        assert!(
            message.contains(orc_proto::BUILD_IDENTIFIER),
            "got: {message}"
        );
        assert!(message.contains("orc daemon restart"), "got: {message}");
    }

    #[test]
    fn connect_accepts_a_daemon_on_the_same_build() {
        let socket = scripted_daemon("matching-welcome", |mut stream| {
            use std::io::Write;
            let _ = read_request_line(&stream);
            let welcome = format!(
                "{{\"type\":\"welcome\",\"version\":1,\"build\":\"{}\"}}\n",
                orc_proto::BUILD_IDENTIFIER
            );
            stream
                .write_all(welcome.as_bytes())
                .expect("write matching welcome");
        });
        assert!(super::BenchClient::connect(&socket).is_ok());
    }

    #[test]
    fn closed_connection_and_malformed_and_oversized_responses_get_distinct_messages() {
        let socket = scripted_daemon("closed", |stream| {
            let _ = read_request_line(&stream);
            drop(stream);
        });
        let Err(closed) = super::BenchClient::connect(&socket) else {
            panic!("closed connection must fail");
        };
        let closed = closed.to_string();
        assert!(closed.contains("closed the connection"), "got: {closed}");
        assert!(closed.contains("orc daemon status"), "got: {closed}");

        let socket = scripted_daemon("no-newline", |mut stream| {
            use std::io::Write;
            let _ = read_request_line(&stream);
            stream
                .write_all(b"{\"type\":\"welcome\",\"version\":1}")
                .expect("write truncated welcome");
            drop(stream);
        });
        let Err(malformed) = super::BenchClient::connect(&socket) else {
            panic!("truncated response must fail");
        };
        let malformed = malformed.to_string();
        assert!(
            malformed.contains("without a trailing newline"),
            "got: {malformed}"
        );

        let socket = scripted_daemon("oversized", |mut stream| {
            use std::io::Write;
            let _ = read_request_line(&stream);
            let body = vec![b'x'; (super::MAX_RESPONSE_BYTES + 1) as usize];
            stream.write_all(&body).expect("write oversized body");
            stream.write_all(b"\n").expect("finish oversized body");
        });
        let Err(oversized) = super::BenchClient::connect(&socket) else {
            panic!("oversized response must fail");
        };
        let oversized = oversized.to_string();
        assert!(oversized.contains("32 MiB cap"), "got: {oversized}");
        assert!(oversized.contains("bytes"), "got: {oversized}");
    }

    #[test]
    fn focus_reports_are_consumed_and_other_bytes_survive() {
        assert_eq!(
            super::strip_focus_reports(b"\x1b[Ipath\x1b[O"),
            b"path".to_vec()
        );
        assert_eq!(
            super::strip_focus_reports(b"\x1b[I\x1b[O"),
            Vec::<u8>::new()
        );
        // Arrow keys and plain escapes pass through untouched.
        assert_eq!(
            super::strip_focus_reports(b"\x1b[A\x1b[B\x1b"),
            b"\x1b[A\x1b[B\x1b".to_vec()
        );
    }

    #[test]
    fn leader_key_parses_safe_letters_and_falls_back_otherwise() {
        assert_eq!(LeaderKey::parse("ctrl-g").byte, 0x07);
        let custom = LeaderKey::parse("ctrl-b");
        assert_eq!(custom.byte, 0x02);
        assert_eq!(custom.label, "ctrl-b");
        // Reserved or malformed labels fall back to ctrl-g.
        for label in [
            "ctrl-m", "ctrl-i", "ctrl-c", "ctrl-q", "alt-g", "", "ctrl-gg",
        ] {
            let parsed = LeaderKey::parse(label);
            assert_eq!(parsed.byte, 0x07, "label {label} must fall back");
            assert_eq!(parsed.label, "ctrl-g");
        }
    }

    #[test]
    fn raw_router_honors_a_configured_leader_byte() {
        let mut router = RawRouter {
            leader_byte: 0x02,
            ..RawRouter::default()
        };
        // ctrl-g is no longer the leader and passes through raw.
        assert_eq!(router.route(b"\x07").0, vec![0x07]);
        // ctrl-b arms the leader; z zooms and v cycles views.
        let (forwarded, actions) = router.route(b"\x02z");
        assert!(forwarded.is_empty());
        assert_eq!(actions, vec![LeaderAction::Zoom]);
        let (forwarded, actions) = router.route(b"\x02v");
        assert!(forwarded.is_empty());
        assert_eq!(actions, vec![LeaderAction::Views]);
        // Double ctrl-b forwards the literal chord byte.
        let (forwarded, actions) = router.route(b"\x02\x02");
        assert_eq!(forwarded, vec![0x02]);
        assert!(actions.is_empty());
    }
}
