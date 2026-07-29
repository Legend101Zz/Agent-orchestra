//! The baton: the filament between conductor and worker, and the signature
//! motion of the app.
//!
//! Implements the pulse spec in `docs/design/visual-identity.md` and its frame
//! generator in the identity HTML (`_fillBaton`): a twelve-cell rail carrying a
//! three-cell packet `▓▒░` that sweeps left to right — conductor to worker,
//! one direction only — at roughly 110 ms per frame, decaying to a dim dotted
//! rail after 400 ms of silence.
//!
//! Everything here is a pure function of a state value. The clock lives in the
//! caller, so a snapshot can pin an exact frame instead of racing one.

use std::time::Duration;

use crate::glyph::Glyphs;
use crate::theme::Slot;

/// Cells in the rail.
pub const CELLS: usize = 12;
/// Distinct frames in one sweep. The packet enters at cell 0 and leaves past
/// cell 11, so the last frame is an empty rail again.
pub const FRAMES: usize = 7;
/// Milliseconds each frame holds.
pub const FRAME_MS: u64 = 110;
/// Silence after which the rail decays to idle.
pub const DECAY: Duration = Duration::from_millis(400);

/// What the rail is doing right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    /// No output for at least [`DECAY`]: a dim dotted rail.
    Idle,
    /// Output is flowing but motion is reduced: a solid accent rail, no
    /// travel and no sweep.
    Steady,
    /// Frame `n` of the packet sweep, `0..FRAMES`.
    Sweeping(usize),
}

impl State {
    /// Resolve the rail's state from the two clocks the caller owns.
    ///
    /// `since_output` is time since the last stdout tick — the spec's trigger.
    /// `since_start` is time since the sweep began, which only matters while
    /// output is still flowing and motion is allowed.
    #[must_use]
    pub fn resolve(reduced_motion: bool, since_output: Duration, since_start: Duration) -> Self {
        if since_output >= DECAY {
            return Self::Idle;
        }
        if reduced_motion {
            return Self::Steady;
        }
        let frame = (since_start.as_millis() / u128::from(FRAME_MS)) % FRAMES as u128;
        Self::Sweeping(frame as usize)
    }
}

/// One cell of the rail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cell {
    /// The rail's dim base under the packet.
    Rail,
    /// The packet's leading cell.
    Head,
    /// The packet's middle cell.
    Body,
    /// The packet's trailing cell.
    Tail,
    /// The idle rail's dotted base.
    Dot,
    /// The reduced-motion rail's solid accent base.
    Solid,
}

impl Cell {
    /// The symbol for this cell, in whichever column the terminal supports.
    ///
    /// The ASCII column keeps all six cells distinct, so the packet is still
    /// legible as a moving shape on a terminal that cannot draw blocks.
    #[must_use]
    pub const fn symbol(self, glyphs: Glyphs) -> &'static str {
        match (self, glyphs.tier()) {
            (Self::Rail, crate::glyph::GlyphTier::Unicode) => "─",
            (Self::Rail, crate::glyph::GlyphTier::Ascii) => "-",
            (Self::Head, crate::glyph::GlyphTier::Unicode) => "▓",
            (Self::Head, crate::glyph::GlyphTier::Ascii) => "#",
            (Self::Body, crate::glyph::GlyphTier::Unicode) => "▒",
            (Self::Body, crate::glyph::GlyphTier::Ascii) => "+",
            (Self::Tail, crate::glyph::GlyphTier::Unicode) => "░",
            (Self::Tail, crate::glyph::GlyphTier::Ascii) => ":",
            (Self::Dot, crate::glyph::GlyphTier::Unicode) => "·",
            (Self::Dot, crate::glyph::GlyphTier::Ascii) => ".",
            (Self::Solid, crate::glyph::GlyphTier::Unicode) => "━",
            (Self::Solid, crate::glyph::GlyphTier::Ascii) => "=",
        }
    }

    /// The semantic slot that colours this cell.
    ///
    /// The packet ramps `brain` at the tail to `confirmed` at the head, which
    /// is the whole metaphor: the conductor's intent arriving as a result.
    #[must_use]
    pub const fn slot(self) -> Slot {
        match self {
            Self::Rail | Self::Dot => Slot::Faint,
            Self::Head => Slot::Confirmed,
            Self::Body | Self::Tail => Slot::Brain,
            Self::Solid => Slot::Glow,
        }
    }
}

/// The twelve cells of the rail in one state.
///
/// The sweep is the identity HTML's generator verbatim: at frame `f` the
/// packet's head sits at cell `2f`, its body at `2f + 1`, its tail at
/// `2f + 2`, and every other cell is dim rail. Cells past the end are simply
/// not drawn, so the packet leaves the rail rather than wrapping.
#[must_use]
pub fn cells(state: State) -> [Cell; CELLS] {
    match state {
        State::Idle => [Cell::Dot; CELLS],
        State::Steady => [Cell::Solid; CELLS],
        State::Sweeping(frame) => {
            let head = frame.min(FRAMES - 1) * 2;
            let mut rail = [Cell::Rail; CELLS];
            for (index, cell) in rail.iter_mut().enumerate() {
                if index == head {
                    *cell = Cell::Head;
                } else if index == head + 1 {
                    *cell = Cell::Body;
                } else if index == head + 2 {
                    *cell = Cell::Tail;
                }
            }
            rail
        }
    }
}

/// The rail rendered as text, for tests and evidence capture.
#[must_use]
pub fn render(state: State, glyphs: Glyphs) -> String {
    cells(state)
        .iter()
        .map(|cell| cell.symbol(glyphs))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{CELLS, Cell, DECAY, FRAMES, State, cells, render};
    use crate::glyph::{GlyphTier, Glyphs};
    use crate::theme::Slot;

    const UNICODE: Glyphs = Glyphs::new(GlyphTier::Unicode);

    #[test]
    fn the_sweep_frames_are_the_design_sheets_frames() {
        // `_fillBaton` in the identity HTML, transcribed: head at 2f, body at
        // 2f+1, tail at 2f+2, dim rail everywhere else, twelve cells wide.
        let expected = [
            "▓▒░─────────",
            "──▓▒░───────",
            "────▓▒░─────",
            "──────▓▒░───",
            "────────▓▒░─",
            "──────────▓▒",
            "────────────",
        ];
        assert_eq!(expected.len(), FRAMES);
        for (frame, want) in expected.iter().enumerate() {
            assert_eq!(
                render(State::Sweeping(frame), UNICODE),
                *want,
                "frame {frame}"
            );
            assert_eq!(want.chars().count(), CELLS, "frame {frame} width");
        }
    }

    #[test]
    fn the_packet_travels_one_direction_only() {
        // Conductor to worker: the head's column strictly increases, and no
        // frame ever places the packet left of where the previous one did.
        let head_at = |frame: usize| {
            cells(State::Sweeping(frame))
                .iter()
                .position(|cell| *cell == Cell::Head)
        };
        let mut previous = None;
        for frame in 0..FRAMES {
            if let Some(head) = head_at(frame) {
                if let Some(previous) = previous {
                    assert!(head > previous, "frame {frame} moved backwards");
                }
                previous = Some(head);
            }
        }
        assert_eq!(head_at(0), Some(0), "the packet enters at the conductor");
        assert_eq!(head_at(6), None, "the packet leaves past the worker");
    }

    #[test]
    fn the_packet_ramps_brain_at_the_tail_to_confirmed_at_the_head() {
        let rail = cells(State::Sweeping(1));
        assert_eq!(rail[2].slot(), Slot::Confirmed, "head");
        assert_eq!(rail[3].slot(), Slot::Brain, "body");
        assert_eq!(rail[4].slot(), Slot::Brain, "tail");
        assert_eq!(rail[0].slot(), Slot::Faint, "base rail");
    }

    #[test]
    fn silence_decays_the_rail_to_a_dim_dotted_base() {
        assert_eq!(
            State::resolve(false, DECAY, Duration::ZERO),
            State::Idle,
            "400 ms of silence is the decay boundary"
        );
        assert_eq!(
            State::resolve(false, DECAY - Duration::from_millis(1), Duration::ZERO),
            State::Sweeping(0),
            "one millisecond short of the boundary is still live"
        );
        assert_eq!(render(State::Idle, UNICODE), "············");
        assert!(
            cells(State::Idle)
                .iter()
                .all(|cell| cell.slot() == Slot::Faint),
            "the idle rail is dim"
        );
    }

    #[test]
    fn reduced_motion_gets_static_rails_and_never_travels() {
        // Live but motion-reduced: one solid accent rail, no packet anywhere.
        let live = State::resolve(true, Duration::from_millis(10), Duration::from_millis(330));
        assert_eq!(live, State::Steady);
        assert_eq!(render(live, UNICODE), "━━━━━━━━━━━━");
        assert!(
            !cells(live)
                .iter()
                .any(|cell| matches!(cell, Cell::Head | Cell::Body | Cell::Tail)),
            "reduced motion must not draw a packet"
        );
        // Idle is the same dotted rail whether motion is reduced or not.
        assert_eq!(State::resolve(true, DECAY, Duration::ZERO), State::Idle);
        // The state does not depend on the sweep clock at all, so nothing can
        // animate: two different sweep clocks give the same rail.
        assert_eq!(
            State::resolve(true, Duration::ZERO, Duration::from_millis(0)),
            State::resolve(true, Duration::ZERO, Duration::from_millis(9_999))
        );
    }

    #[test]
    fn the_sweep_advances_one_frame_per_110_ms_and_loops() {
        let at = |millis: u64| State::resolve(false, Duration::ZERO, Duration::from_millis(millis));
        assert_eq!(at(0), State::Sweeping(0));
        assert_eq!(at(109), State::Sweeping(0));
        assert_eq!(at(110), State::Sweeping(1));
        assert_eq!(at(660), State::Sweeping(6));
        assert_eq!(at(770), State::Sweeping(0), "the sweep loops");
    }

    #[test]
    fn the_ascii_rail_keeps_every_cell_distinct() {
        let ascii = Glyphs::new(GlyphTier::Ascii);
        let all = [
            Cell::Rail,
            Cell::Head,
            Cell::Body,
            Cell::Tail,
            Cell::Dot,
            Cell::Solid,
        ];
        for (index, left) in all.iter().enumerate() {
            for right in &all[index + 1..] {
                assert_ne!(
                    left.symbol(ascii),
                    right.symbol(ascii),
                    "{left:?} and {right:?} collide in ASCII"
                );
            }
        }
        assert_eq!(render(State::Sweeping(0), ascii), "#+:---------");
    }
}
