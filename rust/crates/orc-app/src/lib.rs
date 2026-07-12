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
use orc_proto::{
    ClientRequest, DaemonMetrics, HarnessSummary, LayoutRect, PROTOCOL_VERSION, PaneSequence,
    PaneSnapshot, ServerResponse, SessionSummary, TaskSummary, TerminalColor,
};
use ratatui::Frame;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::{Marker, border};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Points};
use ratatui::widgets::{Block, Borders, Paragraph};
use tachyonfx::{EffectTimer, Interpolation};
use thiserror::Error;

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

/// The two approved Bench themes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeName {
    /// Warm charcoal, bone, brass, and oxblood.
    Ember,
    /// CRT-green monochrome with semantic exceptions.
    Phosphor,
}

impl ThemeName {
    /// Parse a configured theme name.
    #[must_use]
    pub fn named(name: &str) -> Self {
        if name.eq_ignore_ascii_case("phosphor") {
            Self::Phosphor
        } else {
            Self::Ember
        }
    }

    #[must_use]
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ember => "ember",
            Self::Phosphor => "phosphor",
        }
    }
}

#[derive(Clone, Copy)]
struct Theme {
    stage: Color,
    text: Color,
    dim: Color,
    focus: Color,
    pulse: Color,
    shadow: Color,
    attention: Color,
}

impl From<ThemeName> for Theme {
    fn from(value: ThemeName) -> Self {
        match value {
            ThemeName::Ember => Self {
                stage: Color::Rgb(18, 16, 15),
                text: Color::Rgb(225, 215, 194),
                dim: Color::Rgb(91, 80, 65),
                focus: Color::Rgb(209, 158, 77),
                pulse: Color::Rgb(255, 201, 105),
                shadow: Color::Rgb(8, 7, 7),
                attention: Color::Rgb(122, 42, 38),
            },
            ThemeName::Phosphor => Self {
                stage: Color::Rgb(2, 13, 8),
                text: Color::Rgb(151, 255, 190),
                dim: Color::Rgb(38, 99, 61),
                focus: Color::Rgb(111, 255, 160),
                pulse: Color::Rgb(207, 255, 220),
                shadow: Color::Rgb(0, 5, 3),
                attention: Color::Rgb(255, 107, 77),
            },
        }
    }
}

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
    /// Preselected but editable worker choices.
    pub default_workers: Vec<String>,
    /// Configured worker bound.
    pub max_parallel_workers: usize,
    /// Ember or phosphor.
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
                default_workers,
                max_parallel_workers,
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
    pulse: EffectTimer,
    last_tick: Instant,
    theme: Theme,
    session_id: Option<String>,
    layout: Vec<LayoutRect>,
    zoomed: bool,
    dragging: Option<(usize, u16, u16)>,
    raw_router: RawRouter,
    confirmed_panes: std::collections::HashSet<String>,
    baton_kind: BatonKind,
    leader_label: String,
    /// Recoverable command failure shown on the legend line instead of
    /// exiting the client.
    message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BatonKind {
    Settle,
    Dispatch,
    Complete,
    Failed,
}

fn baton_profile(kind: BatonKind) -> (u32, usize, bool) {
    match kind {
        BatonKind::Settle => (700, 1, false),
        BatonKind::Dispatch => (480, 3, false),
        BatonKind::Complete => (760, 2, true),
        BatonKind::Failed => (1050, 1, true),
    }
}

impl StageState {
    fn new(panes: Vec<PaneSnapshot>, theme: ThemeName) -> Self {
        Self {
            panes,
            focus: 0,
            pane_areas: Vec::new(),
            pulse: EffectTimer::from_ms(900, Interpolation::CubicOut),
            last_tick: Instant::now(),
            theme: theme.into(),
            session_id: None,
            layout: Vec::new(),
            zoomed: false,
            dragging: None,
            raw_router: RawRouter::default(),
            confirmed_panes: std::collections::HashSet::new(),
            baton_kind: BatonKind::Settle,
            leader_label: "ctrl-g".to_owned(),
            message: String::new(),
        }
    }

    fn for_session(
        session_id: String,
        panes: Vec<PaneSnapshot>,
        layout: Vec<LayoutRect>,
        theme: ThemeName,
    ) -> Self {
        let mut state = Self::new(panes, theme);
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
            self.pulse.reset();
        }
        self.focus = self.focus.min(self.panes.len().saturating_sub(1));
    }

    fn advance(&mut self) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        let _ = self.pulse.process(elapsed);
    }

    fn set_baton_kind(&mut self, kind: BatonKind) {
        if self.baton_kind != kind {
            self.baton_kind = kind;
            self.pulse = EffectTimer::from_ms(baton_profile(kind).0, Interpolation::CubicOut);
        }
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
    selected: usize,
    message: String,
    dragging: Option<String>,
    width: u16,
    /// A leader press is pending its follow-up key.
    leader: bool,
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
        let brain_choices = home
            .harnesses
            .iter()
            .filter(|harness| harness.roles.iter().any(|role| role == "brain"))
            .map(|harness| harness.id.clone())
            .collect();
        let worker_choices = home
            .harnesses
            .iter()
            .filter(|harness| harness.roles.iter().any(|role| role == "worker"))
            .map(|harness| harness.id.clone())
            .collect();
        Self {
            step: FlowStep::Brain,
            brain_choices,
            brain_index: 0,
            worker_choices,
            selected_workers: home.default_workers.clone(),
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
    runs: orc_tui::App,
    help: bool,
    reduced_motion: bool,
    /// Wall-clock origin for the ambient HOME animation.
    epoch: Instant,
    /// Parsed leader chord shared by STAGE and SCORE.
    leader: LeaderKey,
    /// Session filter shared with the screen-watch thread so snapshots stay
    /// bounded to the attached session.
    watch_session: Arc<Mutex<Option<String>>>,
}

fn render_score(frame: &mut Frame<'_>, score: &mut ScoreState, theme: Theme, leader_label: &str) {
    let area = frame.area();
    score.width = area.width.max(1);
    frame.render_widget(Block::new().style(Style::default().bg(theme.stage)), area);
    let columns = ["backlog", "assigned", "running", "review", "done"];
    let width = (area.width / columns.len() as u16).max(1);
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
        let mut lines = vec![format!(" {}", status.to_ascii_uppercase())];
        for task in score.tasks.iter().filter(|task| task.status == *status) {
            let selected = score
                .tasks
                .get(score.selected)
                .is_some_and(|chosen| chosen.id == task.id);
            lines.push(format!(
                "{} {} {}",
                if selected { "›" } else { " " },
                task.id,
                task.title
            ));
            lines.push(format!(
                "  {} · {}",
                task.assignee.as_deref().unwrap_or("unassigned"),
                if task.isolated { "isolate" } else { "shared" }
            ));
            if let Some(diff) = &task.diff {
                lines.push(format!("  {diff}"));
            }
            if let Some(tokens) = &task.tokens {
                lines.push(format!("  {tokens} tokens"));
            }
            if task.blocked {
                lines.push("  BLOCKED: dependencies".to_owned());
            }
            if selected {
                if let Some(history) = task.history.last() {
                    lines.push(format!("  {} {}", history.actor, history.action));
                }
                if !score.message.is_empty() {
                    lines.push(format!("  ERROR: {}", score.message));
                }
            }
        }
        if lines.len() == 1 {
            lines.push("  no tasks".to_owned());
        }
        frame.render_widget(
            Paragraph::new(lines.join("\n")).style(Style::default().fg(theme.text)),
            column,
        );
    }
    frame.render_widget(
        Paragraph::new(format!(
            " SCORE / {} · j/k select · h/l move · drag column · g stage · V RUNS · {leader} h HOME",
            score.session_id,
            leader = leader_label
        ))
        .style(Style::default().fg(theme.dim)),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
}

/// Sparkle frames for the ambient HOME avatar; the middle frame doubles as
/// the static reduced-motion glyph.
const AVATAR_FRAMES: [&str; 8] = ["·", "✢", "✳", "✻", "✽", "✻", "✳", "✢"];
const AVATAR_STATIC: &str = "✻";
const HOME_TITLE: &str = "PI ORCHESTRA";
const HOME_TAGLINE: &str = "one conductor · a bench of workers · sessions survive detach";

/// Render the animated masthead card and return the row below it.
fn render_home_masthead(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: Theme,
    motion: Option<usize>,
) -> u16 {
    let card_width = area.width.saturating_sub(4).clamp(24, 68);
    let card = Rect::new(
        area.x + 2,
        area.y + 1,
        card_width,
        4.min(area.height.saturating_sub(1)),
    );
    let (avatar, avatar_color) = match motion {
        Some(tick) => (
            AVATAR_FRAMES[tick % AVATAR_FRAMES.len()],
            if tick % 2 == 0 {
                theme.pulse
            } else {
                theme.focus
            },
        ),
        None => (AVATAR_STATIC, theme.focus),
    };
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
                .fg(if lit { theme.pulse } else { theme.focus })
                .add_modifier(Modifier::BOLD),
        ));
    }
    let masthead = Paragraph::new(vec![
        Line::from(title),
        Line::from(Span::styled(
            format!("     {HOME_TAGLINE}"),
            Style::default().fg(theme.dim),
        )),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(theme.dim))
            .style(Style::default().bg(theme.stage)),
    );
    frame.render_widget(masthead, card);
    card.bottom().saturating_add(1)
}

fn render_home(frame: &mut Frame<'_>, state: &HomeState, theme: Theme, motion: Option<usize>) {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::default().bg(theme.stage)), area);
    let body_top = render_home_masthead(frame, area, theme, motion);
    let text = Style::default().fg(theme.text);
    let dim = Style::default().fg(theme.dim);
    let focus = Style::default()
        .fg(theme.focus)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'_>> = Vec::new();
    if let Some(flow) = &state.flow {
        lines.push(Line::styled(
            "  NEW SESSION   1 brain  →  2 worker pool  →  3 cwd",
            dim,
        ));
        lines.push(Line::default());
        match flow.step {
            FlowStep::Brain => {
                lines.push(Line::styled("  STEP 1 / 3   CHOOSE BRAIN", text));
                for (index, brain) in flow.brain_choices.iter().enumerate() {
                    let chosen = index == flow.brain_index;
                    lines.push(Line::styled(
                        format!("  {}  {brain}", if chosen { "BRASS" } else { "     " }),
                        if chosen { focus } else { text },
                    ));
                }
                lines.push(Line::styled(
                    "  ↑/↓ choose · enter continue · esc cancel",
                    dim,
                ));
            }
            FlowStep::Workers => {
                lines.push(Line::styled("  STEP 2 / 3   CHOOSE WORKER POOL", text));
                for (index, worker) in flow.worker_choices.iter().enumerate() {
                    let selected = flow.selected_workers.contains(worker);
                    let chosen = index == flow.worker_index;
                    lines.push(Line::styled(
                        format!(
                            "  {}  [{}] {worker}",
                            if chosen { "BRASS" } else { "     " },
                            if selected { "PRESELECTED" } else { "EDITABLE" }
                        ),
                        if chosen { focus } else { text },
                    ));
                }
                lines.push(Line::styled(
                    "  space edits selection · enter continue",
                    dim,
                ));
            }
            FlowStep::Cwd => {
                lines.push(Line::styled("  STEP 3 / 3   CHOOSE CWD", text));
                lines.push(Line::styled(format!("  > {}", flow.cwd), focus));
                lines.push(Line::styled("  type path · enter launches · esc back", dim));
            }
        }
    } else if state.data.sessions.is_empty() {
        lines.extend([
            Line::styled(
                "  WELCOME TO THE BENCH",
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Line::default(),
            Line::from(vec![
                Span::styled("  Press ", text),
                Span::styled("n", focus),
                Span::styled(" to create a session.", text),
            ]),
            Line::styled(
                "  1 choose a brain · 2 review the worker pool · 3 choose a cwd",
                dim,
            ),
            Line::default(),
            Line::styled(
                "  The brain plans and delegates. Workers execute focused briefs.",
                text,
            ),
            Line::styled(
                "  Hermes + pi-m3 are editable offers; unavailable tools are never selected.",
                dim,
            ),
        ]);
    } else {
        lines.push(Line::styled(
            "  SESSION SHELF",
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ));
        for (index, session) in state.data.sessions.iter().enumerate() {
            let chosen = index == state.selected;
            let marker = if chosen { "BRASS" } else { "     " };
            let attention = if session.attention > 0 {
                format!(" · ATTENTION {}", session.attention)
            } else {
                " · READY".to_owned()
            };
            lines.push(Line::styled(
                format!(
                    "  {marker}  ╭ {} · {} workers{attention}",
                    session.id,
                    session.workers.len()
                ),
                if chosen { focus } else { text },
            ));
            lines.push(Line::styled(
                format!(
                    "         ╰ {}  ·  {}  ·  {}",
                    session.brain, session.cwd, session.updated_at
                ),
                if chosen {
                    Style::default().fg(theme.focus)
                } else {
                    dim
                },
            ));
        }
        lines.push(Line::default());
        lines.push(Line::styled("  enter attach · n new session · V RUNS", dim));
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
        Paragraph::new(lines).style(Style::default().fg(theme.text).bg(theme.stage)),
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
        Paragraph::new(format!(" {text}")).style(Style::default().fg(theme.dim).bg(theme.stage)),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
}

fn render_help(frame: &mut Frame<'_>, theme: Theme, leader: &str) {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::default().bg(theme.stage)), area);
    frame.render_widget(
        Paragraph::new(format!(
            "  PI ORCHESTRA / HELP\n\n  FIRST USE\n  n creates a session: choose a brain, edit worker offers, choose a cwd.\n  The brain plans; available workers receive explicit durable task briefs.\n\n  CONTROL\n  In STAGE everything you type goes to the focused pane. Commands need\n  the leader first: press {leader}, release, then one key.\n  {leader} n/p focus · {leader} z zoom · {leader} s swap · {leader} b SCORE\n  {leader} h HOME · {leader} v views · {leader} ? help · {leader} q detach\n  {leader} twice sends the literal chord to the pane.\n  Outside STAGE, bare V cycles HOME, SCORE, RUNS and ? opens help.\n  Change the leader in ~/.orchestra/harnesses.json (app.leader_key).\n\n  DURABILITY AND RECOVERY\n  Closing the client detaches; pi-orchestra attach replays the session.\n  SCORE is the durable task board. Delivery is shown only after confirmation.\n  Missing executables are UNAVAILABLE. R recovers a supported dead brain.\n  If recovery fails, reattach and inspect SCORE, orc task list, and orc list.\n\n  Esc or ? closes help.",
        ))
        .style(Style::default().fg(theme.text).bg(theme.stage)),
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
            render_home(frame, &shell.home, shell.theme, motion);
        }
        ShellView::Stage => {
            if let Some(stage) = shell.stage.as_mut() {
                render_stage(frame, stage);
            }
        }
        ShellView::Score => {
            if let Some(score) = shell.score.as_mut() {
                render_score(frame, score, shell.theme, &shell.leader.label);
            }
        }
        ShellView::Runs => {
            orc_tui::draw(frame, &mut shell.runs);
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
    let selected_theme = ThemeName::named(&home.theme);
    let reduced_motion = home.reduced_motion;
    let leader = LeaderKey::parse(&home.leader_key);
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
        theme: selected_theme.into(),
        runs: orc_tui::App::new(Some(selected_theme.as_str()))
            .map_err(|error| AppError::Daemon(format!("RUNS ledger unavailable: {error}")))?,
        help: false,
        reduced_motion,
        epoch: Instant::now(),
        leader,
        watch_session: Arc::new(Mutex::new(None)),
    };
    if let Some(session_id) = initial_session {
        attach_stage(&mut commands, &mut shell, session_id, theme)?;
    }
    let (events_tx, events_rx) = mpsc::sync_channel(64);
    spawn_screen_watch(socket, Arc::clone(&shell.watch_session), events_tx.clone());
    spawn_runs_watch(events_tx.clone());

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
    let result = run_shell_loop(&mut terminal, &mut commands, &mut shell, &events_rx, theme);
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
    theme: ThemeName,
) -> Result<()> {
    let session = commands.attach_session(session_id.clone())?;
    let tasks = commands.task_board(session_id.clone())?;
    if let Ok(mut watched) = shell.watch_session.lock() {
        *watched = Some(session_id.clone());
    }
    let mut stage =
        StageState::for_session(session_id.clone(), session.panes, session.layout, theme);
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
        stage.set_baton_kind(BatonKind::Dispatch);
    }
    shell.stage = Some(stage);
    shell.score = Some(ScoreState {
        tasks,
        session_id,
        selected: 0,
        message: String::new(),
        dragging: None,
        width: 1,
        leader: false,
    });
    shell.view = ShellView::Stage;
    Ok(())
}

fn run_shell_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    commands: &mut BenchClient,
    shell: &mut ShellState,
    events: &Receiver<UiEvent>,
    theme: ThemeName,
) -> Result<()> {
    let mut redraw = true;
    let mut requested_sizes = HashMap::new();
    loop {
        if !shell.reduced_motion
            && let Some(stage) = shell.stage.as_mut()
        {
            stage.advance();
        }
        let animating = shell.stage.as_ref().is_some_and(|stage| {
            !shell.reduced_motion && shell.view == ShellView::Stage && !stage.pulse.done()
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
        if redraw || animating || home_ambient || runs_ambient {
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
                        let kind = score
                            .tasks
                            .iter()
                            .filter_map(|task| task.history.last())
                            .find_map(|history| match history.action.as_str() {
                                "delivery_confirmed" => Some(BatonKind::Dispatch),
                                "delivery_failed" => Some(BatonKind::Failed),
                                "done" => Some(BatonKind::Complete),
                                _ => None,
                            })
                            .unwrap_or(BatonKind::Settle);
                        stage.set_baton_kind(kind);
                    }
                }
                let _ = shell.runs.refresh();
                redraw = true;
            }
            Some(UiEvent::Raw(bytes)) => {
                if handle_raw_event(&bytes, commands, shell, theme)? {
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
    theme: ThemeName,
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
                if route_runs_key(shell, key) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ShellView::Home => {
            for key in raw_home_keys(bytes) {
                if handle_home_key(key, commands, shell, theme)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ShellView::Score => {
            let Some(score) = shell.score.as_mut() else {
                return Ok(false);
            };
            if score.leader {
                score.leader = false;
                match bytes {
                    b"h" => shell.view = ShellView::Home,
                    b"v" => shell.view = ShellView::Runs,
                    b"b" => {}
                    b"?" => shell.help = true,
                    b"q" => return Ok(true),
                    _ => {}
                }
                return Ok(false);
            }
            if bytes == [shell.leader.byte] {
                score.leader = true;
                return Ok(false);
            }
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
                match action {
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
            if !forwarded.is_empty() {
                send_focused(commands, stage, forwarded)?;
            }
            Ok(false)
        }
    }
}

/// Route one decoded key into the embedded RUNS control plane.
///
/// Returns true when the whole client should quit. The documented exits act
/// only at the App's top-level dashboard while no text input is active;
/// deeper views and active inputs receive every key, so the embedded
/// legends describe what actually happens.
fn route_runs_key(shell: &mut ShellState, key: KeyEvent) -> bool {
    let busy = shell.runs.input_mode != orc_tui::InputMode::None || shell.runs.help;
    if !busy {
        if key.code == KeyCode::Char('q') {
            return true;
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
    theme: ThemeName,
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
                    match attach_stage(commands, shell, session_id, theme) {
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
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                flow.cwd.push(character);
            }
            KeyCode::Enter => {
                let Some(brain) = flow.brain_choices.get(flow.brain_index).cloned() else {
                    home.message = "No brain harness is configured.".to_owned();
                    return Ok(false);
                };
                match commands.create_session(
                    brain,
                    flow.selected_workers.clone(),
                    flow.cwd.clone(),
                ) {
                    Ok(session_id) => {
                        home.flow = None;
                        match attach_stage(commands, shell, session_id, theme) {
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

fn render_stage(frame: &mut Frame<'_>, state: &mut StageState) {
    let area = frame.area();
    frame.render_widget(
        Block::new().style(Style::default().bg(state.theme.stage)),
        area,
    );
    state.pane_areas = stage_areas(area, state);
    if area.width >= 100 && state.panes.len() >= 2 && !state.zoomed {
        let baton = Rect::new(
            area.x + area.width / 2 - 3,
            area.y + 2,
            8,
            area.height.saturating_sub(4),
        );
        render_baton(frame, baton, state);
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
                true,
                state.confirmed_panes.contains(&pane.id),
                state.theme,
            );
        }
    } else {
        for (index, (pane, pane_area)) in state.panes.iter().zip(areas).enumerate() {
            render_shadow(frame, pane_area, state.theme);
            render_pane(
                frame,
                pane_area,
                pane,
                index == state.focus,
                state.confirmed_panes.contains(&pane.id),
                state.theme,
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
                    .fg(state.theme.attention)
                    .bg(state.theme.stage),
            ),
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        );
    }
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
    let brain_width = inner.width * 53 / 100;
    let worker_x = inner.x + brain_width + 5;
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

fn render_shadow(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let buffer = frame.buffer_mut();
    let right = area.right();
    for row in area.y.saturating_add(1)..area.bottom().saturating_add(1) {
        if let Some(cell) = buffer.cell_mut((right, row)) {
            cell.set_symbol("▐");
            cell.set_style(Style::default().fg(theme.shadow).bg(theme.stage));
        }
    }
    let bottom = area.bottom();
    for col in area.x.saturating_add(1)..area.right() {
        if let Some(cell) = buffer.cell_mut((col, bottom)) {
            cell.set_symbol("▄");
            cell.set_style(Style::default().fg(theme.shadow).bg(theme.stage));
        }
    }
}

fn render_baton(frame: &mut Frame<'_>, area: Rect, state: &StageState) {
    let mut points = Vec::with_capacity(65);
    for index in 0..=64 {
        let t = f64::from(index) / 64.0;
        let inverse = 1.0 - t;
        let x = 5.0 * inverse.powi(3)
            + 25.0 * 3.0 * inverse.powi(2) * t
            + 75.0 * 3.0 * inverse * t.powi(2)
            + 95.0 * t.powi(3);
        let y = 50.0 * inverse.powi(3)
            + 85.0 * 3.0 * inverse.powi(2) * t
            + 15.0 * 3.0 * inverse * t.powi(2)
            + 50.0 * t.powi(3);
        points.push((x, y));
    }
    let (_, width, reverse) = baton_profile(state.baton_kind);
    let alpha = if reverse {
        1.0 - state.pulse.alpha()
    } else {
        state.pulse.alpha()
    };
    let pulse_index = ((points.len() - 1) as f32 * alpha) as usize;
    let start = pulse_index.saturating_sub(width.saturating_sub(1));
    let end = (pulse_index + 1).min(points.len());
    let pulse = &points[start..end];
    frame.render_widget(
        Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, 100.0])
            .y_bounds([0.0, 100.0])
            .paint(|context| {
                context.draw(&Points {
                    coords: &points,
                    color: state.theme.dim,
                });
                context.draw(&Points {
                    coords: pulse,
                    color: state.theme.pulse,
                });
            }),
        area,
    );
}

fn render_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    pane: &PaneSnapshot,
    focus: bool,
    confirmed: bool,
    theme: Theme,
) {
    let border_color = if focus { theme.focus } else { theme.dim };
    let block = Block::default()
        .title(format!(
            " {}  {}{} ",
            pane.title.to_uppercase(),
            pane.state.as_deref().unwrap_or("LIVE"),
            if confirmed { " · TASK CONFIRMED" } else { "" }
        ))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(border_color))
        .title_style(
            Style::default()
                .fg(if focus { theme.focus } else { theme.text })
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(theme.stage).fg(theme.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if pane.state.as_deref() == Some("conductor_down") {
        let elapsed = pane
            .down_at
            .map_or(0, |down| epoch_now().saturating_sub(down));
        let overlay = Rect::new(
            inner.x + inner.width.saturating_sub(34) / 2,
            inner.y + inner.height.saturating_sub(3) / 2,
            inner.width.min(34),
            3.min(inner.height),
        );
        frame.render_widget(
            Paragraph::new(format!("CONDUCTOR DOWN\n{elapsed}s elapsed · R resume")).style(
                Style::default()
                    .fg(theme.text)
                    .bg(theme.attention)
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
                .fg(ratatui_color(source.foreground, theme.text))
                .bg(ratatui_color(source.background, theme.stage));
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

const fn ratatui_color(color: TerminalColor, default: Color) -> Color {
    match color {
        TerminalColor::Default => default,
        TerminalColor::Indexed(index) => Color::Indexed(index),
        TerminalColor::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

#[cfg(test)]
mod tests {
    use orc_proto::{
        HarnessSummary, PaneSnapshot, SessionSummary, TaskHistorySummary, TaskSummary, TerminalCell,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::{
        AVATAR_FRAMES, AVATAR_STATIC, BatonKind, HomeData, HomeState, LeaderAction, LeaderKey,
        RawRouter, ScoreState, ShellState, ShellView, StageState, Theme, ThemeName, baton_profile,
        render_help, render_home, render_score, render_shell, render_stage, route_raw_mouse,
        route_runs_key, score_mouse,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    fn runs_shell(theme_name: ThemeName) -> ShellState {
        let theme = if theme_name == ThemeName::Phosphor {
            orc_tui::PHOSPHOR
        } else {
            orc_tui::EMBER
        };
        ShellState {
            view: ShellView::Runs,
            home: HomeState {
                data: HomeData {
                    sessions: Vec::new(),
                    harnesses: Vec::new(),
                    default_workers: Vec::new(),
                    max_parallel_workers: 3,
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
            runs: orc_tui::App::with_runs(
                vec![
                    ledger_run("worker-live", "running", Some("bench-session")),
                    ledger_run("worker-done", "done", Some("bench-session")),
                    ledger_run("worker-solo", "done", None),
                ],
                theme,
            ),
            help: false,
            reduced_motion: false,
            epoch: std::time::Instant::now(),
            leader: LeaderKey::parse("ctrl-g"),
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
    fn embedded_runs_keys_route_into_the_app_and_documented_exits_stay_reserved() {
        let mut shell = runs_shell(ThemeName::Ember);
        assert!(shell.runs.rows.len() > 1, "fixture must have rows");

        // j/k selection routes into the App.
        let before = shell.runs.selected_row;
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('j'))));
        assert_ne!(shell.runs.selected_row, before, "j must move selection");
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('k'))));
        assert_eq!(shell.runs.selected_row, before, "k must move back");

        // enter expands the selected session group.
        assert!(shell.runs.expanded.is_empty());
        assert!(!route_runs_key(&mut shell, key(KeyCode::Enter)));
        assert!(!shell.runs.expanded.is_empty(), "enter must expand");

        // `/` begins search; literal V and Esc belong to the input.
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('/'))));
        assert_eq!(shell.runs.input_mode, orc_tui::InputMode::Search);
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('V'))));
        assert_eq!(shell.runs.input, "V", "V must type into the input");
        assert_eq!(shell.view, ShellView::Runs);
        assert!(!route_runs_key(&mut shell, key(KeyCode::Esc)));
        assert_eq!(shell.runs.input_mode, orc_tui::InputMode::None);
        assert_eq!(shell.view, ShellView::Runs, "Esc must only cancel input");

        // Esc and h at the dashboard are documented exits to HOME.
        assert!(!route_runs_key(&mut shell, key(KeyCode::Esc)));
        assert_eq!(shell.view, ShellView::Home);
        shell.view = ShellView::Runs;
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('h'))));
        assert_eq!(shell.view, ShellView::Home);

        // q quits the client from the embed.
        shell.view = ShellView::Runs;
        assert!(route_runs_key(&mut shell, key(KeyCode::Char('q'))));
    }

    #[test]
    fn embedded_runs_session_view_keeps_esc_for_the_app_and_updates_the_legend() {
        let mut shell = runs_shell(ThemeName::Ember);
        // Expand the session group, select a child run, open it.
        assert!(!route_runs_key(&mut shell, key(KeyCode::Enter)));
        assert!(!route_runs_key(&mut shell, key(KeyCode::Char('j'))));
        assert!(!route_runs_key(&mut shell, key(KeyCode::Enter)));
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
        assert!(!route_runs_key(&mut shell, key(KeyCode::Tab)));
        assert_ne!(shell.runs.detail_tab, tab_before);

        // Esc returns to the App dashboard, not to HOME.
        assert!(!route_runs_key(&mut shell, key(KeyCode::Esc)));
        assert_eq!(shell.runs.view, orc_tui::View::Dashboard);
        assert_eq!(shell.view, ShellView::Runs);
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
                let mut state = StageState::new(panes(), theme);
                state.confirmed_panes.insert("pane-1".to_owned());
                terminal
                    .draw(|frame| render_stage(frame, &mut state))
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

    #[test]
    fn baton_event_kinds_have_distinct_bounded_profiles() {
        let profiles = [
            baton_profile(BatonKind::Settle),
            baton_profile(BatonKind::Dispatch),
            baton_profile(BatonKind::Complete),
            baton_profile(BatonKind::Failed),
        ];
        assert!(
            profiles
                .iter()
                .all(|(millis, width, _)| *millis <= 1_100 && *width <= 3)
        );
        assert_ne!(profiles[0], profiles[1]);
        assert_ne!(profiles[1], profiles[2]);
        assert_ne!(profiles[2], profiles[3]);
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
                            },
                            HarnessSummary {
                                id: "hermes".to_owned(),
                                roles: vec!["worker".to_owned()],
                                resumable: false,
                            },
                            HarnessSummary {
                                id: "pi-m3".to_owned(),
                                roles: vec!["worker".to_owned()],
                                resumable: false,
                            },
                        ],
                        default_workers: vec!["hermes".to_owned(), "pi-m3".to_owned()],
                        max_parallel_workers: 3,
                        theme: "ember".to_owned(),
                        reduced_motion: false,
                        leader_key: "ctrl-g".to_owned(),
                    },
                    selected: 0,
                    flow: None,
                    message: String::new(),
                };
                terminal
                    .draw(|frame| render_home(frame, &state, Theme::from(theme_name), Some(5)))
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
                assert!(text.contains("Press "));
                assert!(text.contains("to create a session"));
                assert!(text.contains("editable offers"));
                terminal
                    .draw(|frame| render_home(frame, &state, Theme::from(theme_name), None))
                    .expect("render reduced-motion HOME");
                let text = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains(AVATAR_STATIC));
                assert!(text.contains("PI ORCHESTRA"));
                state.data.sessions.push(SessionSummary {
                    id: "session-one".to_owned(),
                    brain: "codex".to_owned(),
                    workers: vec!["hermes".to_owned(), "pi-m3".to_owned()],
                    cwd: "/tmp".to_owned(),
                    updated_at: "2026-07-11T00:00:00Z".to_owned(),
                    attention: 0,
                });
                terminal
                    .draw(|frame| render_home(frame, &state, Theme::from(theme_name), Some(0)))
                    .expect("render HOME shelf");
                let text = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("session-one"));
            }
        }
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
        let mut state = StageState::new(panes(), ThemeName::Ember);
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
                    leader: false,
                };
                terminal
                    .draw(|frame| {
                        render_score(frame, &mut state, Theme::from(theme_name), "ctrl-g")
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
