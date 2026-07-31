#![warn(missing_docs)]
//! Ratatui HOME and STAGE client for the Bench workspace.
//!
//! This crate owns rendering and input forwarding. It must never write
//! registry/session/task files or outlive the daemon-owned PTYs.

use std::collections::BTreeMap;
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
pub mod circuit;
pub mod glyph;
#[cfg(test)]
mod snapshot;
pub mod theme;

use crate::glyph::{Glyph, GlyphTier, Glyphs};
pub use crate::theme::ThemeName;
use crate::theme::{ColorTier, Slot, Theme};

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

/// What a mouse drag on a STAGE pane is doing.
///
/// Only the move half existed before: a press was accepted solely on a pane's
/// title row, and motion rewrote its `x`/`y` while copying `width`/`height`
/// straight back. There was no way to resize a pane with the mouse at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Drag {
    /// Moving the whole pane. Carries where inside it the grab landed, so the
    /// pane does not jump to put its corner under the cursor.
    Move { offset_x: u16, offset_y: u16 },
    /// Resizing from an edge, or from a corner when both hold.
    Resize { right: bool, bottom: bool },
}

/// The smallest pane `stage_areas` will lay out. Resize clamps to the same
/// floor the layout does, so a pane cannot be dragged to a size that would
/// silently spring back on the next frame.
const MIN_PANE: (u16, u16) = (10, 5);

/// A discrete message crossing a connector.
///
/// Distinct from a [`PanePulse`], which reports that a pane *is producing*.
/// This reports that one specific thing *was sent*: it has a source, a
/// destination and an outcome, so it crosses once and lands rather than
/// looping. See "Message in flight" in `docs/design/visual-identity.md`.
struct InFlight {
    /// The worker whose connector carries it, by pane id. The conductor is
    /// always the other end, so this identifies the wire either way.
    worker_id: String,
    direction: circuit::Direction,
    outcome: circuit::Outcome,
    raised: Instant,
}

impl InFlight {
    /// The pane the emote lands on: a dispatch arrives at its worker, a return
    /// arrives back at the conductor.
    fn destination<'a>(&'a self, panes: &'a [PaneSnapshot]) -> Option<&'a PaneSnapshot> {
        match self.direction {
            circuit::Direction::Outbound => panes.iter().find(|pane| pane.id == self.worker_id),
            circuit::Direction::Inbound => panes.first(),
        }
    }

    /// The pane the departure beat plays on — the mirror of
    /// [`Self::destination`]. A dispatch leaves the conductor; an answer
    /// leaves the worker that produced it.
    fn origin<'a>(&'a self, panes: &'a [PaneSnapshot]) -> Option<&'a PaneSnapshot> {
        match self.direction {
            circuit::Direction::Outbound => panes.first(),
            circuit::Direction::Inbound => panes.iter().find(|pane| pane.id == self.worker_id),
        }
    }
}

/// One pane's own output pulse.
///
/// Keyed by pane id rather than held once for the whole stage: a connector that
/// cannot say *which* worker produced the output is decoration, and a single
/// global pulse is what made the shipped rail decoration. Keyed by id and not
/// by index because `s` swaps two panes and a session can gain or lose one, and
/// index-keyed state silently reattributes traffic the moment that happens.
struct PanePulse {
    /// The decay timer. It is reset on every output tick, so `done()` means
    /// "no output for [`baton::DECAY`]" — the spec's trigger for falling back
    /// to the idle rail.
    pulse: EffectTimer,
    /// When this pane's current sweep began, so its packet's frame is a
    /// function of wall-clock rather than of how often the shell repainted.
    sweep_start: Instant,
    /// The rail state this pane's connector last actually *painted*, as
    /// opposed to the one it last computed. The repaint loop compares against
    /// this and forces a draw when they differ, which is what stops a packet
    /// freezing mid-sweep at the moment the decay timer expires. `None` until
    /// the first paint, so the first frame always draws.
    painted: Option<baton::State>,
}

impl PanePulse {
    fn new() -> Self {
        Self {
            pulse: EffectTimer::from_ms(
                u32::try_from(baton::DECAY.as_millis()).unwrap_or(u32::MAX),
                Interpolation::Linear,
            ),
            sweep_start: Instant::now(),
            painted: None,
        }
    }

    /// This pane produced output: restart the decay timer, and begin a sweep
    /// if its rail had gone idle.
    fn mark(&mut self) {
        if self.pulse.done() {
            self.sweep_start = Instant::now();
        }
        self.pulse.reset();
    }

    /// This pane's rail state right now, given the client's motion preference.
    fn state(&self, reduced_motion: bool) -> baton::State {
        let since_output = if self.pulse.done() {
            baton::DECAY
        } else {
            Duration::ZERO
        };
        baton::State::resolve(reduced_motion, since_output, self.sweep_start.elapsed())
    }
}

struct StageState {
    panes: Vec<PaneSnapshot>,
    focus: usize,
    pane_areas: Vec<Rect>,
    /// One pulse per pane, by pane id. `BTreeMap` so iteration order is
    /// deterministic and snapshots do not drift with hashing.
    pulses: BTreeMap<String, PanePulse>,
    /// Messages currently crossing a connector or showing their emote.
    flights: Vec<InFlight>,
    /// Traffic that had no wire to cross, and when STAGE noticed.
    ///
    /// Issue #45's dispatch falls back to a run or dispatch id whenever no
    /// seated pane matches the harness, so a delegation can be entirely real
    /// and entirely elsewhere. The flight raised for one was aimed at nothing:
    /// `retire_flights` dropped it on the first frame with no animation and no
    /// error, which is the one outcome STAGE is not allowed to have (#49's
    /// acceptance check 7). Such a flight is no longer raised at all — the
    /// legend says so instead, for exactly as long as the packet it replaced
    /// would have been on screen, so the note is as transient as the traffic
    /// it stands in for.
    offstage: Vec<(String, Instant)>,
    /// The wiring the last frame planned, kept so the router runs **once** per
    /// frame instead of once per consumer. `emotes`, `any_in_flight` and
    /// `retire_flights` all need a route's length, and each re-planning from
    /// `pane_areas` meant up to four routes per frame for one answer.
    /// Refreshed by `render_stage`, which is the only place `pane_areas` — the
    /// input the router reads — is written.
    wiring: Option<(circuit::Routing, HashMap<String, usize>)>,
    /// How much of each task's history has already been turned into a
    /// message, by task id. Without it every snapshot would re-raise the same
    /// dispatch, and the board is re-read on every snapshot.
    seen_history: HashMap<String, usize>,
    last_tick: Instant,
    theme: Theme,
    glyphs: Glyphs,
    session_id: Option<String>,
    layout: Vec<LayoutRect>,
    /// Whether `layout` holds local changes the daemon has not been told
    /// about.
    ///
    /// `layout` is both what the client *wants* and, until now, implicitly
    /// what it had *sent* — so `persist_stage_layout`'s "did this change?"
    /// test could only ever notice the difference between the rects it asked
    /// for and the rects `stage_areas` clamped them to. A drag writes its
    /// result straight into `layout`, so by the time the compare ran the two
    /// sides already agreed and the move was never persisted at all unless a
    /// clamp happened to bite. Tracking the intent separately is what makes
    /// deferring the write until the mouse is up correct rather than lossy.
    layout_dirty: bool,
    zoomed: bool,
    dragging: Option<(usize, Drag)>,
    raw_router: RawRouter,
    confirmed_panes: std::collections::HashSet<String>,
    leader_label: String,
    /// Whether the trigger grammar can actually fire on this machine.
    ///
    /// The in-pane highlight is pure text analysis: it lights up `delegate:`
    /// whether or not anything is listening for it. On the machine that
    /// reported issue #45 nothing was — no `UserPromptSubmit` hook was
    /// registered at all — so STAGE spent the whole session announcing a
    /// capability that did not exist, which is precisely the "never claim a
    /// capability that wasn't probed" rule.
    ///
    /// `attach_stage` always sets this from `orc_core::trigger_grammar`, the
    /// same probe `pio doctor` reports. It defaults to wired only because a
    /// `StageState` that has not attached has nothing on screen to mislabel.
    trigger_wired: bool,
    /// Recoverable command failure shown on the legend line instead of
    /// exiting the client.
    message: String,
}

impl StageState {
    fn new(panes: Vec<PaneSnapshot>, theme: Theme, glyphs: Glyphs) -> Self {
        let now = Instant::now();
        Self {
            pulses: panes
                .iter()
                .map(|pane| (pane.id.clone(), PanePulse::new()))
                .collect(),
            panes,
            focus: 0,
            pane_areas: Vec::new(),
            flights: Vec::new(),
            offstage: Vec::new(),
            wiring: None,
            seen_history: HashMap::new(),
            last_tick: now,
            theme,
            glyphs,
            session_id: None,
            layout: Vec::new(),
            layout_dirty: false,
            zoomed: false,
            dragging: None,
            raw_router: RawRouter::default(),
            confirmed_panes: std::collections::HashSet::new(),
            leader_label: "ctrl-g".to_owned(),
            trigger_wired: true,
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
        // Diff per pane id, not per index. `zip` truncates to the shorter vec,
        // so the shipped index-wise comparison noticed an added or removed
        // pane only through the length check — and it collapsed the whole
        // result into one global "something moved", which is precisely why the
        // rail could not say who was talking.
        let prior = self
            .panes
            .iter()
            .map(|pane| (pane.id.as_str(), pane.sequence))
            .collect::<HashMap<_, _>>();
        let produced = panes
            .iter()
            .filter(|next| prior.get(next.id.as_str()) != Some(&next.sequence))
            .map(|pane| pane.id.clone())
            .collect::<Vec<_>>();
        self.panes = panes;
        for id in produced {
            self.mark_output(&id);
        }
        // A pane that has gone leaves no pulse behind to be reattributed to
        // whatever takes its id next.
        self.pulses
            .retain(|id, _| self.panes.iter().any(|pane| pane.id == *id));
        self.focus = self.focus.min(self.panes.len().saturating_sub(1));
    }

    /// One pane produced output: pulse that pane's connector and no other.
    fn mark_output(&mut self, pane_id: &str) {
        self.pulses
            .entry(pane_id.to_owned())
            .or_insert_with(PanePulse::new)
            .mark();
    }

    fn advance(&mut self) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        for pulse in self.pulses.values_mut() {
            let _ = pulse.pulse.process(elapsed);
        }
    }

    /// One pane's rail state right now, given the client's motion preference.
    fn baton_state_for(&self, pane_id: &str, reduced_motion: bool) -> baton::State {
        self.pulses
            .get(pane_id)
            .map_or(baton::State::Idle, |pulse| pulse.state(reduced_motion))
    }

    /// The rail state of every worker's connector, in worker order.
    ///
    /// Index `i` is `panes[i + 1]`, matching `circuit::Circuit::routes`. The
    /// conductor has no connector of its own: what a worker's wire reports is
    /// that *worker's* output, which is the whole of AC2. The conductor's own
    /// traffic is a discrete message, not an ambient pulse — see the sheet's
    /// "Message in flight".
    fn traffic(&self, reduced_motion: bool) -> Vec<baton::State> {
        self.panes
            .iter()
            .skip(1)
            .map(|pane| self.baton_state_for(&pane.id, reduced_motion))
            .collect()
    }

    /// Whether any connector resolves to something other than what is on
    /// screen.
    ///
    /// The repaint loop's other reasons to draw are all "something is still
    /// moving". None of them covers the frame on which motion *stops*: when a
    /// decay timer expires, the live flags go false in the same iteration that
    /// its rail first resolves to [`baton::State::Idle`], so without this the
    /// idle rail is computed and never painted and the packet stays stranded
    /// mid-sweep until the next burst. Comparing against the last painted
    /// state also covers the identical Steady→Idle transition under reduced
    /// motion, where there is no animation cadence to fall back on.
    fn baton_needs_repaint(&self, reduced_motion: bool) -> bool {
        self.panes.iter().skip(1).any(|pane| {
            self.pulses
                .get(&pane.id)
                .is_none_or(|pulse| pulse.painted != Some(pulse.state(reduced_motion)))
        })
    }

    /// Learn the board that already existed, raising nothing.
    ///
    /// Attaching to a session with a finished board must not replay every
    /// dispatch it ever made, so the watermark starts at what is already
    /// there. This is the *only* thing that may treat history as old news:
    /// once attached, a task appearing for the first time appeared because
    /// the conductor just delegated it, and that is precisely the event
    /// STAGE exists to show.
    fn seed_task_events(&mut self, tasks: &[TaskSummary]) {
        for task in tasks {
            self.seen_history
                .insert(task.id.clone(), task.history.len());
        }
        self.retain_seen(tasks);
    }

    /// Turn newly-appended task history into messages in flight.
    ///
    /// Only the entries that have appeared since the last look are raised, so
    /// re-reading the board — which happens on every snapshot — does not
    /// re-dispatch the same packet forever.
    ///
    /// A task with no watermark is *new*, not old: it was created after
    /// [`Self::seed_task_events`] ran, so its whole history is news. This used
    /// to skip it, and the cost was the headline gesture of issue #45 — a
    /// `pio orch delegate` from inside a seated pane creates, assigns and
    /// confirms a task between two snapshots, so STAGE saw it for the first
    /// time already finished and animated nothing at all. Every test around
    /// this passed because each one hand-fed a `created`-only board first,
    /// sharing the assumption that a task is always seen before it is
    /// dispatched.
    fn note_task_events(&mut self, tasks: &[TaskSummary]) {
        for task in tasks {
            let seen = self.seen_history.get(&task.id).copied().unwrap_or(0);
            // The watermark only moves once there is a wire to aim at.
            //
            // A task is created and assigned before anything links it to a
            // pane: `pio orch delegate` passes no run to `assign_task`, and it
            // is the detached supervisor's `record_delivery` that writes the
            // link, in a different process some time later. Advancing past
            // `assigned` while the link is still missing threw the outbound
            // packet away — and threw it away *more often the more promptly
            // the board was read*, which is why #45 never saw it and why #49's
            // wake path would have. Holding costs nothing: the entries are
            // re-read, never re-raised, and the watermark catches up in one go
            // the moment the link lands.
            let Some(worker_id) = task.assignee_run.clone() else {
                continue;
            };
            self.seen_history
                .insert(task.id.clone(), task.history.len());
            let traffic = task
                .history
                .iter()
                .skip(seen)
                .filter_map(|entry| circuit::message_for(&entry.action, entry.to.as_deref()))
                .collect::<Vec<_>>();
            if traffic.is_empty() {
                continue;
            }
            // Aimed at a run that is not one of these panes: real traffic,
            // genuinely somewhere else. Say so rather than raising a packet
            // with no route, which is what used to be dropped in silence. One
            // note per message, exactly as the seated branch below raises one
            // flight per message — a single note for a whole batch would have
            // the legend undercount the traffic it exists to admit to.
            if !self.panes.iter().skip(1).any(|pane| pane.id == worker_id) {
                for _ in &traffic {
                    self.offstage.push((worker_id.clone(), Instant::now()));
                }
                continue;
            }
            for (direction, outcome) in traffic {
                if direction == circuit::Direction::Inbound {
                    self.land_outbound(&worker_id);
                }
                self.flights.push(InFlight {
                    worker_id: worker_id.clone(),
                    direction,
                    outcome,
                    raised: Instant::now(),
                });
            }
        }
        self.retain_seen(tasks);
    }

    /// Bring any brief still shown crossing to `worker_id` to its destination,
    /// because something has just come back from there.
    ///
    /// Travel time is a function of the wire's length, so on a long connector
    /// a genuinely fast worker can answer while its own brief is still drawn
    /// mid-flight — and then two packets cross in opposite directions on one
    /// wire, which is the picture issue #49 opens with. The events are real and
    /// their times are real; what is not real is the brief, because an answer
    /// coming back is proof that it arrived. So it arrives: the outbound flight
    /// is advanced to its landing rather than deleted, which keeps its emote on
    /// the worker's card and puts the two beats back in the order they actually
    /// happened.
    fn land_outbound(&mut self, worker_id: &str) {
        let len = self.route_len(worker_id);
        if len == 0 {
            return;
        }
        let landed = Instant::now()
            .checked_sub(circuit::travel_time(len))
            .unwrap_or_else(Instant::now);
        for flight in &mut self.flights {
            if flight.worker_id == worker_id
                && flight.direction == circuit::Direction::Outbound
                && flight.raised > landed
            {
                flight.raised = landed;
            }
        }
    }

    /// Forget watermarks for tasks that have left the board.
    fn retain_seen(&mut self, tasks: &[TaskSummary]) {
        self.seen_history
            .retain(|id, _| tasks.iter().any(|task| task.id == *id));
    }

    /// Drop messages whose emote has run out, and any whose wire has gone.
    ///
    /// "Never leaves residue on the buffer" is a property of the whole frame
    /// being redrawn, but a flight that outlived its pane would keep asking
    /// for frames, so it is retired here rather than left to leak.
    fn retire_flights(&mut self, reduced_motion: bool) {
        // An off-stage note stands in for a packet, so it lives exactly as long
        // as that packet would have.
        self.offstage
            .retain(|(_, raised)| raised.elapsed() < circuit::EMOTE_HOLD);
        let lengths = self.wiring.take();
        if let Some((_, lengths)) = lengths.as_ref() {
            let mut stranded = Vec::new();
            self.flights.retain(|flight| {
                let len = lengths.get(&flight.worker_id).copied().unwrap_or(0);
                if len == 0 {
                    // `note_task_events` only ever aims a flight at a pane that
                    // was on the stage, so reaching here means the wire went
                    // away underneath it: the pane left, or a dragged pane is
                    // covering its whole route. The message is retired either
                    // way — a flight that outlived its wire would keep asking
                    // for frames it cannot use — but it is counted, not
                    // dropped in silence.
                    stranded.push(flight.worker_id.clone());
                    return false;
                }
                circuit::flight(reduced_motion, flight.raised.elapsed(), len)
                    != circuit::Flight::Gone
            });
            for worker_id in stranded {
                self.offstage.push((worker_id, Instant::now()));
            }
        }
        self.wiring = lengths;
    }

    /// Record the wiring a frame just planned, so the consumers below can read
    /// a route's length without planning it again.
    fn record_wiring(&mut self, wiring: Option<&circuit::Circuit>) {
        self.wiring = wiring.map(|wiring| {
            (
                wiring.routing,
                self.panes
                    .iter()
                    .skip(1)
                    .zip(&wiring.routes)
                    .map(|(pane, route)| (pane.id.clone(), route.len()))
                    .collect(),
            )
        });
    }

    /// How long each worker's connector was on the last painted frame, by pane
    /// id. `None` before the first paint, which is when `pane_areas` — and so
    /// the wiring — does not exist yet. Borrowed rather than cloned: three
    /// callers ask per frame, and cloning the map for each of them gave back
    /// most of what routing once instead of four times had saved.
    fn route_lengths(&self) -> Option<&HashMap<String, usize>> {
        self.wiring.as_ref().map(|(_, lengths)| lengths)
    }

    /// One worker's connector length on the last painted frame, or 0.
    fn route_len(&self, worker_id: &str) -> usize {
        self.route_lengths()
            .and_then(|lengths| lengths.get(worker_id))
            .copied()
            .unwrap_or_default()
    }

    /// Which tier the router reached, for the honest fallback note on STAGE.
    fn routing(&self) -> Option<circuit::Routing> {
        self.wiring.as_ref().map(|(routing, _)| *routing)
    }

    /// Whether any message is still crossing or still showing its emote.
    fn any_in_flight(&self, reduced_motion: bool) -> bool {
        self.flights.iter().any(|flight| {
            let len = self.route_len(&flight.worker_id);
            len > 0
                && circuit::flight(reduced_motion, flight.raised.elapsed(), len)
                    != circuit::Flight::Gone
        })
    }

    /// Whether any packet is actually *moving* this instant.
    ///
    /// The narrower half of [`Self::any_in_flight`], and the one that earns the
    /// travel cadence. A landed emote holds for [`circuit::EMOTE_HOLD`] with
    /// only its 90 ms flash boundary to catch, and under reduced motion there
    /// is no travel at all — asking for a frame every 15 ms through either is
    /// the wasted spin [`Self::any_live`]'s doc describes, paid on every hosted
    /// pane on the stage.
    fn any_travelling(&self, reduced_motion: bool) -> bool {
        !reduced_motion
            && self.flights.iter().any(|flight| {
                let len = self.route_len(&flight.worker_id);
                len > 0
                    && matches!(
                        circuit::flight(reduced_motion, flight.raised.elapsed(), len),
                        circuit::Flight::Travelling(_)
                    )
            })
    }

    /// The emote to stamp on each pane this frame, by pane id.
    ///
    /// Two beats now, not one: a message *leaving* stamps its origin and a
    /// message *landing* stamps its destination, which is issue #49's spec
    /// step 1 — the hand-off you can see — paired with the arrival that was
    /// already there. They are read off the same clock as the packet, so
    /// neither can be showing at a moment when the packet is not where it
    /// says.
    ///
    /// Most recent wins when several beats fall on one pane, so the newest
    /// news is what it shows.
    ///
    /// Under reduced motion nothing travels: `circuit::flight` is `Landed`
    /// from the first frame, so there is no departure to see and the arrival
    /// emote carries the whole event. That is the sheet's own reduced-motion
    /// rule — same information, no packet anywhere on the rail — rather than
    /// a beat quietly dropped.
    fn emotes(&self, reduced_motion: bool) -> HashMap<String, circuit::Emote> {
        let mut showing = HashMap::new();
        for flight in &self.flights {
            let len = self.route_len(&flight.worker_id);
            if len == 0 {
                continue;
            }
            let since = flight.raised.elapsed();
            let (pane, beat, held) = match circuit::flight(reduced_motion, since, len) {
                circuit::Flight::Travelling(step) if step < circuit::DEPART_CELLS => (
                    flight.origin(&self.panes),
                    circuit::Beat::Leaving(flight.direction),
                    since,
                ),
                circuit::Flight::Landed { since: held } => {
                    (flight.destination(&self.panes), circuit::Beat::Landed, held)
                }
                circuit::Flight::Travelling(_) | circuit::Flight::Gone => continue,
            };
            if let Some(pane) = pane {
                // "Under reduced motion the flash frame is dropped: it appears
                // already settled, holds, and leaves."
                showing.insert(
                    pane.id.clone(),
                    circuit::Emote {
                        beat,
                        outcome: flight.outcome,
                        flashing: !reduced_motion && held < circuit::EMOTE_FLASH,
                    },
                );
            }
        }
        showing
    }

    /// Whether any *connector* is still mid-pulse.
    ///
    /// Workers only. The conductor has no connector of its own, so its pulse
    /// drives nothing on screen — and a repaint cadence held open by something
    /// that animates nothing is the same wasted spin the trigger rainbow used
    /// to cause.
    fn any_live(&self) -> bool {
        self.panes.iter().skip(1).any(|pane| {
            self.pulses
                .get(&pane.id)
                .is_some_and(|pulse| !pulse.pulse.done())
        })
    }

    /// Record the rail states that were just drawn to the terminal.
    ///
    /// Called from the render path with the exact slice handed to
    /// [`render_stage`], so "last painted" means painted, not computed.
    fn record_painted_traffic(&mut self, traffic: &[baton::State]) {
        for (pane, state) in self.panes.iter().skip(1).zip(traffic) {
            if let Some(pulse) = self.pulses.get_mut(&pane.id) {
                pulse.painted = Some(*state);
            }
        }
    }

    /// Whether any conductor pane currently shows a trigger.
    ///
    /// This drives the trigger rainbow's own cadence and nothing else. It is
    /// deliberately *not* an input to the baton: a pane displaying `delegate:`
    /// with no output has produced no traffic, so the rail decays to idle like
    /// any other silent pane. Pinned by
    /// `a_trigger_animates_the_rainbow_but_never_the_rail`.
    ///
    /// It answers what a pane *shows*, not whether that has anything to
    /// animate; `repaint_reasons` pairs it with the tier, because a terminal
    /// with no gradient has nothing to slide.
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
    /// A durable task board changed on disk.
    ///
    /// Issue #49's defect 4. Task events used to be noticed only inside the
    /// `Snapshot` arm, and `Snapshot` is emitted only when a pane's PTY output
    /// sequence changes — so a delegation between quiet panes was seen
    /// whenever a pane next happened to speak, or after the loop's 30 s
    /// timeout, whichever came first. The board is a set of files this client
    /// can watch directly, so it now does: "something changed" is decoupled
    /// from "a pane said something".
    BoardChanged,
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
            "  PI ORCHESTRA / HELP\n\n  FIRST USE\n  n creates a session: choose a brain, edit worker offers, choose a cwd.\n  The brain plans; available workers receive explicit durable task briefs.\n\n  CONTROL\n  In STAGE everything you type goes to the focused pane. Commands need\n  the leader first: press {leader}, release, then one key.\n  {leader} n/p focus · {leader} z zoom · {leader} s swap · {leader} b SCORE\n  {leader} h HOME · {leader} v views · {leader} ? help · {leader} q detach\n  {leader} twice sends the literal chord to the pane.\n  Outside STAGE, bare V cycles HOME, SCORE, RUNS and ? opens help.\n  Mouse: drag a title to move a pane, an edge or corner to resize it;\n  the layout is remembered. Every other click goes to the focused pane.\n\n  THEME\n  {leader} t cycles nocturne, ember, phosphor on every screen, and\n  asks the daemon to remember it: the next launch opens the same.\n  pio config set theme <name> does it from a shell; pio config get\n  theme reports what is stored. No file to edit.\n  Set the leader with app.leader_key in ~/.orchestra/harnesses.json.\n\n  DURABILITY AND RECOVERY\n  Closing the client detaches; pi-orchestra attach replays the session.\n  SCORE is the durable task board. Delivery is shown only after confirmation.\n  Missing executables are UNAVAILABLE. R recovers a supported dead brain.\n  If recovery fails, reattach and inspect SCORE, orc task list, and orc list.\n\n  Esc or ? closes help.",
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
            let traffic = shell
                .stage
                .as_ref()
                .map(|stage| stage.traffic(shell.reduced_motion))
                .unwrap_or_default();
            if let Some(stage) = shell.stage.as_mut() {
                render_stage(frame, stage, motion, &traffic);
                // What reached the terminal, recorded at the only place that
                // knows it did: `run_shell_loop` compares against this to
                // decide whether a rail still owes the screen a frame.
                stage.record_painted_traffic(&traffic);
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
    spawn_file_watches(&events_tx);

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

/// Which panes carry the steady `✓ TASK CONFIRMED` badge.
///
/// The most recent entry that says something about *how the dispatch went*
/// decides it — not the most recent entry of any kind. Reading
/// `history.last()` only ever worked because `delivery_confirmed` happened to
/// be the last durable word a dispatch wrote, and issue #49 ends that:
/// `execution_succeeded` is appended after it once the worker finishes.
/// `last()` would have quietly taken the badge off every pane on the stage,
/// and no test would have failed.
///
/// `execution_failed` counts, and has to: a worker that took the brief and
/// then died still has a `delivery_confirmed` behind it, so a rule that looked
/// only at deliveries would leave `✓ TASK CONFIRMED` sitting on a pane whose
/// work failed. `execution_succeeded` deliberately does not appear here — it
/// is not a negation, and the badge means the same thing after it as before.
fn confirmed_panes(tasks: &[TaskSummary]) -> std::collections::HashSet<String> {
    tasks
        .iter()
        .filter_map(|task| {
            task.history
                .iter()
                .rev()
                .find(|history| {
                    matches!(
                        history.action.as_str(),
                        "delivery_confirmed" | "delivery_failed" | "execution_failed"
                    )
                })
                .filter(|history| history.action == "delivery_confirmed")
                .and(task.assignee_run.clone())
        })
        .collect()
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
    // Probed once per attach, from the same source `pio doctor` reports. The
    // in-pane highlight is pure text analysis and will happily light up a
    // spell nothing is listening for; this is what lets the badge say so.
    stage.trigger_wired = orc_core::trigger_grammar::trigger_grammar()
        .iter()
        .all(|check| check.ok);
    stage.confirmed_panes = confirmed_panes(&tasks);
    // Seed the history watermark: attaching to a finished board must not
    // replay every dispatch it ever made. Everything that lands *after* this
    // is news, including a task that appears for the first time.
    stage.seed_task_events(&tasks);
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

/// Why the shell may owe the screen a frame, as observed at the top of one
/// loop iteration.
///
/// Split out of `run_shell_loop` so the decision is a value a test can make
/// assertions about. The loop itself is not reachable from a test — it wants a
/// real terminal, a real socket and a live event source — so leaving the guard
/// inline would leave the one thing this issue's first commit changes
/// unverifiable except by eye.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RepaintReasons {
    /// An event arrived since the last draw.
    pending: bool,
    /// A worker's **ambient baton** is mid-pulse and motion is allowed, so its
    /// rail is sweeping. Named before there was anything else in motion; it
    /// has never had anything to do with a message packet, which is what
    /// `in_flight` below reports. The comment here used to say "the packet is
    /// travelling", and issue #49 records the cost of believing it: a faster
    /// poll was assumed to speed the packet up, when the packet's own cadence
    /// was the thing standing still.
    animating: bool,
    /// The baton is mid-pulse but motion is reduced: the rail is static, and
    /// this only exists so the decay to idle is noticed promptly.
    stage_live: bool,
    /// The rail resolves to something other than what is on screen. This is
    /// the only reason that fires on the frame motion *stops*.
    stage_changed: bool,
    /// A conductor pane is showing a trigger, motion is allowed, and there is a
    /// gradient for it to slide. On the monochrome tier there is not: the token
    /// is bold and nothing else, so a 120 ms cadence would be the wasted spin
    /// [`StageState::any_live`] describes.
    trigger_ambient: bool,
    /// A message is crossing a connector, or its emote is still showing.
    in_flight: bool,
    /// A packet is actually *moving* this instant — not merely holding its
    /// landed emote.
    ///
    /// Split from `in_flight` because the two want different cadences, and
    /// conflating them was a regression: `in_flight` stays true for the whole
    /// of [`circuit::EMOTE_HOLD`], 1.2 s during which the only thing that can
    /// change is the 90 ms flash boundary — and under reduced motion, where
    /// `circuit::flight` is `Landed` from the first frame and the flash is
    /// suppressed, nothing on screen can change at all. Holding the shell at
    /// the travel cadence through that redraws every hosted pane ~80 times for
    /// nothing.
    travelling: bool,
    /// A message could not be shown because its worker is not a pane here, and
    /// the legend is saying so. It needs a frame to *stop* saying it: on a
    /// quiet stage nothing else would ask for one, so the note would sit there
    /// until the next unrelated event — up to the loop's 30 s timeout.
    offstage: bool,
    home_ambient: bool,
    runs_ambient: bool,
}

impl RepaintReasons {
    /// Whether to draw this iteration.
    const fn draw(self) -> bool {
        self.pending
            || self.animating
            || self.stage_live
            || self.stage_changed
            || self.trigger_ambient
            || self.in_flight
            || self.offstage
            || self.home_ambient
            || self.runs_ambient
    }

    /// How long the loop may sleep before looking again.
    ///
    /// The shortest cadence any live reason needs, so the tiers are ordered
    /// fastest first. `travelling` leads because it is the quickest of them:
    /// the packet crosses a cell every [`circuit::FLIGHT_MS_PER_CELL`], which
    /// is faster than the baton's frame. `stage_changed` is absent on purpose:
    /// it is satisfied by the draw it just asked for, so it must not hold the
    /// loop at a fast cadence afterwards.
    const fn wait(self) -> Duration {
        if self.travelling {
            // Half a cell's worth of travel, so no cell the packet lands on
            // aliases away between two polls.
            Duration::from_millis(circuit::FLIGHT_MS_PER_CELL / 2)
        } else if self.animating {
            Duration::from_millis(16)
        } else if self.in_flight {
            // A landed emote, holding. Its only boundary is the 90 ms flash,
            // so this stays the cadence the packet itself used before it was
            // given one of its own.
            Duration::from_millis(30)
        } else if self.home_ambient || self.trigger_ambient || self.offstage {
            Duration::from_millis(120)
        } else if self.stage_live || self.runs_ambient {
            Duration::from_millis(500)
        } else {
            Duration::from_secs(30)
        }
    }
}

/// Read the shell's current repaint reasons. Pure apart from the clocks it
/// samples through `StageState`.
fn repaint_reasons(shell: &ShellState, pending: bool) -> RepaintReasons {
    // The help overlay and the other screens do not paint the rail, so no
    // rail-derived reason may fire there: a guard asking for a frame nobody
    // draws is never satisfied and would spin the loop at full tilt.
    let on_stage = !shell.help && shell.view == ShellView::Stage;
    let stage = shell.stage.as_ref().filter(|_| on_stage);
    let live = stage.is_some_and(StageState::any_live);
    RepaintReasons {
        pending,
        animating: live && !shell.reduced_motion,
        stage_live: live && shell.reduced_motion,
        stage_changed: stage.is_some_and(|stage| stage.baton_needs_repaint(shell.reduced_motion)),
        trigger_ambient: !shell.reduced_motion
            && shell.theme.trigger_gradient().is_some()
            && stage.is_some_and(|stage| stage.has_live_trigger()),
        in_flight: stage.is_some_and(|stage| stage.any_in_flight(shell.reduced_motion)),
        travelling: stage.is_some_and(|stage| stage.any_travelling(shell.reduced_motion)),
        offstage: stage.is_some_and(|stage| !stage.offstage.is_empty()),
        home_ambient: !shell.reduced_motion && !shell.help && shell.view == ShellView::Home,
        // The RUNS embed repaints on a modest tick so quota/history updates
        // arriving on the App's internal channel become visible without a
        // keypress. This is data refresh, not animation, so it is kept under
        // reduced_motion; App::refresh is internally rate-limited to 500 ms.
        runs_ambient: !shell.help && shell.view == ShellView::Runs,
    }
}

/// Re-read the attached session's task board and turn what is new into
/// messages.
///
/// Called from both wake paths — a pane produced output, or the board itself
/// changed — because a task event means the same thing whichever noticed it
/// first. A task event is not a stdout tick: it has a source, a destination
/// and an outcome, so it raises a discrete message rather than pulsing the
/// ambient rail, which is what made a dispatch, a returned result and a worker
/// merely printing look identical.
fn read_board(commands: &mut BenchClient, shell: &mut ShellState) {
    let Some(score) = shell.score.as_mut() else {
        return;
    };
    let Ok(tasks) = commands.task_board(score.session_id.clone()) else {
        return;
    };
    score.tasks = tasks;
    if let Some(stage) = shell.stage.as_mut() {
        stage.confirmed_panes = confirmed_panes(&score.tasks);
        stage.note_task_events(&score.tasks);
    }
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
            stage.retire_flights(shell.reduced_motion);
        }
        let reasons = repaint_reasons(shell, redraw);
        if reasons.runs_ambient {
            let _ = shell.runs.refresh();
        }
        if reasons.draw() {
            let mut stdout = io::stdout();
            stdout.sync_update(|_| terminal.draw(|frame| render_shell(frame, shell)))??;
            if shell.view == ShellView::Stage
                && let Some(stage) = shell.stage.as_mut()
            {
                sync_stage_geometry(commands, stage, &mut requested_sizes)?;
            }
            redraw = false;
        }
        let event = match events.recv_timeout(reasons.wait()) {
            Ok(event) => Some(event),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(AppError::EventSource),
        };
        // Decided before the match, because `Snapshot` consumes its panes and
        // the obligation must not be restatable per-arm — see `reads_board`.
        // It runs *after* the match so a snapshot's panes are already applied:
        // `note_task_events` asks whether a task's worker is a pane on this
        // stage, and answering that against last frame's panes would send a
        // live delegation to the off-stage legend.
        let wants_board = event.as_ref().is_some_and(reads_board);
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
                let _ = shell.runs.refresh();
                redraw = true;
            }
            Some(UiEvent::BoardChanged) => {
                // The board changed on disk. That is the event STAGE exists to
                // show, and until now the only way it reached this loop was by
                // riding on a pane's output.
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
        if wants_board {
            read_board(commands, shell);
        }
    }
}

/// Push STAGE's geometry to the daemon — but never mid-drag.
///
/// This is the whole of the "buggy and not fluid" bug. Both halves were
/// debounced only against their *last value*, and during a drag the value
/// changes every frame, so at the 16 ms animating cadence each frame did a
/// blocking `resize` round-trip (daemon → `TIOCSWINSZ` → the hosted CLI
/// reflows its entire screen) *and* a blocking `update_layout` whose handler
/// re-reads `session.json`, mutates it and writes it back through an
/// `fsync`. Up to ~60 socket round-trips, 60 PTY resizes and 60 fsynced
/// rewrites per second while the mouse was down — with the UI thread blocked
/// on each one.
///
/// While the mouse is down the client now talks to nobody. The frame follows
/// the cursor because that is a local repaint; the pane's *contents* reflow
/// once, on release, which is what every tiling window manager does and is
/// why it reads as fluid. The value-debounce underneath then makes that
/// single post-release pass fire exactly once.
///
/// Deferring is safe because nothing observes the layout in between:
/// `update_layout` does not bump the daemon's control sequence, so no other
/// client is waiting on it, and a stale pane size is corrected by the very
/// next frame after release.
fn sync_stage_geometry(
    commands: &mut BenchClient,
    state: &mut StageState,
    requested_sizes: &mut HashMap<String, (u16, u16)>,
) -> Result<()> {
    if state.dragging.is_some() {
        return Ok(());
    }
    resize_to_cards(commands, state, requested_sizes)?;
    persist_stage_layout(commands, state)?;
    Ok(())
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
    if layout != state.layout || state.layout_dirty {
        commands.update_layout(session_id, layout.clone())?;
        state.layout = layout;
        state.layout_dirty = false;
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

/// Every directory the shell watches for durable state it did not write.
///
/// A table rather than three `spawn_*` calls at the top of [`run_initial`],
/// because the wiring is the half of a wake path that rots without anyone
/// noticing. Dropping the board entry restores exactly issue #49's defect 4 —
/// STAGE learning about a delegation only when a PTY happens to tick — and a
/// watcher tested in isolation cannot see it go: `spawn_change_watch` is handed
/// its path and its event by the caller, so it keeps passing while nothing
/// calls it. Naming the set gives the test something to hold.
fn file_watches() -> [FileWatch; 3] {
    [
        FileWatch {
            root: orc_core::registry::home().join("runs"),
            what: "runs",
            raise: || UiEvent::RunsChanged,
        },
        FileWatch {
            root: orc_core::registry::home().join("reports"),
            what: "reports",
            raise: || UiEvent::RunsChanged,
        },
        FileWatch {
            root: board_watch_root(),
            what: "task board",
            raise: || UiEvent::BoardChanged,
        },
    ]
}

/// One directory the shell watches, and what a change under it raises.
struct FileWatch {
    /// The tree to watch. Created if it does not exist yet.
    root: PathBuf,
    /// What it is called when the watcher has to report a failure.
    what: &'static str,
    /// The event a change raises. A plain constructor: the arm that receives
    /// it re-reads whatever it needs, so nothing is carried.
    raise: fn() -> UiEvent,
}

fn spawn_file_watches(sender: &SyncSender<UiEvent>) {
    for watch in file_watches() {
        spawn_change_watch(watch.root, watch.what, watch.raise, sender.clone());
    }
}

/// Whether an event obliges the loop to re-read the durable task board.
///
/// Lifted out of the match arms so the obligation can be tested. `read_board`
/// sitting inline in an arm is the other half of what rots silently: gut the
/// `BoardChanged` arm down to `redraw = true` and the watcher still fires, the
/// shell still wakes, and nothing re-reads the board — with the whole suite
/// green, because no test drives the loop. Both events read it for the same
/// reason: something durable may have moved, and only the board says what.
const fn reads_board(event: &UiEvent) -> bool {
    match event {
        // A pane spoke. The board is re-read because a delegation is usually
        // *why* it spoke, and this was the only path before #49.
        UiEvent::Snapshot(_) => true,
        // The board itself changed on disk — the wake path #49 added.
        UiEvent::BoardChanged => true,
        UiEvent::Raw(_) | UiEvent::Resize | UiEvent::WatchFailed(_) | UiEvent::RunsChanged => false,
    }
}

/// The directory every session's task board lives under — watched so that a
/// delegation between two silent panes is seen when it happens rather than
/// when a pane next speaks.
///
/// The board is written by other processes — `pio orch delegate` in the
/// conductor's pane, and the detached dispatch supervisor minutes later — and
/// none of that touches a PTY. Before this the client learned about it only
/// through `UiEvent::Snapshot`, which the daemon emits when a pane's output
/// sequence changes; between quiet panes the wait was up to 30 s. This is the
/// cheap half of what a dedicated renderer thread would buy: "the board
/// changed" now has its own way in.
///
/// Named rather than inlined so a test can hold it against
/// `orc_core::tasks::task_path` — watching the wrong tree would leave the
/// watcher working perfectly and the shell asleep.
fn board_watch_root() -> PathBuf {
    orc_core::registry::home().join("tasks")
}

/// Wake the shell whenever anything under `path` changes.
///
/// The raised event carries no payload — the arm that receives it re-reads
/// whatever it needs — so `raise` is a plain constructor. Bursts are
/// coalesced: one board mutation creates a lock file, writes the task and
/// removes the lock, and three repaints for one event is the wasted spin the
/// repaint tiers exist to avoid.
fn spawn_change_watch(
    path: PathBuf,
    what: &'static str,
    raise: fn() -> UiEvent,
    sender: SyncSender<UiEvent>,
) {
    thread::spawn(move || {
        if std::fs::create_dir_all(&path).is_err() {
            let _ = sender.send(UiEvent::WatchFailed(format!(
                "{what} watcher could not create its directory"
            )));
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
            let _ = sender.send(UiEvent::WatchFailed(format!(
                "{what} watcher could not start"
            )));
            return;
        };
        if watcher.watch(&path, RecursiveMode::Recursive).is_err() {
            let _ = sender.send(UiEvent::WatchFailed(format!(
                "{what} watcher could not watch {}",
                path.display()
            )));
            return;
        }
        while changes.recv().is_ok() {
            while changes.try_recv().is_ok() {}
            if sender.send(raise()).is_err() {
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
        // The reply carries the name the daemon actually wrote, which is what
        // the next launch will read. Adopt it, so the screen can never show a
        // palette the durable record disagrees with.
        Ok(stored) => {
            let stored = ThemeName::named(&stored);
            if stored != next {
                apply_theme(shell, stored);
            }
            String::new()
        }
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
    // Release first. SGR reports a release as the *same* button code with an
    // `m` suffix, so a press branch that keys on the code alone claims it —
    // and letting go over a pane's title row re-armed the drag instead of
    // ending it. That left `dragging` latched with the mouse up, which is
    // both the sticky pane and, now that geometry defers while a drag is in
    // flight, a client that would never sync again.
    if *code == 3 || suffix == 'm' {
        state.dragging = None;
        return Some(None);
    }
    if *code == 0
        && let Some(index) = pane_index
        && let Some(area) = state.pane_areas.get(index).copied()
        && let Some(kind) = grab(area, column, row)
    {
        state.focus = index;
        state.dragging = Some((index, kind));
        return Some(None);
    }
    if *code == 32
        && let Some((index, kind)) = state.dragging
        && let Some(pane_id) = state.panes.get(index).map(|pane| pane.id.clone())
        && let Some(area) = state.pane_areas.get(index).copied()
    {
        ensure_layout(state);
        state.layout_dirty = true;
        if let Some(rect) = state.layout.iter_mut().find(|rect| rect.pane_id == pane_id) {
            match kind {
                Drag::Move { offset_x, offset_y } => {
                    rect.x = column.saturating_sub(offset_x);
                    rect.y = row.saturating_sub(offset_y);
                    rect.width = area.width;
                    rect.height = area.height;
                }
                Drag::Resize { right, bottom } => {
                    rect.x = area.x;
                    rect.y = area.y;
                    // The dragged edge follows the cursor; the opposite one
                    // stays put, so the pane grows from where it was grabbed.
                    rect.width = if right {
                        column
                            .saturating_sub(area.x)
                            .saturating_add(1)
                            .max(MIN_PANE.0)
                    } else {
                        area.width
                    };
                    rect.height = if bottom {
                        row.saturating_sub(area.y).saturating_add(1).max(MIN_PANE.1)
                    } else {
                        area.height
                    };
                }
            }
        }
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

/// What pressing at `(column, row)` inside `area` grabs, if anything.
///
/// Edges win over the title row, so the top-right corner resizes rather than
/// moves — a corner is the one place a user expects to size from, and the rest
/// of the title bar is still a long, easy move target. Anywhere else in the
/// pane is not a grab at all: it belongs to the hosted CLI, and swallowing it
/// would break click-to-position inside the harness.
const fn grab(area: Rect, column: u16, row: u16) -> Option<Drag> {
    let right = column == area.right().saturating_sub(1);
    let bottom = row == area.bottom().saturating_sub(1);
    if right || bottom {
        return Some(Drag::Resize { right, bottom });
    }
    if row == area.y {
        return Some(Drag::Move {
            offset_x: column.saturating_sub(area.x),
            offset_y: row.saturating_sub(area.y),
        });
    }
    None
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
    traffic: &[baton::State],
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
    // Route once, before anything asks a question about the wiring. Recording
    // it here rather than after the pane loop matters: `emotes` needs a
    // route's length to know whether a message has landed yet, and reading a
    // cache that this frame had not filled left every emote one frame late.
    let wiring = circuit::plan(&state.pane_areas);
    state.record_wiring(wiring.as_ref());
    let areas = state.pane_areas.clone();
    let emotes = state.emotes(motion.is_none());
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
                    emote: emotes.get(&pane.id).copied(),
                    trigger_wired: state.trigger_wired,
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
                    emote: emotes.get(&pane.id).copied(),
                    trigger_wired: state.trigger_wired,
                },
                state.theme,
                state.glyphs,
            );
        }
        // After the panes, so an inlaid rail can sit in a worker's own top
        // border and a wire is never buried under a pane drawn later.
        if let Some(wiring) = wiring.as_ref() {
            render_circuit(frame, state, wiring, traffic, motion.is_none());
        }
    }
    if state.message.is_empty() {
        // AC8 asks for a *stated* fallback, not merely a visible one. It leads
        // the legend rather than trailing it because the width at which the
        // router gives up is also the width at which the legend gets clipped.
        let fallback = if state.routing() == Some(circuit::Routing::Inlaid) {
            "connectors inlaid — too narrow to route · "
        } else {
            ""
        };
        // Same rule for traffic with no wire: a delegation whose worker is not
        // one of these panes is real, and STAGE saying nothing about it is how
        // #45's reporter came to believe their seated Hermes had run the task.
        //
        // The run is named only when there is room for it. A fallback link is
        // a `D-{cwd}-{epoch}-{slug}-{nonce}` dispatch id and routinely runs to
        // forty characters, which at 80 columns — where the inlaid prefix has
        // already claimed half the line — would push every control key off the
        // end and then clip the id it promised to name. The unnamed form still
        // states the thing that matters; SCORE has the id.
        let offstage = match state.offstage.len() {
            0 => String::new(),
            count => {
                let named = (count == 1)
                    .then(|| format!(" — {} is not a pane here", state.offstage[0].0))
                    .filter(|named| fallback.len() + named.len() + 32 <= usize::from(area.width))
                    .unwrap_or_default();
                let plural = if count == 1 { "message" } else { "messages" };
                format!("{count} {plural} crossed no wire{named} · ")
            }
        };
        let legend = format!(
            "{fallback}{offstage}typing goes to the pane — {leader} then: n/p focus · z zoom · s swap · b SCORE · h HOME · ? help · q detach — mouse: drag a title to move, an edge to resize",
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

/// Draw the wiring: one connector per worker, each carrying its own traffic.
///
/// `traffic[i]` is the rail state of the connector to `panes[i + 1]`; a missing
/// entry is idle. The whole frame is decided by the passed states, so a
/// snapshot pins a frame instead of racing the clock — the same property the
/// single rail had, kept while going from one wire to *n*.
///
/// Idle wires are drawn first and live ones after, so where routes share the
/// trunk at the conductor's port a live packet wins over a quiet neighbour.
fn render_circuit(
    frame: &mut Frame<'_>,
    state: &StageState,
    wiring: &circuit::Circuit,
    traffic: &[baton::State],
    reduced_motion: bool,
) {
    let theme = state.theme;
    let glyphs = state.glyphs;
    let shape = |cell: &(u16, u16)| {
        wiring
            .loom
            .binary_search_by_key(cell, |(at, _)| *at)
            .map_or_else(|_| circuit::Wire::default(), |index| wiring.loom[index].1)
    };
    // Structure first: every cell where the wire turns or branches. Those are
    // the loom's own shape and belong to no single route — a junction sits on
    // the path of every worker downstream of it. Flat runs are deliberately
    // left to the rails, so a straight single-worker wire still paints the
    // sheet's `◆ ············ ●` with its endpoint clearance intact rather
    // than filling that clearance with rule.
    for ((x, y), wire) in &wiring.loom {
        if !wire.is_horizontal() {
            paint_cell(
                frame,
                (*x, *y),
                wire.symbol(glyphs),
                theme.state(Slot::Faint),
            );
        }
    }
    let mut order = (0..wiring.routes.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| traffic.get(*index) == Some(&baton::State::Idle));
    for index in order.into_iter().rev() {
        let route = &wiring.routes[index];
        let rail = traffic.get(index).copied().unwrap_or(baton::State::Idle);
        let span = circuit::rail_span(route.len());
        // The ASCII column's endpoints are three cells wide — `(*)` and `[w]`
        // — and a wire cell is one column. Drawing them anyway would shear the
        // rest of the row, so as before the rail is what carries the meaning
        // and the endpoints are what gets dropped; their clearance stays, so
        // the rail itself is identical in both columns.
        let conductor = glyphs.get(Glyph::Conductor);
        let bench = glyphs.get(Glyph::WorkerSeated);
        if span.endpoints && conductor.chars().count() == 1 && bench.chars().count() == 1 {
            paint_cell(frame, route[0], conductor, theme.state(Slot::Brain));
            if let Some(end) = route.last() {
                paint_cell(frame, *end, bench, theme.state(Slot::Worker));
            }
        }
        for (offset, cell) in route.iter().enumerate().skip(span.start).take(span.len) {
            let paint = circuit::paint(rail, shape(cell), offset - span.start, span.len, glyphs);
            paint_cell(frame, *cell, paint.symbol, theme.state(paint.slot));
        }
    }
    render_flights(frame, state, wiring, reduced_motion);
}

/// Draw every message in flight over the wiring.
///
/// Last, so a discrete event wins over the ambient pulse underneath it — a
/// worker can be mid-output and receive a dispatch in the same frame, and the
/// dispatch is the news.
fn render_flights(
    frame: &mut Frame<'_>,
    state: &StageState,
    wiring: &circuit::Circuit,
    reduced_motion: bool,
) {
    let theme = state.theme;
    let glyphs = state.glyphs;
    for flight in &state.flights {
        let Some(route) = state
            .panes
            .iter()
            .skip(1)
            .position(|pane| pane.id == flight.worker_id)
            .and_then(|index| wiring.routes.get(index))
        else {
            continue;
        };
        match circuit::flight(reduced_motion, flight.raised.elapsed(), route.len()) {
            circuit::Flight::Travelling(step) => {
                let at = circuit::along(flight.direction, step, route.len());
                if let Some(cell) = route.get(at) {
                    paint_cell(
                        frame,
                        *cell,
                        circuit::packet(flight.direction, glyphs),
                        // `paint_cell` merges modifiers into whatever the cell
                        // already carries, and the rail underneath the packet
                        // is `Slot::Faint`, i.e. DIM. The shipped packet was
                        // therefore drawn `bold+dim` — it is right there in
                        // `stage-message-dispatch.txt`'s legend — which on most
                        // terminals is neither. The packet has to out-contrast
                        // the wire it is crossing, so it clears the dim it
                        // inherited. This is the whole of the "intensity"
                        // half of #49's Decision 2: one bright cell on a dim
                        // rail, and no trail (see `findings.md`).
                        theme
                            .state(flight.outcome.slot())
                            .remove_modifier(Modifier::DIM),
                    );
                }
            }
            circuit::Flight::Landed { .. } if reduced_motion => {
                // No travel: the whole connector holds solid in the message's
                // colour for the same span the packet would have taken. It
                // clears the inherited DIM for the same reason the packet does
                // — more so, in fact: here the connector *is* the message, so
                // leaving it `bold+dim` would smudge the whole of what reduced
                // motion has to say.
                for cell in route {
                    paint_cell(
                        frame,
                        *cell,
                        baton::Cell::Solid.symbol(glyphs),
                        theme
                            .state(flight.outcome.slot())
                            .remove_modifier(Modifier::DIM),
                    );
                }
            }
            circuit::Flight::Landed { .. } | circuit::Flight::Gone => {}
        }
    }
}

/// Write one symbol into one buffer cell, if it is on screen.
fn paint_cell(frame: &mut Frame<'_>, (x, y): (u16, u16), symbol: &str, style: Style) {
    if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
        cell.set_symbol(symbol);
        cell.set_style(style);
    }
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

/// The marker that opens a badge for a spell nothing is listening for.
///
/// `○` is the register's existing unavailable glyph, so an inert badge reads
/// as unavailable at a glance and never as a second kind of live.
fn trigger_inert_mark(glyphs: Glyphs) -> String {
    format!("· {} ", glyphs.get(Glyph::Unavailable))
}

/// The `· ◆ DELEGATE` badge naming every spell detected in a conductor pane,
/// or the empty string when there is none. A glyph and a label, so the trigger
/// is legible with no colour at all.
///
/// When the grammar is not wired the badge says so in words —
/// `· ○ DELEGATE INERT` — rather than looking identical to a working one.
/// Highlighting a spell that nothing is listening for is a claim about a
/// capability that was never probed: on the machine that reported issue #45
/// no `UserPromptSubmit` hook was registered at all, so every `delegate:`
/// lit up and none of them did anything. Unavailable is still shown, never
/// hidden — the conductor did type the word, and that remains true.
fn trigger_badge(triggers: &[Trigger], wired: bool, glyphs: Glyphs) -> String {
    if triggers.is_empty() {
        return String::new();
    }
    let labels = triggers
        .iter()
        .map(|trigger| trigger.label())
        .collect::<Vec<_>>()
        .join(" ");
    if wired {
        format!(" {}{labels}", trigger_badge_mark())
    } else {
        format!(" {}{labels} INERT", trigger_inert_mark(glyphs))
    }
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
    /// A message leaving or landing here, and whether it is still in its
    /// reverse-video flash frame. Transient: it precedes the steady
    /// `confirmed` badge rather than replacing it, which is why they share one
    /// run of title.
    emote: Option<circuit::Emote>,
    focus: bool,
    confirmed: bool,
    phase: usize,
    /// Whether the trigger grammar can actually fire (issue #45). A spell is
    /// still shown when it cannot — unavailable is never hidden — but it is
    /// labelled inert rather than dressed up as working.
    trigger_wired: bool,
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
        emote,
        trigger_wired,
    } = chrome;
    let (trigger_spans, triggers) = conductor_triggers(pane);
    let badge = trigger_badge(&triggers, trigger_wired, glyphs);
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
            // While a message is leaving or landing it owns this run of title;
            // the steady badge underneath returns when the emote's lifetime
            // runs out, so the two never stack.
            if let Some(emote) = emote {
                format!(" · {} {}", emote.symbol(glyphs), emote.label())
            } else if confirmed {
                format!(" · {} TASK CONFIRMED", glyphs.get(Glyph::Confirmed))
            } else {
                String::new()
            },
            badge,
        ))
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(border_color))
        .title_style(if let Some(emote) = emote {
            // The sheet's three beats, as far as a terminal has them: one
            // reverse-video frame, then the glyph settled into a steady badge
            // in the outcome's own slot.
            let settled = theme.state(emote.slot());
            if emote.flashing {
                settled.add_modifier(Modifier::REVERSED)
            } else {
                settled
            }
        } else if confirmed {
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
    // Resolved once for the pane rather than per cell: the tier cannot change
    // mid-frame, and `None` here is the whole of the monochrome behaviour.
    //
    // An inert grammar takes the same `None` path: the shimmer is what makes
    // a spell look *live*, so it is the one part that must not run when
    // nothing is listening. The token keeps its bold, so it stays legible —
    // the badge is where the bad news is delivered, in words.
    let gradient = trigger_wired.then(|| theme.trigger_gradient()).flatten();
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
            // column of the token takes the next stop of the gradient, and
            // `phase` slides it one stop per motion tick so it flows.
            //
            // The colour is the part that degrades. BOLD is unconditional, so
            // the span still reads when there is no colour to spend at all —
            // which is exactly what `Theme::trigger_gradient` answers on the
            // monochrome tier, so `NO_COLOR` really means no colour here rather
            // than nine truecolor cells. The `◆ LABEL` title badge names the
            // spell in words either way.
            if let Some(span) = trigger_spans
                .iter()
                .find(|span| span.row == row && col >= span.col && col < span.col + span.len)
            {
                style = style
                    .add_modifier(Modifier::BOLD)
                    .remove_modifier(Modifier::REVERSED);
                if let Some(stops) = gradient {
                    let offset = usize::from(col - span.col);
                    style = style.fg(stops[(offset + phase) % stops.len()]);
                }
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

    use std::time::{Duration, Instant};

    use super::{
        AVATAR_FRAMES, Drag, HarnessDiscovery, HashMap, HomeData, HomeState, InFlight,
        LeaderAction, LeaderKey, MIN_PANE, NewSessionFlow, RawRouter, RepaintReasons,
        SINGLE_HARNESS_MESSAGE, ScoreState, ShellState, ShellView, SingleHarnessPlan, StageState,
        Theme, ThemeName, baton, circuit, confirmed_panes, cycle_theme, grab, render_help,
        render_home, render_score, render_shell, render_stage, repaint_reasons, route_leader,
        route_raw_mouse, route_runs_key, score_mouse, stage_areas, sync_stage_geometry,
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
                    .draw(|frame| {
                        render_stage(frame, &mut state, None, &[baton::State::Sweeping(1)])
                    })
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

    /// The gradient a truecolor terminal receives, which is the tier every test
    /// renders with unless it is specifically exercising a degradation.
    fn truecolor_gradient() -> [ratatui::style::Color; crate::theme::TRIGGER_STOPS] {
        Theme::from(ThemeName::Nocturne)
            .trigger_gradient()
            .expect("truecolor has a gradient")
    }

    /// The concatenated symbols of every gradient-highlighted trigger cell, in
    /// buffer (row-major) order. A highlighted cell is BOLD with a foreground
    /// drawn from the truecolor gradient; for a single trigger this is exactly
    /// the token, so tests can assert the prompt glyph is never part of the span.
    ///
    /// Truecolor on purpose: below that tier the stops are ordinary ANSI colours
    /// that theme slots also use, so "fg is a gradient stop" would stop meaning
    /// "this cell is a trigger". The tier cases assert on the token's own cells
    /// instead — see `a_trigger_token_degrades_with_the_colour_tier`.
    fn highlighted_symbols(buffer: &ratatui::buffer::Buffer) -> String {
        let gradient = truecolor_gradient();
        buffer
            .content()
            .iter()
            .filter(|cell| cell.modifier.contains(Modifier::BOLD) && gradient.contains(&cell.fg))
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    /// The cells STAGE painted for the one `delegate:` token on screen, found by
    /// the symbols themselves so it works at any tier and any layout. The title
    /// badge spells the label in capitals, so it cannot be matched by mistake.
    fn trigger_token_cells(buffer: &ratatui::buffer::Buffer) -> Vec<ratatui::buffer::Cell> {
        let token: Vec<String> = "delegate:".chars().map(|ch| ch.to_string()).collect();
        let cells: Vec<&ratatui::buffer::Cell> = buffer.content().iter().collect();
        let start = cells
            .windows(token.len())
            .position(|window| {
                window
                    .iter()
                    .zip(&token)
                    .all(|(cell, want)| cell.symbol() == want)
            })
            .expect("the trigger token is on screen");
        cells[start..start + token.len()]
            .iter()
            .map(|cell| (*cell).clone())
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
    fn an_unwired_grammar_is_badged_inert_instead_of_looking_live() {
        // Issue #45 check 11. The in-pane highlight is pure text analysis: it
        // lights up `delegate:` whether or not anything is listening for it.
        // On the machine that reported the issue nothing was — no
        // UserPromptSubmit hook was registered at all — so STAGE spent the
        // session announcing a capability that did not exist.
        //
        // Unavailable is still shown, never hidden: the conductor did type
        // the word. What changes is that the badge says so, in a word, with
        // its own glyph, and the shimmer that makes a spell look live does
        // not run.
        let stream = b"delegate: build the thing\r\n";
        let render = |wired: bool| {
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("test terminal");
            let mut state = StageState::new(
                vec![conductor_pane(stream)],
                ThemeName::Nocturne.into(),
                GLYPHS,
            );
            state.trigger_wired = wired;
            terminal
                .draw(|frame| render_stage(frame, &mut state, None, &[baton::State::Sweeping(1)]))
                .expect("render stage");
            let buffer = terminal.backend().buffer();
            (
                rendered_text(buffer),
                highlighted_symbols(buffer),
                trigger_token_cells(buffer),
            )
        };

        let (wired_text, wired_span, wired_cells) = render(true);
        let (inert_text, inert_span, inert_cells) = render(false);

        assert!(
            wired_text.contains(&super::trigger_badge_mark()),
            "a wired grammar keeps its live badge: {wired_text:?}"
        );
        assert!(
            !wired_text.contains("INERT"),
            "and never claims to be inert: {wired_text:?}"
        );

        assert!(
            inert_text.contains("DELEGATE INERT"),
            "an unwired grammar says so in words: {inert_text:?}"
        );
        assert!(
            inert_text.contains(&super::trigger_inert_mark(GLYPHS)),
            "with the register's unavailable glyph: {inert_text:?}"
        );
        assert!(
            !inert_text.contains(&super::trigger_badge_mark()),
            "and never wears the live marker: {inert_text:?}"
        );

        // The word is still on screen and still emphasised — unavailable is
        // not hidden, and the conductor did type the spell.
        assert_eq!(
            inert_cells.len(),
            "delegate:".chars().count(),
            "the spell the conductor typed is still painted"
        );
        assert!(
            inert_cells
                .iter()
                .all(|cell| cell.modifier.contains(Modifier::BOLD)),
            "and still emphasised, so it reads as a spell, not as prose"
        );

        // What it loses is the shimmer, which is the part that reads as live.
        assert_eq!(
            wired_span, "delegate:",
            "a wired spell slides through the gradient"
        );
        assert!(
            inert_span.is_empty(),
            "an inert one never does: {inert_span:?}"
        );
        assert_eq!(wired_cells.len(), inert_cells.len());

        // Two genuinely different frames, not two descriptions of one.
        assert_ne!(wired_text, inert_text);
    }

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
                            render_stage(frame, &mut state, None, &[baton::State::Sweeping(1)])
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
                    .draw(|frame| {
                        render_stage(frame, &mut state, None, &[baton::State::Sweeping(1)])
                    })
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
                    .draw(|frame| {
                        render_stage(frame, &mut state, None, &[baton::State::Sweeping(1)])
                    })
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
                .draw(|frame| render_stage(frame, &mut state, None, &[baton::State::Sweeping(1)]))
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
        // renders are byte-identical (no animation), and the token carries the
        // BOLD and the glyph + label badge that are the affordance once colour
        // is gone. Uses a real Claude Code prompt prefix (U+276F, as UTF-8
        // bytes) so the fixture matches a live pane, not a bare stream.
        //
        // Note what this does *not* do: every render here is truecolor, so it
        // shows the bold and the badge are present, never that they are all
        // that is left. Reading it as the latter is how a rainbow that ignored
        // the tier survived #13 — that claim is
        // `a_trigger_token_degrades_with_the_colour_tier`'s.
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
                .draw(|frame| render_stage(frame, &mut state, motion, &[baton::State::Sweeping(1)]))
                .expect("render stage");
            terminal.backend().buffer().clone()
        };
        // The highlighted token's per-cell colours, left to right.
        let gradient = truecolor_gradient();
        let token_colours = |buffer: &ratatui::buffer::Buffer| -> Vec<ratatui::style::Color> {
            buffer
                .content()
                .iter()
                .filter(|cell| {
                    cell.modifier.contains(Modifier::BOLD) && gradient.contains(&cell.fg)
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
                .draw(|frame| render_stage(frame, &mut state, None, &[baton::State::Sweeping(1)]))
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
    fn a_trigger_token_degrades_with_the_colour_tier() {
        // Finding 1 of the #13 review, from the render side. `render_pane` used
        // to apply the gradient with no tier check at all, so `NO_COLOR` was
        // sent nine truecolor cells while `theme.rs` claimed the monochrome tier
        // "drops colour entirely", and `TERM=xterm` was sent 24-bit SGR it
        // cannot render. What each tier receives is pinned here on the rendered
        // buffer rather than only on the map: the nine cells of `delegate:`
        // carry exactly that tier's gradient, one stop per column, and they stay
        // BOLD at every tier — including the one with no colour to spend.
        for tier in [
            ColorTier::TrueColor,
            ColorTier::Ansi256,
            ColorTier::Ansi16,
            ColorTier::Monochrome,
        ] {
            let theme = Theme::new(ThemeName::Nocturne, tier);
            let backend = TestBackend::new(120, 40);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            let mut state =
                StageState::new(vec![conductor_pane(b"delegate: go\r\n")], theme, GLYPHS);
            terminal
                .draw(|frame| render_stage(frame, &mut state, None, &[baton::State::Sweeping(1)]))
                .expect("render stage");
            let buffer = terminal.backend().buffer();
            let token = trigger_token_cells(buffer);
            assert_eq!(token.len(), 9, "{tier:?}: `delegate:` is nine cells");
            for (column, cell) in token.iter().enumerate() {
                assert!(
                    cell.modifier.contains(Modifier::BOLD),
                    "{tier:?}: column {column} lost the bold that carries the token"
                );
            }
            if let Some(stops) = theme.trigger_gradient() {
                let painted: Vec<_> = token.iter().map(|cell| cell.fg).collect();
                let wanted: Vec<_> = (0..token.len())
                    .map(|column| stops[column % stops.len()])
                    .collect();
                assert_eq!(
                    painted, wanted,
                    "{tier:?}: the token is not this tier's gradient, one stop per column"
                );
            } else {
                for (column, cell) in token.iter().enumerate() {
                    assert_eq!(
                        super::theme::describe(cell.fg),
                        "reset",
                        "{tier:?}: column {column} still emits a foreground colour"
                    );
                    assert_eq!(
                        super::theme::describe(cell.bg),
                        "reset",
                        "{tier:?}: column {column} still emits a background colour"
                    );
                }
            }
            // And the badge names the spell in words at every tier — what a
            // monochrome terminal reads instead of a colour.
            let text = rendered_text(buffer);
            assert!(
                text.contains(&super::trigger_badge_mark()),
                "{tier:?}: missing glyph badge"
            );
            assert!(text.contains("DELEGATE"), "{tier:?}: missing label badge");
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

        // STAGE carries the trigger grammar, and it is the screen this
        // assertion used to skip: it ran over SCORE and HOME only, and the
        // STAGE fixtures wrote `codex ready`, which contains no trigger. Nine
        // truecolor cells were being emitted under `NO_COLOR` with nothing
        // watching. A live trigger is now inside the "nothing on screen carries
        // colour" net, so it cannot come back quietly.
        let mut stage = StageState::new(
            vec![conductor_pane(b"delegate: add OAuth login\r\n")],
            theme,
            GLYPHS,
        );
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("mono STAGE terminal");
        terminal
            .draw(|frame| render_stage(frame, &mut stage, None, &[baton::State::Sweeping(2)]))
            .expect("render mono STAGE");
        let buffer = terminal.backend().buffer().clone();
        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| super::theme::describe(cell.fg) == "reset"
                    && super::theme::describe(cell.bg) == "reset"),
            "a NO_COLOR STAGE showing a live trigger emitted colour"
        );
        let token = trigger_token_cells(&buffer);
        assert_eq!(token.len(), 9, "the trigger token is on the mono screen");
        assert!(
            token
                .iter()
                .all(|cell| cell.modifier.contains(Modifier::BOLD)),
            "with no colour left, bold is what carries the trigger — and it is gone"
        );
        let stage_text = rendered_text(&buffer);
        assert!(
            stage_text.contains(&super::trigger_badge_mark()),
            "the glyph badge is the other half of a colourless trigger"
        );
        assert!(
            stage_text.contains("DELEGATE"),
            "…and it must still be named in words"
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
    ///
    /// Matched on a maximal run of exactly [`baton::CELLS`] rail symbols, not
    /// on "this row contains a rail character". Both looser rules pick the
    /// wrong row: `·` is also a pane title's separator and `─` is also the
    /// pane border, and both sit above the rail, so a `find` on either would
    /// return a row that merely looks right. Bounding the run at exactly
    /// twelve is what separates the rail from a border that runs the full
    /// width of a pane. Unicode register only — every caller renders with it.
    fn baton_row(buffer: &ratatui::buffer::Buffer, width: u16) -> String {
        let is_rail = |ch: char| "▓▒░─·━".contains(ch);
        buffer
            .content()
            .chunks(usize::from(width))
            .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect())
            .find(|row: &String| {
                row.split(|ch| !is_rail(ch))
                    .any(|run| run.chars().count() == baton::CELLS)
            })
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
                .draw(|frame| render_stage(frame, &mut stage, None, &[state]))
                .expect("render baton");
            assert!(
                baton_row(terminal.backend().buffer(), width).contains(want),
                "{state:?}: STAGE did not paint {want:?}"
            );
        }
    }

    /// A conductor plus `count` workers, so the topology can be exercised at
    /// the worker counts AC1 names.
    fn bench(count: usize) -> Vec<PaneSnapshot> {
        let mut all = panes();
        let worker = all[1].clone();
        all.truncate(1);
        for index in 0..count {
            let mut next = worker.clone();
            next.id = format!("pane-{}", index + 1);
            next.title = format!("hermes-{index}");
            all.push(next);
        }
        all
    }

    /// Pulse one pane's connector, by index into `stage.panes`.
    fn pulse(stage: &mut StageState, index: usize) {
        let id = stage.panes[index].id.clone();
        stage.mark_output(&id);
    }

    /// Advance one pane's decay timer past the silence window.
    fn decay(stage: &mut StageState, index: usize) {
        let id = stage.panes[index].id.clone();
        if let Some(pulse) = stage.pulses.get_mut(&id) {
            let _ = pulse.pulse.process(baton::DECAY + Duration::from_millis(1));
        }
    }

    /// One pane's own rail state.
    fn rail(stage: &StageState, index: usize, reduced_motion: bool) -> baton::State {
        stage.baton_state_for(&stage.panes[index].id, reduced_motion)
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
                .draw(|frame| render_stage(frame, &mut stage, None, &[state]))
                .expect("render baton");
            baton_row(terminal.backend().buffer(), width)
        };
        let mut stage = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        pulse(&mut stage, 1);
        assert_eq!(
            rail(&stage, 1, true),
            baton::State::Steady,
            "a live pane under reduced motion gets the solid rail"
        );
        assert_eq!(render(rail(&stage, 1, true)), render(baton::State::Steady));
        // Full motion at the same instant is a travelling packet instead.
        assert!(matches!(rail(&stage, 1, false), baton::State::Sweeping(_)));
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
        pulse(&mut stage, 1);
        assert!(matches!(rail(&stage, 1, false), baton::State::Sweeping(_)));
        // Advancing past the decay window with no further output idles it.
        decay(&mut stage, 1);
        assert_eq!(rail(&stage, 1, false), baton::State::Idle);
        assert_eq!(rail(&stage, 1, true), baton::State::Idle);
        // A new snapshot with a fresh sequence is an output tick.
        let mut next = panes();
        next[1].sequence = 2;
        stage.apply_snapshot(next);
        assert!(matches!(rail(&stage, 1, false), baton::State::Sweeping(_)));
    }

    #[test]
    fn output_from_one_worker_animates_that_workers_connector_and_no_other() {
        // AC2, and the whole point of the topology work: the shipped code
        // raised one global pulse if *any* pane's sequence moved, so even the
        // single rail could not say who was talking.
        let mut stage = StageState::new(bench(3), ThemeName::Nocturne.into(), GLYPHS);
        for index in 1..4 {
            decay(&mut stage, index);
        }
        assert!(
            stage
                .traffic(false)
                .iter()
                .all(|state| *state == baton::State::Idle),
            "every wire starts quiet"
        );

        // Advance only worker 2's sequence, exactly as the acceptance check
        // specifies.
        let mut next = bench(3);
        next[2].sequence = 7;
        stage.apply_snapshot(next);

        let traffic = stage.traffic(false);
        assert!(
            matches!(traffic[1], baton::State::Sweeping(_)),
            "worker 2's connector carries its own output: {traffic:?}"
        );
        assert_eq!(traffic[0], baton::State::Idle, "worker 1 stays quiet");
        assert_eq!(traffic[2], baton::State::Idle, "worker 3 stays quiet");
    }

    #[test]
    fn a_swapped_or_departed_pane_never_inherits_another_panes_traffic() {
        // Attribution is keyed by pane id, not by index, because `s` swaps two
        // panes and a session can gain or lose one. Index-keyed state would
        // silently hand worker 1's live pulse to whoever landed in slot 1.
        let mut stage = StageState::new(bench(3), ThemeName::Nocturne.into(), GLYPHS);
        for index in 1..4 {
            decay(&mut stage, index);
        }
        pulse(&mut stage, 1);
        assert!(matches!(stage.traffic(false)[0], baton::State::Sweeping(_)));

        // Worker 1 leaves; the other two shuffle up a slot.
        let mut next = bench(3);
        next.remove(1);
        for pane in &mut next {
            pane.sequence = 1;
        }
        stage.apply_snapshot(next);

        let traffic = stage.traffic(false);
        assert_eq!(traffic.len(), 2, "two workers left");
        assert!(
            traffic.iter().all(|state| *state == baton::State::Idle),
            "the departed worker's pulse went with it: {traffic:?}"
        );
        assert!(
            !stage.pulses.contains_key("pane-1"),
            "and left no state behind to be reattributed"
        );
    }

    /// Draw one shell frame through the real render path and return the rail
    /// row it painted. Going through `render_shell` rather than `render_stage`
    /// is the point: that is where the painted state is recorded, so the loop's
    /// repaint decision is being fed by an actual paint.
    fn paint_rail(shell: &mut ShellState, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 40)).expect("rail terminal");
        terminal
            .draw(|frame| render_shell(frame, shell))
            .expect("render shell");
        baton_row(terminal.backend().buffer(), width)
    }

    #[test]
    fn the_idle_rail_is_painted_not_merely_computed_when_output_stops() {
        // Carry-over from #13, fix 4. The decay was computed correctly and
        // never rendered: on the frame `pulse.done()` flipped, every "still
        // moving" reason went false at once and `redraw` had been cleared
        // after the previous draw, so the loop drew nothing and left a packet
        // stranded mid-sweep. The next burst then restarted it at frame 0.
        //
        // Asserted on the rendered buffer. A test that only checked
        // `baton_state()`'s return value passed before this fix.
        let width = 120;
        let mut shell = stage_shell(panes(), ThemeName::Nocturne, false);
        pulse(shell.stage.as_mut().expect("stage"), 1);

        let live = paint_rail(&mut shell, width);
        assert!(
            live.contains('▓'),
            "output is flowing, so the rail carries the packet: {live:?}"
        );

        // 400 ms of silence. This is the frame the bug dropped.
        decay(shell.stage.as_mut().expect("stage"), 1);
        let reasons = repaint_reasons(&shell, false);
        assert_eq!(
            reasons,
            RepaintReasons {
                stage_changed: true,
                ..RepaintReasons::default()
            },
            "the decay frame must be drawn, and `stage_changed` must be the only \
             reason asking for it — every other reason means 'still moving', \
             which is exactly what has just stopped being true"
        );
        assert!(reasons.draw());

        let decayed = paint_rail(&mut shell, width);
        assert!(
            decayed.contains("◆ ············ ●"),
            "the rail decays to the dim dotted base on screen: {decayed:?}"
        );

        // Having painted it, the loop settles instead of spinning on it.
        let settled = repaint_reasons(&shell, false);
        assert!(
            !settled.draw(),
            "the idle rail is painted once: {settled:?}"
        );
        assert_eq!(settled.wait(), Duration::from_secs(30));
    }

    #[test]
    fn reduced_motion_repaints_the_steady_to_idle_transition_too() {
        // The same hole, with no animation cadence that could accidentally
        // cover it: under reduced motion the rail only ever has two forms, and
        // switching between them is the whole of its motion.
        let width = 120;
        let mut shell = stage_shell(panes(), ThemeName::Nocturne, true);
        pulse(shell.stage.as_mut().expect("stage"), 1);

        let live = paint_rail(&mut shell, width);
        assert!(
            live.contains("◆ ━━━━━━━━━━━━ ●"),
            "live under reduced motion is the solid accent rail: {live:?}"
        );
        assert!(
            !repaint_reasons(&shell, false).animating,
            "reduced motion never animates"
        );

        decay(shell.stage.as_mut().expect("stage"), 1);
        assert!(
            repaint_reasons(&shell, false).stage_changed,
            "Steady -> Idle owes the screen a frame under reduced motion too"
        );
        let decayed = paint_rail(&mut shell, width);
        assert!(
            decayed.contains("◆ ············ ●"),
            "and it is painted: {decayed:?}"
        );
    }

    #[test]
    fn a_trigger_animates_the_rainbow_but_never_the_rail() {
        // Pinning the second half of the carry-over note. `has_live_trigger`
        // reads what a conductor pane *displays*; the rail reports traffic a
        // pane *produced*. They are different signals, so a pane sitting on a
        // `delegate:` line with no output decays to the idle rail like any
        // other silent pane, and the trigger keeps only its own 120 ms cadence
        // rather than holding the whole shell at the baton's 16 ms.
        let width = 120;
        let keyword = Trigger::ALL[0].keyword();
        let stream = format!("{keyword}: build the thing\r\n");
        let bench = panes().remove(1);
        let mut shell = stage_shell(
            vec![conductor_pane(stream.as_bytes()), bench],
            ThemeName::Nocturne,
            false,
        );
        pulse(shell.stage.as_mut().expect("stage"), 1);

        // Let the pulse decay while the trigger stays on screen.
        let _ = paint_rail(&mut shell, width);
        decay(shell.stage.as_mut().expect("stage"), 1);
        assert_eq!(
            rail(shell.stage.as_ref().expect("stage"), 1, false),
            baton::State::Idle,
            "a trigger is not traffic: the rail is idle"
        );

        let mut terminal = Terminal::new(TestBackend::new(width, 40)).expect("rail terminal");
        terminal
            .draw(|frame| render_shell(frame, &mut shell))
            .expect("render shell");
        let buffer = terminal.backend().buffer();
        assert_eq!(
            highlighted_symbols(buffer),
            format!("{keyword}:"),
            "the rainbow is still lit, so the trigger really is live"
        );
        assert!(
            baton_row(buffer, width).contains("◆ ············ ●"),
            "…and the rail beside it is idle all the same"
        );

        let reasons = repaint_reasons(&shell, false);
        assert_eq!(
            reasons,
            RepaintReasons {
                trigger_ambient: true,
                ..RepaintReasons::default()
            },
            "the trigger is the only thing still asking for frames"
        );
        assert_eq!(
            reasons.wait(),
            Duration::from_millis(120),
            "one colour per 120 ms is all the rainbow steps, so a trigger must \
             not hold the loop at the baton's 16 ms cadence"
        );
    }

    #[test]
    fn a_trigger_asks_for_no_frames_when_there_is_no_gradient_to_slide() {
        // The consequence of gating the gradient on the tier, on the loop side.
        // `trigger_ambient` exists to hold the shell at 120 ms so the gradient
        // can step one stop per tick. With no stops there is nothing to step,
        // and repainting a token that cannot change is precisely the wasted spin
        // `any_live`'s doc calls out. Same fixture as the test above, so the
        // only difference between the two halves is the colour tier.
        let width = 120;
        let stream = format!("{}: build the thing\r\n", Trigger::ALL[0].keyword());
        let bench = panes().remove(1);
        let mut shell = stage_shell(
            vec![conductor_pane(stream.as_bytes()), bench],
            ThemeName::Nocturne,
            false,
        );
        pulse(shell.stage.as_mut().expect("stage"), 1);
        let _ = paint_rail(&mut shell, width);
        decay(shell.stage.as_mut().expect("stage"), 1);
        let _ = paint_rail(&mut shell, width);
        assert_eq!(
            repaint_reasons(&shell, false),
            RepaintReasons {
                trigger_ambient: true,
                ..RepaintReasons::default()
            },
            "with colour, the trigger is the one thing still asking for frames"
        );

        // The same screen on a terminal that resolved to no colour at all. The
        // client sets both themes from one resolved value, so both move.
        let mono = Theme::new(ThemeName::Nocturne, ColorTier::Monochrome);
        shell.theme = mono;
        shell.stage.as_mut().expect("stage").theme = mono;
        let reasons = repaint_reasons(&shell, false);
        assert_eq!(
            reasons,
            RepaintReasons::default(),
            "nothing on this screen can change, so nothing may ask for a frame"
        );
        assert!(!reasons.draw());
        assert_eq!(
            reasons.wait(),
            Duration::from_secs(30),
            "a bold token with no gradient must not hold the loop at 120 ms"
        );
    }

    #[test]
    fn runs_watcher_wakes_on_registry_change_without_polling() {
        let root = std::env::temp_dir().join(format!("orc-app-runs-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (sender, receiver) = std::sync::mpsc::sync_channel(4);
        super::spawn_change_watch(
            root.join("runs"),
            "runs",
            || super::UiEvent::RunsChanged,
            sender,
        );
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
                // Edge-resize is new in #38 and has no keybinding to discover
                // it by, so help is the only place a user could find it.
                assert!(
                    text.contains("an edge or corner to resize"),
                    "help must teach mouse resize ({width}x{height})"
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

    /// A task with the given `(action, to)` history, assigned to `pane`.
    ///
    /// The pairs are the words `orc-core` really writes — see
    /// `tests/task_vocabulary.rs`. Spelling them out rather than passing bare
    /// action words is deliberate: completion is a `moved` transition whose
    /// meaning lives entirely in `to`, and the first version of these tests
    /// invented four action words that nothing writes.
    const CREATED: (&str, Option<&str>) = ("created", Some("backlog"));
    const ASSIGNED: (&str, Option<&str>) = ("assigned", None);
    /// The brief reaching the worker. Written at `command.spawn()`, so it is
    /// the outbound journey landing — not the answer coming back, which is
    /// what it was read as until issue #49.
    const CONFIRMED: (&str, Option<&str>) = ("delivery_confirmed", None);
    /// The worker exited and its answer is durable: the real return.
    const ANSWERED: (&str, Option<&str>) = ("execution_succeeded", None);
    const DONE: (&str, Option<&str>) = ("moved", Some("done"));

    fn task_with(id: &str, pane: &str, actions: &[(&str, Option<&str>)]) -> TaskSummary {
        TaskSummary {
            id: id.to_owned(),
            title: "brief".to_owned(),
            status: "running".to_owned(),
            assignee: Some("hermes".to_owned()),
            assignee_run: Some(pane.to_owned()),
            isolated: false,
            isolation: None,
            blocked: false,
            tokens: None,
            diff: None,
            history: actions
                .iter()
                .map(|(action, to)| TaskHistorySummary {
                    at: "2026-07-29T09:00:00Z".to_owned(),
                    actor: "brain".to_owned(),
                    action: (*action).to_owned(),
                    to: to.map(str::to_owned),
                })
                .collect(),
        }
    }

    /// The whole STAGE buffer as text, at a size that routes.
    fn stage_text(state: &mut StageState, reduced_motion: bool) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("stage terminal");
        let traffic = state.traffic(reduced_motion);
        let motion = (!reduced_motion).then_some(0);
        terminal
            .draw(|frame| render_stage(frame, state, motion, &traffic))
            .expect("render stage");
        rendered_text(terminal.backend().buffer())
    }

    #[test]
    fn the_narrow_fallback_says_so_instead_of_only_looking_different() {
        // AC8 asks that connectors "either render or are replaced by a stated
        // fallback". `Routing` recorded which tier the router reached and its
        // doc claimed the UI could "say so", but nothing outside the router's
        // own tests ever read it — the same claims-a-capability shape #13 was
        // pulled up on. It is now read, and this is what reads it.
        let wide = ratatui::layout::Rect::new(0, 0, 120, 40);
        let narrow = ratatui::layout::Rect::new(0, 0, 80, 24);
        let legend_of = |area: ratatui::layout::Rect| {
            let mut state = StageState::new(bench(2), ThemeName::Nocturne.into(), GLYPHS);
            let mut terminal =
                Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
            let traffic = state.traffic(false);
            terminal
                .draw(|frame| render_stage(frame, &mut state, Some(0), &traffic))
                .expect("render");
            (state.routing(), rendered_text(terminal.backend().buffer()))
        };

        let (routing, text) = legend_of(wide);
        assert_eq!(
            routing,
            Some(circuit::Routing::Elbows),
            "wide enough to route"
        );
        assert!(
            !text.contains("too narrow to route"),
            "a routed stage must not claim it fell back"
        );

        let (routing, text) = legend_of(narrow);
        assert_eq!(
            routing,
            Some(circuit::Routing::Inlaid),
            "no gutter to route"
        );
        assert!(
            text.contains("connectors inlaid — too narrow to route"),
            "the fallback has to be stated, not merely visible: {text:?}"
        );
    }

    #[test]
    fn a_dispatch_a_return_and_a_plain_output_tick_look_different() {
        // AC5. `mark_output` used to treat a task event exactly as a stdout
        // tick — "a task event is traffic on the filament too, so it pulses
        // the baton exactly as a stdout tick does" — so a dispatch, a returned
        // result and a worker merely printing produced identical frames.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        state.pane_areas = stage_areas(ratatui::layout::Rect::new(0, 0, 120, 40), &state);

        // 1. A plain stdout tick: the ambient three-cell ramp, nothing else.
        pulse(&mut state, 1);
        let output = stage_text(&mut state, false);
        assert!(output.contains('▓'), "the ambient packet");
        assert!(
            !output.contains('▶') && !output.contains('◀'),
            "output is not a message: {output:?}"
        );
        assert!(!output.contains("TASK DISPATCHED"));

        // 2. A dispatch: outbound, one directional cell, and its own emote.
        state.note_task_events(&[task_with("T1", "pane-1", &[CREATED])]);
        state.note_task_events(&[task_with("T1", "pane-1", &[CREATED, ASSIGNED])]);
        let dispatched = stage_text(&mut state, false);
        assert!(dispatched.contains('▶'), "outbound packet: {dispatched:?}");
        assert!(!dispatched.contains('◀'), "and not the inbound one");

        // 2b. Delivery confirmed is the *same* journey landing, not a return:
        //     it is written at `command.spawn()`. #49.
        state.flights.clear();
        state.note_task_events(&[task_with("T1", "pane-1", &[CREATED, ASSIGNED, CONFIRMED])]);
        let received = stage_text(&mut state, false);
        assert!(
            received.contains('▶') && !received.contains('◀'),
            "a delivery receipt still travels towards the worker: {received:?}"
        );

        // 3. The answer coming back: inbound, the other direction, and only
        //    once the worker has actually finished.
        state.flights.clear();
        state.note_task_events(&[task_with(
            "T1",
            "pane-1",
            &[CREATED, ASSIGNED, CONFIRMED, ANSWERED],
        )]);
        let returned = stage_text(&mut state, false);
        assert!(returned.contains('◀'), "inbound packet: {returned:?}");
        assert!(!returned.contains('▶'), "and not the outbound one");

        // Three genuinely different frames, not three descriptions of one.
        assert_ne!(output, dispatched);
        assert_ne!(dispatched, returned);
        assert_ne!(output, returned);
    }

    #[test]
    fn the_brain_shows_the_hand_off_leaving_and_the_worker_shows_the_answer_leaving() {
        // Issue #49, spec step 1: the departure beat, mirroring the arrival
        // emote that already existed. A dispatch leaves the conductor and an
        // answer leaves the worker, and each says so on the pane it is
        // leaving — so a delegation reads as a hand-off rather than as
        // something that simply appears somewhere else.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        state.pane_areas = stage_areas(ratatui::layout::Rect::new(0, 0, 120, 40), &state);

        let leaving = |state: &mut StageState, direction, outcome| {
            state.flights = vec![InFlight {
                worker_id: "pane-1".to_owned(),
                direction,
                outcome,
                raised: Instant::now(),
            }];
            let _ = stage_text(state, false);
            state.emotes(false)
        };

        let out = leaving(
            &mut state,
            circuit::Direction::Outbound,
            circuit::Outcome::Dispatched,
        );
        assert_eq!(
            out.get("pane-0").map(|emote| emote.label()),
            Some("HANDING OFF"),
            "the conductor says the brief is leaving it: {out:?}"
        );
        assert!(
            !out.contains_key("pane-1"),
            "and the worker says nothing until it arrives: {out:?}"
        );

        let back = leaving(
            &mut state,
            circuit::Direction::Inbound,
            circuit::Outcome::Confirmed,
        );
        assert_eq!(
            back.get("pane-1").map(|emote| emote.label()),
            Some("ANSWERING"),
            "and the answer leaves the worker that produced it: {back:?}"
        );

        // It is bounded by the packet's own travel, never by a hold of its
        // own: once the packet has crossed the sheet's twelve-cell rail's
        // worth of route the beat is over, and by the time it lands the
        // origin is quiet again.
        state.flights = vec![InFlight {
            worker_id: "pane-1".to_owned(),
            direction: circuit::Direction::Outbound,
            outcome: circuit::Outcome::Dispatched,
            raised: Instant::now()
                - Duration::from_millis(circuit::DEPART_CELLS as u64 * circuit::FLIGHT_MS_PER_CELL),
        }];
        let _ = stage_text(&mut state, false);
        let settled = state.emotes(false);
        assert_ne!(
            settled.get("pane-0").map(|emote| emote.label()),
            Some("HANDING OFF"),
            "the beat does not outlive the crossing it announces: {settled:?}"
        );
    }

    #[test]
    fn an_answer_never_overtakes_the_brief_it_is_answering() {
        // Travel time is a function of the wire, so on a long connector a
        // genuinely fast worker answers while its own brief is still drawn
        // mid-flight — and then a ▶ and a ◀ cross in opposite directions on
        // one wire, which is the picture issue #49 opens with. The times are
        // real; the brief in transit is not, because an answer coming back is
        // proof that it arrived.
        let mut state = StageState::new(bench(3), ThemeName::Nocturne.into(), GLYPHS);
        state.seed_task_events(&[]);
        let _ = stage_text(&mut state, false);
        let len = state.route_len("pane-1");
        assert!(
            len > 12,
            "the fixture needs a wire long enough for the overtake: {len}"
        );

        // The brief goes out...
        state.note_task_events(&[task_with("T1", "pane-1", &[CREATED, ASSIGNED, CONFIRMED])]);
        assert!(
            state
                .flights
                .iter()
                .any(|flight| flight.direction == circuit::Direction::Outbound),
            "the hand-off is on the wire"
        );

        // ...and the worker answers long before the packet could have crossed.
        state.note_task_events(&[task_with(
            "T1",
            "pane-1",
            &[CREATED, ASSIGNED, CONFIRMED, ANSWERED],
        )]);
        let text = stage_text(&mut state, false);
        assert!(
            text.contains('◀'),
            "the answer is on its way back: {text:?}"
        );
        assert!(
            !text.contains('▶'),
            "and the brief is no longer shown still crossing towards a worker              that has already answered: {text:?}"
        );
        // It *landed* rather than vanishing: the outbound flight is still
        // there, holding its arrival, so nothing is silently deleted. (The
        // worker's card reads "ANSWERING" rather than the brief's arrival,
        // because the newest news on a pane wins — that rule is unchanged.)
        let outbound = state
            .flights
            .iter()
            .find(|flight| flight.direction == circuit::Direction::Outbound)
            .expect("the brief is still on the board");
        assert!(
            matches!(
                circuit::flight(false, outbound.raised.elapsed(), len),
                circuit::Flight::Landed { .. }
            ),
            "the brief arrives rather than disappearing"
        );
    }

    #[test]
    fn the_packet_is_one_cell_and_draws_no_trail() {
        // Issue #49's Decision 2, made enforceable. `visual-identity.md`
        // defines the packet as "a single directional cell, not the pulse's
        // three-cell ramp", and makes shape one of three legs — with colour,
        // with behaviour — any one of which must be removable while leaving
        // the packet and the ambient ramp tellable apart. With colour gone
        // (monochrome, NO_COLOR) and behaviour needing time to read, shape is
        // the only instantaneous discriminator left, so a trail spends the one
        // leg that has to survive. The decision was to get smoothness from
        // cadence instead; this is what stops a trail arriving later without
        // that argument being made again.
        //
        // The shipped `the_message_vocabulary_survives_no_color_and_the_ascii_column`
        // could not catch this: it asserts on `circuit::packet`'s return value
        // and never counts painted cells.
        //
        // Counted as *cells the flight changed*, not as occurrences of the
        // packet's own glyph. Counting the glyph would have been the same
        // mistake one level down: the trail the issue actually names is drawn
        // from the ramp (`▓▒░·─━`), so a glyph count would sit at one with a
        // comet on the wire. The whole route is rendered twice — once with the
        // flight and once without — and exactly one cell may differ, whatever
        // is painted in it.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        state.pane_areas = stage_areas(ratatui::layout::Rect::new(0, 0, 120, 40), &state);
        // The control render and the carrying ones are taken at different
        // wall-clock instants, and a freshly-built `PanePulse` is *live* — the
        // ambient `▓▒░` ramp sweeps along the rail, so under parallel test load
        // it moves between them and the diff counts the rail's own motion as a
        // trail. Review caught this failing ~1 run in 3 of the full workspace.
        // Decaying every pane first makes the rail static, which is what makes
        // "exactly one cell differs" a statement about the packet at all.
        for index in 0..state.panes.len() {
            decay(&mut state, index);
        }
        let wire = |state: &mut StageState| {
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("stage terminal");
            let traffic = state.traffic(false);
            terminal
                .draw(|frame| render_stage(frame, state, Some(0), &traffic))
                .expect("render stage");
            let buffer = terminal.backend().buffer();
            circuit::plan(&state.pane_areas).expect("wired").routes[0]
                .iter()
                .map(|cell| {
                    buffer
                        .cell(*cell)
                        .map(|painted| painted.symbol().to_owned())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        };

        state.flights.clear();
        let quiet = wire(&mut state);
        for elapsed in [0_u64, 30, 90, 150, 210, 300] {
            for direction in [circuit::Direction::Outbound, circuit::Direction::Inbound] {
                state.flights = vec![InFlight {
                    worker_id: "pane-1".to_owned(),
                    direction,
                    outcome: circuit::Outcome::Dispatched,
                    raised: Instant::now() - Duration::from_millis(elapsed),
                }];
                let carrying = wire(&mut state);
                let changed = quiet
                    .iter()
                    .zip(&carrying)
                    .filter(|(before, after)| before != after)
                    .count();
                assert_eq!(
                    changed, 1,
                    "{direction:?} at {elapsed} ms must change exactly one cell of \
                     the connector — a trail of any glyph spends the shape leg \
                     the design sheet requires. quiet={quiet:?} carrying={carrying:?}"
                );
            }
        }
    }

    #[test]
    fn traffic_aimed_at_a_run_that_is_not_a_pane_is_stated_rather_than_dropped() {
        // Issue #49's acceptance check 7. Dispatch falls back to a run or a
        // dispatch id whenever no seated pane matches the harness, so a
        // delegation can be entirely real and entirely elsewhere. The flight
        // raised for one was aimed at nothing and `retire_flights` dropped it
        // on the first frame: no animation, no error, and a user who reasonably
        // concluded the worker on their screen had done the work.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        state.pane_areas = stage_areas(ratatui::layout::Rect::new(0, 0, 120, 40), &state);
        state.note_task_events(&[task_with("T1", "D-0007", &[CREATED, ASSIGNED])]);
        assert!(
            state.flights.is_empty(),
            "no packet is raised for a wire that does not exist"
        );
        assert_eq!(
            state.offstage.len(),
            1,
            "one note per message it could not show"
        );
        let text = stage_text(&mut state, false);
        assert!(
            text.contains("crossed no wire") && text.contains("D-0007"),
            "STAGE says so, and names what it could not aim at: {text:?}"
        );

        // A whole lifecycle arriving in one board read is three messages, and
        // the legend has to say three: a single note per batch would
        // undercount exactly the traffic this exists to admit to.
        state.offstage.clear();
        state.seen_history.clear();
        state.note_task_events(&[task_with(
            "T2",
            "D-0008",
            &[CREATED, ASSIGNED, CONFIRMED, ANSWERED],
        )]);
        assert_eq!(state.offstage.len(), 3, "aimed, taken and answered");
        assert!(
            stage_text(&mut state, false).contains("3 messages crossed no wire"),
            "and the legend counts messages, not tasks"
        );

        // And it is as transient as the packet it replaced, so the legend does
        // not carry a stale claim.
        state.offstage.clear();
        state.note_task_events(&[task_with("T3", "D-0009", &[CREATED, ASSIGNED])]);
        state.offstage = vec![("D-0007".to_owned(), Instant::now() - circuit::EMOTE_HOLD)];
        state.retire_flights(false);
        assert!(
            !stage_text(&mut state, false).contains("crossed no wire"),
            "the note leaves with the traffic it stood in for"
        );
    }

    #[test]
    fn a_delivery_receipt_does_not_take_the_confirmed_badge_off_when_the_worker_finishes() {
        // `confirmed_panes` read `history.last()`, which only ever worked
        // because `delivery_confirmed` happened to be the last durable word a
        // dispatch wrote. #49 appends `execution_succeeded` after it, so the
        // shipped lookup would have taken the `✓ TASK CONFIRMED` badge off
        // every pane on the stage — silently, with no test failing.
        let entry = |action: &str| TaskHistorySummary {
            at: "2026-07-31T09:00:00Z".to_owned(),
            actor: "brain".to_owned(),
            action: action.to_owned(),
            to: None,
        };
        let mut task = task_with("T1", "pane-1", &[CREATED, ASSIGNED, CONFIRMED]);
        assert!(confirmed_panes(std::slice::from_ref(&task)).contains("pane-1"));

        task.history.push(entry("execution_succeeded"));
        assert!(
            confirmed_panes(std::slice::from_ref(&task)).contains("pane-1"),
            "the worker finishing does not un-deliver its brief"
        );

        // But a worker that took the brief and then *failed* must not keep a
        // ✓ TASK CONFIRMED badge. Its `delivery_confirmed` is still in history
        // and always will be, so a rule that looked only at deliveries would
        // leave the badge sitting on a pane whose work died.
        let mut failed = task_with("T2", "pane-1", &[CREATED, ASSIGNED, CONFIRMED]);
        failed.history.push(entry("execution_failed"));
        assert!(
            !confirmed_panes(std::slice::from_ref(&failed)).contains("pane-1"),
            "a delivered worker that then failed does not read as confirmed"
        );

        // A failed delivery does too, which is the case the lookup exists for.
        task.history.push(entry("delivery_failed"));
        assert!(
            !confirmed_panes(std::slice::from_ref(&task)).contains("pane-1"),
            "a failed delivery takes the badge back"
        );
    }

    #[test]
    fn a_message_with_no_wire_asks_for_the_frame_that_takes_its_note_away() {
        // The off-stage note stands in for a packet, so it has to leave when
        // that packet would have. Nothing else on a quiet stage asks for a
        // frame — that is the whole scenario — so without a repaint reason of
        // its own the note is painted once and then stranded until the next
        // unrelated event, which on the 30 s tier means half a minute of STAGE
        // claiming a message is in the air that left long ago.
        let mut shell = stage_shell(panes(), ThemeName::Nocturne, false);
        // Let the panes fall silent first: a freshly-built pulse is live, and
        // "every pane quiet" is the whole scenario.
        settle(&mut shell, 120);
        let quiet = repaint_reasons(&shell, false);
        assert!(!quiet.draw(), "nothing is happening: {quiet:?}");
        assert_eq!(quiet.wait(), Duration::from_secs(30));

        shell
            .stage
            .as_mut()
            .expect("stage")
            .offstage
            .push(("D-0007".to_owned(), Instant::now()));
        let noting = repaint_reasons(&shell, false);
        assert!(noting.offstage, "the note is a reason: {noting:?}");
        assert!(noting.draw());
        assert!(
            noting.wait() <= circuit::EMOTE_HOLD,
            "and the loop must look again inside the note's own lifetime, not \
             after it: {:?}",
            noting.wait()
        );
    }

    #[test]
    fn a_landed_emote_does_not_hold_the_shell_at_the_travel_cadence() {
        // `in_flight` is true for the whole of EMOTE_HOLD — 1.2 s in which the
        // only boundary is the 90 ms flash — and under reduced motion
        // `circuit::flight` is `Landed` from frame 0 with the flash suppressed,
        // so nothing on screen can change at all. Running the whole shell,
        // hosted panes included, at the packet's 15 ms cadence through that is
        // the wasted spin `any_live`'s doc calls out.
        let flight = |ago: Duration| InFlight {
            worker_id: "pane-1".to_owned(),
            direction: circuit::Direction::Outbound,
            outcome: circuit::Outcome::Dispatched,
            raised: Instant::now() - ago,
        };

        let mut shell = stage_shell(panes(), ThemeName::Nocturne, false);
        settle(&mut shell, 120);
        shell.stage.as_mut().expect("stage").flights = vec![flight(Duration::ZERO)];
        let _ = paint_rail(&mut shell, 120);
        let moving = repaint_reasons(&shell, false);
        assert!(moving.travelling, "a packet is crossing: {moving:?}");
        assert_eq!(
            moving.wait(),
            Duration::from_millis(circuit::FLIGHT_MS_PER_CELL / 2)
        );

        // Landed and holding: still a reason to draw, no longer a reason to
        // draw at the travel cadence. The route here is 16 cells, so travel
        // ends at 480 ms and the emote holds until 1.68 s.
        let landed = circuit::travel_time(shell.stage.as_ref().expect("stage").route_len("pane-1"))
            + Duration::from_millis(100);
        shell.stage.as_mut().expect("stage").flights = vec![flight(landed)];
        let _ = paint_rail(&mut shell, 120);
        let holding = repaint_reasons(&shell, false);
        assert!(holding.in_flight && !holding.travelling, "{holding:?}");
        assert_eq!(holding.wait(), Duration::from_millis(30));

        // Reduced motion never travels at all.
        let mut still = stage_shell(panes(), ThemeName::Nocturne, true);
        settle(&mut still, 120);
        still.stage.as_mut().expect("stage").flights = vec![flight(Duration::ZERO)];
        let _ = paint_rail(&mut still, 120);
        let reasons = repaint_reasons(&still, true);
        assert!(
            !reasons.travelling,
            "reduced motion has no packet to keep up with: {reasons:?}"
        );
    }

    /// Draw once and let every pane's pulse decay, so "the stage is quiet" is
    /// true rather than merely intended. A freshly-built `PanePulse` is live.
    fn settle(shell: &mut ShellState, width: u16) {
        let _ = paint_rail(shell, width);
        if let Some(stage) = shell.stage.as_mut() {
            for index in 0..stage.panes.len() {
                decay(stage, index);
            }
        }
        let _ = paint_rail(shell, width);
    }

    #[test]
    fn the_hand_off_survives_a_board_read_taken_before_the_pane_link_lands() {
        // `pio orch delegate` writes `assigned` with no run — it is the
        // detached supervisor's `record_delivery`, in another process, that
        // links the task to a pane. The watermark used to advance before that
        // link was checked, so a board read landing in between consumed
        // `assigned` and the outbound packet was never raised at all.
        //
        // #45 never saw this because the board was only read when a pane
        // spoke, which is rarely that narrow window. #49's wake path reads it
        // the moment the file changes, which is exactly that window.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        state.seed_task_events(&[]);

        let mut unlinked = task_with("T1", "pane-1", &[CREATED, ASSIGNED]);
        unlinked.assignee_run = None;
        state.note_task_events(&[unlinked]);
        assert!(
            state.flights.is_empty(),
            "there is no wire to aim at yet, so nothing is raised"
        );

        // The supervisor's delivery write lands, carrying the link with it.
        state.note_task_events(&[task_with("T1", "pane-1", &[CREATED, ASSIGNED, CONFIRMED])]);
        assert_eq!(
            state
                .flights
                .iter()
                .filter(|flight| flight.direction == circuit::Direction::Outbound
                    && flight.outcome == circuit::Outcome::Dispatched)
                .count(),
            1,
            "the hand-off is raised once, when it can be aimed — not discarded \
             for having been read a moment too early"
        );
    }

    #[test]
    fn the_board_watcher_watches_where_the_board_is_written() {
        // The other half of the wake path, and the half a watcher test cannot
        // see by itself: watching the wrong directory would leave every
        // assertion below passing and the shell asleep.
        let watched = super::board_watch_root();
        let written = orc_core::tasks::task_path("some-session", "T0001");
        assert!(
            written.starts_with(&watched),
            "the watcher must cover the directory `orc_core::tasks` writes \
             into: watching {watched:?}, board writes {written:?}"
        );
    }

    #[test]
    fn the_shell_actually_installs_the_board_watcher() {
        // Review finding on #49: the two tests below prove the watcher works
        // and watches the right tree, and *neither* can tell whether anything
        // ever starts it. Replacing `run_initial`'s board watcher with
        // `let _ = spawn_board_watch;` left the whole workspace green while
        // defect 4 was fully restored — the wake path gone, STAGE back to
        // learning about a delegation only when a PTY happened to tick.
        //
        // `run_initial` needs a terminal and a daemon socket, so what is held
        // here is the set it spawns from. That closes the silent hole: the
        // board entry cannot be dropped, retargeted at the wrong tree, or made
        // to raise the wrong event without this failing. Deleting the whole
        // `spawn_file_watches` call would still pass — but it also takes runs
        // and reports with it, which is not a change anyone makes by accident.
        let watches = super::file_watches();
        let board = watches
            .iter()
            .find(|watch| watch.what == "task board")
            .expect("the shell watches the task board at all");
        assert!(
            orc_core::tasks::task_path("some-session", "T0001").starts_with(&board.root),
            "and watches the tree `orc_core::tasks` writes into: {:?}",
            board.root
        );
        assert!(
            matches!((board.raise)(), super::UiEvent::BoardChanged),
            "and raises the event the loop reads the board for"
        );
    }

    #[test]
    fn a_board_change_is_what_makes_the_loop_re_read_the_board() {
        // The other half of the same finding. `read_board` used to sit inline
        // in the two match arms, so gutting `BoardChanged` down to
        // `redraw = true` left the watcher firing, the shell waking, and
        // nothing re-reading the board — suite green, feature gone.
        assert!(
            super::reads_board(&super::UiEvent::BoardChanged),
            "the wake path #49 added has to end in a board read, or it is a \
             repaint of unchanged state"
        );
        assert!(
            super::reads_board(&super::UiEvent::Snapshot(Vec::new())),
            "and the pre-#49 path still does too"
        );
        // Everything else must not: a keystroke or a resize paying for a
        // blocking `task_board` round-trip on the render thread is the
        // unfluidity STAGE was already blamed for.
        for quiet in [
            super::UiEvent::Raw(Vec::new()),
            super::UiEvent::Resize,
            super::UiEvent::RunsChanged,
            super::UiEvent::WatchFailed(String::new()),
        ] {
            assert!(
                !super::reads_board(&quiet),
                "{:?} must not cost a board round-trip",
                std::mem::discriminant(&quiet)
            );
        }
    }

    #[test]
    fn the_board_watcher_wakes_the_shell_with_every_pane_silent() {
        // Issue #49's acceptance check 4. Task events were noticed only inside
        // the `UiEvent::Snapshot` arm, and the daemon emits `Snapshot` only
        // when a pane's PTY output sequence changes — so a delegation between
        // two quiet panes waited for a pane to speak, or for the loop's 30 s
        // timeout. The board is a set of files, and this client now watches
        // them itself. No PTY is involved anywhere in this test.
        let root = std::env::temp_dir().join(format!("orc-app-board-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (sender, receiver) = std::sync::mpsc::sync_channel(4);
        super::spawn_change_watch(
            root.join("tasks"),
            "task board",
            || super::UiEvent::BoardChanged,
            sender,
        );
        let board = root.join("tasks").join("session-key");
        // Watcher registration is asynchronous, so a single early write can
        // land before the watch exists; rewriting until it arrives removes the
        // timing assumption without weakening the claim.
        let deadline = Instant::now() + Duration::from_secs(10);
        let woke = loop {
            std::fs::create_dir_all(&board).expect("create watched board");
            std::fs::write(board.join("T0001.json"), b"{}\n").expect("write watched task");
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(super::UiEvent::BoardChanged) => break true,
                Ok(other) => panic!(
                    "unexpected watcher event: {:?}",
                    std::mem::discriminant(&other)
                ),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        break false;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("watcher thread stopped")
                }
            }
        };
        assert!(
            woke,
            "a task-board write never woke the shell within 10s — far inside \
             the 30s the loop would otherwise have slept"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_emote_lands_on_the_receiving_pane_and_leaves_without_residue() {
        // AC6. A dispatch lands on its worker; a return lands back on the
        // conductor. Both go away, and neither leaves anything behind.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        state.pane_areas = stage_areas(ratatui::layout::Rect::new(0, 0, 120, 40), &state);
        let quiet = stage_text(&mut state, false);

        let outbound = InFlight {
            worker_id: "pane-1".to_owned(),
            direction: circuit::Direction::Outbound,
            outcome: circuit::Outcome::Dispatched,
            raised: Instant::now(),
        };
        assert_eq!(
            outbound
                .destination(&state.panes)
                .map(|pane| pane.id.clone()),
            Some("pane-1".to_owned()),
            "a dispatch arrives at its worker"
        );
        let inbound = InFlight {
            worker_id: "pane-1".to_owned(),
            direction: circuit::Direction::Inbound,
            outcome: circuit::Outcome::Confirmed,
            raised: Instant::now(),
        };
        assert_eq!(
            inbound
                .destination(&state.panes)
                .map(|pane| pane.id.clone()),
            Some("pane-0".to_owned()),
            "a return arrives back at the conductor"
        );

        // Landed: the emote is on screen, in the sheet's existing wording.
        let at = |ago: Duration| InFlight {
            worker_id: outbound.worker_id.clone(),
            direction: outbound.direction,
            outcome: outbound.outcome,
            raised: Instant::now() - ago,
        };
        state.flights = vec![at(Duration::from_millis(600))];
        let landed = stage_text(&mut state, false);
        assert!(
            landed.contains("TASK DISPATCHED"),
            "the emote stamps on the receiving pane: {landed:?}"
        );

        // Past its stated lifetime: gone, and the buffer is what it was.
        state.flights = vec![at(circuit::EMOTE_HOLD + Duration::from_secs(2))];
        state.retire_flights(false);
        assert!(state.flights.is_empty(), "a spent flight is retired");
        assert_eq!(
            stage_text(&mut state, false),
            quiet,
            "and leaves no residue: the pane is exactly as it was before"
        );
    }

    #[test]
    fn a_task_delegated_after_attach_animates_even_if_it_finishes_between_snapshots() {
        // Issue #45, check 1. The board is polled on snapshots, but
        // `pio orch delegate` from inside a seated pane creates, assigns and
        // confirms a task in one synchronous call — so STAGE's first sighting
        // of it can already be the finished article. Treating that first
        // sighting as history animated nothing at all, which is exactly the
        // "STAGE never moved" the issue reports.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        // Attach: the board is empty, so the seeding pass learns nothing.
        state.seed_task_events(&[]);

        state.note_task_events(&[task_with(
            "T1",
            "pane-1",
            &[CREATED, ASSIGNED, CONFIRMED, ANSWERED],
        )]);
        assert_eq!(
            state
                .flights
                .iter()
                .map(|flight| (flight.direction, flight.outcome))
                .collect::<Vec<_>>(),
            vec![
                (circuit::Direction::Outbound, circuit::Outcome::Dispatched),
                (circuit::Direction::Outbound, circuit::Outcome::Confirmed),
                (circuit::Direction::Inbound, circuit::Outcome::Confirmed),
            ],
            "the brief is aimed, the worker takes it, and the answer comes back \
             — three beats, and only the last of them is inbound (#49)"
        );
        assert!(
            state
                .flights
                .iter()
                .all(|flight| flight.worker_id == "pane-1"),
            "and all of them are on the seated worker's wire"
        );

        // Still idempotent: re-reading the same board raises nothing more.
        let raised = state.flights.len();
        state.note_task_events(&[task_with(
            "T1",
            "pane-1",
            &[CREATED, ASSIGNED, CONFIRMED, ANSWERED],
        )]);
        assert_eq!(
            state.flights.len(),
            raised,
            "a re-read is not a re-dispatch"
        );
    }

    #[test]
    fn a_landed_emote_is_never_replayed_by_re_reading_the_board() {
        // The board is re-read on every snapshot, so a naive "the last history
        // entry is `done`" test would re-raise the same packet forever. Only
        // entries that are new since the last look count — and the board that
        // already existed when we attached is history, not news.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        let finished = [task_with("T1", "pane-1", &[CREATED, ASSIGNED, DONE])];

        state.seed_task_events(&finished);
        assert!(
            state.flights.is_empty(),
            "attaching to a finished board replays nothing"
        );
        for _ in 0..5 {
            state.note_task_events(&finished);
        }
        assert!(
            state.flights.is_empty(),
            "and re-reading it raises nothing either"
        );

        // A genuinely new entry does raise exactly one.
        state.note_task_events(&[task_with(
            "T1",
            "pane-1",
            &[CREATED, ASSIGNED, DONE, CONFIRMED],
        )]);
        assert_eq!(state.flights.len(), 1);
    }

    #[test]
    fn reduced_motion_lands_the_message_without_ever_travelling() {
        // AC7. Under reduced motion the packet does not cross: the connector
        // holds solid in the message's colour and the emote appears already
        // settled. Same information, no travel anywhere on the rail.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        state.pane_areas = stage_areas(ratatui::layout::Rect::new(0, 0, 120, 40), &state);
        state.flights = vec![InFlight {
            worker_id: "pane-1".to_owned(),
            direction: circuit::Direction::Outbound,
            outcome: circuit::Outcome::Dispatched,
            raised: Instant::now(),
        }];

        // And it holds *legibly*. `paint_cell` merges modifiers into whatever
        // the cell already carries, and the rail underneath was just painted
        // `Slot::Faint` — which is DIM. Here the connector IS the message, so
        // a `bold+dim` smudge would be the whole of what reduced motion has to
        // say. (Issue #49: the travelling packet had exactly this defect, and
        // fixing only that arm left this one — the more important one —
        // untouched.)
        {
            // The rail underneath has to be the *idle* one for this to bite:
            // `Slot::Faint` is the DIM slot, while a live reduced-motion rail
            // is `Slot::Glow` and already bold. A silent worker receiving a
            // dispatch is both the common case and the one that smudges.
            decay(&mut state, 1);
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("stage terminal");
            let traffic = state.traffic(true);
            terminal
                .draw(|frame| render_stage(frame, &mut state, None, &traffic))
                .expect("render stage");
            let buffer = terminal.backend().buffer();
            let route = circuit::plan(&state.pane_areas).expect("wired").routes[0].clone();
            let solid = baton::Cell::Solid.symbol(GLYPHS);
            let mut checked = 0;
            for cell in &route {
                let Some(painted) = buffer.cell(*cell) else {
                    continue;
                };
                if painted.symbol() != solid {
                    continue;
                }
                checked += 1;
                assert!(
                    !painted.modifier.contains(Modifier::DIM),
                    "the reduced-motion connector must not inherit the rail's \
                     DIM at {cell:?}: {:?}",
                    painted.modifier
                );
            }
            assert!(checked > 0, "the connector was actually painted solid");
        }

        let still = stage_text(&mut state, true);
        assert!(
            !still.contains('▶') && !still.contains('◀'),
            "no packet is drawn under reduced motion: {still:?}"
        );
        assert!(
            still.contains("━━━"),
            "the connector holds solid in the message's colour instead"
        );
        assert!(
            still.contains("TASK DISPATCHED"),
            "and the emote is there from the first frame, already settled"
        );
        // Two different clocks paint the same frame: nothing can animate.
        state.flights = vec![InFlight {
            worker_id: "pane-1".to_owned(),
            direction: circuit::Direction::Outbound,
            outcome: circuit::Outcome::Dispatched,
            raised: Instant::now() - Duration::from_millis(400),
        }];
        assert_eq!(stage_text(&mut state, true), still);
    }

    #[test]
    fn the_message_vocabulary_survives_no_color_and_the_ascii_column() {
        // AC7 again, on the two degradation axes. Direction is carried by the
        // packet's own glyph and outcome by the emote's word and glyph, so
        // neither depends on colour; and both columns keep the two directions
        // distinct from each other and from the ambient packet.
        for glyphs in [GLYPHS, Glyphs::new(GlyphTier::Ascii)] {
            let out = circuit::packet(circuit::Direction::Outbound, glyphs);
            let back = circuit::packet(circuit::Direction::Inbound, glyphs);
            assert_ne!(out, back, "the two directions never share a symbol");
            assert_eq!(out.chars().count(), 1, "one cell, against the ramp's three");
            assert_eq!(back.chars().count(), 1);
            for frame in 0..baton::FRAMES {
                for cell in baton::cells(baton::State::Sweeping(frame)) {
                    assert_ne!(cell.symbol(glyphs), out, "{cell:?} collides with outbound");
                    assert_ne!(cell.symbol(glyphs), back, "{cell:?} collides with inbound");
                }
            }
        }

        // Monochrome: every outcome still readable, by glyph and by word.
        let mono = Theme::new(ThemeName::Nocturne, ColorTier::Monochrome);
        let mut state = StageState::new(panes(), mono, GLYPHS);
        state.pane_areas = stage_areas(ratatui::layout::Rect::new(0, 0, 120, 40), &state);
        for (outcome, want) in [
            (circuit::Outcome::Dispatched, "TASK DISPATCHED"),
            (circuit::Outcome::Confirmed, "TASK CONFIRMED"),
            (circuit::Outcome::Failed, "TASK FAILED"),
        ] {
            state.flights = vec![InFlight {
                worker_id: "pane-1".to_owned(),
                direction: circuit::Direction::Outbound,
                outcome,
                raised: Instant::now() - Duration::from_millis(600),
            }];
            let text = stage_text(&mut state, false);
            assert!(text.contains(want), "{outcome:?} with colour removed");
            assert!(
                text.contains(GLYPHS.get(outcome.glyph())),
                "{outcome:?} pairs its word with a glyph"
            );
        }
    }

    #[test]
    fn six_workers_all_producing_still_repaint_inside_one_frame() {
        // AC9. The animating cadence is 16 ms, so "stays responsive" has a
        // number attached: a full STAGE repaint with every connector live, a
        // message in flight and its emote showing has to fit inside one of
        // those frames or the loop cannot keep the cadence it asks for.
        //
        // The ceiling is deliberately loose — this repo already has three
        // storage-dependent wall-clock flakes on an external-SSD checkout, and
        // a tight budget here would be a fourth. The measurement, not the
        // bound, is the evidence; it is printed and recorded in docs/notes/.
        const FRAMES: u32 = 200;
        let mut state = StageState::new(bench(6), ThemeName::Nocturne.into(), GLYPHS);
        for index in 1..7 {
            pulse(&mut state, index);
        }
        state.flights = vec![InFlight {
            worker_id: "pane-3".to_owned(),
            direction: circuit::Direction::Outbound,
            outcome: circuit::Outcome::Dispatched,
            raised: Instant::now(),
        }];
        let mut terminal = Terminal::new(TestBackend::new(150, 44)).expect("bench terminal");

        // Warm the buffers so allocation is not counted as repaint cost.
        for _ in 0..10 {
            let traffic = state.traffic(false);
            terminal
                .draw(|frame| render_stage(frame, &mut state, Some(0), &traffic))
                .expect("warm");
        }
        let started = Instant::now();
        for frame_index in 0..FRAMES {
            let traffic = state.traffic(false);
            terminal
                .draw(|frame| render_stage(frame, &mut state, Some(frame_index as usize), &traffic))
                .expect("bench");
        }
        let per_frame = started.elapsed() / FRAMES;
        println!(
            "AC9 repaint cost: 6 workers all live + 1 message in flight, 150x44, \
             {FRAMES} frames -> {:.3} ms/frame",
            per_frame.as_secs_f64() * 1_000.0
        );
        assert!(
            per_frame < Duration::from_millis(16),
            "a full repaint must fit the 16 ms animating cadence, took {per_frame:?}"
        );
    }

    /// Feed one SGR mouse sequence to STAGE, asserting the client consumed it.
    fn mouse(state: &mut StageState, code: u16, column: u16, row: u16, suffix: char) {
        // The wire is 1-based; `route_raw_mouse` converts back to 0-based.
        let sequence = format!("\x1b[<{code};{};{}{suffix}", column + 1, row + 1);
        assert!(
            route_raw_mouse(sequence.as_bytes(), state).is_some(),
            "{sequence:?} was not consumed by STAGE"
        );
    }

    /// Every request the scripted daemon has reported so far. Safe to read
    /// without waiting: the daemon reports each line *before* it writes the
    /// reply, and every client call blocks on that reply.
    fn drain(requests: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
        std::iter::from_fn(|| requests.try_recv().ok()).collect()
    }

    fn count_of(requests: &[String], kind: &str) -> usize {
        requests
            .iter()
            .filter(|line| line.contains(&format!("\"type\":\"{kind}\"")))
            .count()
    }

    #[test]
    fn dragging_a_pane_edge_resizes_it_and_the_title_bar_still_moves_it() {
        // Edge-resize did not exist: a press was accepted only on a pane's
        // title row, and motion rewrote x/y while copying width/height back
        // unchanged. AC3 says "dragging a pane edge", so there has to be one.
        let mut state = StageState::new(panes(), ThemeName::Nocturne.into(), GLYPHS);
        let screen = ratatui::layout::Rect::new(0, 0, 120, 40);
        state.pane_areas = stage_areas(screen, &state);
        let area = state.pane_areas[0];

        // The right edge resizes; the interior belongs to the hosted CLI.
        assert_eq!(
            grab(area, area.right() - 1, area.y + 4),
            Some(Drag::Resize {
                right: true,
                bottom: false
            })
        );
        assert_eq!(
            grab(area, area.x + 4, area.bottom() - 1),
            Some(Drag::Resize {
                right: false,
                bottom: true
            })
        );
        assert_eq!(
            grab(area, area.right() - 1, area.bottom() - 1),
            Some(Drag::Resize {
                right: true,
                bottom: true
            }),
            "a corner sizes in both axes"
        );
        assert_eq!(
            grab(area, area.x + 4, area.y),
            Some(Drag::Move {
                offset_x: 4,
                offset_y: 0
            }),
            "the title bar is still a long, easy move target"
        );
        assert_eq!(
            grab(area, area.x + 4, area.y + 4),
            None,
            "the interior is the harness's, not ours"
        );

        // Drag that right edge twenty columns left.
        mouse(&mut state, 0, area.right() - 1, area.y + 4, 'M');
        mouse(&mut state, 32, area.right() - 21, area.y + 4, 'M');
        state.pane_areas = stage_areas(screen, &state);
        assert_eq!(
            state.pane_areas[0].width,
            area.width - 20,
            "the dragged edge follows the cursor"
        );
        assert_eq!(state.pane_areas[0].x, area.x, "the opposite edge stays put");
        assert_eq!(
            state.pane_areas[0].height, area.height,
            "and the other axis is untouched"
        );

        // It cannot be dragged below the floor `stage_areas` would enforce.
        mouse(&mut state, 32, area.x, area.y + 4, 'M');
        state.pane_areas = stage_areas(screen, &state);
        assert!(state.pane_areas[0].width >= MIN_PANE.0);
    }

    #[test]
    fn a_drag_issues_no_daemon_traffic_until_the_mouse_comes_up() {
        // AC3 and AC4, counted rather than felt.
        //
        // Both halves of the geometry sync were debounced only against their
        // last *value*, and during a drag the value changes every frame — so
        // at the 16 ms animating cadence every frame did a blocking `resize`
        // round-trip (daemon → TIOCSWINSZ → the hosted CLI reflows its whole
        // screen) and a blocking `update_layout` whose handler re-reads
        // `session.json`, mutates it and writes it back through an fsync.
        // Sixty drag frames used to mean 120 blocking round-trips.
        let (mut client, requests) = client_on_scripted_daemon("drag-rpc", "{\"type\":\"ack\"}\n");
        let mut state = StageState::for_session(
            "bench-alpha".to_owned(),
            panes(),
            Vec::new(),
            ThemeName::Nocturne.into(),
            GLYPHS,
        );
        let screen = ratatui::layout::Rect::new(0, 0, 120, 40);
        state.pane_areas = stage_areas(screen, &state);
        let mut sizes = HashMap::new();

        // Settle first, so what the drag itself costs is what gets counted.
        sync_stage_geometry(&mut client, &mut state, &mut sizes).expect("initial sync");
        assert!(
            !drain(&requests).is_empty(),
            "geometry does reach the daemon when nothing is being dragged"
        );

        const FRAMES: u16 = 60;
        let area = state.pane_areas[0];
        mouse(&mut state, 0, area.right() - 1, area.y + 4, 'M');
        for step in 0..FRAMES {
            // Left forty columns, then back, so the value genuinely differs
            // every frame — which is exactly the condition the old
            // value-debounce could not survive.
            mouse(
                &mut state,
                32,
                area.right() - 1 - step % 40,
                area.y + 4,
                'M',
            );
            // Exactly what the shell does each frame: draw, then sync.
            state.pane_areas = stage_areas(screen, &state);
            sync_stage_geometry(&mut client, &mut state, &mut sizes).expect("sync mid-drag");
        }
        assert!(
            drain(&requests).is_empty(),
            "{FRAMES} drag frames must issue no socket round-trip and no \
             session.json write at all — the frame follows the cursor because \
             that is a local repaint"
        );

        // Mouse up. One pass, and the value-debounce underneath makes it one.
        mouse(
            &mut state,
            0,
            area.right() - 1 - (FRAMES - 1) % 40,
            area.y + 4,
            'm',
        );
        state.pane_areas = stage_areas(screen, &state);
        sync_stage_geometry(&mut client, &mut state, &mut sizes).expect("sync on release");
        let landed = drain(&requests);
        assert_eq!(
            count_of(&landed, "update_layout"),
            1,
            "exactly one layout write for the whole drag: {landed:?}"
        );
        assert_eq!(
            count_of(&landed, "resize"),
            1,
            "and one resize, for the one pane whose size changed: {landed:?}"
        );

        // And it stays settled: a further frame with nothing moving is silent.
        sync_stage_geometry(&mut client, &mut state, &mut sizes).expect("sync after release");
        assert!(
            drain(&requests).is_empty(),
            "the post-release pass fires once, not every frame after it"
        );
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

    /// Connect a real client to a daemon that answers the handshake, then hands
    /// every later request to `reply` and reports the raw line it received.
    fn client_on_scripted_daemon(
        name: &str,
        reply: &'static str,
    ) -> (super::BenchClient, std::sync::mpsc::Receiver<String>) {
        let (sent, received) = std::sync::mpsc::channel();
        let socket = scripted_daemon(name, move |mut stream| {
            use std::io::Write;
            let _ = read_request_line(&stream);
            let welcome = format!(
                "{{\"type\":\"welcome\",\"version\":1,\"build\":\"{}\"}}\n",
                orc_proto::BUILD_IDENTIFIER
            );
            if stream.write_all(welcome.as_bytes()).is_err() {
                return;
            }
            // Report every subsequent request verbatim, so a test can assert on
            // the bytes that actually crossed the socket.
            while let Ok(line) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                read_request_line(&stream)
            })) {
                if line.is_empty() || sent.send(line).is_err() {
                    return;
                }
                if stream.write_all(reply.as_bytes()).is_err() {
                    return;
                }
            }
        });
        let client = super::BenchClient::connect(&socket).expect("connect to scripted daemon");
        (client, received)
    }

    /// The seam AC1 rests on: a keystroke has to *emit* the persistence request.
    ///
    /// Every other client test drives the switcher with `commands: None`, which
    /// never executes the `Some(commands)` branch — so gutting `cycle_theme`'s
    /// daemon round trip leaves the whole suite green while the theme silently
    /// stops surviving a relaunch. This asserts the wire bytes.
    #[test]
    fn leader_t_emits_set_theme_for_the_name_it_just_cycled_to() {
        let (mut client, requests) = client_on_scripted_daemon(
            "leader-t-persist",
            "{\"type\":\"theme_set\",\"theme\":\"ember\"}\n",
        );
        let mut shell = four_screen_shell(ThemeName::Nocturne);
        shell.view = ShellView::Home;

        let chord = [shell.leader.byte];
        assert_eq!(
            route_leader(&chord, Some(&mut client), &mut shell),
            Some(false)
        );
        assert_eq!(
            route_leader(b"t", Some(&mut client), &mut shell),
            Some(false)
        );

        let request = requests
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the client must send a request when the theme is cycled");
        assert_eq!(
            request.trim(),
            r#"{"type":"set_theme","theme":"ember"}"#,
            "the persistence request must name the theme just cycled to"
        );
        // The local switch still happened, on every screen.
        assert_eq!(shell.theme.name(), ThemeName::Ember);
        assert_eq!(
            shell.stage.as_ref().map(|stage| stage.theme.name()),
            Some(ThemeName::Ember)
        );
        assert_eq!(shell.runs.theme, Theme::from(ThemeName::Ember).runs_theme());
        // A successful save says nothing; the screen is the feedback.
        assert!(shell.home.message.is_empty(), "{:?}", shell.home.message);
    }

    /// Persistence is best-effort by design: the switch is local and always
    /// succeeds, so a daemon that refuses must leave the new palette on screen
    /// and say why — not revert, and not fail silently.
    #[test]
    fn a_refused_save_keeps_the_switch_and_reports_it_on_the_message_line() {
        let (mut client, requests) = client_on_scripted_daemon(
            "leader-t-refused",
            "{\"type\":\"error\",\"message\":\"registry is read-only\"}\n",
        );
        let mut shell = four_screen_shell(ThemeName::Nocturne);
        shell.view = ShellView::Runs;

        assert!(!route_runs_key(
            &mut shell,
            key(KeyCode::Char('t')),
            Some(&mut client)
        ));
        assert!(
            requests
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("a request must still be attempted")
                .contains("set_theme")
        );

        // Applied locally on every screen despite the refusal.
        assert_eq!(shell.theme.name(), ThemeName::Ember);
        assert_eq!(shell.runs.theme, Theme::from(ThemeName::Ember).runs_theme());
        assert_eq!(
            shell.stage.as_ref().map(|stage| stage.theme.name()),
            Some(ThemeName::Ember)
        );
        // And the reason lands on the screen the user is looking at.
        assert!(
            shell.runs.message.starts_with("theme not saved: "),
            "expected an honest failure message, got {:?}",
            shell.runs.message
        );
        assert!(
            shell.runs.message.contains("registry is read-only"),
            "the daemon's reason must survive: {:?}",
            shell.runs.message
        );
    }

    /// `ThemeSet` carries the name the daemon actually wrote, so the client
    /// renders what a relaunch will read rather than what it optimistically
    /// applied.
    #[test]
    fn the_client_adopts_the_name_the_daemon_says_it_wrote() {
        let (mut client, requests) = client_on_scripted_daemon(
            "leader-t-resolved",
            "{\"type\":\"theme_set\",\"theme\":\"phosphor\"}\n",
        );
        let mut shell = four_screen_shell(ThemeName::Nocturne);
        shell.view = ShellView::Home;

        cycle_theme(&mut shell, Some(&mut client));
        assert!(
            requests
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("a request must be sent")
                .contains(r#""theme":"ember""#),
            "the client asked for the next theme in its own cycle"
        );

        // The daemon says it stored phosphor, so that is what must render.
        assert_eq!(shell.theme.name(), ThemeName::Phosphor);
        assert_eq!(
            shell.runs.theme,
            Theme::from(ThemeName::Phosphor).runs_theme()
        );
        assert_eq!(
            shell.stage.as_ref().map(|stage| stage.theme.name()),
            Some(ThemeName::Phosphor)
        );
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
