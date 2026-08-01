//! The glyph register: a symbol for every state, so colour is never
//! load-bearing alone.
//!
//! Transcribed from the glyph table in `docs/design/visual-identity.md`. Each
//! entry carries three things: the Unicode glyph the design draws, the Nerd
//! Font icon it corresponds to, and an ASCII fallback that survives a terminal
//! which cannot render the Unicode form at all.
//!
//! **What the detection actually probes.** A terminal cannot be asked which
//! font it is using, so this module does not claim to know. What it can
//! establish is whether the session is UTF-8, which is what decides between
//! the Unicode and ASCII columns — the register's glyphs are ordinary
//! geometric-shape and box-drawing codepoints, not Private Use Area icons.
//! `ORC_NERD_FONT=1` is honoured as a user's explicit statement that their
//! terminal renders the register single-cell, and `ORC_GLYPHS` overrides the
//! probe outright. Nothing here infers a font from a terminal name.

/// A concept in the register. Every state the UI can show has one.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Glyph {
    /// The conductor — the one expensive brain.
    Conductor,
    /// A worker seated on the bench.
    WorkerSeated,
    /// An empty seat on the bench.
    WorkerIdle,
    /// The baton filament connecting conductor to worker.
    Baton,
    /// Output flowing: the conductor is producing.
    Pulse,
    /// A task whose delivery was confirmed.
    Confirmed,
    /// A task queued or assigned but not started.
    Pending,
    /// A task in progress.
    InProgress,
    /// A failed task or a dead pane.
    Failed,
    /// The conductor pane is down and recoverable.
    ConductorDown,
    /// A durable session running detached.
    Detached,
    /// A durable session on the shelf.
    DurableSession,
    /// A harness resolved on PATH.
    Available,
    /// A harness not resolved on PATH.
    Unavailable,
    /// The affordance that recovers a down conductor.
    RecoveryHint,
    /// A worker throughput sparkline.
    Sparkline,
    /// The rail down the left of the brief sidecar's band (issue #49 phase 3).
    RevealRail,
    /// The brief sidecar is open on this pane.
    RevealOpen,
    /// More of the brief exists below what is shown.
    RevealMore,
    /// The byte log holds more above the window that was read.
    RevealClip,
    /// Something was elided to fit.
    Elide,
}

/// One register row.
struct Entry {
    /// The glyph the design draws.
    unicode: &'static str,
    /// The Nerd Font icon this glyph stands for, recorded so the register
    /// stays traceable to the design sheet.
    nerd: &'static str,
    /// What a non-UTF-8 terminal gets instead.
    ascii: &'static str,
}

const fn entry(unicode: &'static str, nerd: &'static str, ascii: &'static str) -> Entry {
    Entry {
        unicode,
        nerd,
        ascii,
    }
}

impl Glyph {
    /// Every registered concept, so a new state cannot be added without a
    /// glyph and an ASCII fallback.
    pub const ALL: [Self; 21] = [
        Self::Conductor,
        Self::WorkerSeated,
        Self::WorkerIdle,
        Self::Baton,
        Self::Pulse,
        Self::Confirmed,
        Self::Pending,
        Self::InProgress,
        Self::Failed,
        Self::ConductorDown,
        Self::Detached,
        Self::DurableSession,
        Self::Available,
        Self::Unavailable,
        Self::RecoveryHint,
        Self::Sparkline,
        Self::RevealRail,
        Self::RevealOpen,
        Self::RevealMore,
        Self::RevealClip,
        Self::Elide,
    ];

    const fn entry(self) -> Entry {
        match self {
            Self::Conductor => entry("◆", "nf-md-brain", "(*)"),
            Self::WorkerSeated => entry("●", "nf-cod-server_process", "[w]"),
            Self::WorkerIdle => entry("○", "nf-cod-circle_outline", "( )"),
            Self::Baton => entry("╺━╸", "box-drawing (native)", "->"),
            Self::Pulse => entry("⠿", "braille (native)", "~"),
            Self::Confirmed => entry("✓", "nf-fa-check", "[x]"),
            Self::Pending => entry("◔", "nf-md-timer_sand", "..."),
            Self::InProgress => entry("◑", "nf-md-progress_clock", ">>"),
            Self::Failed => entry("✕", "nf-fa-times", "X"),
            Self::ConductorDown => entry("⏻", "nf-md-power_sleep", "DOWN"),
            Self::Detached => entry("⊘", "nf-md-lan_disconnect", "~/~"),
            Self::DurableSession => entry("⏻", "nf-md-content_save_move", "[S]"),
            Self::Available => entry("●", "nf-fa-check_circle", "+"),
            Self::RevealRail => entry("▌", "block elements (native)", "|"),
            Self::RevealOpen => entry("▤", "nf-md-text_box_outline", "[B]"),
            Self::RevealMore => entry("⏷", "nf-md-chevron_down", "v"),
            Self::RevealClip => entry("⏶", "nf-md-chevron_up", "^"),
            // Registered rather than written as a bare literal so reveal code
            // has nothing to mojibake on a non-UTF-8 terminal. `clip_ellipsis`
            // (lib.rs:1586) still pushes a hard `…` on the ASCII tier; that is
            // a pre-existing defect, reported not fixed here, and reveal code
            // does not call it.
            Self::Elide => entry("…", "nf-md-dots_horizontal", "..."),
            Self::Unavailable => entry("○", "nf-fa-circle_o", "-"),
            Self::RecoveryHint => entry("↺", "nf-md-restore", "R"),
            Self::Sparkline => entry("▁▂▃▅▇", "block elements (native)", ".:|"),
        }
    }

    /// The Nerd Font icon name this concept maps to in the design sheet.
    #[must_use]
    pub const fn nerd_name(self) -> &'static str {
        self.entry().nerd
    }
}

/// Which column of the register a terminal gets.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GlyphTier {
    /// The design's Unicode glyphs.
    #[default]
    Unicode,
    /// The ASCII fallback column.
    Ascii,
}

impl GlyphTier {
    /// Probe the process environment for the terminal's glyph support.
    #[must_use]
    pub fn detect() -> Self {
        Self::from_env(|key| std::env::var(key).ok())
    }

    /// The probe, with the environment injected so it is testable.
    ///
    /// `ORC_GLYPHS` is an explicit answer and wins. `ORC_NERD_FONT` is the
    /// user asserting a Nerd Font is installed, which implies UTF-8. Failing
    /// both, the locale decides: a session that is not UTF-8 cannot render the
    /// register, so it gets ASCII rather than mojibake.
    #[must_use]
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        if let Some(forced) = get("ORC_GLYPHS") {
            match forced.trim().to_ascii_lowercase().as_str() {
                "ascii" | "plain" => return Self::Ascii,
                "unicode" | "nerd" => return Self::Unicode,
                _ => {}
            }
        }
        if get("ORC_NERD_FONT").is_some_and(|value| {
            let value = value.trim();
            !value.is_empty() && value != "0"
        }) {
            return Self::Unicode;
        }
        for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
            let Some(locale) = get(key) else {
                continue;
            };
            let locale = locale.to_ascii_lowercase();
            return if locale.contains("utf-8") || locale.contains("utf8") {
                Self::Unicode
            } else {
                Self::Ascii
            };
        }
        Self::Ascii
    }
}

/// The register resolved against one terminal.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Glyphs {
    tier: GlyphTier,
}

impl Glyphs {
    /// Build a register for a known tier.
    #[must_use]
    pub const fn new(tier: GlyphTier) -> Self {
        Self { tier }
    }

    /// Which column this register is answering from.
    #[must_use]
    pub const fn tier(self) -> GlyphTier {
        self.tier
    }

    /// The symbol for one concept.
    #[must_use]
    pub const fn get(self, glyph: Glyph) -> &'static str {
        let entry = glyph.entry();
        match self.tier {
            GlyphTier::Unicode => entry.unicode,
            GlyphTier::Ascii => entry.ascii,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Glyph, GlyphTier, Glyphs};

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key| {
            owned
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn every_concept_has_both_a_glyph_and_an_ascii_fallback() {
        let unicode = Glyphs::new(GlyphTier::Unicode);
        let ascii = Glyphs::new(GlyphTier::Ascii);
        for glyph in Glyph::ALL {
            assert!(!unicode.get(glyph).is_empty(), "{glyph:?} unicode");
            assert!(!ascii.get(glyph).is_empty(), "{glyph:?} ascii");
            assert!(!glyph.nerd_name().is_empty(), "{glyph:?} nerd name");
            assert!(
                ascii.get(glyph).is_ascii(),
                "{glyph:?} fallback is not ASCII: {:?}",
                ascii.get(glyph)
            );
        }
    }

    #[test]
    fn the_states_a_reader_must_tell_apart_never_share_a_symbol() {
        // Colour is never load-bearing alone, so the states that a monochrome
        // terminal has to separate must separate on the glyph itself.
        let distinct = [
            Glyph::Confirmed,
            Glyph::Pending,
            Glyph::InProgress,
            Glyph::Failed,
            Glyph::ConductorDown,
            Glyph::Detached,
        ];
        for tier in [GlyphTier::Unicode, GlyphTier::Ascii] {
            let glyphs = Glyphs::new(tier);
            for (index, left) in distinct.iter().enumerate() {
                for right in &distinct[index + 1..] {
                    assert_ne!(
                        glyphs.get(*left),
                        glyphs.get(*right),
                        "{tier:?}: {left:?} and {right:?} share a symbol"
                    );
                }
            }
        }
        // Available and unavailable are the other pair a reader must separate.
        for tier in [GlyphTier::Unicode, GlyphTier::Ascii] {
            let glyphs = Glyphs::new(tier);
            assert_ne!(
                glyphs.get(Glyph::Available),
                glyphs.get(Glyph::Unavailable),
                "{tier:?}: PATH availability is not distinguishable"
            );
        }
    }

    #[test]
    fn the_register_matches_the_design_sheet() {
        let glyphs = Glyphs::new(GlyphTier::Unicode);
        assert_eq!(glyphs.get(Glyph::Conductor), "◆");
        assert_eq!(glyphs.get(Glyph::WorkerSeated), "●");
        assert_eq!(glyphs.get(Glyph::WorkerIdle), "○");
        assert_eq!(glyphs.get(Glyph::Pulse), "⠿");
        assert_eq!(glyphs.get(Glyph::Confirmed), "✓");
        assert_eq!(glyphs.get(Glyph::Pending), "◔");
        assert_eq!(glyphs.get(Glyph::InProgress), "◑");
        assert_eq!(glyphs.get(Glyph::Failed), "✕");
        assert_eq!(glyphs.get(Glyph::ConductorDown), "⏻");
        assert_eq!(glyphs.get(Glyph::Detached), "⊘");
        assert_eq!(Glyph::Conductor.nerd_name(), "nf-md-brain");
        assert_eq!(Glyph::Confirmed.nerd_name(), "nf-fa-check");
    }

    #[test]
    fn glyph_tier_is_probed_from_the_locale_never_from_a_terminal_name() {
        assert_eq!(
            GlyphTier::from_env(env(&[("LANG", "en_US.UTF-8")])),
            GlyphTier::Unicode
        );
        assert_eq!(
            GlyphTier::from_env(env(&[("LC_ALL", "C.utf8")])),
            GlyphTier::Unicode
        );
        assert_eq!(GlyphTier::from_env(env(&[("LANG", "C")])), GlyphTier::Ascii);
        assert_eq!(
            GlyphTier::from_env(env(&[("LC_ALL", "POSIX"), ("LANG", "en_US.UTF-8")])),
            GlyphTier::Ascii,
            "LC_ALL outranks LANG"
        );
        // No locale at all: assume nothing, fall back.
        assert_eq!(
            GlyphTier::from_env(env(&[("TERM", "xterm-256color")])),
            GlyphTier::Ascii
        );
        // Explicit user statements win.
        assert_eq!(
            GlyphTier::from_env(env(&[("ORC_NERD_FONT", "1"), ("LANG", "C")])),
            GlyphTier::Unicode
        );
        assert_eq!(
            GlyphTier::from_env(env(&[("ORC_NERD_FONT", "0"), ("LANG", "C")])),
            GlyphTier::Ascii
        );
        assert_eq!(
            GlyphTier::from_env(env(&[("ORC_GLYPHS", "ascii"), ("LANG", "en_US.UTF-8")])),
            GlyphTier::Ascii
        );
        assert_eq!(
            GlyphTier::from_env(env(&[("ORC_GLYPHS", "unicode"), ("LANG", "C")])),
            GlyphTier::Unicode
        );
        // An unparseable override falls through to the probe.
        assert_eq!(
            GlyphTier::from_env(env(&[("ORC_GLYPHS", "emoji"), ("LANG", "C")])),
            GlyphTier::Ascii
        );
    }
}
