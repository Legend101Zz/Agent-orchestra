//! The brief sidecar: what a worker was actually handed (issue #49 phase 3).
//!
//! #45 was filed because someone watched a `delegate:` go out, watched their
//! seated Hermes sit there, and concluded that Hermes had run the task. It had
//! not. A *second* process had, headless, in a worktree, and the only record of
//! it was a JSON blob. Decision 1 on #49 settled what to do about that: the
//! brief is **not** delivered into the pane, it is drawn over the pane's card —
//! so what you see is the brief that was really delivered, on the pane the
//! receipt names, with the fact that it went somewhere else stated in words.
//!
//! ## The rules this module exists to keep
//!
//! Every one of them is a way the feature could lie, and each is held by a test
//! named in `docs/notes/2026-08-01-issue-49-phase3-evidence.md`.
//!
//! - **Never show a character the worker has not produced.** Worker bytes reach
//!   the screen only through [`orc_core::dispatch_progress::tail_lines`], which
//!   returns whole *completed* lines from the append-only byte log. Nothing is
//!   reconstructed, re-rendered or inferred.
//! - **Never turn an empty log into "thinking".** `read_until(b'\n')` means
//!   whether anything is observable is the *child's* decision — a
//!   block-buffered Python worker delivered its first line at 2.187 s, i.e.
//!   nothing until exit, while the same worker with `-u` delivered at 0.018 s.
//!   So the vocabulary is "no complete line observed since T", and the words in
//!   the `BANNED` table are refused outright.
//! - **Always state which "nothing here" fact is on screen.** Absent progress,
//!   declared-but-absent artifacts, present-and-empty, and capped are four
//!   different facts with four different sentences; see `status_line`.
//! - **Never fall back to `record.stdout`.** It is unbounded for any adapter
//!   with an extractor (issue #55, measured at 409,600 bytes with no truncation
//!   marker). `orc_core::dispatch::DispatchBrief` has no such field, so this is
//!   structural rather than a rule anyone has to remember.
//! - **Never run an extractor.** `extractable` is true only for `pi`; for
//!   `codex`, `claude` and `hermes` `extract_adapter_event` returns
//!   `(None, None)`. The entitlement is bytes, for every adapter, so there is
//!   exactly one code path and it renders no prose.
//!
//! Nothing here takes a `&mut BenchClient`, so the type system is the guarantee
//! that the sidecar cannot write to a pane.

use orc_core::dispatch::DispatchBrief;
use orc_core::dispatch_progress::ProgressFacts;

use crate::glyph::{Glyph, Glyphs};

/// Rows the band takes when it has the room.
pub(crate) const REVEAL_ROWS: u16 = 7;
/// Fewer rows than this and the band refuses rather than showing a stub.
pub(crate) const REVEAL_MIN_ROWS: u16 = 5;
/// Narrower than this and it refuses too.
pub(crate) const REVEAL_MIN_COLS: u16 = 24;
/// Rows of hosted CLI the pane keeps whatever happens.
pub(crate) const REVEAL_KEEP_ROWS: u16 = 2;
/// Prompt lines held in memory, so a 16 KiB brief is not re-wrapped per frame.
pub(crate) const REVEAL_PROMPT_MAX_LINES: usize = 64;
/// Bytes of each held prompt line, cut at a `char_indices` boundary.
pub(crate) const REVEAL_PROMPT_MAX_COLS: usize = 200;
/// Width below which the header wraps and the narrow wording is used.
pub(crate) const REVEAL_NARROW_COLS: u16 = 40;

/// Words this module must never render.
///
/// Every one is a claim about the **worker**, and `read_until(b'\n')` means we
/// cannot make one: what we observe is our own reading, not the worker's state.
/// `no complete line` is permitted because it is a statement about the
/// observation.
///
/// Held by `no_reveal_string_says_thinking_or_quiet`, which scans this module's
/// string literals *and* a rendered golden's text grid — the second half
/// catching a word assembled with `format!` that never appears as a literal.
#[cfg(test)]
pub(crate) const BANNED: [&str; 11] = [
    "thinking",
    "quiet",
    "silent",
    "idle",
    "waiting",
    "working",
    "busy",
    "processing",
    "no output",
    "stalled",
    "hung",
];

/// Which of a task's two workers a brief belongs to.
///
/// A reviewed task has two dispatch records with the same `task` and different
/// `purpose`, delivered to two different panes. Without this the reviewer's
/// brief is drawn on the executor's card, which is issue #51 defect 3 wearing a
/// different hat.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Lane {
    /// The worker that did the work.
    Executor,
    /// The independent reviewer.
    Reviewer,
}

/// The newest brief on `briefs` for `(task, lane, link)`.
///
/// `link` is what the board carries as `assignee_run` / `reviewer_run`, i.e.
/// `deliver`'s `confirmed_link`: **the selected pane, else the requested run,
/// else the dispatch id**. Matching `pane_id`, then `run`, then `id` walks that
/// same three-step in reverse, so a dispatch that never reached a pane is
/// *found* rather than silently missing — which is the case that produced #49
/// acceptance check 7.
///
/// `briefs` arrives `updated_at`-descending from `read_briefs`, so the first
/// match is the newest and no sort happens here.
pub(crate) fn newest_for<'a>(
    briefs: &'a [DispatchBrief],
    task: &str,
    lane: Lane,
    link: &str,
) -> Option<&'a DispatchBrief> {
    briefs.iter().find(|brief| {
        brief.task == task
            && lane_of(brief) == lane
            && (brief.pane_id.as_deref() == Some(link)
                || brief.run.as_deref() == Some(link)
                || brief.id == link)
    })
}

/// Absent `purpose` is executor work; `review` is the reviewer.
fn lane_of(brief: &DispatchBrief) -> Lane {
    match brief.purpose.as_deref() {
        Some("review") => Lane::Reviewer,
        _ => Lane::Executor,
    }
}

/// Where the progress artifacts stand, as five distinguishable facts.
///
/// The distinction that is easy to lose and expensive to lose is
/// [`Self::Undeclared`] against [`Self::NotStreaming`]. Every design proposal
/// for this feature derived "this supervisor is not streaming" from
/// `record.progress.is_none()` alone, and that is false: `open_progress`
/// creates all three artifacts *before* `on_started` writes `record.progress`,
/// and the handshake-failed path returns before any record write at all. So
/// files routinely exist with the field absent. The discriminator is the field
/// **and** a `stat` of the derived paths, which is why
/// `orc_core::dispatch::progress_paths` is public and total.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Progress {
    /// No `progress` on the record and no artifacts on disk.
    NotStreaming,
    /// No `progress` on the record but the artifacts are there.
    Undeclared { attempt: u32 },
    /// `progress` names a journal that is not on disk.
    NamedButAbsent { journal: String },
    /// The journal is there and readable.
    Observed(Box<ProgressFacts>),
}

/// One composed sidecar, ready to draw.
///
/// Built once per resolve and re-composed only when the underlying facts move,
/// so a repaint costs no allocation and no I/O.
#[derive(Clone, Debug)]
pub(crate) struct Reveal {
    /// The dispatch this is about.
    pub dispatch: String,
    /// The task it was delivered for.
    pub task: String,
    /// The harness key it went to.
    pub harness: String,
    /// The attempt ordinal, when the record declares one.
    ///
    /// **Never defaulted to `Some(1)`.** A record with no `progress` has not
    /// told us which attempt it is on, and guessing would put `a1` on screen
    /// beside bytes from a retry.
    pub attempt: Option<u32>,
    /// The worktree or session directory the worker actually ran in.
    pub cwd: Option<String>,
    /// The delivered brief, sanitised and bounded.
    pub prompt: Vec<String>,
    /// How many lines the brief really has.
    pub prompt_lines_total: usize,
    /// How large it really is.
    pub prompt_bytes: usize,
    /// Whether any control character was replaced on the way in.
    pub controls_replaced: bool,
    /// Where the progress artifacts stand.
    pub progress: Progress,
    /// The newest complete worker lines, verbatim.
    pub lines: Vec<String>,
    /// The board watermark this was resolved from — the invalidation key.
    pub resolved_from: usize,
}

/// Bytes of the byte log read per refresh.
///
/// The band shows one line; this is generous enough that the newest line is in
/// the window even for long JSON transport lines, and small enough that a
/// 256 KiB log costs a seek and 4 KiB rather than a quarter of a megabyte.
pub(crate) const REVEAL_TAIL_BYTES: u64 = 4096;

/// Build a sidecar from a durable brief and whatever is on disk beside it.
///
/// Reads at most: one `stat`-or-open of the journal, one bounded read of it,
/// and one bounded tail of the stdout log. It performs no write, takes no lock,
/// and calls neither `read_dispatch` nor `list_dispatches` — both of which
/// reconcile, and reconciling terminates a worker's process group and appends
/// to the task board.
pub(crate) fn compose(brief: &DispatchBrief, resolved_from: usize, glyphs: Glyphs) -> Reveal {
    let (prompt, prompt_lines_total, controls_replaced) = hold_prompt(&brief.prompt, glyphs);
    let attempt = brief.progress.as_ref().map(|progress| progress.attempt);
    // Paths are derived, never taken from the record: `write_dispatch` takes no
    // lock and `reconcile_record` rewrites the whole record, so the field is
    // discoverability and must never be the only way to find bytes that exist.
    let paths = orc_core::dispatch::progress_paths(&brief.session, &brief.id, attempt.unwrap_or(1));
    let (progress, lines) = match (brief.progress.as_ref(), attempt) {
        (None, _) => {
            // The distinction every design of this feature got wrong until it
            // was measured: `open_progress` creates all three artifacts BEFORE
            // `on_started` writes `record.progress`, and the handshake-failed
            // path returns before any record write at all. So "no field" and
            // "no bytes" are different facts and only a `stat` tells them apart.
            if paths.journal.exists() || paths.stdout_log.exists() {
                (Progress::Undeclared { attempt: 1 }, Vec::new())
            } else {
                (Progress::NotStreaming, Vec::new())
            }
        }
        (Some(_), _) => match orc_core::dispatch_progress::read_progress(&paths.journal) {
            Ok(Some(view)) => {
                let facts = orc_core::dispatch_progress::facts(&view);
                let lines =
                    orc_core::dispatch_progress::tail_lines(&paths.stdout_log, REVEAL_TAIL_BYTES)
                        .unwrap_or_default();
                let lines = lines
                    .iter()
                    .rev()
                    .take(1)
                    .map(|line| sanitise(line, glyphs).0)
                    .collect();
                (Progress::Observed(Box::new(facts)), lines)
            }
            // `read_progress` never returns `Err` for a malformed journal, so
            // this arm is a missing file: the record named artifacts that are
            // not there, which is a third state and gets its own sentence.
            _ => (
                Progress::NamedButAbsent {
                    journal: paths
                        .journal
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                },
                Vec::new(),
            ),
        },
    };
    Reveal {
        dispatch: brief.id.clone(),
        task: brief.task.clone(),
        harness: brief.harness.clone(),
        attempt,
        cwd: brief.cwd.clone(),
        prompt,
        prompt_lines_total,
        prompt_bytes: brief.prompt.len(),
        controls_replaced,
        progress,
        lines,
        resolved_from,
    }
}

/// Replace every control character, and say so.
///
/// The prompt is brain-written and the worker's bytes are attacker-adjacent:
/// without this a worker emitting `\x1b[2J\x1b[H ✓ TASK CONFIRMED` into its own
/// stdout could clear the screen or forge STAGE's own chrome inside the card
/// that is supposed to be evidence. `\x1b` is valid UTF-8, so dropping invalid
/// UTF-8 does not cover it — this is a separate rule and it applies to both
/// sides.
///
/// `\t` expands to a single space rather than vanishing, because a brief's
/// indentation is meaning.
pub(crate) fn sanitise(text: &str, glyphs: Glyphs) -> (String, bool) {
    let replacement = if glyphs.get(Glyph::RevealRail) == "|" {
        '.'
    } else {
        '·'
    };
    let mut replaced = false;
    let out = text
        .chars()
        .map(|character| {
            if character == '\t' {
                ' '
            } else if character.is_control() {
                replaced = true;
                replacement
            } else {
                character
            }
        })
        .collect();
    (out, replaced)
}

/// Cut `text` to `cols` display columns, marking the cut.
///
/// Counts `chars` rather than grapheme width, which is the same approximation
/// `clip_ellipsis` already makes; the difference from that helper is that this
/// one takes its ellipsis from the glyph register, so the ASCII tier gets `...`
/// rather than a hard `…` the terminal cannot draw.
pub(crate) fn clip(text: &str, cols: usize, glyphs: Glyphs) -> String {
    if text.chars().count() <= cols {
        return text.to_owned();
    }
    let mark = glyphs.get(Glyph::Elide);
    let room = cols.saturating_sub(mark.chars().count());
    let mut out: String = text.chars().take(room).collect();
    out.push_str(mark);
    out
}

/// Hold a brief at a bounded size, without losing the count of what was cut.
pub(crate) fn hold_prompt(prompt: &str, glyphs: Glyphs) -> (Vec<String>, usize, bool) {
    let mut replaced_any = false;
    let total = prompt.lines().count();
    let held = prompt
        .lines()
        .take(REVEAL_PROMPT_MAX_LINES)
        .map(|line| {
            let (clean, replaced) = sanitise(line, glyphs);
            replaced_any |= replaced;
            // Cut at a char boundary, so a 16 KiB single-line prompt is one
            // stored line rather than a re-wrap on every frame.
            clean.chars().take(REVEAL_PROMPT_MAX_COLS).collect()
        })
        .collect();
    (held, total, replaced_any)
}

/// Render a durable timestamp without inventing precision it does not have.
///
/// `now_iso()` emits `%Y-%m-%dT%H:%M:%S+00:00`. The time-of-day is taken only
/// when the string really ends `+00:00`; otherwise the whole string is shown,
/// clipped. It never appends a `Z` the record does not carry, never
/// parses-and-reformats, and never renders an elapsed counter — which would be
/// *our* clock and would make two renders of unchanged facts differ.
pub(crate) fn clock(stamp: &str) -> String {
    // `get` rather than an index, and this is not defensive habit: `len()` is
    // BYTES, so the shipped `&stamp[11..19]` panicked on a stamp whose first
    // field was multi-byte — byte 11 landing inside a code point. This runs on
    // the render thread, in a module whose whole posture is that one bad file
    // must never blank the screen, and `read_progress` is deliberately
    // infallible so a corrupt `t` reaches here rather than being rejected.
    if stamp.ends_with("+00:00")
        && let Some(time) = stamp.get(11..19)
    {
        format!("{time} UTC")
    } else {
        stamp.to_owned()
    }
}

/// The header: what this is, and — the whole point — that it is not this pane.
pub(crate) fn header(reveal: &Reveal, cols: usize, glyphs: Glyphs) -> String {
    let attempt = reveal
        .attempt
        .map_or_else(|| "attempt not declared".to_owned(), |n| format!("a{n}"));
    // The negation is unconditional and has no second form. `record.pane_id` is
    // an *attribution* — `deliver` auto-selects a seated pane, so it is usually
    // `Some` — but the supervisor spawns a separate child process with the
    // prompt on argv or stdin. Nothing in orc-core writes to a pane. A clause
    // reading "delivered to this pane" would therefore have manufactured #45
    // inside the feature built to fix it.
    // Where it ran instead: the worktree a *different process* worked in. The
    // single most convincing artefact against #45's belief, so it leads the
    // optional clauses when there is room.
    let cwd = reveal
        .cwd
        .as_deref()
        .map(|path| format!(" · cwd {path}"))
        .unwrap_or_default();
    // Said rather than done silently: a reader must know the text was altered.
    let controls = if reveal.controls_replaced {
        " · control chars shown as ·"
    } else {
        ""
    };
    let widest = format!(
        "ORC BRIEF · {} · {} · {} · {} · sidecar worker, not this pane's CLI{cwd}{controls}",
        reveal.dispatch, reveal.task, reveal.harness, attempt
    );
    let full = format!(
        "ORC BRIEF · {} · {} · {} · {} · sidecar worker, not this pane's CLI{controls}",
        reveal.dispatch, reveal.task, reveal.harness, attempt
    );
    let medium = format!(
        "ORC BRIEF · {} · {} · not this pane's CLI",
        reveal.task, reveal.harness
    );
    let short = format!("ORC BRIEF · {} · not this pane", reveal.task);
    let tiny = "ORC BRIEF · not here".to_owned();
    for candidate in [widest, full, medium, short, tiny] {
        if candidate.chars().count() <= cols {
            return candidate;
        }
    }
    clip("ORC BRIEF · not here", cols, glyphs)
}

/// The status row: which of the "nothing here" facts is being shown.
///
/// Every row is **stream-scoped** (`STDOUT`). An unlabelled `0 B observed` is
/// false for a worker whose output all went to stderr.
pub(crate) fn status_line(reveal: &Reveal, cols: usize, glyphs: Glyphs) -> String {
    let narrow = cols < usize::from(REVEAL_NARROW_COLS);
    let body = match &reveal.progress {
        Progress::NotStreaming => {
            if narrow {
                format!("{} NO PROGRESS", glyphs.get(Glyph::WorkerIdle))
            } else {
                format!(
                    "{} PROGRESS UNAVAILABLE · this supervisor is not streaming",
                    glyphs.get(Glyph::WorkerIdle)
                )
            }
        }
        Progress::Undeclared { attempt } => {
            if narrow {
                format!("{} NOT DECLARED", glyphs.get(Glyph::WorkerIdle))
            } else {
                format!(
                    "{} PROGRESS NOT DECLARED · artifacts found for attempt {attempt}",
                    glyphs.get(Glyph::WorkerIdle)
                )
            }
        }
        Progress::NamedButAbsent { journal } => {
            if narrow {
                format!("{} NAMED, ABSENT", glyphs.get(Glyph::WorkerIdle))
            } else {
                format!(
                    "{} PROGRESS NAMED BUT ABSENT · {journal} is not on disk",
                    glyphs.get(Glyph::WorkerIdle)
                )
            }
        }
        Progress::Observed(facts) => observed_line(facts, narrow, glyphs),
    };
    let mut line = body;
    if let Progress::Observed(facts) = &reveal.progress {
        // Order is fixed so a golden diff reads the same way every time.
        if facts.capped_before_first_line() {
            // The hole a file length calls quiet: 300 KB observed, nothing
            // mirrored, because one line was larger than the whole cap.
            line.push_str(" · CAPPED before the first line fit");
        } else if facts.capped() {
            if narrow {
                line.push_str(" · CAPPED");
            } else {
                line.push_str(&format!(
                    " · CAPPED {} of {} B mirrored",
                    facts.stdout.kept, facts.stdout.bytes
                ));
            }
        }
        if let Some(reason) = &facts.closed {
            line = format!("CLOSED {reason} · {line}");
        }
        if facts.torn {
            line.push_str(" · journal tail torn");
        }
    }
    clip(&line, cols, glyphs)
}

/// The five observed states, told apart by counters rather than by file length.
fn observed_line(facts: &ProgressFacts, narrow: bool, glyphs: Glyphs) -> String {
    let (lines, bytes) = (facts.stdout.lines, facts.stdout.bytes);
    let since = facts
        .newest_stdout_line_at
        .as_deref()
        .map(clock)
        .or_else(|| {
            facts
                .opened_at
                .as_deref()
                .map(|opened| format!("the worker started {}", clock(opened)))
        })
        .unwrap_or_else(|| "an unknown time".to_owned());
    if lines == 0 && bytes == 0 {
        return if narrow {
            format!("{} 0 ln · 0 B", glyphs.get(Glyph::Pending))
        } else {
            format!(
                "{} STDOUT 0 lines · 0 B · looked at {since}, nothing observed yet",
                glyphs.get(Glyph::Pending)
            )
        };
    }
    if lines == 0 {
        // Bytes but no terminator: the block-buffered case. This is the one
        // sentence the whole vocabulary exists for.
        return if narrow {
            format!("{} 0 ln · {bytes} B", glyphs.get(Glyph::Pending))
        } else {
            format!(
                "{} STDOUT 0 lines · {bytes} B · no complete line observed since {since}",
                glyphs.get(Glyph::Pending)
            )
        };
    }
    if narrow {
        format!("{} {lines} ln · {bytes} B", glyphs.get(Glyph::InProgress))
    } else {
        format!(
            "{} STDOUT {lines} lines · {bytes} B · newest line at {since}",
            glyphs.get(Glyph::InProgress)
        )
    }
}

/// The truncation marker, which **replaces** the last brief row rather than
/// being appended to it.
///
/// Appending would let a count be mistaken for content. `{n}` is exact —
/// `prompt_lines_total - shown` — and never "many".
pub(crate) fn more_marker(reveal: &Reveal, shown: usize, cols: usize, glyphs: Glyphs) -> String {
    let hidden = reveal.prompt_lines_total.saturating_sub(shown);
    let size = if reveal.prompt_bytes >= 1024 {
        format!("{:.1} KiB brief", reveal.prompt_bytes as f64 / 1024.0)
    } else {
        format!("{} B brief", reveal.prompt_bytes)
    };
    let mark = glyphs.get(Glyph::Elide);
    for candidate in [
        format!("{mark} +{hidden} more lines · {size}"),
        format!("{mark} +{hidden} more lines"),
        format!("{mark} +{hidden} more"),
        format!("{mark} +{hidden}"),
    ] {
        if candidate.chars().count() <= cols {
            return candidate;
        }
    }
    clip(&format!("{mark} +{hidden}"), cols, glyphs)
}

/// What the rule row says: the band's own cost, and how to close it.
pub(crate) fn rule_label(band: u16, of: u16, leader: &str, cols: usize) -> String {
    let full = format!("COVERING {band} OF {of} ROWS · {leader} i closes");
    let short = format!("{leader} i closes");
    if full.chars().count() + 4 <= cols {
        full
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BANNED, Lane, Progress, Reveal, clock, compose, header, hold_prompt, more_marker,
        newest_for, sanitise, status_line,
    };
    use crate::glyph::{GlyphTier, Glyphs};
    use orc_core::dispatch::DispatchBrief;
    use orc_core::dispatch_progress::{ProgressFacts, StreamCounters};

    const UNICODE: Glyphs = Glyphs::new(GlyphTier::Unicode);
    const ASCII: Glyphs = Glyphs::new(GlyphTier::Ascii);

    fn brief(id: &str, task: &str, purpose: Option<&str>, link: &str) -> DispatchBrief {
        DispatchBrief {
            id: id.to_owned(),
            session: "S".to_owned(),
            task: task.to_owned(),
            prompt: "do the thing".to_owned(),
            harness: "pi-m3".to_owned(),
            cwd: None,
            pane_id: Some(link.to_owned()),
            run: None,
            purpose: purpose.map(str::to_owned),
            status: "confirmed".to_owned(),
            execution_status: Some("running".to_owned()),
            progress: None,
            updated_at: "2026-08-01T00:00:00+00:00".to_owned(),
            extra: Default::default(),
        }
    }

    fn reveal(progress: Progress) -> Reveal {
        Reveal {
            dispatch: "D-pi-m3-1-x-0001".to_owned(),
            task: "T0007".to_owned(),
            harness: "pi-m3".to_owned(),
            attempt: Some(1),
            cwd: None,
            prompt: vec!["line one".to_owned()],
            prompt_lines_total: 1,
            prompt_bytes: 8,
            controls_replaced: false,
            progress,
            lines: Vec::new(),
            resolved_from: 4,
        }
    }

    fn observed(lines: u64, bytes: u64, kept: u64) -> Progress {
        Progress::Observed(Box::new(ProgressFacts {
            stdout: StreamCounters {
                bytes,
                lines,
                dropped: 0,
                kept,
            },
            stderr: StreamCounters::default(),
            newest_stdout_line_at: Some("2026-08-01T12:04:31+00:00".to_owned()),
            opened_at: Some("2026-08-01T12:00:00+00:00".to_owned()),
            closed: None,
            torn: false,
        }))
    }

    /// `ORC_HOME` is process-global, so the two tests that redirect it
    /// serialize through this — the same reason `orc-core`'s dispatch tests
    /// carry one. Without it they race each other's home directory.
    fn home_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Point `ORC_HOME` at a fresh directory for the duration of one test.
    ///
    /// `#[allow(unsafe_code)]` rather than crate-wide: `set_var` is unsound
    /// beside concurrent readers, and `home_lock` is what makes it sound here.
    #[allow(unsafe_code)]
    fn redirect_home(dir: &std::path::Path) {
        // SAFETY: every caller holds `home_lock`, and nothing else in this
        // crate's unit tests reads ORC_HOME.
        unsafe { std::env::set_var("ORC_HOME", dir) };
    }

    fn tempdir(label: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("orc-reveal-{label}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        dir
    }

    /// **Worker bytes are sanitised on the way to the screen.**
    ///
    /// The rule the module leads with, and until review of PR #57 it was held
    /// by nothing: `sanitise` was well tested *as a pure function*, while the
    /// path that carries **worker** bytes to it runs inside `compose` and had
    /// no test caller. Replacing the call with a plain clone left 392 passed.
    ///
    /// This writes real bytes to a real byte log and a real journal, so the
    /// adversary is where it would be in production — in the file.
    ///
    /// Mutation (call site): drop `sanitise` from the tail-line map in
    /// `compose`, i.e. `.map(|line| line.clone())`.
    #[test]
    fn a_forged_badge_in_the_workers_own_bytes_cannot_reach_the_screen() {
        let _guard = home_lock();
        let home = tempdir("forge");
        redirect_home(&home);

        let mut record = brief("D-forge", "T1", None, "w1");
        record.session = "S-forge".to_owned();
        record.progress = Some(orc_core::dispatch::DispatchProgress {
            attempt: 1,
            stdout_log: "D-forge.a1.out.log".to_owned(),
            stderr_log: "D-forge.a1.err.log".to_owned(),
            journal: "D-forge.a1.progress.jsonl".to_owned(),
            adapter: "pi".to_owned(),
            extractable: true,
            log_max_bytes: 262_144,
            extra: Default::default(),
        });
        let paths = orc_core::dispatch::progress_paths("S-forge", "D-forge", 1);
        std::fs::create_dir_all(paths.journal.parent().expect("dir")).expect("mkdir");
        // A worker trying to clear the screen and forge STAGE's own chrome
        // inside the card that exists to be evidence against it.
        std::fs::write(
            &paths.stdout_log,
            "benign first line\n\x1b[2J\x1b[H FORGED \u{2713} TASK CONFIRMED\r\n",
        )
        .expect("write log");
        std::fs::write(
            &paths.journal,
            concat!(
                r#"{"v":1,"seq":0,"t":"2026-08-01T12:00:00+00:00","attempt":1,"kind":"open","adapter":"pi","extractable":true}"#,
                "\n",
                r#"{"v":1,"seq":1,"t":"2026-08-01T12:00:05+00:00","attempt":1,"kind":"note","stdout":{"bytes":60,"lines":2,"dropped":0,"kept":60},"stderr":{"bytes":0,"lines":0,"dropped":0,"kept":0}}"#,
                "\n",
            ),
        )
        .expect("write journal");

        let composed = compose(&record, 1, UNICODE);

        assert!(
            !composed.lines.is_empty(),
            "the worker's bytes must reach the sidecar at all, or this test \
             proves nothing about what happens to them"
        );
        for line in &composed.lines {
            assert!(
                !line.contains('\x1b'),
                "an escape sequence from the worker's own stdout reached the \
                 composed sidecar: {line:?}"
            );
            assert!(
                line.chars().all(|c| !c.is_control()),
                "no control character may survive: {line:?}"
            );
        }
        // The text is still there — this replaces, it does not delete, so the
        // reader can see what the worker tried.
        assert!(
            composed.lines.iter().any(|line| line.contains("FORGED")),
            "the attempt is shown, defanged, rather than hidden: {:?}",
            composed.lines
        );
        // Newest line, not oldest.
        assert!(
            !composed
                .lines
                .iter()
                .any(|line| line.contains("benign first")),
            "the sidecar shows the NEWEST complete line: {:?}",
            composed.lines
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **`compose` really produces `Undeclared`, and tells it from `NotStreaming`.**
    ///
    /// `the_nothing_here_facts_are_told_apart_in_words` builds the four
    /// `Progress` variants **by hand** and checks their sentences differ.
    /// Nothing checked that `compose` ever *produces* `Undeclared` — delete the
    /// `exists()` stat, return `NotStreaming` always, and the suite stayed
    /// green. That is a regression to precisely the mistake this type's
    /// docstring exists to forbid: `open_progress` creates all three artifacts
    /// *before* `mark_started` writes `record.progress`, and the
    /// handshake-failed path returns before any record write at all.
    ///
    /// Mutation (call site): collapse the `paths.journal.exists()` arm in
    /// `compose` into `NotStreaming`.
    #[test]
    fn compose_tells_absent_progress_from_absent_artifacts() {
        let _guard = home_lock();
        let home = tempdir("undeclared");
        redirect_home(&home);

        let mut record = brief("D-bare", "T1", None, "w1");
        record.session = "S-bare".to_owned();
        record.progress = None;

        // (i) No field and no artifacts: this supervisor is not streaming.
        let composed = compose(&record, 1, UNICODE);
        assert_eq!(
            composed.progress,
            Progress::NotStreaming,
            "no field and nothing on disk is `NotStreaming`"
        );

        // (ii) Same record, but the artifacts exist — which is an ordinary
        //      state, not a theoretical one.
        let paths = orc_core::dispatch::progress_paths("S-bare", "D-bare", 1);
        std::fs::create_dir_all(paths.journal.parent().expect("dir")).expect("mkdir");
        std::fs::write(&paths.stdout_log, "").expect("empty log");
        let composed = compose(&record, 1, UNICODE);
        assert_eq!(
            composed.progress,
            Progress::Undeclared { attempt: 1 },
            "artifacts on disk with no field is a DIFFERENT fact — collapsing \
             the two is the lie `DispatchProgress`'s doc forbids"
        );

        // And the two say different things on screen.
        let bare = status_line(
            &Reveal {
                progress: Progress::NotStreaming,
                ..composed.clone()
            },
            120,
            UNICODE,
        );
        let found = status_line(&composed, 120, UNICODE);
        assert_ne!(bare, found);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **The reviewer's brief is never drawn on the executor's card.**
    ///
    /// A reviewed task has two records with the same `task`. Without the lane
    /// filter the first match wins and the executor's pane shows the reviewer's
    /// brief — issue #51 defect 3 in a new place.
    ///
    /// Mutation: drop `lane_of(brief) == lane` from `newest_for`.
    #[test]
    fn a_reviewed_task_keeps_its_two_briefs_on_their_own_panes() {
        let briefs = vec![
            brief("D-review", "T1", Some("review"), "pane-reviewer"),
            brief("D-exec", "T1", None, "pane-worker"),
        ];
        assert_eq!(
            newest_for(&briefs, "T1", Lane::Executor, "pane-worker")
                .expect("executor brief")
                .id,
            "D-exec"
        );
        assert_eq!(
            newest_for(&briefs, "T1", Lane::Reviewer, "pane-reviewer")
                .expect("reviewer brief")
                .id,
            "D-review"
        );
        // And the link must match too: the executor's lane on the reviewer's
        // pane is not a thing.
        assert!(newest_for(&briefs, "T1", Lane::Executor, "pane-reviewer").is_none());
    }

    /// **A dispatch that never reached a pane is found, not silently missing.**
    ///
    /// `confirmed_link` falls back to the requested run and then to the
    /// dispatch id, so matching only `pane_id` loses exactly the delegations
    /// acceptance check 7 is about.
    #[test]
    fn a_brief_is_found_by_run_and_by_dispatch_id_not_only_by_pane() {
        let mut by_run = brief("D-run", "T2", None, "unused");
        by_run.pane_id = None;
        by_run.run = Some("W-7".to_owned());
        assert!(newest_for(&[by_run], "T2", Lane::Executor, "W-7").is_some());

        let mut by_id = brief("D-id", "T3", None, "unused");
        by_id.pane_id = None;
        by_id.run = None;
        assert!(newest_for(&[by_id], "T3", Lane::Executor, "D-id").is_some());
    }

    /// **The four "nothing here" facts are four different sentences.**
    ///
    /// Pairwise distinct, and none a substring of another — the substring half
    /// matters because "NO PROGRESS" inside "NO PROGRESS DECLARED" would read
    /// as the same fact to anyone scanning.
    ///
    /// Mutation: derive `NotStreaming` from `progress.is_none()` alone, which
    /// collapses it with `Undeclared`.
    #[test]
    fn the_nothing_here_facts_are_told_apart_in_words() {
        let states = [
            Progress::NotStreaming,
            Progress::Undeclared { attempt: 1 },
            Progress::NamedButAbsent {
                journal: "D-x.a1.progress.jsonl".to_owned(),
            },
            observed(0, 0, 0),
        ];
        let lines: Vec<String> = states
            .into_iter()
            .map(|state| status_line(&reveal(state), 120, UNICODE))
            .collect();
        for (i, left) in lines.iter().enumerate() {
            for (j, right) in lines.iter().enumerate() {
                if i == j {
                    continue;
                }
                assert_ne!(left, right, "two states rendered identically");
                assert!(
                    !left.contains(right.as_str()),
                    "{left:?} contains {right:?} — a reader scanning cannot \
                     tell these apart"
                );
            }
        }
    }

    /// **Bytes with no terminator is never "no output" and never "thinking".**
    #[test]
    fn bytes_without_a_complete_line_say_so_precisely() {
        let line = status_line(&reveal(observed(0, 300, 300)), 120, UNICODE);
        assert!(
            line.contains("0 lines · 300 B · no complete line observed since"),
            "got {line:?}"
        );
        for word in BANNED {
            assert!(
                !line.to_lowercase().contains(word),
                "{line:?} contains the banned word {word:?}"
            );
        }
    }

    /// **Capped is decided by `kept < bytes`, never by the file's length.**
    ///
    /// Including the case a length gets exactly backwards: one line larger than
    /// the whole cap leaves `kept == 0` beside an empty file, and the honest
    /// answer is CAPPED, not quiet.
    #[test]
    fn capped_is_decided_by_the_journal_not_the_file() {
        let normal = status_line(&reveal(observed(9, 300_000, 262_144)), 120, UNICODE);
        assert!(
            normal.contains("CAPPED 262144 of 300000 B mirrored"),
            "{normal:?}"
        );

        let hole = status_line(&reveal(observed(1, 300_001, 0)), 120, UNICODE);
        assert!(
            hole.contains("CAPPED before the first line fit"),
            "the log is empty and the worker wrote 300 KB: {hole:?}"
        );
        assert!(
            !hole.contains("nothing observed yet"),
            "which is exactly what a file-length discriminator would say"
        );

        let uncapped = status_line(&reveal(observed(9, 300, 300)), 120, UNICODE);
        assert!(!uncapped.contains("CAPPED"), "{uncapped:?}");
    }

    /// **No rendered string uses a banned word, in any state or width.**
    #[test]
    fn no_status_line_says_thinking_or_quiet() {
        let states = [
            Progress::NotStreaming,
            Progress::Undeclared { attempt: 2 },
            Progress::NamedButAbsent {
                journal: "j".to_owned(),
            },
            observed(0, 0, 0),
            observed(0, 12, 12),
            observed(4, 812, 812),
            observed(1, 300_001, 0),
        ];
        for state in states {
            for cols in [24_usize, 39, 40, 120] {
                let line = status_line(&reveal(state.clone()), cols, UNICODE).to_lowercase();
                for word in BANNED {
                    assert!(
                        !line.contains(word),
                        "width {cols}: {line:?} contains {word:?}"
                    );
                }
            }
        }
    }

    /// **Control characters never reach the terminal, from either side.**
    ///
    /// A worker emitting `\x1b[2J\x1b[H ✓ TASK CONFIRMED` into its own stdout
    /// would otherwise clear the screen or forge STAGE's chrome inside the card
    /// that is supposed to be the evidence against it.
    #[test]
    fn control_characters_are_replaced_and_the_replacement_is_announced() {
        let (clean, replaced) = sanitise("\x1b[2J\x1b[H FORGED \r ✓ TASK CONFIRMED", UNICODE);
        assert!(replaced, "the caller must be able to say so");
        assert!(!clean.contains('\x1b'), "{clean:?}");
        assert!(!clean.contains('\r'), "{clean:?}");
        assert!(
            clean.chars().all(|c| !c.is_control()),
            "no control character survives: {clean:?}"
        );
        // The text is still legible — this replaces, it does not delete.
        assert!(clean.contains("FORGED"), "{clean:?}");

        let (tabbed, _) = sanitise("a\tb", UNICODE);
        assert_eq!(tabbed, "a b", "a tab is indentation, not a control glyph");

        // The ASCII tier gets an ASCII replacement.
        let (ascii, _) = sanitise("\x07", ASCII);
        assert_eq!(ascii, ".");
    }

    /// **Truncation replaces the last row and states an exact count.**
    #[test]
    fn truncation_is_visible_exact_and_replaces_rather_than_appends() {
        let prompt = (0..400).map(|n| format!("line {n}\n")).collect::<String>();
        let (held, total, _) = hold_prompt(&prompt, UNICODE);
        assert_eq!(total, 400);
        assert!(held.len() <= 64, "held bounded: {}", held.len());

        let mut with_prompt = reveal(Progress::NotStreaming);
        with_prompt.prompt = held;
        with_prompt.prompt_lines_total = total;
        with_prompt.prompt_bytes = prompt.len();
        let marker = more_marker(&with_prompt, 3, 60, UNICODE);
        assert!(
            marker.contains("+397 more lines"),
            "exact, never 'many': {marker:?}"
        );
        assert!(marker.contains("KiB brief"), "{marker:?}");

        // Narrow ladder still states the count.
        let narrow = more_marker(&with_prompt, 3, 14, UNICODE);
        assert!(narrow.contains("397"), "{narrow:?}");
        // ASCII tier gets an ASCII ellipsis.
        let ascii = more_marker(&with_prompt, 3, 60, ASCII);
        assert!(ascii.starts_with("..."), "{ascii:?}");
        assert!(!ascii.contains('…'), "{ascii:?}");
    }

    /// **A 16 KiB single-line brief is held as one bounded line.**
    #[test]
    fn a_single_line_brief_is_bounded_rather_than_rewrapped() {
        let prompt = "x".repeat(16 * 1024);
        let (held, total, _) = hold_prompt(&prompt, UNICODE);
        assert_eq!(total, 1);
        assert_eq!(held.len(), 1);
        assert!(held[0].chars().count() <= 200, "{}", held[0].len());
    }

    /// **The header always carries the negation, at every width.**
    ///
    /// This is acceptance check 10: the thing that stops a reader concluding
    /// their seated CLI ran the task.
    #[test]
    fn the_header_says_it_is_not_this_pane_at_every_width() {
        let subject = reveal(Progress::NotStreaming);
        for cols in [20_usize, 24, 30, 40, 64, 120] {
            let line = header(&subject, cols, UNICODE);
            assert!(line.starts_with("ORC BRIEF"), "width {cols}: {line:?}");
            assert!(
                line.contains("not this pane") || line.contains("not here"),
                "width {cols}: the negation must survive every width: {line:?}"
            );
            assert!(
                line.chars().count() <= cols,
                "width {cols}: overflowed with {line:?}"
            );
        }
    }

    /// **A timestamp is shown, never invented.**
    #[test]
    fn a_durable_timestamp_is_rendered_without_inventing_a_zone() {
        assert_eq!(clock("2026-08-01T12:04:31+00:00"), "12:04:31 UTC");
        // Not `+00:00`: shown whole rather than relabelled.
        assert_eq!(
            clock("2026-08-01T12:04:31+05:30"),
            "2026-08-01T12:04:31+05:30"
        );
        assert_eq!(clock("garbage"), "garbage");
        // A multi-byte first field: byte 11 is inside a code point. The shipped
        // version panicked here, on the render thread.
        assert_eq!(
            clock("あああああああ+00:00"),
            "あああああああ+00:00",
            "a corrupt stamp is shown whole, never panicked on"
        );
        assert_eq!(clock(""), "");
        assert_eq!(clock("+00:00"), "+00:00");
    }

    /// **`attempt` is never guessed.**
    #[test]
    fn an_undeclared_attempt_is_said_rather_than_assumed() {
        let mut subject = reveal(Progress::NotStreaming);
        subject.attempt = None;
        let line = header(&subject, 200, UNICODE);
        assert!(line.contains("attempt not declared"), "{line:?}");
        assert!(
            !line.contains("a1"),
            "guessing a1 beside a retry's bytes: {line:?}"
        );
    }
}
