mod app;
mod snapshot;
mod theme;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;

pub use app::App;
pub use theme::{EMBER, PHOSPHOR, Theme};

pub fn run(theme: Option<&str>) -> Result<()> {
    let mut app = App::new(theme)?;
    let mut terminal = ratatui::try_init()?;
    execute!(io::stdout(), EnableMouseCapture)?;
    let result = (|| -> Result<()> {
        let mut redraw = true;
        loop {
            redraw |= app.refresh()?;
            if redraw {
                terminal.draw(|frame| ui::draw(frame, &mut app))?;
                redraw = false;
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.handle_key(key) {
                        break;
                    }
                    redraw = true;
                }
                Event::Mouse(mouse) => {
                    app.handle_mouse(mouse);
                    redraw = true;
                }
                Event::Resize(_, _) => redraw = true,
                _ => {}
            }
        }
        Ok(())
    })();
    execute!(io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use orc_core::model::{RunMeta, Tokens};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::{App, EMBER, ui};

    fn run(id: &str, status: &str, session: Option<&str>) -> RunMeta {
        RunMeta {
            id: id.to_owned(),
            task: "Audit the registry and report evidence".to_owned(),
            brain: "codex".to_owned(),
            cwd: "/tmp".to_owned(),
            provider: "minimax".to_owned(),
            model: "MiniMax-M3".to_owned(),
            pid: None,
            status: status.to_owned(),
            started_at: "2026-07-10T12:00:00+00:00".to_owned(),
            created_ts: 1.0,
            ended_at: None,
            exit_code: None,
            tokens: Tokens {
                estimated_total: 42_000,
                ..Tokens::default()
            },
            session: session.map(str::to_owned),
            name: None,
            mode: Some("rpc".to_owned()),
            retry_of: None,
            handoff_from: None,
            attention: (status == "failed").then(|| "handoff_needed".to_owned()),
            failure_kind: None,
            brain_model: Some("GPT-5".to_owned()),
            extra: BTreeMap::new(),
            run_dir: None,
        }
    }

    #[test]
    fn dashboard_and_split_workspace_render_without_emoji() {
        let backend = TestBackend::new(150, 44);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::with_runs(
            vec![
                run("worker-a", "running", Some("session-v3")),
                run("worker-b", "failed", Some("session-v3")),
            ],
            EMBER,
        );
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let dashboard = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(dashboard.contains("N E E D S   A T T E N T I O N"));
        assert!(!dashboard.contains('🧠'));
        assert!(!dashboard.contains('🤖'));

        app.expanded.insert("session-v3".to_owned());
        app.rebuild_rows();
        app.selected_row = 1;
        app.open_selected();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let session = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(session.contains("C O N T R O L L E R   /   W O R K E R S"));
        assert!(session.contains("MINIMAX M3"));
        assert!(session.contains("CONVERSATION"));
    }
}
