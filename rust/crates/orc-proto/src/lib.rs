#![warn(missing_docs)]
//! Versioned messages exchanged by the Bench daemon and terminal client.
//!
//! This crate owns wire-compatible data only. It must never perform I/O or
//! mutate session state.

use serde::{Deserialize, Serialize};

/// Protocol version implemented by this build.
pub const PROTOCOL_VERSION: u16 = 1;

/// Build identity of this binary: crate version plus compile-time git commit.
///
/// `PROTOCOL_VERSION` alone cannot distinguish a same-version daemon running
/// older code, so the daemon reports this string in [`ServerResponse::Welcome`]
/// and clients compare it against their own value.
pub const BUILD_IDENTIFIER: &str =
    concat!(env!("CARGO_PKG_VERSION"), "+", env!("ORC_BUILD_COMMIT"));

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

fn default_leader_key() -> String {
    "ctrl-g".to_owned()
}

/// Runtime state of one daemon-hosted pane reported by `daemon status`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DaemonPaneStatus {
    /// Stable pane identifier.
    pub id: String,
    /// Owning session when launched from STAGE.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Harness registry key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// `brain` or `worker`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Whether the hosted child process is still running.
    pub live: bool,
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

/// Lightweight durable task card rendered by SCORE.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskSummary {
    /// Stable T-prefixed task identifier.
    pub id: String,
    /// Card title.
    pub title: String,
    /// Written lifecycle state.
    pub status: String,
    /// Assigned worker or pane when known.
    pub assignee: Option<String>,
    /// Linked pane or run when known.
    pub assignee_run: Option<String>,
    /// Whether this task owns an isolated worktree.
    pub isolated: bool,
    /// Plain isolation state or reason.
    pub isolation: Option<String>,
    /// Unfinished dependencies.
    pub blocked: bool,
    /// Exact or estimated worker tokens when linked.
    pub tokens: Option<String>,
    /// Review worktree diff summary when available.
    pub diff: Option<String>,
    /// Last actor-attributed task event.
    pub history: Vec<TaskHistorySummary>,
}

/// One actor-attributed history line for SCORE detail.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskHistorySummary {
    /// Event timestamp.
    pub at: String,
    /// `human` or `brain`.
    pub actor: String,
    /// Event action.
    pub action: String,
    /// Resulting state when applicable.
    pub to: Option<String>,
}

/// Phase 4A dispatch request sent by an actor to a configured worker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DispatchCommand {
    /// Owning Bench session identifier.
    pub session_id: String,
    /// Stable task identifier in the same session.
    pub task_id: String,
    /// `human` or `brain`.
    pub actor: String,
    /// Configured worker harness key.
    pub harness: String,
    /// Optional pane linkage recorded for replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// Optional run linkage recorded for replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// Bounded prompt body delivered to the harness.
    pub prompt: String,
    /// Optional bounded timeout override in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
}

/// Summary of one durable dispatch record returned to clients.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DispatchSummary {
    /// Stable `D`-prefixed dispatch identifier.
    pub id: String,
    /// Owning Bench session identifier.
    pub session_id: String,
    /// Stable task identifier in the same session.
    pub task_id: String,
    /// `human` or `brain`.
    pub actor: String,
    /// Configured worker harness key.
    pub harness: String,
    /// Optional pane linkage recorded for replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// Optional run linkage recorded for replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// Effective command line recorded for the delivery.
    pub command_line: String,
    /// Current durable delivery state.
    pub status: String,
    /// Exit code reported by the harness, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Failure reason when the delivery did not succeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
    /// Plain failure detail when one is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Last mutation timestamp.
    pub updated_at: String,
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
    /// Request the latest screens for every pane, or one session's panes.
    Snapshot {
        /// Restrict the reply to one session's panes when set. Session-bound
        /// clients must set this so unrelated sessions cannot inflate the
        /// response toward the wire cap.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
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
    /// Read a durable SCORE board without scanning task files in the client.
    TaskBoard {
        /// Owning session identifier.
        session_id: String,
    },
    /// Move a task through the core state machine as a human action.
    MoveTask {
        /// Owning session identifier.
        session_id: String,
        /// Stable task ID.
        task_id: String,
        /// Requested written status.
        status: String,
    },
    /// Dispatch one bounded command to a configured worker harness.
    Dispatch {
        /// Dispatch command with explicit actor and session.
        command: DispatchCommand,
    },
    /// Read every durable dispatch record for a session.
    DispatchBoard {
        /// Owning session identifier.
        session_id: String,
    },
    /// Read daemon identity, build, and hosted-pane liveness.
    DaemonStatus,
}

/// A response sent from the daemon to one client.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerResponse {
    /// Protocol negotiation succeeded.
    Welcome {
        /// Daemon protocol version.
        version: u16,
        /// Daemon build identity; empty when the daemon predates the field.
        #[serde(default)]
        build: String,
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
        /// Configured leader chord label, e.g. `ctrl-g`.
        #[serde(default = "default_leader_key")]
        leader_key: String,
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
    /// Durable SCORE cards for one session.
    TaskBoard {
        /// Owning session identifier.
        session_id: String,
        /// Parseable task cards; corrupt sibling records are omitted.
        tasks: Vec<TaskSummary>,
    },
    /// Result of a single recorded dispatch.
    Dispatched {
        /// Durable summary of the recorded dispatch.
        record: DispatchSummary,
    },
    /// Durable dispatch records for one session.
    DispatchBoard {
        /// Owning session identifier.
        session_id: String,
        /// Durable dispatch summaries, newest first.
        records: Vec<DispatchSummary>,
    },
    /// Daemon identity, build, and hosted-pane liveness.
    DaemonStatus {
        /// Daemon process identifier.
        pid: u32,
        /// Daemon build identity.
        build: String,
        /// Daemon protocol version.
        protocol: u16,
        /// Bound socket path when known.
        #[serde(default)]
        socket: String,
        /// Hosted panes with liveness.
        panes: Vec<DaemonPaneStatus>,
        /// Currently attached clients.
        attached_clients: usize,
    },
    /// A recoverable protocol or command failure.
    Error {
        /// Plain-language failure message.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        BUILD_IDENTIFIER, ClientRequest, PROTOCOL_VERSION, PaneSnapshot, ServerResponse,
        TerminalCell,
    };

    #[test]
    fn build_identifier_carries_version_and_commit() {
        assert!(BUILD_IDENTIFIER.starts_with(env!("CARGO_PKG_VERSION")));
        let commit = BUILD_IDENTIFIER
            .split_once('+')
            .map(|(_, commit)| commit)
            .unwrap_or_default();
        assert!(!commit.is_empty(), "commit segment must never be empty");
    }

    #[test]
    fn welcome_without_build_field_decodes_for_wire_compatibility() {
        // A daemon predating the build handshake sends only the version.
        let legacy: ServerResponse =
            serde_json::from_str(r#"{"type":"welcome","version":1}"#).expect("decode old welcome");
        assert_eq!(
            legacy,
            ServerResponse::Welcome {
                version: 1,
                build: String::new(),
            }
        );
        let current = ServerResponse::Welcome {
            version: PROTOCOL_VERSION,
            build: BUILD_IDENTIFIER.to_owned(),
        };
        let encoded = serde_json::to_vec(&current).expect("encode welcome");
        let decoded: ServerResponse = serde_json::from_slice(&encoded).expect("decode welcome");
        assert_eq!(decoded, current);
    }

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
