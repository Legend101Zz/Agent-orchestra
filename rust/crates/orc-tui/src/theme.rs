use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub panel: Color,
    pub surface: Color,
    pub border: Color,
    pub border_focus: Color,
    pub text: Color,
    pub text_dim: Color,
    pub label: Color,
    pub accent: Color,
    pub accent_2: Color,
    pub ok: Color,
    pub warn: Color,
    pub error: Color,
    pub running: Color,
    pub killed: Color,
    pub controller_codex: Color,
    pub controller_claude: Color,
    pub controller_human: Color,
}

const fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::Rgb(red, green, blue)
}

/// The flagship, transcribed from `docs/design/visual-identity.md` through the
/// same slot correspondence the embedded ledger uses
/// (`orc_app::theme::Theme::runs_theme`), so a standalone `pio runs` in
/// nocturne renders the identity palette rather than an approximation of it.
pub const NOCTURNE: Theme = Theme {
    name: "nocturne",
    bg: rgb(0x0a, 0x0c, 0x11),
    panel: rgb(0x10, 0x13, 0x1b),
    surface: rgb(0x17, 0x1b, 0x26),
    border: rgb(0x26, 0x2c, 0x3a),
    border_focus: rgb(0x39, 0x42, 0x5a),
    text: rgb(0xc4, 0xca, 0xd6),
    text_dim: rgb(0x45, 0x4c, 0x60),
    label: rgb(0x72, 0x7b, 0x8f),
    accent: rgb(0x5a, 0xd1, 0xc8),
    accent_2: rgb(0x5a, 0xd1, 0xc8),
    ok: rgb(0xe6, 0xb4, 0x50),
    warn: rgb(0x8a, 0x93, 0xa6),
    error: rgb(0xe0, 0x7a, 0x80),
    running: rgb(0x8e, 0xa2, 0xff),
    killed: rgb(0x56, 0x5e, 0x70),
    controller_codex: rgb(0x5a, 0xd1, 0xc8),
    controller_claude: rgb(0x8e, 0xa2, 0xff),
    controller_human: rgb(0x72, 0x7b, 0x8f),
};

pub const EMBER: Theme = Theme {
    name: "ember",
    bg: rgb(22, 18, 14),
    panel: rgb(29, 24, 20),
    surface: rgb(36, 30, 24),
    border: rgb(58, 47, 36),
    border_focus: rgb(255, 158, 61),
    text: rgb(232, 221, 207),
    text_dim: rgb(138, 122, 102),
    label: rgb(160, 139, 111),
    accent: rgb(255, 158, 61),
    accent_2: rgb(255, 210, 122),
    ok: rgb(158, 203, 90),
    warn: rgb(255, 184, 77),
    error: rgb(255, 92, 71),
    running: rgb(255, 210, 122),
    killed: rgb(196, 122, 184),
    controller_codex: rgb(122, 208, 255),
    controller_claude: rgb(232, 161, 255),
    controller_human: rgb(201, 201, 201),
};

pub const PHOSPHOR: Theme = Theme {
    name: "phosphor",
    bg: rgb(5, 10, 6),
    panel: rgb(8, 17, 8),
    surface: rgb(11, 22, 12),
    border: rgb(29, 58, 32),
    border_focus: rgb(77, 255, 122),
    text: rgb(184, 240, 192),
    text_dim: rgb(74, 122, 82),
    label: rgb(95, 154, 104),
    accent: rgb(77, 255, 122),
    accent_2: rgb(200, 255, 212),
    ok: rgb(77, 255, 122),
    warn: rgb(232, 255, 90),
    error: rgb(255, 107, 77),
    running: rgb(200, 255, 212),
    killed: rgb(159, 184, 168),
    controller_codex: rgb(107, 232, 255),
    controller_claude: rgb(157, 255, 184),
    controller_human: rgb(136, 170, 144),
};

impl Theme {
    /// Every approved theme in cycle order, flagship first.
    pub const ALL: [Self; 3] = [NOCTURNE, EMBER, PHOSPHOR];

    /// Resolve a configured theme name, falling back to the flagship.
    ///
    /// All three approved names resolve here, so the standalone ledger and the
    /// embedded one cannot disagree about which theme a name refers to.
    #[must_use]
    pub fn named(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|theme| name.eq_ignore_ascii_case(theme.name))
            .unwrap_or(NOCTURNE)
    }

    /// The next theme in the cycle: nocturne, ember, phosphor, nocturne.
    #[must_use]
    pub fn other(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|theme| theme.name == self.name)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    #[must_use]
    pub fn status(self, status: &str) -> Color {
        match status {
            "running" => self.running,
            "starting" => self.warn,
            "done" => self.ok,
            "failed" => self.error,
            "killed" => self.killed,
            _ => self.text_dim,
        }
    }

    #[must_use]
    pub fn controller(self, brain: &str) -> Color {
        match brain {
            "codex" => self.controller_codex,
            "claude" => self.controller_claude,
            _ => self.controller_human,
        }
    }
}
