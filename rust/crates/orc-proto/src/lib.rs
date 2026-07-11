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
}

/// Lightweight pane version used by the daemon's blocking event wait.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneSequence {
    /// Stable pane identifier.
    pub id: String,
    /// Current output sequence.
    pub sequence: u64,
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
    /// A recoverable protocol or command failure.
    Error {
        /// Plain-language failure message.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{ClientRequest, PROTOCOL_VERSION, ServerResponse};

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
    }
}
