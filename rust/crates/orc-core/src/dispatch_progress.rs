//! Durable partial output for a live dispatch (issue #49 phase 2).
//!
//! `drain_to_eof` reads the worker's pipes on their own threads — which is
//! what fixed #28's deadlock — but accumulated only into an in-memory
//! `Captured`, and `Drain::finish` runs after the child exits. Between spawn
//! and exit the durable record said nothing at all: measured on a worker
//! emitting a line every 200 ms for six seconds, `record.stdout` was `0` at
//! every sample and the record on disk was byte-for-byte identical for the
//! whole run.
//!
//! This module makes that window durable, in two artifacts with two different
//! authorities. Keeping them apart is the design, not an implementation
//! detail:
//!
//! - **The byte logs** hold worker bytes and *nothing else* — no marker, no
//!   counter, no timestamp, no rendering. Because an append-only file never
//!   removes and never re-renders, it structurally cannot reproduce either lie
//!   the in-memory capture contains: `Captured::raw()`'s `tail` pops from the
//!   front, so a mid-flight render is not a prefix of the final one; and
//!   `Captured::result()` swaps raw transport for prose the first time an
//!   extractor fires. The log's whole claim is *"the worker wrote these bytes
//!   to this stream, in this order, and byte N is byte N forever"*, which is
//!   the strongest prefix guarantee in this codebase. Because it contains no
//!   orchestrator bytes, there is also nothing in it for a worker to forge.
//! - **The journal** holds orchestrator statements and nothing else: exact
//!   cumulative counters, a one-shot capability declaration, and why the
//!   attempt ended. Its claim is *"the supervisor observed these totals at
//!   this instant"*. It is written from the supervisor's main thread, so its
//!   `close` is ordered against `persist_terminal` rather than racing it.
//!
//! **Neither is ever fsynced, and "durable" here means flushed.** Every byte
//! has left this process, so a SIGKILLed supervisor — the case that previously
//! destroyed everything — loses nothing already written; a power cut or kernel
//! panic can lose the tail. That is exactly the tier `runs/<id>/output.log`
//! already uses for streamed worker output beside an atomically-rewritten
//! `meta.json`. The alternative is not affordable and the number is measured
//! rather than assumed: one `sync_all` costs ~4 ms here, against ~4 us for a
//! held-open unsynced append.
//!
//! ## The honesty floor this cannot clear
//!
//! `drain_to_eof` reads `read_until(b'\n')`, so nothing is observable until a
//! newline arrives, and whether one arrives is the *child's* decision. Most
//! runtimes switch stdout from line-buffered to block-buffered when it is a
//! pipe rather than a TTY: measured, a Python worker printing every 50 ms for
//! two seconds delivered its first line at 2.187 s — i.e. nothing until exit —
//! while the same worker with `-u` delivered at 0.018 s. No persistence design
//! can change that, because the bytes are in the child's buffer and not in our
//! pipe.
//!
//! So the vocabulary here is scoped to what was actually probed. The journal
//! reports **lines and bytes observed**; a reader may say "no complete line
//! observed since T" and must not say "the worker is quiet".

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::dispatch::{PROGRESS_LOG_MAX_BYTES, ProgressPaths};
use crate::registry::now_iso;

/// Floor between two progress notes.
///
/// Derived, not chosen. `orch::await_delegation` polls `latest_dispatch` every
/// [`crate::orch::DEFAULT_AWAIT_POLL_MS`], which makes it the fastest durable
/// reader in the tree; publishing more often than any reader reads is
/// amplification with no observer. Referenced rather than copied so the two
/// cannot drift, and pinned by an identity assertion in the tests.
///
/// **The floor only ever removes a write — it can never manufacture one.**
/// Every note is caused by a counter having moved, so a silent worker produces
/// no notes at all rather than a heartbeat saying "still nothing". That is what
/// makes the cadence real state rather than a clock, and it is the property a
/// mutation has to break.
pub(crate) const PROGRESS_NOTE_MIN_INTERVAL: Duration =
    Duration::from_millis(crate::orch::DEFAULT_AWAIT_POLL_MS);

/// The floor between two progress notes, for tests that pin it to the reader
/// it is derived from.
///
/// Exposed as a function rather than making the constant public because the
/// constant is an implementation choice, while the *identity* with
/// [`crate::orch::DEFAULT_AWAIT_POLL_MS`] is the durable claim.
#[must_use]
pub const fn note_min_interval() -> Duration {
    PROGRESS_NOTE_MIN_INTERVAL
}

/// Exact, monotone counters for one stream of one attempt.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct StreamCounters {
    /// Bytes read from the pipe. Counts everything, including what the bounded
    /// in-memory window later drops and what the log declines to mirror.
    pub bytes: u64,
    /// Complete newline-terminated lines read. A block-buffered worker's
    /// output is invisible here until it flushes — see the module note.
    pub lines: u64,
    /// Bytes evicted from the in-memory capture's sliding window.
    pub dropped: u64,
    /// Bytes this attempt's log actually holds: the length of the prefix a
    /// reader may trust. Equal to `bytes` until the cap, then frozen.
    pub kept: u64,
}

/// Why an attempt stopped producing output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// Both pipes reached EOF and the child was reaped.
    Eof,
    /// The supervisor's wall clock ran out and the process group was signalled.
    Timeout,
    /// The delivery handshake failed after spawn, so the child was killed.
    HandshakeFailed,
    /// `try_wait` itself errored.
    WaitError,
}

impl CloseReason {
    /// Stable wire word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eof => "eof",
            Self::Timeout => "timeout",
            Self::HandshakeFailed => "handshake_failed",
            Self::WaitError => "wait_error",
        }
    }
}

/// One attempt's verbatim mirror of one worker stream.
///
/// Owned by exactly one thread — the drain thread reading that pipe — and
/// moved into it, never shared, so no lock is needed and none is taken.
pub(crate) struct ProgressLog {
    file: Option<File>,
    kept: usize,
}

impl ProgressLog {
    /// Create the log empty, before any byte exists.
    ///
    /// Creating it at spawn rather than on first write is what lets a reader
    /// tell "we looked and there was nothing yet" (present, zero length) from
    /// "this supervisor is not streaming" (absent). Buying that distinction
    /// costs one `create` and no periodic heartbeat.
    ///
    /// Never fails a dispatch. A worker's output is not worth refusing to run
    /// it for, so an open error yields a disabled writer and one durable
    /// warning on the record.
    pub(crate) fn create(path: &Path) -> (Self, Option<String>) {
        match OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
        {
            Ok(file) => (
                Self {
                    file: Some(file),
                    kept: 0,
                },
                None,
            ),
            Err(error) => (
                Self {
                    file: None,
                    kept: 0,
                },
                Some(format!(
                    "ORC WARNING: progress log {} could not be opened, so this worker's \
                     partial output is not durable: {error}",
                    path.display()
                )),
            ),
        }
    }

    /// Append one complete line verbatim, clipped at [`PROGRESS_LOG_MAX_BYTES`].
    ///
    /// Contains only what the worker wrote. A partial line is never written —
    /// the clip stops at the cap and the log then stops growing for the rest of
    /// the attempt, which the journal states as `kept` so that a reader can
    /// tell a capped log from a quiet one.
    ///
    /// Errors are dropped deliberately. This runs on the thread whose job is to
    /// keep the pipe empty; #28 was a deadlock caused by that thread not
    /// draining, and failing to mirror is never worth risking it. Measured, the
    /// append is free: 20k lines through a real pipe took 185-209 ms whether
    /// mirrored or not, with "read only" not systematically fastest.
    pub(crate) fn append(&mut self, line: &[u8]) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if self.kept >= PROGRESS_LOG_MAX_BYTES {
            return;
        }
        let room = PROGRESS_LOG_MAX_BYTES - self.kept;
        let chunk = &line[..room.min(line.len())];
        if file.write_all(chunk).is_ok() {
            self.kept += chunk.len();
            let _ = file.flush();
        }
    }

    /// Bytes this log holds.
    pub(crate) const fn kept(&self) -> u64 {
        self.kept as u64
    }
}

/// One attempt's orchestrator-owned counters stream.
///
/// Owned by the supervisor's **main** thread. That placement is the fix for the
/// one thing a drain-thread writer cannot guarantee: the timeout branch takes a
/// non-joining snapshot on purpose, so a drain thread may still be running when
/// the terminal record is written. A main-thread `close` is totally ordered
/// with `persist_terminal`, and it survives every exit path — including the two
/// that return before the wait loop is ever entered.
pub(crate) struct ProgressJournal {
    file: Option<File>,
    attempt: u32,
    seq: u64,
    last_note: Instant,
    last: (StreamCounters, StreamCounters),
    closed: bool,
}

/// One line of the journal. Versioned and additive; unknown fields are
/// tolerated by every reader, as everything durable in this repo is.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProgressFrame {
    /// Grammar version.
    pub v: u32,
    /// Position in this attempt's journal, from 0.
    pub seq: u64,
    /// When the supervisor made this observation.
    pub t: String,
    /// 1-based attempt ordinal.
    pub attempt: u32,
    /// `open`, `note` or `close`.
    pub kind: String,
    /// Adapter, on the `open` frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    /// Whether prose can be extracted, on the `open` frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractable: Option<bool>,
    /// Why the attempt ended, on the `close` frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Cumulative stdout counters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<StreamCounters>,
    /// Cumulative stderr counters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<StreamCounters>,
}

impl ProgressJournal {
    /// Open the journal and write its one-shot `open` frame.
    ///
    /// The capability declaration goes here, once, before any byte exists, so
    /// that "no prose available" is a stated fact rather than something a
    /// reader has to infer from an empty answer.
    pub(crate) fn open(
        path: &Path,
        attempt: u32,
        adapter: &str,
        extractable: bool,
    ) -> (Self, Option<String>) {
        let opened = OpenOptions::new().create(true).append(true).open(path);
        let (file, warning) = match opened {
            Ok(file) => (Some(file), None),
            Err(error) => (
                None,
                Some(format!(
                    "ORC WARNING: progress journal {} could not be opened: {error}",
                    path.display()
                )),
            ),
        };
        let mut journal = Self {
            file,
            attempt,
            seq: 0,
            last_note: Instant::now(),
            last: (StreamCounters::default(), StreamCounters::default()),
            closed: false,
        };
        journal.write(&ProgressFrame {
            v: 1,
            seq: 0,
            t: now_iso(),
            attempt,
            kind: "open".to_owned(),
            adapter: Some(adapter.to_owned()),
            extractable: Some(extractable),
            reason: None,
            stdout: None,
            stderr: None,
        });
        (journal, warning)
    }

    /// Write a note only if a counter actually moved and the floor has elapsed.
    ///
    /// Gate A — change — is what makes this real state: no new bytes means no
    /// write, ever, so a silent worker's journal length and mtime do not move
    /// at all. Gate B — the floor — only ever removes a write.
    pub(crate) fn note(&mut self, stdout: StreamCounters, stderr: StreamCounters) {
        if self.file.is_none() || self.closed {
            return;
        }
        if (stdout, stderr) == self.last {
            return;
        }
        if self.last_note.elapsed() < PROGRESS_NOTE_MIN_INTERVAL {
            return;
        }
        self.last = (stdout, stderr);
        self.last_note = Instant::now();
        let (seq, attempt) = (self.next_seq(), self.attempt_hint());
        self.write(&ProgressFrame {
            v: 1,
            seq,
            t: now_iso(),
            attempt,
            kind: "note".to_owned(),
            adapter: None,
            extractable: None,
            reason: None,
            stdout: Some(stdout),
            stderr: Some(stderr),
        });
    }

    /// Write the final frame. Unconditional: the attempt really ended, which is
    /// real state regardless of whether any counter moved.
    pub(crate) fn close(
        &mut self,
        reason: CloseReason,
        stdout: StreamCounters,
        stderr: StreamCounters,
    ) {
        if self.file.is_none() || self.closed {
            return;
        }
        let (seq, attempt) = (self.next_seq(), self.attempt_hint());
        self.write(&ProgressFrame {
            v: 1,
            seq,
            t: now_iso(),
            attempt,
            kind: "close".to_owned(),
            adapter: None,
            extractable: None,
            reason: Some(reason.as_str().to_owned()),
            stdout: Some(stdout),
            stderr: Some(stderr),
        });
        self.closed = true;
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    const fn attempt_hint(&self) -> u32 {
        self.attempt
    }

    #[allow(clippy::needless_pass_by_ref_mut)]
    fn write(&mut self, frame: &ProgressFrame) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let Ok(mut line) = serde_json::to_vec(frame) else {
            return;
        };
        line.push(b'\n');
        if file.write_all(&line).is_ok() {
            let _ = file.flush();
        }
    }
}

/// Everything a reader can learn about one attempt without opening the logs.
#[derive(Clone, Debug)]
pub struct ProgressView {
    /// Every complete frame, in order.
    pub frames: Vec<ProgressFrame>,
    /// Whether the journal's last complete line was a `close`.
    pub closed: bool,
    /// Whether a torn trailing line was discarded — a SIGKILL mid-`write_all`
    /// on an append-only file can leave one.
    pub torn_tail: bool,
}

/// Read one attempt's journal, tolerantly.
///
/// **Never returns `Err` for a malformed journal, and this is deliberate rather
/// than lax.** A stricter reader would be a hazard the moment anything on the
/// listing path used it: `list_dispatches` drops a record whose reconcile
/// returns `Err`, so one truncated sidecar could remove a dispatch from every
/// listing and leave `orch await` polling a record it can no longer see. Phase
/// 2 keeps this off that path entirely, and the tolerance means a future caller
/// cannot re-introduce the hazard by accident.
///
/// A missing journal is `Ok(None)` — not an error, and not an empty view, since
/// "no journal" and "a journal with no frames" are different facts.
pub fn read_progress(path: &Path) -> Result<Option<ProgressView>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut frames = Vec::new();
    let mut torn_tail = false;
    let ends_complete = bytes.last() == Some(&b'\n');
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.split('\n').collect();
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let is_last_content =
            index + 1 == lines.len() || (index + 2 == lines.len() && ends_complete);
        match serde_json::from_str::<ProgressFrame>(line) {
            Ok(frame) => frames.push(frame),
            Err(_) if is_last_content && !ends_complete => torn_tail = true,
            Err(_) => torn_tail = true,
        }
    }
    let closed = frames.last().is_some_and(|frame| frame.kind == "close");
    Ok(Some(ProgressView {
        frames,
        closed,
        torn_tail,
    }))
}

/// Bytes each of an attempt's two logs holds right now.
///
/// A missing log reports 0, which is why a caller that needs to tell "not
/// streaming" from "nothing yet" must check existence rather than length —
/// [`crate::dispatch::DispatchProgress`] documents which fact is which.
#[must_use]
pub fn progress_lengths(paths: &ProgressPaths) -> (u64, u64) {
    let len = |path: &Path| std::fs::metadata(path).map_or(0, |meta| meta.len());
    (len(&paths.stdout_log), len(&paths.stderr_log))
}
