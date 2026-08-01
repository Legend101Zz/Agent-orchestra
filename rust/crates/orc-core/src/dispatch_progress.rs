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

use crate::dispatch::PROGRESS_LOG_MAX_BYTES;
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
    /// Latched once a line has been declined, so the log's bytes stay a
    /// contiguous prefix of the stream rather than a subset with a hole.
    capped: bool,
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
                    capped: false,
                },
                None,
            ),
            Err(error) => (
                Self {
                    file: None,
                    kept: 0,
                    capped: true,
                },
                Some(format!(
                    "ORC WARNING: progress log {} could not be opened, so this worker's \
                     partial output is not durable: {error}",
                    path.display()
                )),
            ),
        }
    }

    /// Append one whole line verbatim, or nothing.
    ///
    /// Contains only what the worker wrote. **A partial line is never written**,
    /// and that is a rule about this function rather than a hope about the
    /// arithmetic: a line that does not fit under [`PROGRESS_LOG_MAX_BYTES`] is
    /// declined in full and the log stops growing for the rest of the attempt.
    ///
    /// Clipping mid-line was the first implementation and it was wrong twice
    /// over. It ends the log on a byte boundary — for a JSON transport that is
    /// a truncated object no reader can parse, and for UTF-8 a split
    /// code point — and it silently makes the last line of the log something
    /// the worker never emitted. Declining costs at most one line's worth of
    /// the cap and keeps the log's one claim ("these are the worker's bytes")
    /// unqualified.
    ///
    /// A reader tells a capped log from a quiet one by **`kept` against
    /// `bytes`**, not by the file's length and not by `kept` against
    /// `log_max_bytes`. The `log_max_bytes` form was the first version of this
    /// sentence and it is wrong at exactly the boundary it exists for: a single
    /// line larger than the whole cap is declined in full, so a worker that
    /// emitted 300 KB as one line leaves `kept == 0`, which is nowhere near
    /// `log_max_bytes` and reads as *quiet*. `kept < bytes` says "the
    /// supervisor observed more than this log holds" in every capped case,
    /// including that one.
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
        if self.capped || self.kept + line.len() > PROGRESS_LOG_MAX_BYTES {
            // Latch, so a later short line cannot slip in after a long one was
            // declined and leave the log out of order.
            self.capped = true;
            return;
        }
        if file.write_all(line).is_ok() {
            self.kept += line.len();
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

/// The newest whole lines of one attempt's byte log, bounded (issue #49 phase 3).
///
/// A reveal has a handful of rows to fill and the log has up to
/// [`crate::dispatch::PROGRESS_LOG_MAX_BYTES`] (256 KiB) to offer, so reading it
/// whole to show its last four lines would be 256 KiB of I/O per repaint. This
/// seeks instead: at most `limit` bytes are read however large the file is.
///
/// **Every byte returned is a byte the worker wrote, and every line returned is
/// a line the worker finished.** Both ends of the window need repair, and for
/// different reasons:
///
/// - The **start** is found by discarding everything up to and including the
///   first `\n` in the window. That is what makes the result whole lines rather
///   than a byte slice, and it is also what makes it valid UTF-8 without
///   depending on a lossy conversion: a code point cannot straddle a `\n`.
/// - The **end** needs repair too, which is easy to get wrong. `ProgressLog`'s
///   *cap* never writes a partial line — a line that does not fit is declined
///   in full and the log latches. But `drain_to_eof`
///   (`dispatch_supervisor.rs:356-378`) calls `log.append` on whatever
///   `read_until(b'\n')` returns, and at EOF that is the unterminated
///   remainder. So a worker that exits mid-line **does** leave a log whose last
///   line has no terminator.
///
///   That fragment is dropped. Not because the bytes are untrustworthy — they
///   are the worker's — but because showing it would contradict the counters
///   shown beside it: `StreamCounters::lines` counts terminators, which is what
///   `an_unterminated_final_chunk_is_not_counted_as_a_line` pins, and a reveal
///   that displays four lines next to a journal saying three is a reveal that
///   has started lying. It is also the block-buffered-worker case the module
///   note is about: "no complete line observed" is the only claim available,
///   and a fragment is not a complete line.
///
/// Returns `None` if the file does not exist. An empty `Vec` means the file
/// exists and holds nothing yet — which is
/// [`crate::dispatch::DispatchProgress`]'s "we looked and there was nothing
/// yet", and a caller must not render it as the worker being quiet. A window
/// that lands mid-first-line of a log with no newline at all yields an empty
/// `Vec` too, and that is correct: no *complete* line has been observed.
///
/// The bytes are decoded with `from_utf8_lossy` only as a last resort against a
/// torn write; on a well-formed log it is a borrow and costs nothing.
pub fn tail_lines(path: &Path, limit: u64) -> Option<Vec<String>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len == 0 {
        return Some(Vec::new());
    }
    let window = limit.min(len);
    // `len - window` is the only seek, so a 256 KiB log costs one `seek` and
    // one `read` of `window` bytes rather than a full read.
    file.seek(SeekFrom::Start(len - window)).ok()?;
    let mut bytes = vec![0_u8; usize::try_from(window).ok()?];
    let read = file.read(&mut bytes).ok()?;
    bytes.truncate(read);
    // Whole lines only. When the window starts at byte 0 there is nothing to
    // discard; otherwise the first line is a fragment of one already shown or
    // already past, and a fragment is exactly what this must never return.
    let from_line_start = if window == len {
        &bytes[..]
    } else {
        match bytes.iter().position(|byte| *byte == b'\n') {
            Some(newline) => &bytes[newline + 1..],
            None => &[],
        }
    };
    // Drop an unterminated tail. `str::lines` yields it as though it were a
    // line, which is the whole of the bug this guards: at EOF `drain_to_eof`
    // appends whatever `read_until` returned, terminator or not.
    let complete = match from_line_start.iter().rposition(|byte| *byte == b'\n') {
        Some(last_newline) => &from_line_start[..=last_newline],
        None => &[],
    };
    Some(
        String::from_utf8_lossy(complete)
            .lines()
            .map(str::to_owned)
            .collect(),
    )
}

/// What a reader can say about one attempt without claiming anything the
/// supervisor did not observe (issue #49 phase 3).
///
/// Derived from a [`ProgressView`], but the derivation is the point: two of the
/// three fields cannot be read off the newest frame, and getting either wrong
/// produces a sentence about the *worker* out of a fact about our *polling*.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProgressFacts {
    /// Cumulative stdout counters as of the newest frame carrying them.
    pub stdout: StreamCounters,
    /// Cumulative stderr counters as of the newest frame carrying them.
    pub stderr: StreamCounters,
    /// When a **complete stdout line** was last observed to appear.
    ///
    /// **Not the newest frame's `t`.** The journal is stamped whenever *any*
    /// counter moves, and `note` compares the whole `(stdout, stderr)` pair —
    /// so a worker writing only to stderr re-stamps the journal without
    /// producing a single stdout line. Reading `frames.last().t` would then
    /// make "newest line at 12:04:31" advance while stdout had been still for
    /// minutes, which is a claim about the worker made out of a fact about
    /// stderr.
    ///
    /// This is the timestamp of the newest frame whose `stdout.lines` strictly
    /// exceeded its predecessor's. `None` when no frame in the journal shows a
    /// line appearing — in which case a reader must fall back to the `open`
    /// frame's time and say "since the worker started", never invent one.
    pub newest_stdout_line_at: Option<String>,
    /// When the attempt was opened, for that fallback.
    pub opened_at: Option<String>,
    /// Why the attempt ended, if it has.
    pub closed: Option<String>,
    /// Whether any line in the journal failed to parse.
    pub torn: bool,
}

impl ProgressFacts {
    /// Whether the log holds less than the supervisor observed.
    ///
    /// **The honest capped discriminator, and the reason it is `kept < bytes`
    /// rather than `kept` against `log_max_bytes`:** a single line larger than
    /// the whole cap is declined in full, so a worker that emitted 300 KB as
    /// one line leaves `kept == 0` — nowhere near the cap, and indistinguishable
    /// from quiet under the other rule. It is also why the file's own length
    /// answers nothing.
    #[must_use]
    pub const fn capped(&self) -> bool {
        self.stdout.kept < self.stdout.bytes
    }

    /// The cap bit before a single line ever fit, so the log is empty and the
    /// worker was not.
    #[must_use]
    pub const fn capped_before_first_line(&self) -> bool {
        self.stdout.kept == 0 && self.stdout.bytes > 0
    }
}

/// Reduce a journal to what a reader may state.
///
/// Pure, so the derivation above is testable without a worker.
#[must_use]
pub fn facts(view: &ProgressView) -> ProgressFacts {
    let mut facts = ProgressFacts {
        torn: view.torn_tail,
        ..ProgressFacts::default()
    };
    // The baseline is zero lines, before any frame carries counters at all.
    // That matters: the first `note` frame is usually where lines go 0 -> 1,
    // and a "have I seen counters yet" guard would skip exactly that frame and
    // report `None` for a worker whose only line arrived promptly.
    let mut previous_lines = 0_u64;
    for frame in &view.frames {
        if frame.kind == "open" {
            facts.opened_at = Some(frame.t.clone());
        }
        if frame.kind == "close" {
            facts.closed.clone_from(&frame.reason);
        }
        if let Some(stdout) = frame.stdout {
            // Strictly greater: `>=` would re-stamp on every frame once any
            // line existed, which is the same lie by a different route.
            if stdout.lines > previous_lines {
                facts.newest_stdout_line_at = Some(frame.t.clone());
            }
            previous_lines = stdout.lines;
            facts.stdout = stdout;
        }
        if let Some(stderr) = frame.stderr {
            facts.stderr = stderr;
        }
    }
    facts
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
    let text = String::from_utf8_lossy(&bytes);
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        // Any unparseable line is a torn one. The position guard this replaced
        // distinguished a trailing tear from a mid-file one and then did the
        // same thing in both arms, which is a distinction the caller cannot
        // observe and could not fail.
        match serde_json::from_str::<ProgressFrame>(line) {
            Ok(frame) => frames.push(frame),
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
