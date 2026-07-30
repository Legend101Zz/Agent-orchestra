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
//! matching glyph in [`crate::glyph`], and on the monochrome tier every colour
//! **this map** answers with — the slots and the trigger gradient alike —
//! resolves to [`Color::Reset`], so the UI must still read through layout,
//! glyph, bold, and reverse.
//!
//! One colour the crate emits does not come from this map: a hosted pane's own
//! SGR, which [`Theme::pane_color`] replays from the wire at every tier. It is
//! not theming and it is not tier-gated — see its doc. So "monochrome emits no
//! colour" is a claim about what pi-orchestra paints, not about what a harness
//! running inside a pane prints.

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

    /// The next theme in the switcher's cycle: nocturne, ember, phosphor,
    /// nocturne. The order is [`Self::ALL`], so the flagship is where a cycle
    /// starts and returns to.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Nocturne => Self::Ember,
            Self::Ember => Self::Phosphor,
            Self::Phosphor => Self::Nocturne,
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

/// The number of colour stops in the trigger gradient.
pub const TRIGGER_STOPS: usize = 7;

/// The per-character gradient a highlighted conductor trigger cycles through,
/// column by column, so a spell shimmers like Claude Code's `ultrathink`.
///
/// Naming these stops explicitly rather than as slots is an owner-approved
/// exception to the slot rule (LOG.md, 2026-07-24): a seven-colour sweep has no
/// single-slot equivalent. It is *not* an exception to the tier rule, so the
/// stops carry the same three columns as every row of the map above — 24-bit,
/// the nearest xterm-256 index, and the base ANSI colour each collapses to —
/// and [`ColorTier::Monochrome`] drops them entirely like everything else here.
///
/// Meaning never rides on the gradient: the token stays **bold** and keeps its
/// `◆ LABEL` title badge, and on a monochrome terminal those two are the whole
/// of the effect. The colour is decoration on top. It lives here because this
/// module owns every literal colour in the crate.
const TRIGGER_GRADIENT: [SlotColors; TRIGGER_STOPS] = [
    slot(0xff_6b6b, 203, Color::LightRed),     // red
    slot(0xff_a94d, 215, Color::Yellow),       // orange
    slot(0xff_e066, 221, Color::LightYellow),  // yellow
    slot(0x63_e6be, 79, Color::LightGreen),    // green
    slot(0x4d_abf7, 75, Color::LightBlue),     // blue
    slot(0xb1_97fc, 141, Color::LightMagenta), // indigo
    slot(0xf7_83ac, 211, Color::Magenta),      // violet
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

    /// Pick one row of the map's three columns for this tier.
    ///
    /// Every colour **the map** answers with comes through here — slots and the
    /// trigger gradient both — which is what makes "the monochrome tier answers
    /// `Reset`" a property of this module rather than a promise each caller has
    /// to keep. [`Theme::pane_color`] is the one method that does not resolve
    /// through here, because it replays a pane's own SGR rather than theming.
    fn resolve(self, colors: &SlotColors) -> Color {
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

    /// Resolve one semantic slot for this theme and tier.
    ///
    /// Under [`ColorTier::Monochrome`] every slot answers [`Color::Reset`]:
    /// the terminal's own foreground and background. That is the honest
    /// answer — with no colour to spend, state has to read from glyph,
    /// bold, dim, and reverse instead.
    #[must_use]
    pub fn slot(self, slot: Slot) -> Color {
        self.resolve(&self.name.palette().slots[slot as usize])
    }

    /// The trigger gradient this terminal receives, or `None` when it has none
    /// to spend.
    ///
    /// Tier by tier, so the answer is on the record rather than implied:
    ///
    /// | tier | a trigger token receives |
    /// |---|---|
    /// | [`ColorTier::TrueColor`] | the seven 24-bit stops, the design's native form |
    /// | [`ColorTier::Ansi256`] | the seven nearest xterm-cube indices |
    /// | [`ColorTier::Ansi16`] | seven distinct base ANSI colours |
    /// | [`ColorTier::Monochrome`] | `None` — no colour at all |
    ///
    /// `None` is what `NO_COLOR` asks for and it costs the affordance nothing:
    /// the caller keeps the token **bold** at every tier and the `◆ LABEL`
    /// title badge names the spell in words.
    #[must_use]
    pub fn trigger_gradient(self) -> Option<[Color; TRIGGER_STOPS]> {
        if self.tier == ColorTier::Monochrome {
            return None;
        }
        Some(TRIGGER_GRADIENT.map(|stop| self.resolve(&stop)))
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
    ///
    /// **It is deliberately not tier-gated**, and that is the one place the
    /// module's "monochrome emits no colour" stops being true of the screen: a
    /// harness printing SGR inside its pane puts that colour on a `NO_COLOR`
    /// terminal, because quantising or discarding it would be editing another
    /// program's output. In practice the hosted process inherits `NO_COLOR`
    /// itself and prints none. Only the [`Slot`] fallback — the pane's *default*
    /// foreground and background — is resolved through the tier.
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

    #[test]
    fn the_trigger_gradient_degrades_with_the_tier_and_vanishes_without_colour() {
        // The #13 carry-over, at the source. The rainbow used to be a bare
        // `[Color; 7]` of RGB stops that every tier received verbatim, so a
        // monochrome terminal was sent nine truecolor cells while this module
        // claimed it "drops colour entirely" and a 16-colour terminal was sent
        // SGR it cannot render. It is a row of the map like any other now.
        //
        // The theme is irrelevant — the gradient is the trigger's, not the
        // palette's — so every theme must answer identically, and that is
        // asserted rather than assumed.
        for name in ThemeName::ALL {
            assert_eq!(
                Theme::new(name, ColorTier::TrueColor).trigger_gradient(),
                Some([
                    Color::Rgb(0xff, 0x6b, 0x6b),
                    Color::Rgb(0xff, 0xa9, 0x4d),
                    Color::Rgb(0xff, 0xe0, 0x66),
                    Color::Rgb(0x63, 0xe6, 0xbe),
                    Color::Rgb(0x4d, 0xab, 0xf7),
                    Color::Rgb(0xb1, 0x97, 0xfc),
                    Color::Rgb(0xf7, 0x83, 0xac),
                ]),
                "{name:?}: truecolor is the design's native form, unchanged"
            );
            assert_eq!(
                Theme::new(name, ColorTier::Ansi256).trigger_gradient(),
                Some([
                    Color::Indexed(203),
                    Color::Indexed(215),
                    Color::Indexed(221),
                    Color::Indexed(79),
                    Color::Indexed(75),
                    Color::Indexed(141),
                    Color::Indexed(211),
                ]),
                "{name:?}: ansi256 receives the nearest cube index per stop"
            );
            assert_eq!(
                Theme::new(name, ColorTier::Ansi16).trigger_gradient(),
                Some([
                    Color::LightRed,
                    Color::Yellow,
                    Color::LightYellow,
                    Color::LightGreen,
                    Color::LightBlue,
                    Color::LightMagenta,
                    Color::Magenta,
                ]),
                "{name:?}: ansi16 receives seven base ANSI colours, not truecolor SGR"
            );
            assert_eq!(
                Theme::new(name, ColorTier::Monochrome).trigger_gradient(),
                None,
                "{name:?}: NO_COLOR means no colour — the token is carried by bold \
                 and the badge, which is all this tier has"
            );
        }
        // Seven stops that collapse onto six colours would be a gradient with a
        // seam in it, so the 16-colour row has to stay a permutation.
        let sixteen = Theme::new(ThemeName::Nocturne, ColorTier::Ansi16)
            .trigger_gradient()
            .expect("ansi16 has a gradient");
        for (index, colour) in sixteen.iter().enumerate() {
            assert!(
                !matches!(colour, Color::Rgb(..) | Color::Indexed(_) | Color::Reset),
                "stop {index} must collapse to a base ANSI colour"
            );
            assert!(
                !sixteen[index + 1..].contains(colour),
                "stop {index} is repeated later in the gradient"
            );
        }
    }

    #[test]
    fn the_trigger_gradients_ansi256_column_really_is_the_nearest_index() {
        // The design sheet asks for the fallback column to be "a nearest-color
        // pass", so the transcribed indices are checked against one rather than
        // trusted. Searched over 16..=255 — the cube and the greyscale ramp —
        // because 0..=15 are whatever the terminal's own scheme says they are
        // and so cannot be measured from here.
        let level = |index: u32| [0, 95, 135, 175, 215, 255][index as usize];
        let palette = |index: u32| -> (i32, i32, i32) {
            if index < 232 {
                let cube = index - 16;
                (level(cube / 36), level((cube / 6) % 6), level(cube % 6))
            } else {
                let grey = 8 + 10 * (index as i32 - 232);
                (grey, grey, grey)
            }
        };
        let truecolor = Theme::new(ThemeName::Nocturne, ColorTier::TrueColor)
            .trigger_gradient()
            .expect("truecolor has a gradient");
        let indexed = Theme::new(ThemeName::Nocturne, ColorTier::Ansi256)
            .trigger_gradient()
            .expect("ansi256 has a gradient");
        for (stop, (wanted, got)) in truecolor.iter().zip(indexed.iter()).enumerate() {
            let Color::Rgb(red, green, blue) = *wanted else {
                panic!("stop {stop} is not truecolor");
            };
            let distance = |index: u32| {
                let (r, g, b) = palette(index);
                (r - i32::from(red)).pow(2)
                    + (g - i32::from(green)).pow(2)
                    + (b - i32::from(blue)).pow(2)
            };
            let nearest = (16..=255)
                .min_by_key(|index| distance(*index))
                .unwrap_or(16);
            assert_eq!(
                *got,
                Color::Indexed(u8::try_from(nearest).unwrap_or(u8::MAX)),
                "stop {stop} is not the nearest xterm-256 index"
            );
        }
    }

    /// Every `.rs` file under `root`, at any depth, relative to it and sorted.
    ///
    /// The gate below used to use a flat `read_dir`, which made a widget module
    /// one directory down invisible to it: a hex literal in `src/widgets/mod.rs`
    /// passed. AGENTS.md's own "prefer new modules over growing a file past ~600
    /// lines" makes `src/<subdir>/` the likely next move, so the walk descends.
    fn rust_sources(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut pending = vec![root.to_path_buf()];
        let mut found = Vec::new();
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).expect("read source directory") {
                let path = entry.expect("source entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    found.push(path.strip_prefix(root).expect("under src").to_path_buf());
                }
            }
        }
        found.sort();
        found
    }

    /// A declaration with any visibility qualifier removed.
    ///
    /// `pub`, `pub(crate)`, `pub(super)`, `pub(in crate::widgets)` — all of them,
    /// because a floor that only understood `pub` and `pub(crate)` would quietly
    /// stop tracking a module declared any other way.
    fn strip_visibility(line: &str) -> &str {
        let Some(rest) = line.strip_prefix("pub") else {
            return line;
        };
        match rest.chars().next() {
            // `pub(...)` — skip the whole restriction, however it is spelled.
            Some('(') => rest
                .split_once(')')
                .map_or(line, |(_, after)| after.trim_start()),
            // `pub mod x;`. Anything else starting with "pub" is an identifier
            // such as `pubs`, and must not be treated as a qualifier.
            Some(character) if character.is_whitespace() => rest.trim_start(),
            _ => line,
        }
    }

    /// Every source file the crate's module graph reaches, from its two roots.
    ///
    /// `mod name;` in `src/lib.rs` resolves to `src/name.rs` or
    /// `src/name/mod.rs`, and inside `src/dir/mod.rs` it resolves under `dir`.
    /// Any visibility qualifier is accepted (see [`strip_visibility`]) and
    /// `#[path = "..."]` is honoured, so the floor does not lose a module just
    /// because of how it was spelled.
    /// This reads the *source text*, so it is an independent statement of what
    /// has to be scanned: a walk that stopped descending fails the coverage
    /// assertion instead of quietly meeting a constant floor. A colour literal
    /// can only reach a build through a declared module, so this is also the
    /// exact set that matters.
    fn declared_sources(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut pending: Vec<std::path::PathBuf> = ["lib.rs", "main.rs"]
            .into_iter()
            .map(std::path::PathBuf::from)
            .filter(|relative| root.join(relative).is_file())
            .collect();
        let mut found: Vec<std::path::PathBuf> = Vec::new();
        while let Some(relative) = pending.pop() {
            if found.contains(&relative) {
                continue;
            }
            let body = std::fs::read_to_string(root.join(&relative)).expect("read module");
            // Where this file's children live: beside a crate or directory root,
            // in a directory named after any other module.
            let parent = if matches!(
                relative.file_name().and_then(|name| name.to_str()),
                Some("lib.rs" | "main.rs" | "mod.rs")
            ) {
                relative
                    .parent()
                    .unwrap_or(std::path::Path::new(""))
                    .to_path_buf()
            } else {
                relative.with_extension("")
            };
            found.push(relative);
            // `#[path = "..."]` overrides where the *next* declaration's file
            // lives, so it has to be carried across one line.
            let mut explicit: Option<String> = None;
            for line in body.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("#[path") {
                    explicit = rest.split('"').nth(1).map(str::to_owned);
                    continue;
                }
                let declaration = strip_visibility(line);
                // `mod tests { .. }` is inline and has no file of its own, so
                // only a declaration terminated by `;` names one.
                let named = declaration
                    .strip_prefix("mod ")
                    .and_then(|rest| rest.strip_suffix(';'))
                    .map(str::trim);
                let Some(name) = named else {
                    // The attribute applied to whatever this is, not to a later
                    // `mod`; blank lines and comments do not break the pairing.
                    if !line.is_empty() && !line.starts_with("//") {
                        explicit = None;
                    }
                    continue;
                };
                let candidates = match explicit.take() {
                    Some(path) => vec![parent.join(path)],
                    None => vec![
                        parent.join(format!("{name}.rs")),
                        parent.join(name).join("mod.rs"),
                    ],
                };
                if let Some(hit) = candidates
                    .into_iter()
                    .find(|candidate| root.join(candidate).is_file())
                {
                    pending.push(hit);
                }
            }
        }
        found.sort();
        found
    }

    /// The one file exempt from the colour scan: the theme map itself.
    ///
    /// Matched on the **whole** relative path, not on the file name. Keying it
    /// on the name was harmless while the walk was flat and unsound once it
    /// recursed: any `src/<anydir>/theme.rs` would have been exempt, and
    /// splitting this file (1100+ lines against AGENTS.md's ~600 guidance) into
    /// `src/theme/mod.rs` + `src/theme/palette.rs` would have exempted the map
    /// and nothing else — the opposite of what the gate is for.
    const THEME_MAP: &str = "theme.rs";

    /// Scan a source tree for colour literals: a `#rrggbb` string, an
    /// `Rgb`/`Indexed` constructor with numeric arguments, or any named
    /// `Color::` variant. `Color::Rgb(red, green, blue)` built from
    /// *identifiers* is allowed, because that is a hosted pane replaying its own
    /// SGR rather than the app theming itself, and that path lives in
    /// `Theme::pane_color` anyway.
    ///
    /// Takes the root as an argument so the rules can be exercised against a
    /// synthetic tree — which is the only way to assert what the gate does with
    /// a file it must *not* exempt without planting one in the real crate.
    /// Returns the files scanned and the offences, `<relative path>:<line>`.
    fn colour_offences(root: &std::path::Path) -> (Vec<std::path::PathBuf>, Vec<String>) {
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
        let mut scanned = Vec::new();
        let mut offences = Vec::new();
        for relative in rust_sources(root) {
            if relative == std::path::Path::new(THEME_MAP) {
                continue;
            }
            // The subdirectory, if any, is part of the name: `mod.rs` alone
            // would not say which module failed.
            let name = relative.display().to_string();
            let body = std::fs::read_to_string(root.join(&relative)).expect("read source");
            scanned.push(relative);
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
        (scanned, offences)
    }

    /// Build a throwaway source tree under `root`, so the gate's own rules can
    /// be asserted on files that must not exist in the real crate.
    fn write_tree(root: &std::path::Path, files: &[(&str, &str)]) {
        let _ = std::fs::remove_dir_all(root);
        for (relative, body) in files {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create module directory");
            }
            std::fs::write(&path, body).expect("write module");
        }
    }

    #[test]
    fn the_exemption_is_the_theme_map_itself_and_nothing_else_named_like_it() {
        // The review's second finding. The exemption used to match on the file
        // *name*, which was harmless while the walk was flat: there was only one
        // `theme.rs` to find. Once the walk recursed, every `src/<anydir>/
        // theme.rs` became exempt — so the very split this file is heading for
        // (`src/theme/mod.rs` + `src/theme/palette.rs`, at 1100+ lines against
        // AGENTS.md's ~600 guidance) would have exempted the map and nothing
        // else. Asserted on a synthetic tree because proving it needs a file the
        // real crate must not contain.
        let root = std::env::temp_dir().join(format!("orc-app-gate-{}", std::process::id()));
        write_tree(
            &root,
            &[
                (
                    "theme.rs",
                    "Color::Rgb(1, 2, 3) // the map may name colours\n",
                ),
                ("widgets/theme.rs", "let sneaky = Color::Indexed(199);\n"),
                ("widgets/deep/panel.rs", "let hex = \"#ff00aa\";\n"),
                ("honest.rs", "theme.slot(Slot::Fg)\n"),
            ],
        );
        let (scanned, offences) = colour_offences(&root);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(
            offences.len(),
            2,
            "expected the two literals below the root to be reported: {offences:?}"
        );
        assert!(
            offences
                .iter()
                .any(|hit| hit.starts_with("widgets/theme.rs:")),
            "a `theme.rs` that is not THE theme map must be scanned like any \
             other file: {offences:?}"
        );
        assert!(
            offences
                .iter()
                .any(|hit| hit.starts_with("widgets/deep/panel.rs:")),
            "the scan must reach any depth: {offences:?}"
        );
        assert_eq!(scanned.len(), 3, "only the root theme.rs may be skipped");
        assert!(
            !scanned.contains(&std::path::PathBuf::from(THEME_MAP)),
            "the theme map itself is the one file exempt from its own rule"
        );
    }

    #[test]
    fn the_module_graph_probe_reads_every_way_a_module_can_be_declared() {
        // The review's fourth finding: the probe stripped only `pub ` and
        // `pub(crate) `, so `pub(super)`, `pub(in ...)` and `#[path]` modules
        // were invisible to the floor — it would have under-counted the tree it
        // exists to track. Inline `mod tests { .. }` must stay invisible, since
        // it has no file, and `pubs` is an identifier, not a qualifier.
        let root = std::env::temp_dir().join(format!("orc-app-graph-{}", std::process::id()));
        write_tree(
            &root,
            &[
                (
                    "lib.rs",
                    "pub mod plain;\n\
                     pub(crate) mod crated;\n\
                     pub(super) mod supered;\n\
                     pub(in crate::plain) mod scoped;\n\
                     mod bare;\n\
                     #[path = \"elsewhere/renamed.rs\"]\n\
                     mod aliased;\n\
                     #[cfg(test)]\n\
                     mod gated;\n\
                     let pubs = 1;\n\
                     #[cfg(test)]\n\
                     mod tests {\n\
                     }\n",
                ),
                ("plain.rs", "pub mod nested;\n"),
                ("plain/nested.rs", ""),
                ("crated.rs", ""),
                ("supered.rs", ""),
                ("scoped.rs", ""),
                ("bare/mod.rs", ""),
                ("elsewhere/renamed.rs", ""),
                ("gated.rs", ""),
                ("orphan.rs", "// on disk, declared by nobody\n"),
            ],
        );
        let declared = declared_sources(&root);
        let _ = std::fs::remove_dir_all(&root);

        let names: Vec<String> = declared
            .iter()
            .map(|path| path.display().to_string().replace('\\', "/"))
            .collect();
        for wanted in [
            "lib.rs",
            "plain.rs",
            "plain/nested.rs",
            "crated.rs",
            "supered.rs",
            "scoped.rs",
            "bare/mod.rs",
            "elsewhere/renamed.rs",
            "gated.rs",
        ] {
            assert!(
                names.iter().any(|name| name == wanted),
                "{wanted} is declared but the probe missed it: {names:?}"
            );
        }
        assert!(
            !names.iter().any(|name| name == "orphan.rs"),
            "the probe reports the module graph, not the directory listing: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.contains("tests")),
            "an inline `mod tests {{ .. }}` has no file to find: {names:?}"
        );
    }

    /// The grep gate: widget code names slots, never colours.
    ///
    /// Every source file in the crate except [`THEME_MAP`] is scanned — at any
    /// depth. Two assertions guard the walk itself, and they catch different
    /// failures: the coverage loop catches a walk that stopped descending, and
    /// the exemption count catches an exemption that matched more than the one
    /// file it is allowed to.
    #[test]
    fn no_hex_literals_outside_the_theme_map() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let walked = rust_sources(&src);
        let (scanned, offences) = colour_offences(&src);
        // The floor tracks the tree: every file the crate declares as a module
        // must be one the walk reached, named individually so a failure says
        // which module went unscanned rather than only that a count is short.
        let declared = declared_sources(&src);
        assert!(
            declared.len() >= 2,
            "the module-graph probe found {} files — it stopped parsing, so the \
             floor below means nothing",
            declared.len()
        );
        for relative in &declared {
            assert!(
                walked.contains(relative),
                "the gate never looked at {}, which the crate declares as a module",
                relative.display()
            );
        }
        // Order matters: if the map is split into `src/theme/` the exemption
        // matches nothing, and the count below would then fire with a message
        // blaming the opposite problem. This one names the real cause.
        assert!(
            walked
                .iter()
                .any(|path| path == std::path::Path::new(THEME_MAP)),
            "there is no {THEME_MAP} to exempt — if the map moved, point \
             THEME_MAP at it rather than letting the gate scan its own table"
        );
        // Exactly one file may be skipped. A count against `declared` would add
        // nothing here — the loop above already proves `declared` is a subset of
        // `walked` — whereas this fires the moment the exemption swallows a
        // second file, which is how keying it on the file name went wrong.
        assert_eq!(
            scanned.len() + 1,
            walked.len(),
            "the gate walked to {} files and scanned {}: the {THEME_MAP} exemption \
             is swallowing more than the theme map",
            walked.len(),
            scanned.len()
        );
        assert!(
            offences.is_empty(),
            "colour literals outside the theme map ({} files scanned):\n{}",
            scanned.len(),
            offences.join("\n")
        );
    }

    #[test]
    fn the_cycle_visits_every_theme_and_returns_to_the_flagship() {
        let mut seen = Vec::new();
        let mut name = ThemeName::default();
        for _ in 0..ThemeName::ALL.len() {
            seen.push(name);
            name = name.next();
        }
        assert_eq!(
            seen,
            vec![ThemeName::Nocturne, ThemeName::Ember, ThemeName::Phosphor],
            "the switcher must reach all three, flagship first"
        );
        assert_eq!(name, ThemeName::Nocturne, "the cycle must close");
    }

    #[test]
    fn the_embedded_and_standalone_ledgers_resolve_every_name_the_same_way() {
        // `orc_tui::Theme::named` used to know only ember and phosphor, so
        // "nocturne" silently answered EMBER. The standalone ledger must
        // resolve every approved name, and its flagship must be the same
        // palette the embed renders rather than a second transcription that
        // can drift.
        for name in ThemeName::ALL {
            assert_eq!(
                orc_tui::Theme::named(name.as_str()).name,
                name.as_str(),
                "{name:?} must resolve to itself"
            );
        }
        assert_eq!(
            orc_tui::Theme::named("nocturne"),
            Theme::from(ThemeName::Nocturne).runs_theme(),
            "the two ledgers disagree about the flagship palette"
        );
        assert_eq!(
            orc_tui::Theme::named("chartreuse").name,
            ThemeName::Nocturne.as_str(),
            "an unknown name resolves to the flagship, not to ember"
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
