#![warn(missing_docs)]
//! Versioned messages exchanged by the Bench daemon and terminal client.
//!
//! This crate owns wire-compatible data only. It must never perform I/O or
//! mutate session state.

use serde::{Deserialize, Serialize};

/// Protocol version implemented by this build.
pub const PROTOCOL_VERSION: u16 = 1;

/// A color captured from a hosted terminal cell.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum TerminalColor {
    /// Use the enclosing terminal's default color.
    #[default]
    Default,
    /// An ANSI palette index.
    Indexed(u8),
    /// A true-color value.
    Rgb(u8, u8, u8),
}

impl TerminalColor {
    fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One styled terminal cell in row-major order.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalCell {
    /// Text stored in this cell, including combining characters.
    #[serde(default, rename = "t", skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Foreground color.
    #[serde(
        default,
        rename = "f",
        skip_serializing_if = "TerminalColor::is_default"
    )]
    pub foreground: TerminalColor,
    /// Background color.
    #[serde(
        default,
        rename = "g",
        skip_serializing_if = "TerminalColor::is_default"
    )]
    pub background: TerminalColor,
    /// Whether the cell is bold.
    #[serde(default, rename = "o", skip_serializing_if = "is_false")]
    pub bold: bool,
    /// Whether the cell is dimmed.
    #[serde(default, rename = "d", skip_serializing_if = "is_false")]
    pub dim: bool,
    /// Whether the cell is italic.
    #[serde(default, rename = "i", skip_serializing_if = "is_false")]
    pub italic: bool,
    /// Whether the cell is underlined.
    #[serde(default, rename = "u", skip_serializing_if = "is_false")]
    pub underline: bool,
    /// Whether foreground and background are inverted.
    #[serde(default, rename = "v", skip_serializing_if = "is_false")]
    pub inverse: bool,
}

/// A complete replayable screen for one hosted pane.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneSnapshot {
    /// Stable pane identifier.
    pub id: String,
    /// Human-facing harness label.
    pub title: String,
    /// Visible row count.
    pub rows: u16,
    /// Visible column count.
    pub cols: u16,
    /// Cursor row and column.
    pub cursor: (u16, u16),
    /// Monotonic output sequence used by the adaptive frame clock.
    pub sequence: u64,
    /// Row-major terminal cells, always bounded by `rows * cols`.
    pub cells: Vec<TerminalCell>,
    /// Owning session when launched from STAGE.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Harness registry key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// `brain` or `worker`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Plain runtime state word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Epoch seconds at which a conductor was observed down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down_at: Option<u64>,
}

/// HOME shelf summary for one durable session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionSummary {
    /// Stable session identifier.
    pub id: String,
    /// Brain harness key.
    pub brain: String,
    /// Worker harness keys.
    pub workers: Vec<String>,
    /// Session working directory.
    pub cwd: String,
    /// Last durable update time.
    pub updated_at: String,
    /// Number of panes needing attention.
    pub attention: usize,
}

/// HOME harness choice and recovery capability.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessSummary {
    /// Stable harness key.
    pub id: String,
    /// Valid roles.
    pub roles: Vec<String>,
    /// Whether conductor resume arguments are configured.
    pub resumable: bool,
}

/// Persisted rectangle sent through a daemon mutation command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LayoutRect {
    /// Stable pane identifier.
    pub pane_id: String,
    /// Horizontal cell coordinate.
    pub x: u16,
    /// Vertical cell coordinate.
    pub y: u16,
    /// Card width.
    pub width: u16,
    /// Card height.
    pub height: u16,
    /// Stable ensemble order.
    pub order: usize,
}

/// Lightweight pane version used by the daemon's blocking event wait.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneSequence {
    /// Stable pane identifier.
    pub id: String,
    /// Current output sequence.
    pub sequence: u64,
}

/// Bounded-output counters for one daemon-owned pane.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneMetrics {
    /// Stable pane identifier.
    pub id: String,
    /// Bytes read from the PTY since spawn.
    pub bytes_read: u64,
    /// Reader chunks merged into canonical terminal state.
    pub output_chunks: u64,
    /// Full screen snapshots requested by attached clients.
    pub snapshots: u64,
    /// Intermediate output generations skipped between delivered snapshots.
    pub coalesced_updates: u64,
}

/// Aggregate daemon backpressure counters.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DaemonMetrics {
    /// Metrics in stable pane launch order.
    pub panes: Vec<PaneMetrics>,
    /// Current attached client count.
    pub attached_clients: usize,
}

/// A request sent from a Bench client to the daemon.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientRequest {
    /// Negotiate a protocol version.
    Hello {
        /// Client protocol version.
        version: u16,
    },
    /// Request the latest screens for every pane.
    Snapshot,
    /// Block until pane output changes or a bounded timeout expires.
    Wait {
        /// Last sequences observed by this client.
        sequences: Vec<PaneSequence>,
        /// Maximum wait, capped by the daemon.
        timeout_ms: u64,
    },
    /// Forward bytes to a focused pane.
    Input {
        /// Target pane identifier.
        pane_id: String,
        /// Bytes to write verbatim to the PTY master.
        bytes: Vec<u8>,
    },
    /// Resize a hosted pane.
    Resize {
        /// Target pane identifier.
        pane_id: String,
        /// New row count.
        rows: u16,
        /// New column count.
        cols: u16,
    },
    /// Measure socket round-trip latency without touching a harness.
    Ping {
        /// Caller-provided nonce.
        nonce: u64,
    },
    /// Request bounded-output and client counters.
    Metrics,
    /// Read HOME session and harness choices.
    Home,
    /// Create and launch one durable Bench session.
    CreateSession {
        /// Brain harness key.
        brain: String,
        /// User-edited worker harness keys.
        workers: Vec<String>,
        /// Session working directory.
        cwd: String,
    },
    /// Fetch replay for one durable session.
    AttachSession {
        /// Stable session identifier.
        session_id: String,
    },
    /// Persist STAGE card rectangles through the daemon/core writer.
    UpdateLayout {
        /// Stable session identifier.
        session_id: String,
        /// Complete bounded layout.
        layout: Vec<LayoutRect>,
    },
    /// Recover a dead conductor when its harness supports resume.
    RespawnConductor {
        /// Stable brain pane identifier.
        pane_id: String,
    },
}

/// A response sent from the daemon to one client.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerResponse {
    /// Protocol negotiation succeeded.
    Welcome {
        /// Daemon protocol version.
        version: u16,
    },
    /// Current pane screens.
    Snapshot {
        /// Pane snapshots in stable launch order.
        panes: Vec<PaneSnapshot>,
    },
    /// Current sequences after a blocking event wait.
    Changed {
        /// Latest pane sequences.
        sequences: Vec<PaneSequence>,
    },
    /// A mutation was accepted.
    Ack,
    /// Socket latency response.
    Pong {
        /// Nonce copied from the request.
        nonce: u64,
    },
    /// Current bounded-output and client counters.
    Metrics {
        /// Aggregate counters.
        metrics: DaemonMetrics,
    },
    /// HOME shelf and launch choices.
    Home {
        /// Durable sessions newest first.
        sessions: Vec<SessionSummary>,
        /// Configured harness choices.
        harnesses: Vec<HarnessSummary>,
        /// Preselected, editable workers.
        default_workers: Vec<String>,
        /// Configured worker bound.
        max_parallel_workers: usize,
        /// Theme constrained to ember or phosphor by the client.
        theme: String,
        /// Reduced-motion preference.
        reduced_motion: bool,
    },
    /// Newly created session identifier.
    SessionCreated {
        /// Stable session identifier.
        session_id: String,
    },
    /// Session replay plus its durable compositor layout.
    SessionAttached {
        /// Replayable session panes.
        panes: Vec<PaneSnapshot>,
        /// Persisted card rectangles, possibly empty for a new session.
        layout: Vec<LayoutRect>,
    },
    /// A recoverable protocol or command failure.
    Error {
        /// Plain-language failure message.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{ClientRequest, PROTOCOL_VERSION, PaneSnapshot, ServerResponse, TerminalCell};

    #[test]
    fn messages_round_trip_as_additive_json() {
        let request = ClientRequest::Hello {
            version: PROTOCOL_VERSION,
        };
        let encoded = serde_json::to_vec(&request).expect("encode request");
        let decoded: ClientRequest = serde_json::from_slice(&encoded).expect("decode request");
        assert_eq!(decoded, request);

        let response = ServerResponse::Pong { nonce: 42 };
        let encoded = serde_json::to_vec(&response).expect("encode response");
        let decoded: ServerResponse = serde_json::from_slice(&encoded).expect("decode response");
        assert_eq!(decoded, response);

        let additive: ClientRequest =
            serde_json::from_str(r#"{"type":"hello","version":1,"future_capability":true}"#)
                .expect("decode additive hello");
        assert_eq!(additive, request);
    }

    #[test]
    fn compact_default_cells_keep_snapshot_bounded() {
        let snapshot = ServerResponse::Snapshot {
            panes: vec![PaneSnapshot {
                id: "pane".to_owned(),
                title: "fixture".to_owned(),
                rows: 30,
                cols: 90,
                cursor: (0, 0),
                sequence: 1,
                cells: vec![TerminalCell::default(); 30 * 90],
                session_id: None,
                harness: None,
                role: None,
                state: None,
                down_at: None,
            }],
        };
        let encoded = serde_json::to_vec(&snapshot).expect("encode compact snapshot");
        assert!(
            encoded.len() < 9_000,
            "compact snapshot grew to {} bytes",
            encoded.len()
        );
    }
}
