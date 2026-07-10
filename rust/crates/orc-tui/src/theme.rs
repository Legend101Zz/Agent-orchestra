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
    #[must_use]
    pub fn named(name: &str) -> Self {
        if name == "phosphor" { PHOSPHOR } else { EMBER }
    }

    #[must_use]
    pub const fn other(self) -> Self {
        if self.name.as_bytes()[0] == b'e' {
            PHOSPHOR
        } else {
            EMBER
        }
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
