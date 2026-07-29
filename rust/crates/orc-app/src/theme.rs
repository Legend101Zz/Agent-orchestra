//! The single theme map: seventeen semantic slots across three themes.
//!
//! This module is the **only** place in the crate allowed to name a colour.
//! Widget code asks for a [`Slot`] and gets back whatever the active theme and
//! [`ColorTier`] resolve it to, so swapping a theme is remapping this table and
//! nothing else. `tests::no_hex_literals_outside_the_theme_map` enforces that.
//!
//! The palette is transcribed from `docs/design/visual-identity.md` and its
//! source of truth, `docs/design/visual-identity/Pi-Orchestra Identity.dc.html`
//! (`_fillTokens`), including that sheet's nearest-ANSI-256 fallback column.
//!
//! Colour is never load-bearing alone: every slot that names a *state* has a
//! matching glyph in [`crate::glyph`], and the monochrome tier drops colour
//! entirely so the UI must still read through layout, glyph, bold, and reverse.

use ratatui::style::{Color, Modifier, Style};

/// The number of semantic slots in the theme map.
pub const SLOTS: usize = 17;

/// One semantic role a colour can play. Widget code names these, never a hex.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Slot {
    /// `bg.base` — house lights; the deepest surface behind everything.
    Bg,
    /// `bg.surface` — panel fill, one step above the base.
    Surface,
    /// `bg.overlay` — floating pane or modal fill.
    Overlay,
    /// `border.dim` — inactive pane frame.
    Border,
    /// `border.active` — focused pane frame.
    BorderHi,
    /// `fg.default` — 80% of all text.
    Fg,
    /// `fg.muted` — metadata and labels.
    Muted,
    /// `fg.faint` — disabled text and hint keys.
    Faint,
    /// `◆` conductor accent.
    Brain,
    /// `●` bench accent.
    Worker,
    /// `✓` task confirmed.
    Confirmed,
    /// `◔` queued or assigned.
    Pending,
    /// `✕` failed or dead.
    Failed,
    /// `●` harness available on PATH.
    Avail,
    /// `○` harness not resolved on PATH.
    Unavail,
    /// Selection fill.
    Sel,
    /// Motion and pulse accent.
    Glow,
}

impl Slot {
    /// Every slot, in table order — used by the palette tests so a new slot
    /// cannot be added to the map without being covered.
    pub const ALL: [Self; SLOTS] = [
        Self::Bg,
        Self::Surface,
        Self::Overlay,
        Self::Border,
        Self::BorderHi,
        Self::Fg,
        Self::Muted,
        Self::Faint,
        Self::Brain,
        Self::Worker,
        Self::Confirmed,
        Self::Pending,
        Self::Failed,
        Self::Avail,
        Self::Unavail,
        Self::Sel,
        Self::Glow,
    ];

    /// The slot's name as the design sheet writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bg => "bg",
            Self::Surface => "surface",
            Self::Overlay => "overlay",
            Self::Border => "border",
            Self::BorderHi => "border-hi",
            Self::Fg => "fg",
            Self::Muted => "muted",
            Self::Faint => "faint",
            Self::Brain => "brain",
            Self::Worker => "worker",
            Self::Confirmed => "confirmed",
            Self::Pending => "pending",
            Self::Failed => "failed",
            Self::Avail => "avail",
            Self::Unavail => "unavail",
            Self::Sel => "sel",
            Self::Glow => "glow",
        }
    }
}

/// How much colour the terminal was found to support.
///
/// The design brief is three layers, each of which must stand alone:
/// monochrome is *usable*, sixteen ANSI colours are *readable*, truecolor is
/// *beautiful*. Nothing here is assumed — the tier comes from a probe of the
/// environment, and [`ColorTier::Monochrome`] emits no colour at all rather
/// than pretending a palette exists.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColorTier {
    /// 24-bit RGB, the design's native form.
    #[default]
    TrueColor,
    /// The xterm-256 cube, using the design sheet's nearest-index column.
    Ansi256,
    /// The sixteen base ANSI colours.
    Ansi16,
    /// `NO_COLOR`, `TERM=dumb`: no colour at all, only glyph/bold/reverse.
    Monochrome,
}

impl ColorTier {
    /// Probe the process environment for the terminal's colour support.
    #[must_use]
    pub fn detect() -> Self {
        Self::from_env(|key| std::env::var(key).ok())
    }

    /// The probe itself, with the environment injected so it is testable.
    ///
    /// Order matters. An explicit `ORC_COLOR` wins because a user who names a
    /// tier has out-argued any heuristic. `NO_COLOR` (present, per
    /// <https://no-color.org>) and `TERM=dumb` are refusals and come next.
    /// Only then do the positive signals apply.
    #[must_use]
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        if let Some(forced) = get("ORC_COLOR") {
            match forced.trim().to_ascii_lowercase().as_str() {
                "truecolor" | "24bit" => return Self::TrueColor,
                "256" | "ansi256" => return Self::Ansi256,
                "16" | "ansi16" => return Self::Ansi16,
                "none" | "0" | "mono" | "monochrome" => return Self::Monochrome,
                _ => {}
            }
        }
        if get("NO_COLOR").is_some() {
            return Self::Monochrome;
        }
        let term = get("TERM").unwrap_or_default().to_ascii_lowercase();
        if term.is_empty() || term == "dumb" {
            return Self::Monochrome;
        }
        let colorterm = get("COLORTERM").unwrap_or_default().to_ascii_lowercase();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") || term.contains("direct")
        {
            return Self::TrueColor;
        }
        if term.contains("256") {
            return Self::Ansi256;
        }
        Self::Ansi16
    }
}

/// The three approved themes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThemeName {
    /// FLAGSHIP. Stage at night: near-black blue, cool teal conductor,
    /// periwinkle bench, warm gold confirmations.
    #[default]
    Nocturne,
    /// Anchor. Warm charcoal and brass, a firelit study; olive confirmations.
    Ember,
    /// Anchor, mono. CRT green: one hue, five luminances, 16-colour safe.
    Phosphor,
}

impl ThemeName {
    /// Every approved theme, so snapshot coverage tracks the palette set
    /// automatically as themes are added.
    pub const ALL: [Self; 3] = [Self::Nocturne, Self::Ember, Self::Phosphor];

    /// Parse a configured theme name, falling back to the flagship.
    #[must_use]
    pub fn named(name: &str) -> Self {
        if name.eq_ignore_ascii_case("phosphor") {
            Self::Phosphor
        } else if name.eq_ignore_ascii_case("ember") {
            Self::Ember
        } else {
            Self::Nocturne
        }
    }

    /// The lowercase name written to `~/.orchestra/harnesses.json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nocturne => "nocturne",
            Self::Ember => "ember",
            Self::Phosphor => "phosphor",
        }
    }

    const fn palette(self) -> &'static Palette {
        match self {
            Self::Nocturne => &NOCTURNE,
            Self::Ember => &EMBER,
            Self::Phosphor => &PHOSPHOR,
        }
    }
}

/// One slot's three renderings: truecolor hex, nearest xterm-256 index, and
/// the base ANSI colour it collapses to on a sixteen-colour terminal.
struct SlotColors {
    /// `0xRRGGBB`, written exactly as the design sheet writes it.
    rgb: u32,
    /// The design sheet's nearest-ANSI-256 column.
    ansi256: u8,
    /// The 16-colour collapse.
    ansi16: Color,
}

const fn slot(rgb: u32, ansi256: u8, ansi16: Color) -> SlotColors {
    SlotColors {
        rgb,
        ansi256,
        ansi16,
    }
}

/// One theme's seventeen slots, indexed by [`Slot`] declaration order.
struct Palette {
    slots: [SlotColors; SLOTS],
}

// ---------------------------------------------------------------------------
// The map. Rows are in `Slot::ALL` order; hex and ANSI-256 come verbatim from
// the design sheet's token table (`glow` mirrors the theme's motion accent,
// which the sheet documents in prose rather than as a table row).
// ---------------------------------------------------------------------------

const NOCTURNE: Palette = Palette {
    slots: [
        slot(0x0a_0c11, 233, Color::Black),     // bg
        slot(0x10_131b, 234, Color::Black),     // surface
        slot(0x17_1b26, 235, Color::Black),     // overlay
        slot(0x26_2c3a, 238, Color::DarkGray),  // border
        slot(0x39_425a, 240, Color::Gray),      // border-hi
        slot(0xc4_cad6, 251, Color::White),     // fg
        slot(0x72_7b8f, 244, Color::Gray),      // muted
        slot(0x45_4c60, 240, Color::DarkGray),  // faint
        slot(0x5a_d1c8, 80, Color::Cyan),       // brain
        slot(0x8e_a2ff, 111, Color::LightBlue), // worker
        slot(0xe6_b450, 179, Color::Yellow),    // confirmed
        slot(0x8a_93a6, 245, Color::Gray),      // pending
        slot(0xe0_7a80, 174, Color::LightRed),  // failed
        slot(0x6f_d08c, 114, Color::Green),     // avail
        slot(0x56_5e70, 242, Color::DarkGray),  // unavail
        slot(0x1c_2740, 237, Color::Blue),      // sel
        slot(0x5a_d1c8, 80, Color::LightCyan),  // glow
    ],
};

const EMBER: Palette = Palette {
    slots: [
        slot(0x15_110c, 234, Color::Black),       // bg
        slot(0x1d_1710, 235, Color::Black),       // surface
        slot(0x28_2016, 236, Color::Black),       // overlay
        slot(0x3d_3122, 238, Color::DarkGray),    // border
        slot(0x5a_4630, 240, Color::Gray),        // border-hi
        slot(0xe9_ddc7, 223, Color::White),       // fg
        slot(0x9a_8a6c, 137, Color::Gray),        // muted
        slot(0x5f_5238, 58, Color::DarkGray),     // faint
        slot(0xd7_a355, 179, Color::Yellow),      // brain
        slot(0xcf_8148, 173, Color::LightRed),    // worker
        slot(0xa9_bd63, 107, Color::Green),       // confirmed
        slot(0x8f_7d5c, 137, Color::Gray),        // pending
        slot(0xd1_704a, 167, Color::Red),         // failed
        slot(0xb3_c56a, 107, Color::LightGreen),  // avail
        slot(0x6a_5c42, 101, Color::DarkGray),    // unavail
        slot(0x33_270f, 237, Color::Magenta),     // sel
        slot(0xd7_a355, 179, Color::LightYellow), // glow
    ],
};

const PHOSPHOR: Palette = Palette {
    slots: [
        slot(0x05_0c07, 232, Color::Black),       // bg
        slot(0x08_130c, 233, Color::Black),       // surface
        slot(0x0c_1c12, 234, Color::Black),       // overlay
        slot(0x16_4a29, 22, Color::Green),        // border
        slot(0x1f_6b3a, 28, Color::LightGreen),   // border-hi
        slot(0x48_f57a, 83, Color::LightGreen),   // fg
        slot(0x27_9c4b, 35, Color::Green),        // muted
        slot(0x15_5f2e, 22, Color::DarkGray),     // faint
        slot(0xa9_ffc3, 157, Color::LightGreen),  // brain
        slot(0x48_f57a, 83, Color::Green),        // worker
        slot(0xd6_ffe2, 195, Color::White),       // confirmed
        slot(0x27_9c4b, 35, Color::Green),        // pending
        slot(0xb6_ff54, 154, Color::LightYellow), // failed
        slot(0x7d_ff9e, 120, Color::LightGreen),  // avail
        slot(0x15_5f2e, 22, Color::DarkGray),     // unavail
        slot(0x0f_3d22, 22, Color::Green),        // sel
        slot(0x48_f57a, 83, Color::LightGreen),   // glow
    ],
};

/// The per-character gradient a highlighted conductor trigger cycles through,
/// column by column, so a spell shimmers like Claude Code's `ultrathink`.
///
/// Meaning never rides on the rainbow alone: the token stays **bold** and keeps
/// its `◆ LABEL` title badge, both of which survive a monochrome terminal — the
/// colour is decoration on top. It lives here because this module owns every
/// literal colour in the crate.
pub const TRIGGER_RAINBOW: [Color; 7] = [
    Color::Rgb(255, 107, 107), // red
    Color::Rgb(255, 169, 77),  // orange
    Color::Rgb(255, 224, 102), // yellow
    Color::Rgb(99, 230, 190),  // green
    Color::Rgb(77, 171, 247),  // blue
    Color::Rgb(177, 151, 252), // indigo
    Color::Rgb(247, 131, 172), // violet
];

/// Name a resolved colour for a snapshot or an evidence dump.
///
/// Naming colours is this module's job like everything else about them, which
/// is also why the grep gate can stay absolute: no other file in the crate has
/// any reason to mention a `Color` variant.
#[must_use]
pub fn describe(color: Color) -> String {
    match color {
        Color::Reset => "reset".to_owned(),
        Color::Rgb(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
        Color::Indexed(index) => format!("ansi{index}"),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// A theme resolved against one terminal's colour support.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Theme {
    name: ThemeName,
    tier: ColorTier,
}

impl Theme {
    /// Build a theme for a known colour tier.
    #[must_use]
    pub const fn new(name: ThemeName, tier: ColorTier) -> Self {
        Self { name, tier }
    }

    /// Which theme this is.
    #[must_use]
    pub const fn name(self) -> ThemeName {
        self.name
    }

    /// How much colour this theme is allowed to emit.
    #[must_use]
    pub const fn tier(self) -> ColorTier {
        self.tier
    }

    /// Resolve one semantic slot for this theme and tier.
    ///
    /// Under [`ColorTier::Monochrome`] every slot answers [`Color::Reset`]:
    /// the terminal's own foreground and background. That is the honest
    /// answer — with no colour to spend, state has to read from glyph,
    /// bold, dim, and reverse instead.
    #[must_use]
    pub fn slot(self, slot: Slot) -> Color {
        let colors = &self.name.palette().slots[slot as usize];
        match self.tier {
            ColorTier::TrueColor => {
                let rgb = colors.rgb;
                Color::Rgb(
                    ((rgb >> 16) & 0xff) as u8,
                    ((rgb >> 8) & 0xff) as u8,
                    (rgb & 0xff) as u8,
                )
            }
            ColorTier::Ansi256 => Color::Indexed(colors.ansi256),
            ColorTier::Ansi16 => colors.ansi16,
            ColorTier::Monochrome => Color::Reset,
        }
    }

    /// A style for a state, pairing the slot's colour with the modifier that
    /// keeps the state readable once colour is stripped.
    ///
    /// Emphatic states (a confirmation, a failure, an accent) go **bold**;
    /// recessive ones (queued, unavailable, metadata) go dim. Combined with
    /// each state's glyph this is what makes the monochrome tier usable.
    #[must_use]
    pub fn state(self, slot: Slot) -> Style {
        let style = Style::default().fg(self.slot(slot));
        match slot {
            Slot::Brain
            | Slot::Worker
            | Slot::Confirmed
            | Slot::Failed
            | Slot::Avail
            | Slot::Glow => style.add_modifier(Modifier::BOLD),
            Slot::Muted | Slot::Faint | Slot::Pending | Slot::Unavail => {
                style.add_modifier(Modifier::DIM)
            }
            _ => style,
        }
    }

    /// The selection fill.
    ///
    /// With colour available this is the `sel` slot behind default text; with
    /// none it is reverse video, which is the only selection cue a monochrome
    /// terminal has.
    #[must_use]
    pub fn selection(self) -> Style {
        if self.tier == ColorTier::Monochrome {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
                .fg(self.slot(Slot::Fg))
                .bg(self.slot(Slot::Sel))
                .add_modifier(Modifier::BOLD)
        }
    }

    /// Translate a hosted pane's own SGR colour, falling back to a slot when
    /// the pane asked for the terminal default.
    ///
    /// This replays what the harness printed; it is not theming, which is why
    /// it takes its colour from the wire rather than from the map.
    #[must_use]
    pub fn pane_color(self, color: orc_proto::TerminalColor, default: Slot) -> Color {
        match color {
            orc_proto::TerminalColor::Default => self.slot(default),
            orc_proto::TerminalColor::Indexed(index) => Color::Indexed(index),
            orc_proto::TerminalColor::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
        }
    }

    /// The embedded RUNS ledger's theme, so `orc-tui` renders in the same
    /// palette as the rest of the client instead of its own two-theme set.
    #[must_use]
    pub fn runs_theme(self) -> orc_tui::Theme {
        orc_tui::Theme {
            name: self.name.as_str(),
            bg: self.slot(Slot::Bg),
            panel: self.slot(Slot::Surface),
            surface: self.slot(Slot::Overlay),
            border: self.slot(Slot::Border),
            border_focus: self.slot(Slot::BorderHi),
            text: self.slot(Slot::Fg),
            text_dim: self.slot(Slot::Faint),
            label: self.slot(Slot::Muted),
            accent: self.slot(Slot::Brain),
            accent_2: self.slot(Slot::Glow),
            ok: self.slot(Slot::Confirmed),
            warn: self.slot(Slot::Pending),
            error: self.slot(Slot::Failed),
            running: self.slot(Slot::Worker),
            killed: self.slot(Slot::Unavail),
            controller_codex: self.slot(Slot::Brain),
            controller_claude: self.slot(Slot::Worker),
            controller_human: self.slot(Slot::Muted),
        }
    }
}

macro_rules! slot_accessors {
    ($($method:ident => $slot:ident,)*) => {
        impl Theme {
            $(
                /// This theme's colour for the matching semantic slot.
                #[must_use]
                pub fn $method(self) -> Color {
                    self.slot(Slot::$slot)
                }
            )*
        }
    };
}

slot_accessors! {
    bg => Bg,
    surface => Surface,
    overlay => Overlay,
    border => Border,
    border_hi => BorderHi,
    fg => Fg,
    muted => Muted,
    faint => Faint,
    brain => Brain,
    worker => Worker,
    confirmed => Confirmed,
    pending => Pending,
    failed => Failed,
    avail => Avail,
    unavail => Unavail,
    sel => Sel,
    glow => Glow,
}

impl From<ThemeName> for Theme {
    /// The truecolor rendering of a theme — the form snapshots and tests pin.
    fn from(name: ThemeName) -> Self {
        Self::new(name, ColorTier::TrueColor)
    }
}

#[cfg(test)]
mod tests {
    use super::{ColorTier, Slot, Theme, ThemeName};
    use ratatui::style::{Color, Modifier};

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
    fn every_theme_answers_every_slot_in_every_tier() {
        for name in ThemeName::ALL {
            for tier in [
                ColorTier::TrueColor,
                ColorTier::Ansi256,
                ColorTier::Ansi16,
                ColorTier::Monochrome,
            ] {
                let theme = Theme::new(name, tier);
                for slot in Slot::ALL {
                    let color = theme.slot(slot);
                    match tier {
                        ColorTier::TrueColor => {
                            assert!(matches!(color, Color::Rgb(..)), "{name:?} {slot:?}");
                        }
                        ColorTier::Ansi256 => {
                            assert!(matches!(color, Color::Indexed(_)), "{name:?} {slot:?}");
                        }
                        ColorTier::Ansi16 => assert!(
                            !matches!(color, Color::Rgb(..) | Color::Indexed(_) | Color::Reset),
                            "{name:?} {slot:?} must collapse to a base ANSI colour"
                        ),
                        ColorTier::Monochrome => {
                            assert_eq!(color, Color::Reset, "{name:?} {slot:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_flagship_palette_matches_the_design_sheet_exactly() {
        // Spot-checks against `_fillTokens` in the identity HTML. If the map is
        // ever retyped, these are the rows that catch a transposed digit.
        let nocturne = Theme::from(ThemeName::Nocturne);
        assert_eq!(nocturne.bg(), Color::Rgb(0x0a, 0x0c, 0x11));
        assert_eq!(nocturne.brain(), Color::Rgb(0x5a, 0xd1, 0xc8));
        assert_eq!(nocturne.worker(), Color::Rgb(0x8e, 0xa2, 0xff));
        assert_eq!(nocturne.confirmed(), Color::Rgb(0xe6, 0xb4, 0x50));

        let ember = Theme::from(ThemeName::Ember);
        assert_eq!(ember.bg(), Color::Rgb(0x15, 0x11, 0x0c));
        assert_eq!(ember.confirmed(), Color::Rgb(0xa9, 0xbd, 0x63));

        let phosphor = Theme::from(ThemeName::Phosphor);
        assert_eq!(phosphor.bg(), Color::Rgb(0x05, 0x0c, 0x07));
        assert_eq!(phosphor.fg(), Color::Rgb(0x48, 0xf5, 0x7a));

        // The ANSI-256 column is the sheet's, not a recomputation of it.
        let indexed = Theme::new(ThemeName::Nocturne, ColorTier::Ansi256);
        assert_eq!(indexed.bg(), Color::Indexed(233));
        assert_eq!(indexed.surface(), Color::Indexed(234));
        assert_eq!(indexed.overlay(), Color::Indexed(235));
        assert_eq!(indexed.border(), Color::Indexed(238));
        assert_eq!(indexed.border_hi(), Color::Indexed(240));
        assert_eq!(indexed.fg(), Color::Indexed(251));
        assert_eq!(indexed.muted(), Color::Indexed(244));
        assert_eq!(indexed.faint(), Color::Indexed(240));
    }

    #[test]
    fn phosphor_is_one_hue_and_the_others_are_not() {
        // The mono theme's identity claim: every chromatic slot is a green.
        let phosphor = Theme::from(ThemeName::Phosphor);
        for slot in [
            Slot::Fg,
            Slot::Muted,
            Slot::Brain,
            Slot::Worker,
            Slot::Avail,
        ] {
            let Color::Rgb(red, green, blue) = phosphor.slot(slot) else {
                panic!("{slot:?} is not truecolor");
            };
            assert!(
                green > red && green > blue,
                "{slot:?} is not a phosphor green"
            );
        }
    }

    #[test]
    fn each_theme_is_distinct_from_the_others() {
        for slot in [Slot::Bg, Slot::Fg, Slot::Brain] {
            let colors: Vec<_> = ThemeName::ALL
                .into_iter()
                .map(|name| Theme::from(name).slot(slot))
                .collect();
            assert_ne!(colors[0], colors[1], "{slot:?} nocturne vs ember");
            assert_ne!(colors[1], colors[2], "{slot:?} ember vs phosphor");
            assert_ne!(colors[0], colors[2], "{slot:?} nocturne vs phosphor");
        }
    }

    #[test]
    fn theme_names_round_trip_and_default_to_the_flagship() {
        for name in ThemeName::ALL {
            assert_eq!(ThemeName::named(name.as_str()), name);
        }
        assert_eq!(ThemeName::named("NOCTURNE"), ThemeName::Nocturne);
        assert_eq!(ThemeName::named("Ember"), ThemeName::Ember);
        assert_eq!(ThemeName::named(""), ThemeName::Nocturne);
        assert_eq!(ThemeName::named("teal"), ThemeName::Nocturne);
        assert_eq!(ThemeName::default(), ThemeName::Nocturne);
    }

    #[test]
    fn color_tier_is_probed_never_assumed() {
        // NO_COLOR wins over any positive signal, at any value including empty.
        assert_eq!(
            ColorTier::from_env(env(&[
                ("NO_COLOR", ""),
                ("COLORTERM", "truecolor"),
                ("TERM", "xterm-256color"),
            ])),
            ColorTier::Monochrome
        );
        assert_eq!(
            ColorTier::from_env(env(&[("NO_COLOR", "1"), ("TERM", "xterm-256color")])),
            ColorTier::Monochrome
        );
        // A dumb or absent terminal is monochrome, not a guess at 16 colours.
        assert_eq!(
            ColorTier::from_env(env(&[("TERM", "dumb")])),
            ColorTier::Monochrome
        );
        assert_eq!(ColorTier::from_env(env(&[])), ColorTier::Monochrome);
        // Positive signals, most specific first.
        assert_eq!(
            ColorTier::from_env(env(&[
                ("TERM", "xterm-256color"),
                ("COLORTERM", "truecolor")
            ])),
            ColorTier::TrueColor
        );
        assert_eq!(
            ColorTier::from_env(env(&[("TERM", "xterm-direct")])),
            ColorTier::TrueColor
        );
        assert_eq!(
            ColorTier::from_env(env(&[("TERM", "screen-256color")])),
            ColorTier::Ansi256
        );
        assert_eq!(
            ColorTier::from_env(env(&[("TERM", "xterm")])),
            ColorTier::Ansi16
        );
        // An explicit request outranks every heuristic, including NO_COLOR.
        assert_eq!(
            ColorTier::from_env(env(&[("ORC_COLOR", "16"), ("NO_COLOR", "1")])),
            ColorTier::Ansi16
        );
        assert_eq!(
            ColorTier::from_env(env(&[("ORC_COLOR", "none"), ("COLORTERM", "truecolor")])),
            ColorTier::Monochrome
        );
        // An unparseable override falls through to the probe rather than
        // silently picking a tier the user did not ask for.
        assert_eq!(
            ColorTier::from_env(env(&[("ORC_COLOR", "beautiful"), ("TERM", "xterm")])),
            ColorTier::Ansi16
        );
    }

    #[test]
    fn state_styles_stay_distinguishable_without_colour() {
        let mono = Theme::new(ThemeName::Nocturne, ColorTier::Monochrome);
        for slot in [Slot::Confirmed, Slot::Failed, Slot::Avail, Slot::Brain] {
            assert!(
                mono.state(slot).add_modifier.contains(Modifier::BOLD),
                "{slot:?} must stay bold without colour"
            );
        }
        for slot in [Slot::Pending, Slot::Unavail, Slot::Muted, Slot::Faint] {
            assert!(
                mono.state(slot).add_modifier.contains(Modifier::DIM),
                "{slot:?} must stay dim without colour"
            );
        }
        assert!(
            mono.selection().add_modifier.contains(Modifier::REVERSED),
            "monochrome selection must be reverse video"
        );
        assert_eq!(
            mono.selection().bg,
            None,
            "monochrome selection has no fill"
        );
        // With colour the selection is a fill, not reverse video.
        let color = Theme::from(ThemeName::Nocturne);
        assert_eq!(color.selection().bg, Some(color.sel()));
    }

    /// The grep gate: widget code names slots, never colours.
    ///
    /// Every source file in the crate except this one is scanned for a colour
    /// literal — a `#rrggbb` string, an `Rgb`/`Indexed` constructor with
    /// numeric arguments, or any named `Color::` variant. `Color::Rgb(red,
    /// green, blue)` built from *identifiers* is allowed, because that is a
    /// hosted pane replaying its own SGR rather than the app theming itself,
    /// and that path lives here in `Theme::pane_color` anyway.
    #[test]
    fn no_hex_literals_outside_the_theme_map() {
        const NAMED: [&str; 18] = [
            "Color::Black",
            "Color::Red",
            "Color::Green",
            "Color::Yellow",
            "Color::Blue",
            "Color::Magenta",
            "Color::Cyan",
            "Color::Gray",
            "Color::DarkGray",
            "Color::LightRed",
            "Color::LightGreen",
            "Color::LightYellow",
            "Color::LightBlue",
            "Color::LightMagenta",
            "Color::LightCyan",
            "Color::White",
            "Color::Reset",
            "Color::from_u32",
        ];
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut scanned = 0_usize;
        let mut offences = Vec::new();
        for entry in std::fs::read_dir(&src).expect("read src") {
            let path = entry.expect("src entry").path();
            if path.extension().is_none_or(|ext| ext != "rs")
                || path.file_name().is_some_and(|name| name == "theme.rs")
            {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let body = std::fs::read_to_string(&path).expect("read source");
            scanned += 1;
            for (number, line) in body.lines().enumerate() {
                let hit = |needle: &str| {
                    line.split(needle).skip(1).any(|rest| {
                        rest.trim_start()
                            .starts_with(|character: char| character.is_ascii_digit())
                    })
                };
                let hex = line.split('#').skip(1).any(|rest| {
                    let digits: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
                    digits.len() == 6
                });
                if hex
                    || hit("Color::Rgb(")
                    || hit("Color::Indexed(")
                    || NAMED.iter().any(|named| line.contains(named))
                {
                    offences.push(format!("{name}:{}: {}", number + 1, line.trim()));
                }
            }
        }
        assert!(
            scanned >= 4,
            "the gate scanned only {scanned} files — it is not looking where the widgets are"
        );
        assert!(
            offences.is_empty(),
            "colour literals outside the theme map ({scanned} files scanned):\n{}",
            offences.join("\n")
        );
    }

    #[test]
    fn the_runs_embed_borrows_this_map_rather_than_its_own_two_themes() {
        for name in ThemeName::ALL {
            let theme = Theme::from(name);
            let runs = theme.runs_theme();
            assert_eq!(runs.name, name.as_str());
            assert_eq!(runs.bg, theme.bg());
            assert_eq!(runs.accent, theme.brain());
            assert_ne!(runs, orc_tui::EMBER, "{name:?} must not fall back");
            assert_ne!(runs, orc_tui::PHOSPHOR, "{name:?} must not fall back");
        }
    }
}
