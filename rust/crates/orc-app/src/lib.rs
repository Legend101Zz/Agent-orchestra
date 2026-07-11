#![warn(missing_docs)]
//! Ratatui client for the Phase-one Bench spike.
//!
//! This crate owns rendering and input forwarding. It must never write
//! registry/session/task files or outlive the daemon-owned PTYs.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::SynchronizedUpdate;
use crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    KeyboardEnhancementFlags, MouseButton, MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use orc_proto::{
    ClientRequest, DaemonMetrics, PROTOCOL_VERSION, PaneSequence, PaneSnapshot, ServerResponse,
    TerminalColor,
};
use ratatui::Frame;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::{Marker, border};
use ratatui::widgets::canvas::{Canvas, Points};
use ratatui::widgets::{Block, Borders};
use tachyonfx::{EffectTimer, Interpolation};
use thiserror::Error;

const MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;

/// Errors produced by the Bench spike client.
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
}

#[derive(Clone, Copy)]
struct Theme {
    stage: Color,
    text: Color,
    dim: Color,
    focus: Color,
    pulse: Color,
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
            },
            ThemeName::Phosphor => Self {
                stage: Color::Rgb(2, 13, 8),
                text: Color::Rgb(151, 255, 190),
                dim: Color::Rgb(38, 99, 61),
                focus: Color::Rgb(111, 255, 160),
                pulse: Color::Rgb(207, 255, 220),
            },
        }
    }
}

/// A version-negotiated connection used for command and benchmark traffic.
pub struct BenchClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl BenchClient {
    /// Connect to a daemon and verify its protocol version.
    pub fn connect(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket)?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut client = Self { stream, reader };
        match client.request(&ClientRequest::Hello {
            version: PROTOCOL_VERSION,
        })? {
            ServerResponse::Welcome { version } if version == PROTOCOL_VERSION => Ok(client),
            ServerResponse::Error { message } => Err(AppError::Daemon(message)),
            response => Err(AppError::Daemon(format!(
                "unexpected hello response: {response:?}"
            ))),
        }
    }

    /// Fetch complete replayable screens.
    pub fn snapshot(&mut self) -> Result<Vec<PaneSnapshot>> {
        match self.request(&ClientRequest::Snapshot)? {
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
        if read == 0 || read as u64 > MAX_RESPONSE_BYTES || !bytes.ends_with(b"\n") {
            return Err(AppError::Daemon("invalid or oversized response".to_owned()));
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

struct StageState {
    panes: Vec<PaneSnapshot>,
    focus: usize,
    pane_areas: Vec<Rect>,
    pulse: EffectTimer,
    last_tick: Instant,
    leader_at: Option<Instant>,
    theme: Theme,
}

impl StageState {
    fn new(panes: Vec<PaneSnapshot>, theme: ThemeName) -> Self {
        Self {
            panes,
            focus: 0,
            pane_areas: Vec::new(),
            pulse: EffectTimer::from_ms(900, Interpolation::CubicOut),
            last_tick: Instant::now(),
            leader_at: None,
            theme: theme.into(),
        }
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
}

enum UiEvent {
    Terminal(Event),
    Snapshot(Vec<PaneSnapshot>),
    WatchFailed,
}

/// Run the interactive STAGE spike until the leader-key detach command.
pub fn run(socket: PathBuf, theme: ThemeName) -> Result<()> {
    let mut commands = BenchClient::connect(&socket)?;
    let panes = commands.snapshot()?;
    let mut state = StageState::new(panes, theme);
    let (events_tx, events_rx) = mpsc::sync_channel(64);
    spawn_terminal_events(events_tx.clone());
    spawn_screen_watch(socket, events_tx);

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
    let result = run_loop(&mut terminal, &mut commands, &mut state, &events_rx);
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

fn run_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    commands: &mut BenchClient,
    state: &mut StageState,
    events: &Receiver<UiEvent>,
) -> Result<()> {
    let mut redraw = true;
    let mut requested_sizes = HashMap::new();
    loop {
        state.advance();
        if redraw || !state.pulse.done() {
            let mut stdout = io::stdout();
            stdout.sync_update(|_| terminal.draw(|frame| render_stage(frame, state)))??;
            resize_to_cards(commands, state, &mut requested_sizes)?;
            redraw = false;
        }
        let wait = if state.pulse.done() {
            Duration::from_secs(30)
        } else {
            Duration::from_millis(16)
        };
        let event = match events.recv_timeout(wait) {
            Ok(event) => Some(event),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(AppError::EventSource),
        };
        match event {
            Some(UiEvent::Snapshot(panes)) => {
                state.apply_snapshot(panes);
                redraw = true;
            }
            Some(UiEvent::Terminal(event)) => {
                if handle_terminal_event(event, commands, state)? {
                    return Ok(());
                }
                redraw = true;
            }
            Some(UiEvent::WatchFailed) => return Err(AppError::EventSource),
            None => {}
        }
    }
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

fn spawn_terminal_events(sender: SyncSender<UiEvent>) {
    thread::spawn(move || {
        while let Ok(event) = event::read() {
            if sender.send(UiEvent::Terminal(event)).is_err() {
                break;
            }
        }
    });
}

fn spawn_screen_watch(socket: PathBuf, sender: SyncSender<UiEvent>) {
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let mut client = BenchClient::connect(&socket)?;
            let mut sequences = Vec::new();
            loop {
                let next = client.wait(sequences.clone(), Duration::from_secs(30))?;
                if next != sequences {
                    sequences = next;
                    let panes = client.snapshot()?;
                    if sender.send(UiEvent::Snapshot(panes)).is_err() {
                        return Ok(());
                    }
                }
            }
        })();
        if result.is_err() {
            let _ = sender.send(UiEvent::WatchFailed);
        }
    });
}

fn handle_terminal_event(
    event: Event,
    commands: &mut BenchClient,
    state: &mut StageState,
) -> Result<bool> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL) {
                if state
                    .leader_at
                    .is_some_and(|at| at.elapsed() <= Duration::from_millis(700))
                {
                    send_focused(commands, state, vec![0x07])?;
                    state.leader_at = None;
                } else {
                    state.leader_at = Some(Instant::now());
                }
                return Ok(false);
            }
            if state
                .leader_at
                .is_some_and(|at| at.elapsed() <= Duration::from_millis(700))
            {
                state.leader_at = None;
                match key.code {
                    KeyCode::Char('q') => return Ok(true),
                    KeyCode::Char('n') | KeyCode::Tab => {
                        if !state.panes.is_empty() {
                            state.focus = (state.focus + 1) % state.panes.len();
                        }
                        return Ok(false);
                    }
                    KeyCode::Char('p') | KeyCode::BackTab => {
                        if !state.panes.is_empty() {
                            state.focus = state
                                .focus
                                .checked_sub(1)
                                .unwrap_or_else(|| state.panes.len().saturating_sub(1));
                        }
                        return Ok(false);
                    }
                    _ => {}
                }
            }
            if let Some(bytes) = key_bytes(key) {
                send_focused(commands, state, bytes)?;
            }
        }
        Event::Paste(text) => {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            send_focused(commands, state, bytes)?;
        }
        Event::Mouse(mouse) => {
            if let Some(bytes) = mouse_bytes(mouse, state) {
                send_focused(commands, state, bytes)?;
            }
        }
        Event::Resize(_, _) => {
            for (pane, area) in state.panes.iter().zip(&state.pane_areas) {
                commands.resize(
                    pane.id.clone(),
                    area.height.saturating_sub(2),
                    area.width.saturating_sub(2),
                )?;
            }
        }
        _ => {}
    }
    Ok(false)
}

fn send_focused(commands: &mut BenchClient, state: &StageState, bytes: Vec<u8>) -> Result<()> {
    if let Some(pane) = state.panes.get(state.focus) {
        commands.input(pane.id.clone(), bytes)?;
    }
    Ok(())
}

fn key_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let mut bytes = match key.code {
        KeyCode::Char(character) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let code = character.to_ascii_lowercase() as u32;
            if (u32::from(b'a')..=u32::from(b'z')).contains(&code) {
                vec![(code - u32::from(b'a') + 1) as u8]
            } else {
                character.to_string().into_bytes()
            }
        }
        KeyCode::Char(character) => character.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        _ => return None,
    };
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

fn mouse_bytes(mouse: MouseEvent, state: &StageState) -> Option<Vec<u8>> {
    let area = *state.pane_areas.get(state.focus)?;
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if !inner.contains((mouse.column, mouse.row).into()) {
        return None;
    }
    let x = mouse.column.saturating_sub(inner.x) + 1;
    let y = mouse.row.saturating_sub(inner.y) + 1;
    let (code, suffix) = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => (0, 'M'),
        MouseEventKind::Down(MouseButton::Middle) => (1, 'M'),
        MouseEventKind::Down(MouseButton::Right) => (2, 'M'),
        MouseEventKind::Up(_) => (3, 'm'),
        MouseEventKind::Drag(MouseButton::Left) => (32, 'M'),
        MouseEventKind::Drag(MouseButton::Middle) => (33, 'M'),
        MouseEventKind::Drag(MouseButton::Right) => (34, 'M'),
        MouseEventKind::ScrollUp => (64, 'M'),
        MouseEventKind::ScrollDown => (65, 'M'),
        MouseEventKind::ScrollLeft => (66, 'M'),
        MouseEventKind::ScrollRight => (67, 'M'),
        MouseEventKind::Moved => (35, 'M'),
    };
    Some(format!("\x1b[<{code};{x};{y}{suffix}").into_bytes())
}

fn render_stage(frame: &mut Frame<'_>, state: &mut StageState) {
    let area = frame.area();
    frame.render_widget(
        Block::new().style(Style::default().bg(state.theme.stage)),
        area,
    );
    let horizontal = area.width >= 100;
    let chunks = if horizontal {
        Layout::default()
            .direction(Direction::Horizontal)
            .margin(1)
            .constraints([
                Constraint::Percentage(46),
                Constraint::Length(8),
                Constraint::Percentage(46),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Percentage(46),
                Constraint::Length(3),
                Constraint::Percentage(46),
            ])
            .split(area)
    };
    state.pane_areas = if chunks.len() >= 3 {
        vec![chunks[0], chunks[2]]
    } else {
        Vec::new()
    };
    if chunks.len() >= 3 && state.panes.len() >= 2 {
        render_baton(frame, chunks[1], state);
    }
    let areas = state.pane_areas.clone();
    for (index, (pane, pane_area)) in state.panes.iter().zip(areas).enumerate() {
        render_pane(frame, pane_area, pane, index == state.focus, state.theme);
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
    let pulse_index = ((points.len() - 1) as f32 * state.pulse.alpha()) as usize;
    let pulse = [points[pulse_index.min(points.len() - 1)]];
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
                    coords: &pulse,
                    color: state.theme.pulse,
                });
            }),
        area,
    );
}

fn render_pane(frame: &mut Frame<'_>, area: Rect, pane: &PaneSnapshot, focus: bool, theme: Theme) {
    let border_color = if focus { theme.focus } else { theme.dim };
    let block = Block::default()
        .title(format!(
            " {}  {} ",
            pane.title.to_uppercase(),
            pane.sequence
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

const fn ratatui_color(color: TerminalColor, default: Color) -> Color {
    match color {
        TerminalColor::Default => default,
        TerminalColor::Indexed(index) => Color::Indexed(index),
        TerminalColor::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

#[cfg(test)]
mod tests {
    use orc_proto::{PaneSnapshot, TerminalCell};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::{StageState, ThemeName, key_bytes, render_stage};

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
                }
            })
            .collect()
    }

    #[test]
    fn key_encoding_preserves_control_and_navigation() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![3])
        );
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Some(b"\x1b[A".to_vec())
        );
    }

    #[test]
    fn stage_snapshots_cover_both_themes_and_sizes() {
        for (width, height) in [(150, 44), (72, 30)] {
            for theme in [ThemeName::Ember, ThemeName::Phosphor] {
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).expect("test terminal");
                let mut state = StageState::new(panes(), theme);
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
            }
        }
    }
}
