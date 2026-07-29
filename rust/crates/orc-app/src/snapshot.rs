//! Committed `TestBackend` snapshots for every screen in every theme.
//!
//! A snapshot records two grids — the symbols the screen painted, and a style
//! key per cell — plus the legend those keys resolve to. Recording the style
//! is the point: a theme change that swapped nothing but colour would leave a
//! text-only snapshot identical, and the whole of this issue is colour.
//!
//! Golden files live in `tests/snapshots/`. `cargo test` compares against them
//! and fails on any drift; `ORC_UPDATE_SNAPSHOTS=1 cargo test -p orc-app`
//! rewrites them, and the diff is the review.

use std::fmt::Write as _;
use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

use crate::glyph::{GlyphTier, Glyphs};
use crate::theme::{ColorTier, Theme, ThemeName, describe};

/// The snapshot viewport. Comfortably above the design's 80×24 minimum and
/// wide enough for STAGE's two-column layout and its baton.
const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;

/// The printable style keys, in first-seen order.
const KEYS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+/=<>?@#$%&*";

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

fn describe_modifiers(modifier: Modifier) -> String {
    let mut names = Vec::new();
    for (flag, name) in [
        (Modifier::BOLD, "bold"),
        (Modifier::DIM, "dim"),
        (Modifier::ITALIC, "italic"),
        (Modifier::UNDERLINED, "underline"),
        (Modifier::REVERSED, "reverse"),
    ] {
        if modifier.contains(flag) {
            names.push(name);
        }
    }
    if names.is_empty() {
        "-".to_owned()
    } else {
        names.join("+")
    }
}

/// Serialise a rendered buffer into the committed snapshot format.
fn serialize(header: &str, buffer: &Buffer) -> String {
    let width = usize::from(buffer.area.width);
    let mut legend: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut styles = String::new();
    for row in buffer.content().chunks(width) {
        for cell in row {
            text.push_str(if cell.symbol().is_empty() {
                " "
            } else {
                cell.symbol()
            });
            let entry = format!(
                "fg={} bg={} mod={}",
                describe(cell.fg),
                describe(cell.bg),
                describe_modifiers(cell.modifier)
            );
            let index = legend
                .iter()
                .position(|known| *known == entry)
                .unwrap_or_else(|| {
                    legend.push(entry);
                    legend.len() - 1
                });
            // More distinct styles than keys would mean the palette had gone
            // wild; say so rather than aliasing two styles onto one key.
            assert!(
                index < KEYS.len(),
                "{header}: {} distinct styles exceeds the {} snapshot keys",
                legend.len(),
                KEYS.len()
            );
            styles.push(char::from(KEYS[index]));
        }
        text.push('\n');
        styles.push('\n');
    }
    let mut out = String::new();
    let _ = writeln!(out, "{header}");
    let _ = writeln!(out, "--- text ---");
    out.push_str(&text);
    let _ = writeln!(out, "--- styles ---");
    out.push_str(&styles);
    let _ = writeln!(out, "--- legend ---");
    for (index, entry) in legend.iter().enumerate() {
        let _ = writeln!(out, "{} {entry}", char::from(KEYS[index]));
    }
    out
}

/// Render one screen and compare it to (or rewrite) its golden file.
fn gate(name: &str, header: &str, draw: impl FnOnce(&mut ratatui::Frame<'_>)) {
    let backend = TestBackend::new(WIDTH, HEIGHT);
    let mut terminal = Terminal::new(backend).expect("snapshot terminal");
    terminal.draw(draw).expect("render snapshot");
    let actual = serialize(header, terminal.backend().buffer());
    let path = snapshot_dir().join(format!("{name}.txt"));
    if std::env::var_os("ORC_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(snapshot_dir()).expect("create snapshot dir");
        std::fs::write(&path, &actual).expect("write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing snapshot {}: {error}\nrun ORC_UPDATE_SNAPSHOTS=1 cargo test -p orc-app",
            path.display()
        )
    });
    if expected == actual {
        return;
    }
    let first_diff = expected
        .lines()
        .zip(actual.lines())
        .enumerate()
        .find(|(_, (left, right))| left != right)
        .map_or_else(
            || "length differs".to_owned(),
            |(line, (left, right))| format!("line {}:\n  want {left}\n  got  {right}", line + 1),
        );
    panic!(
        "snapshot {} drifted.\n{first_diff}\nrun ORC_UPDATE_SNAPSHOTS=1 cargo test -p orc-app to accept",
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use orc_proto::{HarnessSummary, PaneSnapshot, SessionSummary, TaskSummary, TerminalCell};

    use super::{ColorTier, GlyphTier, Glyphs, HEIGHT, Theme, ThemeName, WIDTH, gate};
    use crate::baton;
    use crate::{
        HomeData, HomeState, ScoreState, ShellState, ShellView, StageState, render_home,
        render_score, render_shell, render_stage,
    };

    const UNICODE: Glyphs = Glyphs::new(GlyphTier::Unicode);

    fn home_data(theme: ThemeName) -> HomeData {
        HomeData {
            sessions: vec![
                SessionSummary {
                    id: "bench-alpha".to_owned(),
                    brain: "codex".to_owned(),
                    workers: vec!["hermes".to_owned(), "pi-m3".to_owned()],
                    cwd: "/repo".to_owned(),
                    updated_at: "2026-07-29T09:00:00Z".to_owned(),
                    attention: 0,
                    workers_live: 2,
                    workers_total: 2,
                    conductor: "live".to_owned(),
                },
                SessionSummary {
                    id: "bench-beta".to_owned(),
                    brain: "claude".to_owned(),
                    workers: vec!["hermes".to_owned()],
                    cwd: "/repo".to_owned(),
                    updated_at: "2026-07-29T08:00:00Z".to_owned(),
                    attention: 1,
                    workers_live: 1,
                    workers_total: 2,
                    conductor: "down".to_owned(),
                },
            ],
            harnesses: vec![
                HarnessSummary {
                    id: "codex".to_owned(),
                    roles: vec!["brain".to_owned()],
                    resumable: true,
                    available: true,
                    dispatch_verified: true,
                },
                HarnessSummary {
                    id: "pi-m3".to_owned(),
                    roles: vec!["worker".to_owned()],
                    resumable: false,
                    available: false,
                    dispatch_verified: false,
                },
            ],
            discovered: Vec::new(),
            default_workers: vec!["pi-m3".to_owned()],
            max_parallel_workers: 3,
            single_harness: None,
            theme: theme.as_str().to_owned(),
            reduced_motion: false,
            leader_key: "ctrl-g".to_owned(),
        }
    }

    fn score_state() -> ScoreState {
        let card = |id: &str, title: &str, status: &str, blocked: bool| TaskSummary {
            id: id.to_owned(),
            title: title.to_owned(),
            status: status.to_owned(),
            assignee: Some("pi-m3".to_owned()),
            assignee_run: None,
            isolated: true,
            isolation: Some("ready".to_owned()),
            blocked,
            tokens: Some("1.2k".to_owned()),
            diff: Some("+4 -1".to_owned()),
            history: Vec::new(),
        };
        ScoreState {
            session_id: "bench-alpha".to_owned(),
            reports: HashMap::new(),
            tasks: vec![
                card("T0001", "queued brief", "backlog", false),
                card("T0002", "blocked brief", "assigned", true),
                card("T0003", "running brief", "running", false),
                card("T0004", "review brief", "review", false),
                card("T0005", "done brief", "done", false),
            ],
            selected: 0,
            message: String::new(),
            dragging: None,
            width: WIDTH,
        }
    }

    fn stage_panes() -> Vec<PaneSnapshot> {
        [("codex", "brain"), ("pi-m3", "worker")]
            .into_iter()
            .enumerate()
            .map(|(index, (title, role))| {
                let mut cells = vec![TerminalCell::default(); 24 * 60];
                // One grapheme per cell, exactly as a real pane grid arrives,
                // so the snapshot's text grid stays column-aligned.
                for (column, glyph) in format!("{title} ready").chars().enumerate() {
                    cells[column].text = glyph.to_string();
                }
                PaneSnapshot {
                    id: format!("pane-{index}"),
                    title: title.to_owned(),
                    rows: 24,
                    cols: 60,
                    cursor: (0, 0),
                    sequence: 1,
                    cells,
                    session_id: None,
                    harness: None,
                    role: Some(role.to_owned()),
                    state: None,
                    down_at: None,
                }
            })
            .collect()
    }

    fn runs_shell(theme: Theme) -> ShellState {
        let mut shell = ShellState {
            view: ShellView::Runs,
            home: HomeState {
                data: home_data(theme.name()),
                selected: 0,
                flow: None,
                message: String::new(),
            },
            stage: None,
            score: None,
            theme,
            glyphs: UNICODE,
            runs: orc_tui::App::with_runs(Vec::new(), theme.runs_theme()),
            reports: Vec::new(),
            help: false,
            reduced_motion: true,
            epoch: std::time::Instant::now(),
            leader: crate::LeaderKey::parse("ctrl-g"),
            leader_pending: false,
            watch_session: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        shell.runs.theme = theme.runs_theme();
        shell
    }

    /// HOME / STAGE / SCORE / RUNS in each theme, gated against the committed
    /// golden files. Both the text and the per-cell style are compared, so a
    /// palette regression cannot slip through as "the words are the same".
    #[test]
    fn every_screen_is_snapshotted_in_every_theme() {
        for name in ThemeName::ALL {
            let theme = Theme::from(name);
            let slug = name.as_str();

            let home = HomeState {
                data: home_data(name),
                selected: 0,
                flow: None,
                message: String::new(),
            };
            gate(
                &format!("home-{slug}"),
                &format!("HOME {slug} truecolor {WIDTH}x{HEIGHT}"),
                |frame| render_home(frame, &home, theme, UNICODE, None, "ctrl-g"),
            );

            let mut stage = StageState::new(stage_panes(), theme, UNICODE);
            gate(
                &format!("stage-{slug}"),
                &format!("STAGE {slug} truecolor {WIDTH}x{HEIGHT}"),
                |frame| render_stage(frame, &mut stage, None, baton::State::Sweeping(2)),
            );

            let mut score = score_state();
            gate(
                &format!("score-{slug}"),
                &format!("SCORE {slug} truecolor {WIDTH}x{HEIGHT}"),
                |frame| render_score(frame, &mut score, theme, UNICODE, "ctrl-g"),
            );

            let mut runs = runs_shell(theme);
            gate(
                &format!("runs-{slug}"),
                &format!("RUNS {slug} truecolor {WIDTH}x{HEIGHT}"),
                |frame| render_shell(frame, &mut runs),
            );
        }
    }

    /// The `NO_COLOR` tier, gated the same way. Its legend is the evidence for
    /// the claim: with every slot resolved to `reset`, what separates the
    /// states is glyph, bold, dim, and reverse — nothing else.
    #[test]
    fn the_monochrome_tier_is_snapshotted_too() {
        let theme = Theme::new(ThemeName::Nocturne, ColorTier::Monochrome);
        let home = HomeState {
            data: home_data(ThemeName::Nocturne),
            selected: 0,
            flow: None,
            message: String::new(),
        };
        gate(
            "home-no-color",
            &format!("HOME nocturne monochrome {WIDTH}x{HEIGHT}"),
            |frame| render_home(frame, &home, theme, UNICODE, None, "ctrl-g"),
        );
        let mut score = score_state();
        gate(
            "score-no-color",
            &format!("SCORE nocturne monochrome {WIDTH}x{HEIGHT}"),
            |frame| render_score(frame, &mut score, theme, UNICODE, "ctrl-g"),
        );
        let mut stage = StageState::new(stage_panes(), theme, UNICODE);
        gate(
            "stage-no-color",
            &format!("STAGE nocturne monochrome {WIDTH}x{HEIGHT}"),
            |frame| render_stage(frame, &mut stage, None, baton::State::Sweeping(2)),
        );
    }

    /// The ANSI-256 tier, where the design sheet's fallback column is what
    /// actually reaches the terminal.
    #[test]
    fn the_ansi_256_tier_is_snapshotted_too() {
        let theme = Theme::new(ThemeName::Nocturne, ColorTier::Ansi256);
        let home = HomeState {
            data: home_data(ThemeName::Nocturne),
            selected: 0,
            flow: None,
            message: String::new(),
        };
        gate(
            "home-ansi256",
            &format!("HOME nocturne ansi256 {WIDTH}x{HEIGHT}"),
            |frame| render_home(frame, &home, theme, UNICODE, None, "ctrl-g"),
        );
    }

    /// The ASCII glyph column, for a terminal that is not UTF-8 at all.
    #[test]
    fn the_ascii_glyph_column_is_snapshotted_too() {
        let theme = Theme::from(ThemeName::Nocturne);
        let ascii = Glyphs::new(GlyphTier::Ascii);
        let home = HomeState {
            data: home_data(ThemeName::Nocturne),
            selected: 0,
            flow: None,
            message: String::new(),
        };
        gate(
            "home-ascii",
            &format!("HOME nocturne truecolor ascii-glyphs {WIDTH}x{HEIGHT}"),
            |frame| render_home(frame, &home, theme, ascii, None, "ctrl-g"),
        );
        let mut stage = StageState::new(stage_panes(), theme, ascii);
        gate(
            "stage-ascii",
            &format!("STAGE nocturne truecolor ascii-glyphs {WIDTH}x{HEIGHT}"),
            |frame| render_stage(frame, &mut stage, None, baton::State::Sweeping(2)),
        );
    }
}
