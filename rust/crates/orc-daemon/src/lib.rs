#![warn(missing_docs)]
//! Unix-socket daemon skeleton for the Bench PTY spike.
//!
//! The daemon owns hosted PTYs and screen replay. It must never render UI or
//! make orchestration policy decisions.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use orc_proto::{ClientRequest, PROTOCOL_VERSION, PaneSequence, PaneSnapshot, ServerResponse};
use orc_pty::{HostedPane, UpdateSignal};
use thiserror::Error;
use tracing::{info, warn};

const MAX_MESSAGE_BYTES: u64 = 1024 * 1024;
const MAX_CLIENTS: usize = 16;

/// Errors emitted by the spike daemon.
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
}

/// Result type returned by daemon operations.
pub type Result<T> = std::result::Result<T, DaemonError>;

/// Canonical pane state shared by every attached client.
pub struct Daemon {
    panes: Vec<Mutex<HostedPane>>,
    signal: UpdateSignal,
}

impl Daemon {
    /// Construct a daemon whose panes share one output signal.
    #[must_use]
    pub fn new(panes: Vec<HostedPane>, signal: UpdateSignal) -> Self {
        Self {
            panes: panes.into_iter().map(Mutex::new).collect(),
            signal,
        }
    }

    fn sequences(&self) -> Result<Vec<PaneSequence>> {
        self.panes
            .iter()
            .map(|pane| {
                let pane = pane.lock().map_err(|_| DaemonError::Poisoned)?;
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
        self.panes
            .iter()
            .map(|pane| {
                pane.lock()
                    .map_err(|_| DaemonError::Poisoned)?
                    .snapshot()
                    .map_err(DaemonError::from)
            })
            .collect()
    }

    fn with_pane<T>(
        &self,
        pane_id: &str,
        action: impl FnOnce(&mut HostedPane) -> Result<T>,
    ) -> Result<Option<T>> {
        for pane in &self.panes {
            let mut pane = pane.lock().map_err(|_| DaemonError::Poisoned)?;
            if pane.id() == pane_id {
                return action(&mut pane).map(Some);
            }
        }
        Ok(None)
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
        }
    }
}

/// Serve clients until the process is terminated.
pub fn serve(socket_path: &Path, daemon: Arc<Daemon>) -> Result<()> {
    if socket_path.exists() {
        fs::remove_file(socket_path)?;
    }
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(socket_path)?;
    let clients = Arc::new(AtomicUsize::new(0));
    info!(socket = %socket_path.display(), "orcd listening");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if clients.fetch_add(1, Ordering::AcqRel) >= MAX_CLIENTS {
                    clients.fetch_sub(1, Ordering::AcqRel);
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
                let clients = Arc::clone(&clients);
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
                    clients.fetch_sub(1, Ordering::AcqRel);
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
    loop {
        let mut bytes = Vec::new();
        let read = reader
            .by_ref()
            .take(MAX_MESSAGE_BYTES + 1)
            .read_until(b'\n', &mut bytes)?;
        if read == 0 {
            return Ok(());
        }
        if read as u64 > MAX_MESSAGE_BYTES || !bytes.ends_with(b"\n") {
            write_response(
                &mut stream,
                &ServerResponse::Error {
                    message: "protocol message exceeds 1 MiB".to_owned(),
                },
            )?;
            return Ok(());
        }
        let request = serde_json::from_slice::<ClientRequest>(&bytes)?;
        let response = daemon.respond(request)?;
        write_response(&mut stream, &response)?;
    }
}

fn write_response(stream: &mut UnixStream, response: &ServerResponse) -> Result<()> {
    serde_json::to_writer(&mut *stream, response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::thread;
    use std::time::{Duration, Instant};

    use orc_proto::{ClientRequest, PROTOCOL_VERSION, ServerResponse};
    use orc_pty::{HostedPane, update_signal};

    use super::{Daemon, handle_client};

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
}
