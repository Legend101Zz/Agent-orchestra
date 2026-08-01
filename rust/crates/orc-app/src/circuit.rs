//! The circuit: how the conductor is wired to *n* workers, and which wire is
//! carrying traffic.
//!
//! [`crate::baton`] owns one rail's frames — twelve cells, seven frames,
//! one direction — and is out of scope for #38. This module owns everything
//! around it: where a wire runs when there is more than one worker, what shape
//! each of its cells is, and how a fixed twelve-cell rail is laid onto a path
//! whose length nobody chose in advance.
//!
//! Routing is specified in `docs/archive/notes/2026-07-29-stage-circuit-topology.md`.
//! In short: one port on the conductor, a vertical trunk in the gutter, one
//! horizontal spur per worker — the n8n shape — degrading to a trunk that hugs
//! the conductor's edge when the gutter is narrow, and to a rail inlaid in each
//! worker's own top border when there is no gutter at all.
//!
//! Everything here is a pure function of geometry and a state value. The clock
//! lives in the caller, exactly as it does for the baton, so a snapshot pins a
//! frame instead of racing one.

use std::collections::BTreeMap;
use std::time::Duration;

use ratatui::layout::Rect;

use crate::baton;
use crate::glyph::{Glyph, GlyphTier, Glyphs};
use crate::theme::Slot;

/// Columns of gutter below which elbow routing has nowhere to go and the
/// narrow fallback takes over: one column to leave the conductor, one for the
/// trunk, one to reach the worker.
const MIN_GUTTER: u16 = 3;

/// Cells of rail inlaid into a worker's top border in narrow mode.
const INLAY: u16 = 6;

/// One cell of wire, described by the sides it connects to.
///
/// Junctions are not chosen by the router; they fall out of which neighbours a
/// cell was linked to, so a tap is a `├` because it genuinely connects up, down
/// and right — not because some branch of the routing code said "tee here".
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Wire {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

impl Wire {
    /// Whether this cell is a plain left-to-right run.
    ///
    /// The design sheet specifies the filament as a horizontal rule, so these
    /// cells keep the baton's own symbols — the dotted idle base, the dim rail,
    /// the solid reduced-motion bar. Box drawing only appears where the sheet
    /// had nothing to say, which is anywhere the wire turns.
    #[must_use]
    pub const fn is_horizontal(self) -> bool {
        !self.up && !self.down
    }

    /// The box-drawing symbol for this cell, in whichever column the terminal
    /// supports.
    ///
    /// A module-local table rather than new [`crate::glyph::Glyph`] variants:
    /// the register is the *concept* vocabulary (conductor, worker, confirmed)
    /// and is out of scope for #38, while structural chrome already lives
    /// outside it — `border::ROUNDED` does, and `baton::Cell` keeps its own
    /// table for exactly this reason. ASCII collapses every junction to `+`,
    /// which loses the turn direction but keeps the wire legible; nothing else
    /// in the ASCII column is a lone `+` in structural position.
    #[must_use]
    pub const fn symbol(self, glyphs: Glyphs) -> &'static str {
        let ascii = matches!(glyphs.tier(), GlyphTier::Ascii);
        match (self.up, self.down, self.left, self.right) {
            (false, false, _, _) => {
                if ascii {
                    "-"
                } else {
                    "─"
                }
            }
            (true, true, false, false) => {
                if ascii {
                    "|"
                } else {
                    "│"
                }
            }
            (false, true, false, true) => {
                if ascii {
                    "+"
                } else {
                    "╭"
                }
            }
            (true, false, false, true) => {
                if ascii {
                    "+"
                } else {
                    "╰"
                }
            }
            (false, true, true, false) => {
                if ascii {
                    "+"
                } else {
                    "╮"
                }
            }
            (true, false, true, false) => {
                if ascii {
                    "+"
                } else {
                    "╯"
                }
            }
            (true, true, false, true) => {
                if ascii {
                    "+"
                } else {
                    "├"
                }
            }
            (true, true, true, false) => {
                if ascii {
                    "+"
                } else {
                    "┤"
                }
            }
            (false, true, true, true) => {
                if ascii {
                    "+"
                } else {
                    "┬"
                }
            }
            (true, false, true, true) => {
                if ascii {
                    "+"
                } else {
                    "┴"
                }
            }
            (true, true, true, true) => {
                if ascii {
                    "+"
                } else {
                    "┼"
                }
            }
            // A dead end: linked upward or downward and nowhere else. Drawn as
            // a vertical stub rather than dropped, so a route whose neighbour
            // was clipped away still shows which way it was heading.
            (_, _, false, false) => {
                if ascii {
                    "|"
                } else {
                    "│"
                }
            }
        }
    }
}

/// How the conductor reached the workers, for honest reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Routing {
    /// A trunk in the gutter with one spur per worker.
    Elbows,
    /// No gutter to route through: a rail inlaid in each worker's top border.
    Inlaid,
}

/// The wiring for one frame: every wire cell's shape, plus the path each
/// worker's traffic travels.
#[derive(Clone, Debug)]
pub struct Circuit {
    /// Which fallback tier produced this, so the UI can say so rather than
    /// silently drawing less.
    pub routing: Routing,
    /// Every wire cell, in a deterministic order so snapshots are stable.
    pub loom: Vec<((u16, u16), Wire)>,
    /// One path per worker, in traversal order: conductor end first. Index `i`
    /// is pane `i + 1`, matching `StageState::panes`.
    pub routes: Vec<Vec<(u16, u16)>>,
}

/// Link two adjacent cells, marking the facing side on each.
fn link(cells: &mut BTreeMap<(u16, u16), Wire>, from: (u16, u16), to: (u16, u16)) {
    let (fx, fy) = from;
    let (tx, ty) = to;
    let a = cells.entry(from).or_default();
    if tx > fx {
        a.right = true;
    } else if tx < fx {
        a.left = true;
    } else if ty > fy {
        a.down = true;
    } else if ty < fy {
        a.up = true;
    }
    let b = cells.entry(to).or_default();
    if fx > tx {
        b.right = true;
    } else if fx < tx {
        b.left = true;
    } else if fy > ty {
        b.down = true;
    } else if fy < ty {
        b.up = true;
    }
}

/// Walk a run of cells, linking each to the next.
fn trace(cells: &mut BTreeMap<(u16, u16), Wire>, run: &[(u16, u16)]) {
    if let [only] = run {
        cells.entry(*only).or_default();
        return;
    }
    for pair in run.windows(2) {
        link(cells, pair[0], pair[1]);
    }
}

/// Inclusive horizontal run.
fn row_run(y: u16, from: u16, to: u16) -> Vec<(u16, u16)> {
    if from > to {
        return Vec::new();
    }
    (from..=to).map(|x| (x, y)).collect()
}

/// Inclusive vertical run, in travel order from `from` to `to`.
fn column_run(x: u16, from: u16, to: u16) -> Vec<(u16, u16)> {
    if from <= to {
        (from..=to).map(|y| (x, y)).collect()
    } else {
        (to..=from).rev().map(|y| (x, y)).collect()
    }
}

/// Plan the wiring for a stage.
///
/// `panes[0]` is the conductor and the rest are workers, matching
/// `StageState::pane_areas`. Returns `None` only when there is nothing to wire.
///
/// Geometry comes from the rects that were actually painted, never from the
/// layout formula that produced them: after a drag those two disagree, and the
/// old single rail — whose x came from `conductor_width(area)` — pointed at
/// empty stage as soon as a pane moved.
#[must_use]
pub fn plan(panes: &[Rect]) -> Option<Circuit> {
    let (brain, workers) = panes.split_first()?;
    if workers.is_empty() {
        return None;
    }
    let leftmost = workers.iter().map(|worker| worker.x).min()?;
    // Wide enough to route through, and every worker genuinely to the right of
    // the conductor. The stacked sub-100-column layout fails the second test,
    // which is what sends it to the inlaid fallback.
    let routed = leftmost >= brain.right().saturating_add(1 + MIN_GUTTER)
        && workers.iter().all(|worker| worker.x > brain.right());
    if routed {
        let mut wiring = elbows(*brain, workers, leftmost);
        wiring.clip(panes);
        Some(wiring)
    } else {
        // The inlaid fallback is deliberately drawn *on* a pane's own top
        // border, so clipping it against that pane would erase it.
        Some(inlaid(workers))
    }
}

impl Circuit {
    /// Drop every wire cell that would be painted inside a pane.
    ///
    /// In the layouts `stage_areas` computes there is nothing to clip — the
    /// gutter is the gutter. But once a pane has been dragged it can be
    /// anywhere, including straight over the trunk, and AC1's "no connector
    /// overlaps a pane" has to hold then too. Painting a wire across a hosted
    /// CLI's output would be worse than a wire with a gap in it, and the gap
    /// is honest: it says the pane is on top.
    fn clip(&mut self, panes: &[Rect]) {
        let covered = |cell: &(u16, u16)| panes.iter().any(|pane| pane.contains((*cell).into()));
        self.loom.retain(|(at, _)| !covered(at));
        for route in &mut self.routes {
            route.retain(|cell| !covered(cell));
        }
    }
}

/// The vertical middle of a rect, never its border rows.
fn middle(rect: Rect) -> u16 {
    rect.y
        .saturating_add(rect.height / 2)
        .clamp(rect.y, rect.bottom().saturating_sub(1))
}

fn elbows(brain: Rect, workers: &[Rect], leftmost: u16) -> Circuit {
    // One column clear of the conductor's drop shadow, which claims
    // `brain.right()`.
    let start_x = brain.right().saturating_add(1);
    // Two columns out, but always at least two short of the nearest worker.
    // When the gutter is tight this clamp collapses the trunk onto the
    // conductor's edge, which is the "bus" tier — same code, longer spurs.
    let trunk_x = start_x
        .saturating_add(1)
        .min(leftmost.saturating_sub(2))
        .max(start_x);
    let spurs = workers.iter().map(|worker| middle(*worker));
    let rows = spurs.clone().collect::<Vec<_>>();
    let (top, bottom) = (
        rows.iter().copied().min().unwrap_or_default(),
        rows.iter().copied().max().unwrap_or_default(),
    );
    // The port sits at the centre of the fan rather than at the conductor's
    // own middle. With one worker that makes the wire dead straight — the
    // sheet's canonical picture — instead of a one-row jog nobody asked for.
    let port_y = ((u32::from(top) + u32::from(bottom)) / 2) as u16;
    let port_y = port_y.clamp(brain.y, brain.bottom().saturating_sub(1));

    let mut cells = BTreeMap::new();
    let root = row_run(port_y, start_x, trunk_x);
    trace(&mut cells, &root);
    let mut routes = Vec::with_capacity(workers.len());
    for (worker, spur_y) in workers.iter().zip(rows) {
        // Trunk from the port to this worker's row, then out to its left edge.
        let trunk = column_run(trunk_x, port_y, spur_y);
        let spur = row_run(spur_y, trunk_x, worker.x.saturating_sub(1));
        trace(&mut cells, &trunk);
        trace(&mut cells, &spur);
        // Traversal order, conductor end first, with the two shared cells at
        // the trunk tap counted once.
        let mut route = root.clone();
        route.extend(trunk.iter().skip(1).copied());
        route.extend(spur.iter().skip(1).copied());
        route.dedup();
        routes.push(route);
    }
    Circuit {
        routing: Routing::Elbows,
        loom: cells.into_iter().collect(),
        routes,
    }
}

/// No gutter: give each worker a rail at its own threshold instead.
///
/// Inlaid into the top border, at the far end from the corner. It costs no
/// rows, exists at every width down to 80×24 and at every worker count, and
/// keeps motion legible. What it gives up is routing — you see the endpoint's
/// state, not the path — which is stated rather than hidden.
///
/// Right-aligned rather than tucked against the left corner because the title
/// is drawn from the left and grows: harness name, state, a `✓ TASK CONFIRMED`
/// stamp and a trigger badge all share that run of border.
fn inlaid(workers: &[Rect]) -> Circuit {
    let mut cells = BTreeMap::new();
    let mut routes = Vec::with_capacity(workers.len());
    for worker in workers {
        let to = worker.right().saturating_sub(2);
        let from = to
            .saturating_sub(INLAY.saturating_sub(1))
            .max(worker.x.saturating_add(1));
        let run = row_run(worker.y, from, to);
        trace(&mut cells, &run);
        routes.push(run);
    }
    Circuit {
        routing: Routing::Inlaid,
        loom: cells.into_iter().collect(),
        routes,
    }
}

/// Which cells of a route carry the rail, once its endpoints are accounted for.
///
/// The sheet draws the filament between a `◆` conductor and a `●` bench with a
/// space of clearance either side — four cells of decoration. Where the route
/// affords them the rail keeps **exactly** the sheet's twelve cells and samples
/// one-to-one, which is why the single-worker case still paints the frames
/// `_fillBaton` generates. Where it does not, the endpoints are what gets
/// dropped: the rail is what carries the meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    /// Whether the `◆`/`●` endpoints fit.
    pub endpoints: bool,
    /// Index of the first rail cell in the route.
    pub start: usize,
    /// How many cells the rail spans.
    pub len: usize,
}

/// Split a route into endpoints and rail.
#[must_use]
pub const fn rail_span(route: usize) -> Span {
    if route >= baton::CELLS + 4 {
        Span {
            endpoints: true,
            start: 2,
            len: route - 4,
        }
    } else {
        Span {
            endpoints: false,
            start: 0,
            len: route,
        }
    }
}

/// Milliseconds the in-flight packet takes to cross **one** cell.
///
/// The design sheet writes this speed as "~60 ms/frame, two cells/frame"
/// (`docs/design/visual-identity.md`, "Message in flight"). That is the same
/// 33⅓ cells per second this constant expresses, but read off a *frame
/// counter*: the packet sat still for 60 ms and then jumped two cells, which
/// is ~16 fps of visible motion however often the shell repainted, and is the
/// whole of issue #49's "the line animation is not smooth".
///
/// The speed is deliberately unchanged — faster than the ambient sweep's
/// 110 ms, so a dispatch still reads as a snap rather than as flow, and the
/// committed goldens still catch the packet in the same cell. What changed is
/// that [`flight`] divides the elapsed clock by this instead of multiplying a
/// frame index, so the packet crosses one cell at a time and drawing more
/// often genuinely samples finer.
pub const FLIGHT_MS_PER_CELL: u64 = 30;

/// How long the landing emote holds once the packet arrives.
pub const EMOTE_HOLD: Duration = Duration::from_millis(1_200);

/// The sheet's first beat: the row flashes reverse-video for one frame before
/// the glyph settles.
pub const EMOTE_FLASH: Duration = Duration::from_millis(90);

/// How much of a route the *departure* beat holds for, in cells.
///
/// The mirror of the landing emote (issue #49, spec step 1: the brain hands
/// off and you see it leave). Its length is `min(DEPART_CELLS, route) ×
/// FLIGHT_MS_PER_CELL`, so it can never outlive the packet it announces: on a
/// route shorter than this the packet lands first and the beat ends with it.
///
/// Be precise about what that does and does not buy, because the first version
/// of this doc overclaimed. Every route the router actually plans in the wide
/// layout is longer than twelve cells, so in practice the beat is a **constant
/// 360 ms** — the wall-clock cost of crossing the sheet's own rail. It is not
/// derived from anything the worker did, and it does not vary with the event.
/// What "real state only" gets here is narrower and still worth having: the
/// beat starts at a real event, it is bounded by real geometry rather than by
/// a number picked to look right, and it cannot outlast the packet. The
/// *duration* of a hand-off is not something STAGE claims to show.
pub const DEPART_CELLS: usize = baton::CELLS;

/// Which way a discrete message is travelling.
///
/// The ambient baton is one direction only — that rule is the baton's and is
/// unchanged. A message is a separate vocabulary that carries its direction in
/// its own glyph; see "Message in flight" in the design sheet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    /// Conductor to worker: a dispatch leaving.
    Outbound,
    /// Worker to conductor: a result coming back.
    Inbound,
}

/// What a message turned out to be.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// A brief has been handed to a worker.
    Dispatched,
    /// Delivery was confirmed, or the task finished.
    Confirmed,
    /// Delivery failed.
    Failed,
}

impl Outcome {
    /// The slot that colours this message. All three already exist — no new
    /// slot, which would reorder three palettes and every golden's legend.
    #[must_use]
    pub const fn slot(self) -> Slot {
        match self {
            Self::Dispatched => Slot::Brain,
            Self::Confirmed => Slot::Confirmed,
            Self::Failed => Slot::Failed,
        }
    }

    /// The glyph the landing emote stamps, from the existing register.
    #[must_use]
    pub const fn glyph(self) -> Glyph {
        match self {
            Self::Dispatched => Glyph::InProgress,
            Self::Confirmed => Glyph::Confirmed,
            Self::Failed => Glyph::Failed,
        }
    }

    /// The emote's label. Extends the sheet's `✓ TASK CONFIRMED` wording
    /// rather than starting a second vocabulary.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dispatched => "TASK DISPATCHED",
            Self::Confirmed => "TASK CONFIRMED",
            Self::Failed => "TASK FAILED",
        }
    }
}

/// Which beat of a message's life a pane is showing.
///
/// The app has one landing language (see "Signature moments" in the design
/// sheet), so a departure is drawn with the same three beats as an arrival —
/// one reverse-video flash frame, then a settled badge — rather than
/// inventing a second vocabulary. What differs is the word and the glyph: a
/// departure is named by the packet's own directional symbol, which is where
/// the sheet already puts direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Beat {
    /// A message is leaving this pane, travelling this way.
    Leaving(Direction),
    /// A message has arrived here.
    Landed,
}

/// One pane's message beat this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Emote {
    /// Leaving, or landed.
    pub beat: Beat,
    /// What the message turned out to be. Colours both beats, so a return
    /// that failed leaves the worker in the failure slot rather than
    /// announcing itself in the confirmed one.
    pub outcome: Outcome,
    /// Still inside the sheet's single reverse-video flash frame.
    pub flashing: bool,
}

impl Emote {
    /// The glyph paired with the word, so colour is never load-bearing alone.
    #[must_use]
    pub const fn symbol(self, glyphs: Glyphs) -> &'static str {
        match self.beat {
            Beat::Leaving(direction) => packet(direction, glyphs),
            Beat::Landed => glyphs.get(self.outcome.glyph()),
        }
    }

    /// The beat's word. Departures are named in the present participle and
    /// arrivals in the past, which is the difference a glance has to read.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self.beat {
            Beat::Leaving(Direction::Outbound) => "HANDING OFF",
            Beat::Leaving(Direction::Inbound) => "ANSWERING",
            Beat::Landed => self.outcome.label(),
        }
    }

    /// The slot that colours it — the outcome's own, never a new one.
    #[must_use]
    pub const fn slot(self) -> Slot {
        self.outcome.slot()
    }
}

/// Which of a reviewed task's two workers a message is about.
///
/// A contracted task is executed by one harness and judged by another, and
/// `orch::review` picks the reviewer independently of the executor — so they
/// are usually different panes. The board records the two links separately
/// (`assignee_run` and `reviewer_run`); this is what says which one a given
/// history entry should be aimed at. Before issue #51 there was one link and
/// every message went down it, so a reviewer's brief and verdict were drawn
/// crossing the *executor's* connector and stamping the executor's card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lane {
    /// The worker doing the task.
    Executor,
    /// The independent reviewer judging it.
    Reviewer,
}

/// Which lane one task-history action belongs to.
///
/// Spelled out rather than tested as a `review_` prefix: the prefix happens to
/// be exact today, and a future `reviewed` or `review_requested` written by the
/// conductor rather than by a reviewer dispatch would silently join the wrong
/// lane. The whole table is pinned in `tests/task_vocabulary.rs` beside the
/// classification it accompanies.
#[must_use]
pub const fn lane_for(action: &str) -> Lane {
    match action.as_bytes() {
        b"review_delivery_confirmed"
        | b"review_delivery_failed"
        | b"review_execution_succeeded"
        | b"review_execution_failed"
        | b"review_execution_orphaned" => Lane::Reviewer,
        _ => Lane::Executor,
    }
}

/// Classify one task-history entry into the message vocabulary.
///
/// Takes the action *and* the destination status, because completion is not an
/// action at all: `orc_core::tasks::move_task` records **every** status
/// transition as `moved` and puts the target in `to`. The first version of this
/// matched on the action word alone and on four words nothing in the workspace
/// writes (`dispatched`, `delivery_started`, `done`, `failed`), so a finished
/// task raised nothing and three arms were dead. Every test around it passed,
/// because they all built their own history and shared the same wrong
/// assumption — which is why the test that pins this now drives the real
/// `orc-core` API (`tests/task_vocabulary.rs`).
///
/// **`delivery_confirmed` is not a return.** Issue #49: it is written by
/// `dispatch_supervisor::mark_started` from the `on_started` callback,
/// immediately after `command.spawn()`, and its own detail string says
/// "worker running". Classifying it `Inbound` meant the answer appeared to
/// come back milliseconds after the brief left, for every dispatch, however
/// long the worker actually took — the whole board's story about a delegation
/// was over before the work began. It is now what it always was: the outbound
/// journey *arriving*, confirmed, at the worker. The genuine return is
/// `execution_succeeded`, appended by `persist_terminal` once the child has
/// exited and both its pipes are drained.
///
/// The full vocabulary `orc-core` writes is nineteen actions. Only traffic
/// between the conductor and a worker animates; the rest stay silent, and each
/// silence is a decision rather than an omission:
///
/// - `created`, `isolated`, `report_persisted` — bookkeeping.
/// - `delivery_queued` — the brief is waiting on a quota slot. Nothing has
///   left the conductor yet, so nothing should appear to cross the wire.
/// - `merged` — always follows `moved`→`done`, which has already animated.
/// - `moved` to anything but `done` — an intermediate status change, not an
///   arrival.
///
/// Takes primitives rather than a proto type so this module stays free of the
/// protocol and the vocabulary is testable from anywhere. The whole table is
/// pinned against the real API in `tests/task_vocabulary.rs`.
#[must_use]
pub fn message_for(action: &str, to: Option<&str>) -> Option<(Direction, Outcome)> {
    match (action, to) {
        // The brief being handed over. `reassigned` is the same event with a
        // different previous owner.
        ("assigned" | "reassigned", _) => Some((Direction::Outbound, Outcome::Dispatched)),
        // The same journey, landing: the worker took the brief and started.
        // Deliberately animated as well as `assigned`, even though the two are
        // usually microseconds apart on a first dispatch, because `assigned` is
        // not always written. `orch::review` dispatches a reviewer without ever
        // assigning, and `orch::delegate` skips `assign_task` for a task that
        // is already `running` — a retry against the same worker. On both
        // paths this is the *only* record that a brief left the conductor, so
        // silencing it would lose the hand-off entirely. Where both do occur
        // they read as sent-then-taken along one wire, in two colours, rather
        // than as the two-packets-in-opposite-directions blip #49 reported.
        ("delivery_confirmed" | "review_delivery_confirmed", _) => {
            Some((Direction::Outbound, Outcome::Confirmed))
        }
        // The answer coming back. `execution_*` are the supervisor's own
        // `ExecutionStatus` words, written at the moment the worker exited.
        ("execution_succeeded" | "review_execution_succeeded", _) | ("moved", Some("done")) => {
            Some((Direction::Inbound, Outcome::Confirmed))
        }
        // `drop_task` and the isolation/merge failures record their own action
        // word rather than a `moved` transition — which is exactly the kind of
        // thing guessing at this table got wrong.
        // `execution_orphaned` is a return like the rest of this arm — the
        // brief left, and what came back is that nobody is coming back. It is
        // a *failure* on the wire without being `execution_failed` on the
        // board: those two mean different things and the board keeps them
        // apart, because an orphaned worker was killed mid-flight and nothing
        // knows what it had done. STAGE has three outcome slots and no fourth
        // is warranted for it — a new slot would reorder three palettes and
        // every golden's legend to say "failed, differently".
        (
            "execution_failed"
            | "review_execution_failed"
            | "execution_orphaned"
            | "review_execution_orphaned"
            | "delivery_failed"
            | "review_delivery_failed"
            | "dropped"
            | "merge_conflict"
            | "isolation_unavailable",
            _,
        ) => Some((Direction::Inbound, Outcome::Failed)),
        _ => None,
    }
}

/// Where a message has got to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flight {
    /// Still crossing: the index along the route, in traversal order.
    Travelling(usize),
    /// Arrived; the emote is showing on the destination pane. Carries how
    /// long it has been showing, which is what separates the sheet's
    /// reverse-video flash frame from the settled badge after it.
    Landed {
        /// How long the emote has been showing.
        since: Duration,
    },
    /// Finished. The emote's lifetime has run out and it leaves no residue.
    Gone,
}

/// How long a packet takes to cross a route of `len` cells.
///
/// A pure function of the route, so the emote's clock starts when the packet
/// really arrived rather than at a boundary chosen for looks: a two-cell route
/// lands in 60 ms and a forty-cell one in 1.2 s.
#[must_use]
pub const fn travel_time(len: usize) -> Duration {
    Duration::from_millis((len as u64).saturating_mul(FLIGHT_MS_PER_CELL))
}

/// Resolve a message's progress from the clock the caller owns.
///
/// Position is a continuous function of `since_raised` and nothing else, so
/// the poll cadence is free: drawing twice as often samples the same travel
/// twice as finely and cannot speed it up, slow it down, or move the packet at
/// a given instant. That is the "no fake timers" property, and it is what
/// makes the packet advance one cell at a time rather than in two-cell jumps.
///
/// Under reduced motion there is no travel at all: the message is `Landed`
/// from the first frame and simply holds, so the connector goes solid in its
/// colour and the emote appears already settled. Same information, no packet
/// anywhere on the rail.
#[must_use]
pub fn flight(reduced_motion: bool, since_raised: Duration, len: usize) -> Flight {
    if reduced_motion {
        return if since_raised < EMOTE_HOLD {
            Flight::Landed {
                since: since_raised,
            }
        } else {
            Flight::Gone
        };
    }
    let step = usize::try_from(since_raised.as_millis() / u128::from(FLIGHT_MS_PER_CELL))
        .unwrap_or(usize::MAX);
    if step < len {
        return Flight::Travelling(step);
    }
    let travel = travel_time(len);
    if since_raised < travel.saturating_add(EMOTE_HOLD) {
        Flight::Landed {
            since: since_raised.saturating_sub(travel),
        }
    } else {
        Flight::Gone
    }
}

/// The packet's own symbol, which is what carries its direction.
///
/// One cell against the ambient pulse's three, so the two stay tellable apart
/// with colour removed entirely.
#[must_use]
pub const fn packet(direction: Direction, glyphs: Glyphs) -> &'static str {
    match (direction, glyphs.tier()) {
        (Direction::Outbound, GlyphTier::Unicode) => "▶",
        (Direction::Outbound, GlyphTier::Ascii) => ">",
        (Direction::Inbound, GlyphTier::Unicode) => "◀",
        (Direction::Inbound, GlyphTier::Ascii) => "<",
    }
}

/// Map a travel step onto a route cell.
///
/// A route is always ordered conductor-end first, so an inbound message walks
/// it backwards rather than needing a second route.
#[must_use]
pub const fn along(direction: Direction, step: usize, len: usize) -> usize {
    match direction {
        Direction::Outbound => step,
        Direction::Inbound => len.saturating_sub(1).saturating_sub(step),
    }
}

/// What to paint in one cell of a route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Paint {
    /// The symbol, already resolved for the terminal's glyph tier.
    pub symbol: &'static str,
    /// The semantic slot that colours it. Resolved by the caller, so this
    /// module never sees a `Color`.
    pub slot: Slot,
    /// Whether this cell is part of the travelling packet rather than the wire
    /// underneath it.
    pub packet: bool,
}

/// Lay the twelve-cell rail onto a path of `len` cells and read off cell
/// `index`.
///
/// The rail's frame maths is #13's and stays untouched, so a route of any
/// length *samples* it: path cell `j` of `L` takes rail cell `j * 12 / L`. The
/// packet stretches on a long wire and compresses on a short one.
///
/// A packet cell keeps its block symbol whatever shape the wire is, so the
/// packet visibly rounds a corner; a wire cell keeps the baton's symbol where
/// the wire runs flat — that is the sheet's dotted idle rule and its solid
/// reduced-motion bar — and its own box drawing where the wire turns.
#[must_use]
pub fn paint(state: baton::State, wire: Wire, index: usize, len: usize, glyphs: Glyphs) -> Paint {
    let rail = baton::cells(state);
    let sampled = if len <= 1 {
        0
    } else {
        (index * baton::CELLS / len).min(baton::CELLS - 1)
    };
    let cell = rail[sampled];
    let packet = matches!(
        cell,
        baton::Cell::Head | baton::Cell::Body | baton::Cell::Tail
    );
    Paint {
        symbol: if packet || wire.is_horizontal() {
            cell.symbol(glyphs)
        } else {
            wire.symbol(glyphs)
        },
        slot: cell.slot(),
        packet,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use ratatui::layout::Rect;

    use super::{
        Circuit, EMOTE_HOLD, FLIGHT_MS_PER_CELL, Flight, Paint, Routing, Wire, flight, paint, plan,
        rail_span, travel_time,
    };
    use crate::baton;
    use crate::glyph::{GlyphTier, Glyphs};
    use crate::theme::Slot;

    const UNICODE: Glyphs = Glyphs::new(GlyphTier::Unicode);
    const ASCII: Glyphs = Glyphs::new(GlyphTier::Ascii);

    /// Where a render loop polling every `interval_ms` would draw the packet,
    /// as `(elapsed_ms, cell)` pairs for every frame it is still travelling.
    ///
    /// This is the whole of the acceptance check: two loops running at
    /// different cadences over the same wall clock, compared against each
    /// other rather than against a number someone chose.
    fn sampled(interval_ms: u64, len: usize) -> Vec<(u64, usize)> {
        (0..)
            .map(|tick| tick * interval_ms)
            .take_while(|elapsed| *elapsed <= travel_time(len).as_millis() as u64)
            .filter_map(
                |elapsed| match flight(false, Duration::from_millis(elapsed), len) {
                    Flight::Travelling(step) => Some((elapsed, step)),
                    Flight::Landed { .. } | Flight::Gone => None,
                },
            )
            .collect()
    }

    #[test]
    fn the_packet_crosses_one_cell_at_a_time_and_skips_none_of_them() {
        // Issue #49 AC3: "halving the frame interval must not change where the
        // packet is at time t".
        //
        // Half of that is structural and needs no test — and a test for it
        // would be a tautology worth naming, because one was written here
        // first. `flight` takes a `Duration` and nothing else, so its result
        // cannot depend on a poll cadence for any implementation, including the
        // one this issue replaced; sampling the same pure function on a 30 ms
        // grid and a 15 ms grid and finding they agree proves only that the
        // coarse grid is a subset of the fine one.
        //
        // The half that *did* fail is this one. Two cells per 60 ms frame means
        // the packet is never drawn in an odd-numbered cell at all, however
        // fast the loop polls — the chunkiness the issue reports is the
        // quantisation, not the frame rate. Reverting `flight` to the shipped
        // arithmetic fails here and nowhere else.
        const LEN: usize = 40;
        let fine = sampled(FLIGHT_MS_PER_CELL / 2, LEN);
        for pair in fine.windows(2) {
            assert!(
                pair[1].1 <= pair[0].1 + 1,
                "the packet jumped from cell {} to cell {} between {} ms and {} ms",
                pair[0].1,
                pair[1].1,
                pair[0].0,
                pair[1].0
            );
        }
        let visited = fine.iter().map(|(_, cell)| *cell).collect::<BTreeSet<_>>();
        assert_eq!(
            visited,
            (0..LEN).collect::<BTreeSet<_>>(),
            "every cell of the route is somewhere the packet is actually drawn"
        );
    }

    #[test]
    fn the_sheets_travel_speed_is_unchanged() {
        // Smoothing the cadence must not turn the sheet's "reads as a snap"
        // into flow. The twelve-cell rail took `6 frames * 60 ms` before and
        // takes `12 cells * 30 ms` now: the same 360 ms, which is why the
        // committed goldens catch the packet in the same cell.
        assert_eq!(travel_time(baton::CELLS), Duration::from_millis(360));
        assert_eq!(
            flight(false, Duration::from_millis(180), 40),
            Flight::Travelling(6),
            "the snapshot fixture's 180 ms is still six cells along"
        );
        // And the emote's clock starts when the packet really arrived.
        assert_eq!(
            flight(false, travel_time(12), 12),
            Flight::Landed {
                since: Duration::ZERO
            }
        );
        assert_eq!(
            flight(false, travel_time(12) + EMOTE_HOLD, 12),
            Flight::Gone
        );
    }

    #[test]
    fn a_worker_that_finishes_fast_is_not_shown_a_slow_animation() {
        // AC6, on the one boundary this module owns: no phase here has a
        // duration chosen for looks. Travel is `len` cells of real route at a
        // fixed speed, so a two-cell connector lands in 60 ms — the packet
        // never stretches to fill a longer story than the wire tells.
        assert_eq!(travel_time(2), Duration::from_millis(60));
        assert_eq!(
            flight(false, Duration::from_millis(60), 2),
            Flight::Landed {
                since: Duration::ZERO
            },
            "a short wire lands as soon as the packet has crossed it"
        );
        // The converse: nothing about elapsed time can make a packet land
        // early on a long wire.
        assert!(matches!(
            flight(false, Duration::from_millis(60), 40),
            Flight::Travelling(_)
        ));
    }

    /// The conductor and `count` workers in the wide layout `stage_areas`
    /// produces, so the routing is tested against real geometry.
    fn wide(count: usize) -> Vec<Rect> {
        let inner = Rect::new(1, 1, 117, 37);
        let brain_width = 62;
        let mut areas = vec![Rect::new(inner.x, inner.y + 3, brain_width, 29)];
        let worker_x = inner.x + brain_width + 17;
        let width = inner.right() - worker_x;
        let count16 = count as u16;
        let height = (inner.height - (count16 - 1)) / count16;
        for index in 0..count16 {
            areas.push(Rect::new(
                worker_x,
                inner.y + index * (height + 1),
                width,
                height,
            ));
        }
        areas
    }

    /// The rail portion of a route, rendered as text — the same split the
    /// draw path makes, so a test sees what the terminal would.
    fn render_row(circuit: &Circuit, route: usize, state: baton::State, glyphs: Glyphs) -> String {
        let path = &circuit.routes[route];
        let span = rail_span(path.len());
        path.iter()
            .enumerate()
            .skip(span.start)
            .take(span.len)
            .map(|(index, cell)| {
                let wire = circuit
                    .loom
                    .iter()
                    .find(|(at, _)| at == cell)
                    .map_or_else(Wire::default, |(_, wire)| *wire);
                paint(state, wire, index - span.start, span.len, glyphs).symbol
            })
            .collect()
    }

    #[test]
    fn every_worker_gets_its_own_route_at_one_three_and_six() {
        // AC1: the topology scales. Every worker is wired, and no wire cell
        // lands inside any pane.
        for count in [1, 3, 6] {
            let areas = wide(count);
            let circuit = plan(&areas).expect("wired");
            assert_eq!(circuit.routing, Routing::Elbows, "{count} workers");
            assert_eq!(
                circuit.routes.len(),
                count,
                "{count} workers: one route each"
            );
            for (index, route) in circuit.routes.iter().enumerate() {
                assert!(!route.is_empty(), "{count} workers: worker {index} unwired");
                // A route starts beside the conductor and ends beside its own
                // worker: it connects those two and nothing else.
                let worker = areas[index + 1];
                assert_eq!(
                    route.last().copied(),
                    Some((worker.x - 1, worker.y + worker.height / 2)),
                    "{count} workers: worker {index} arrives at its own left edge"
                );
                assert_eq!(
                    route.first().map(|cell| cell.0),
                    Some(areas[0].right() + 1),
                    "{count} workers: worker {index} leaves the conductor"
                );
            }
            for ((x, y), _) in &circuit.loom {
                for (index, pane) in areas.iter().enumerate() {
                    assert!(
                        !pane.contains((*x, *y).into()),
                        "{count} workers: wire cell {:?} overlaps pane {index}",
                        (x, y)
                    );
                }
            }
        }
    }

    #[test]
    fn a_dragged_pane_clips_the_wiring_rather_than_being_painted_over() {
        // AC1's "no connector overlaps a pane" was only ever tested against the
        // layouts `stage_areas` computes, where the gutter is the gutter and
        // there is nothing to clip. A drag puts a pane anywhere — including
        // straight over the trunk — and that case was unguarded: the wire was
        // painted across the hosted CLI's own output.
        // The fixture has to *overlap*, which the first version of this test
        // did not: it aimed at `routes[0].max()`, which is the far end of a
        // spur rather than the trunk, so the pane landed six columns clear and
        // its rows missed the one spur row in range by one. `clip` had nothing
        // to remove, so removing `clip` changed nothing and the test passed
        // either way. The case that actually occurs is one worker dragged
        // across *another* worker's spur row, so that is what this builds.
        let mut areas = wide(3);
        let spur_row = areas[2].y + areas[2].height / 2;
        let covered = (75, spur_row);
        let clear = (67, spur_row);
        // Worker 3, dragged left and up until it sits on worker 2's spur. Its
        // left edge stays right of the conductor's gutter so the layout is
        // still routed rather than falling back to the inlaid tier.
        areas[3] = Rect::new(70, spur_row - 4, 30, 10);
        assert!(
            areas[3].contains(covered.into()),
            "fixture must actually cover part of a wire"
        );

        let circuit = plan(&areas).expect("wired");
        assert_eq!(circuit.routing, Routing::Elbows, "still routed, not inlaid");

        // The property: nothing is painted inside a pane.
        for ((x, y), _) in &circuit.loom {
            for (index, pane) in areas.iter().enumerate() {
                assert!(
                    !pane.contains((*x, *y).into()),
                    "wire cell {:?} is painted inside pane {index}",
                    (x, y)
                );
            }
        }
        for (index, route) in circuit.routes.iter().enumerate() {
            for cell in route {
                assert!(
                    !areas.iter().any(|pane| pane.contains((*cell).into())),
                    "worker {index}'s packet would travel through a pane at {cell:?}"
                );
            }
        }

        // And the positive half, which is what stops this being vacuous: the
        // covered cell is gone *and* the rest of the same wire is still there,
        // so the loom was clipped rather than never having reached that far.
        let has = |cell: (u16, u16)| circuit.loom.iter().any(|(at, _)| *at == cell);
        assert!(
            !has(covered),
            "the covered cell {covered:?} must be clipped"
        );
        assert!(
            has(clear),
            "the uncovered part of the same wire {clear:?} must survive"
        );
        assert!(
            circuit.routes.iter().any(|route| !route.is_empty()),
            "clipping must not erase the whole loom"
        );
    }

    #[test]
    fn a_single_worker_is_the_design_sheets_straight_rail() {
        // The sheet's canonical picture is one horizontal filament. Putting the
        // port at the centre of the fan rather than at the conductor's own
        // middle is what keeps it straight instead of adding a one-row jog.
        let circuit = plan(&wide(1)).expect("wired");
        let route = &circuit.routes[0];
        let rows = route.iter().map(|(_, y)| *y).collect::<Vec<_>>();
        assert!(
            rows.windows(2).all(|pair| pair[0] == pair[1]),
            "one worker routes dead straight: {rows:?}"
        );
        assert_eq!(
            route.len(),
            baton::CELLS + 4,
            "and it is exactly the sheet's twelve cells plus its two endpoints"
        );
        // With the endpoints taken off, the rail samples one-to-one, so what
        // STAGE paints for one worker is still `_fillBaton`'s own frames.
        assert!(rail_span(route.len()).endpoints);
        for (state, want) in [
            (baton::State::Sweeping(0), "▓▒░─────────"),
            (baton::State::Sweeping(3), "──────▓▒░───"),
            (baton::State::Idle, "············"),
            (baton::State::Steady, "━━━━━━━━━━━━"),
        ] {
            assert_eq!(
                render_row(&circuit, 0, state, UNICODE),
                want,
                "{state:?} is the design sheet's frame, unchanged"
            );
        }
    }

    #[test]
    fn the_trunk_taps_are_junctions_not_guesses() {
        // With three workers the middle one's row is both the conductor's port
        // and a tap, so it connects on all four sides; the outer two terminate
        // the trunk. None of that is chosen by the router — it falls out of
        // which neighbours each cell was linked to.
        let circuit = plan(&wide(3)).expect("wired");
        let shape = |x: u16, y: u16| {
            circuit
                .loom
                .iter()
                .find(|(at, _)| *at == (x, y))
                .map(|(_, wire)| wire.symbol(UNICODE))
        };
        let trunk_x = circuit.routes[0]
            .iter()
            .map(|(x, _)| *x)
            .max_by_key(|x| circuit.routes[0].iter().filter(|(cx, _)| cx == x).count())
            .expect("trunk column");
        let rows = (0..3)
            .map(|index| {
                let worker = wide(3)[index + 1];
                worker.y + worker.height / 2
            })
            .collect::<Vec<_>>();
        assert_eq!(shape(trunk_x, rows[0]), Some("╭"), "top of the trunk");
        assert_eq!(shape(trunk_x, rows[1]), Some("┼"), "the port is also a tap");
        assert_eq!(shape(trunk_x, rows[2]), Some("╰"), "bottom of the trunk");
        assert_eq!(
            shape(trunk_x, rows[0] + 1),
            Some("│"),
            "and plain trunk in between"
        );
    }

    #[test]
    fn stacked_panes_get_an_inlaid_rail_rather_than_no_rail_at_all() {
        // AC8. Below the routing width `stage_areas` stacks every pane
        // full-width, so there is no gutter and a left-to-right wire would be
        // meaningless. The shipped code simply stopped drawing, which is the
        // silent disappearance this forbids.
        let stacked = vec![
            Rect::new(1, 1, 70, 7),
            Rect::new(1, 9, 70, 7),
            Rect::new(1, 17, 70, 7),
        ];
        let circuit = plan(&stacked).expect("wired");
        assert_eq!(circuit.routing, Routing::Inlaid);
        assert_eq!(circuit.routes.len(), 2, "a connector per worker, still");
        for (index, route) in circuit.routes.iter().enumerate() {
            let worker = stacked[index + 1];
            assert!(!route.is_empty(), "worker {index} has a visible connector");
            assert!(
                route.iter().all(|(_, y)| *y == worker.y),
                "worker {index}'s connector sits in its own top border"
            );
            assert!(
                route.iter().all(|(x, _)| *x < worker.right() - 1),
                "worker {index}'s connector stays inside its own corner"
            );
        }
        assert!(
            render_row(&circuit, 0, baton::State::Sweeping(0), UNICODE).contains('▓'),
            "and motion still reads there"
        );
    }

    #[test]
    fn the_packet_rounds_a_corner_as_a_packet_and_the_wire_shows_through_behind_it() {
        // The two halves of the sampling rule. A packet cell keeps its block
        // whatever shape the wire is, so travel stays legible around a bend;
        // a wire cell keeps its box drawing so the topology stays legible
        // when nothing is moving.
        let corner = Wire {
            up: true,
            down: false,
            left: false,
            right: true,
        };
        let travelling = paint(baton::State::Sweeping(0), corner, 0, 12, UNICODE);
        assert_eq!(
            travelling,
            Paint {
                symbol: "▓",
                slot: Slot::Confirmed,
                packet: true
            }
        );
        let quiet = paint(baton::State::Idle, corner, 0, 12, UNICODE);
        assert_eq!(
            quiet,
            Paint {
                symbol: "╰",
                slot: Slot::Faint,
                packet: false
            },
            "an idle corner is still a corner"
        );
    }

    #[test]
    fn a_long_route_stretches_the_rail_rather_than_truncating_it() {
        // #13 owns the twelve-cell frame maths and it is out of scope here, so
        // a route of any length samples it instead of re-deriving it. Every
        // frame's packet must still appear, at a position that tracks the
        // frame.
        let long = 40;
        let wire = Wire {
            up: false,
            down: false,
            left: true,
            right: true,
        };
        let head_at = |frame: usize| {
            (0..long).find(|index| {
                paint(baton::State::Sweeping(frame), wire, *index, long, UNICODE).symbol == "▓"
            })
        };
        let mut previous = None;
        for frame in 0..baton::FRAMES - 1 {
            let head = head_at(frame).unwrap_or_else(|| panic!("frame {frame} lost its packet"));
            if let Some(previous) = previous {
                assert!(head > previous, "frame {frame} moved backwards");
            }
            previous = Some(head);
        }
        assert_eq!(
            head_at(0),
            Some(0),
            "the packet still enters at the conductor"
        );
    }

    #[test]
    fn the_ascii_column_keeps_every_wire_cell_one_column_wide() {
        // Nothing in the register enforces width parity — `get` returns a
        // `&'static str` and several concepts are multi-cell — so a wire
        // alphabet that widened in ASCII would shear the whole loom by a
        // column. Both tiers, every shape.
        for up in [false, true] {
            for down in [false, true] {
                for left in [false, true] {
                    for right in [false, true] {
                        let wire = Wire {
                            up,
                            down,
                            left,
                            right,
                        };
                        for glyphs in [UNICODE, ASCII] {
                            assert_eq!(
                                wire.symbol(glyphs).chars().count(),
                                1,
                                "{wire:?} in {:?}",
                                glyphs.tier()
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(
            render_row(
                &plan(&wide(1)).expect("wired"),
                0,
                baton::State::Sweeping(0),
                ASCII
            ),
            "#+:---------",
            "and the ASCII rail is still the baton's own alphabet where it runs flat"
        );
    }
}
