#![warn(missing_docs)]
//! Bounded PTY hosting and terminal-screen capture for Bench panes.
//!
//! This crate owns child PTYs and vt parsing. It must never implement client
//! policy, task mutation, or provider traffic interception.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use orc_proto::{PaneSnapshot, TerminalCell, TerminalColor};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use thiserror::Error;

const SCROLLBACK_ROWS: usize = 2_000;
const MAX_ROWS: u16 = 200;
const MAX_COLS: u16 = 400;

/// Shared event signal used to wake daemon clients when any pane changes.
pub type UpdateSignal = Arc<(Mutex<u64>, Condvar)>;

/// Create a fresh shared pane-output signal.
#[must_use]
pub fn update_signal() -> UpdateSignal {
    Arc::new((Mutex::new(0), Condvar::new()))
}

/// Failures produced while hosting or interacting with a PTY.
#[derive(Debug, Error)]
pub enum PtyError {
    /// A portable-pty operation failed.
    #[error("PTY operation failed: {0}")]
    Portable(String),
    /// Reading or writing the PTY failed.
    #[error("PTY I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// Shared screen state was poisoned by a panic.
    #[error("PTY screen state is unavailable")]
    Poisoned,
    /// A requested terminal size exceeded the bounded grid.
    #[error("PTY size must be within 1..={MAX_ROWS} rows and 1..={MAX_COLS} columns")]
    Size,
}

/// Result type returned by PTY operations.
pub type Result<T> = std::result::Result<T, PtyError>;

/// A live child process with a bounded, replayable terminal screen.
pub struct HostedPane {
    id: String,
    title: String,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Mutex<Box<dyn Write + Send>>,
    parser: Arc<Mutex<vt100::Parser>>,
    sequence: Arc<AtomicU64>,
}

impl HostedPane {
    /// Spawn a command in a new PTY at the requested size.
    pub fn spawn(
        id: impl Into<String>,
        title: impl Into<String>,
        program: &str,
        args: &[String],
        cwd: &Path,
        rows: u16,
        cols: u16,
    ) -> Result<Self> {
        Self::spawn_with_signal(id, title, program, args, cwd, rows, cols, update_signal())
    }

    /// Spawn a command using a signal shared with sibling panes.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_signal(
        id: impl Into<String>,
        title: impl Into<String>,
        program: &str,
        args: &[String],
        cwd: &Path,
        rows: u16,
        cols: u16,
        signal: UpdateSignal,
    ) -> Result<Self> {
        validate_size(rows, cols)?;
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|error| PtyError::Portable(error.to_string()))?;
        let mut command = CommandBuilder::new(program);
        command.args(args);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| PtyError::Portable(error.to_string()))?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| PtyError::Portable(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| PtyError::Portable(error.to_string()))?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_ROWS)));
        let sequence = Arc::new(AtomicU64::new(0));
        let parser_for_reader = Arc::clone(&parser);
        let sequence_for_reader = Arc::clone(&sequence);
        let signal_for_reader = Arc::clone(&signal);
        thread::Builder::new()
            .name("orc-pty-reader".to_owned())
            .spawn(move || {
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let Ok(read) = reader.read(&mut buffer) else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    let Ok(mut parser) = parser_for_reader.lock() else {
                        break;
                    };
                    parser.process(&buffer[..read]);
                    drop(parser);
                    sequence_for_reader.fetch_add(1, Ordering::Release);
                    let (epoch, changed) = &*signal_for_reader;
                    if let Ok(mut epoch) = epoch.lock() {
                        *epoch = epoch.wrapping_add(1);
                        changed.notify_all();
                    }
                }
            })
            .map_err(PtyError::Io)?;
        Ok(Self {
            id: id.into(),
            title: title.into(),
            master: pair.master,
            child,
            writer: Mutex::new(writer),
            parser,
            sequence,
        })
    }

    /// Return this pane's stable identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Return the current output sequence without taking the screen lock.
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }

    /// Write bytes to the child without interpreting them.
    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|_| PtyError::Poisoned)?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    /// Resize both the kernel PTY and the captured terminal grid.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        validate_size(rows, cols)?;
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| PtyError::Portable(error.to_string()))?;
        self.parser
            .lock()
            .map_err(|_| PtyError::Poisoned)?
            .screen_mut()
            .set_size(rows, cols);
        Ok(())
    }

    /// Capture a bounded, styled snapshot suitable for detach/reattach replay.
    pub fn snapshot(&self) -> Result<PaneSnapshot> {
        let parser = self.parser.lock().map_err(|_| PtyError::Poisoned)?;
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let mut cells = Vec::with_capacity(usize::from(rows) * usize::from(cols));
        for row in 0..rows {
            for col in 0..cols {
                let cell = screen.cell(row, col);
                cells.push(cell.map_or_else(TerminalCell::default, terminal_cell));
            }
        }
        Ok(PaneSnapshot {
            id: self.id.clone(),
            title: self.title.clone(),
            rows,
            cols,
            cursor: screen.cursor_position(),
            sequence: self.sequence.load(Ordering::Acquire),
            cells,
        })
    }

    /// Return whether the child has exited without blocking.
    pub fn has_exited(&mut self) -> Result<bool> {
        self.child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(PtyError::Io)
    }

    /// Terminate the child process.
    pub fn kill(&mut self) -> Result<()> {
        self.child.kill().map_err(PtyError::Io)
    }
}

fn validate_size(rows: u16, cols: u16) -> Result<()> {
    if rows == 0 || cols == 0 || rows > MAX_ROWS || cols > MAX_COLS {
        return Err(PtyError::Size);
    }
    Ok(())
}

impl Drop for HostedPane {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn terminal_cell(cell: &vt100::Cell) -> TerminalCell {
    TerminalCell {
        text: cell.contents().to_owned(),
        foreground: terminal_color(cell.fgcolor()),
        background: terminal_color(cell.bgcolor()),
        bold: cell.bold(),
        dim: cell.dim(),
        italic: cell.italic(),
        underline: cell.underline(),
        inverse: cell.inverse(),
    }
}

const fn terminal_color(color: vt100::Color) -> TerminalColor {
    match color {
        vt100::Color::Default => TerminalColor::Default,
        vt100::Color::Idx(index) => TerminalColor::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => TerminalColor::Rgb(red, green, blue),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::HostedPane;

    #[test]
    fn captures_cjk_color_and_screen_replay() {
        let args = vec![
            "-c".to_owned(),
            "printf '\\033[31mBench 世界\\033[0m'".to_owned(),
        ];
        let mut pane = HostedPane::spawn("p1", "fixture", "sh", &args, Path::new("/tmp"), 8, 40)
            .expect("spawn fixture");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !pane.has_exited().expect("poll child") && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let snapshot = pane.snapshot().expect("capture screen");
        let text = snapshot
            .cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        assert!(text.contains("Bench 世界"));
        assert!(
            snapshot
                .cells
                .iter()
                .any(|cell| cell.foreground != Default::default())
        );
    }
}
