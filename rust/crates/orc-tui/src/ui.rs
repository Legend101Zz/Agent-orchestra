use chrono::{DateTime, Utc};
use orc_core::metrics::{delegated_value, run_cost, worker_stats};
use orc_core::model::RunMeta;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use serde_json::Value;

use crate::app::{App, DisplayRow, InputMode, View};
use crate::theme::Theme;

fn spaced(value: &str) -> String {
    value
        .to_uppercase()
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn fmt_tokens(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1e6)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1e3)
    } else {
        value.to_string()
    }
}

fn fmt_cost(value: f64) -> String {
    if value == 0.0 {
        "—".to_owned()
    } else if value < 0.01 {
        format!("${value:.4}")
    } else {
        format!("${value:.2}")
    }
}

fn fmt_elapsed(run: &RunMeta) -> String {
    let Ok(start) = DateTime::parse_from_rfc3339(&run.started_at) else {
        return "—".to_owned();
    };
    let end = run
        .ended_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .unwrap_or_else(|| Utc::now().into());
    let seconds = (end - start).num_seconds().max(0);
    if seconds >= 3600 {
        format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60)
    } else if seconds >= 60 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn mark(status: &str) -> &'static str {
    match status {
        "running" => "●",
        "starting" => "◐",
        "done" => "■",
        "failed" => "×",
        "killed" => "○",
        "orphaned" => "·",
        _ => "?",
    }
}

fn panel<'a>(title: &'a str, theme: Theme, focus: bool) -> Block<'a> {
    let display = if title.chars().count() <= 24 {
        spaced(title)
    } else {
        title.to_uppercase()
    };
    Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(if focus {
            theme.border_focus
        } else {
            theme.border
        }))
        .title(Span::styled(
            display,
            Style::default()
                .fg(theme.label)
                .add_modifier(Modifier::BOLD),
        ))
}

fn meter(percent: Option<f64>, width: usize, theme: Theme) -> Vec<Span<'static>> {
    let Some(percent) = percent else {
        return vec![Span::styled("unavailable", Style::default().fg(theme.warn))];
    };
    let width = width.max(8);
    let filled = ((percent.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    (0..width)
        .map(|index| {
            if index >= filled {
                return Span::styled("─", Style::default().fg(theme.border));
            }
            let fraction = index as f64 / width as f64;
            let color = if fraction < 0.10 {
                theme.error
            } else if fraction < 0.25 {
                theme.warn
            } else if fraction < 0.60 {
                theme.accent_2
            } else {
                theme.ok
            };
            Span::styled("━", Style::default().fg(color))
        })
        .collect()
}

fn braille(values: &[f64], width: usize) -> String {
    const STEPS: &[char] = &['⠀', '⡀', '⡄', '⡆', '⡇', '⣇', '⣧', '⣷', '⣿'];
    if values.is_empty() {
        return "⠀".repeat(width);
    }
    let max = values.iter().copied().fold(1.0_f64, f64::max);
    let start = values.len().saturating_sub(width);
    let mut result = "⠀".repeat(width.saturating_sub(values.len()));
    for value in &values[start..] {
        let step = ((*value / max) * (STEPS.len() - 1) as f64).round() as usize;
        result.push(STEPS[step.min(STEPS.len() - 1)]);
    }
    result
}

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    frame.render_widget(
        Block::default().style(Style::default().bg(app.theme.bg).fg(app.theme.text)),
        frame.area(),
    );
    match app.view {
        View::Dashboard => dashboard(frame, app),
        View::Session => session(frame, app),
        View::Settings => settings(frame, app),
    }
    if app.help {
        help(frame, app);
    }
    if app.input_mode != InputMode::None {
        input(frame, app);
    }
}

fn header(frame: &mut Frame<'_>, app: &App, area: Rect, context: &str) {
    let active = app
        .snapshot
        .runs
        .iter()
        .filter(|run| run.is_running())
        .count();
    let attention = app
        .snapshot
        .runs
        .iter()
        .filter(|run| run.status == "failed" || run.attention.is_some())
        .count();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  ORC",
                Style::default()
                    .fg(app.theme.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" / ", Style::default().fg(app.theme.border)),
            Span::styled(context.to_owned(), Style::default().fg(app.theme.accent)),
            Span::styled("    ACTIVE ", Style::default().fg(app.theme.label)),
            Span::styled(active.to_string(), Style::default().fg(app.theme.running)),
            Span::styled("    ATTENTION ", Style::default().fg(app.theme.label)),
            Span::styled(attention.to_string(), Style::default().fg(app.theme.error)),
            Span::styled(
                format!("    {}", app.theme.name),
                Style::default().fg(app.theme.text_dim),
            ),
        ]))
        .style(Style::default().bg(app.theme.panel)),
        area,
    );
}

fn dashboard(frame: &mut Frame<'_>, app: &mut App) {
    let log_height = if frame.area().height >= 34 { 8 } else { 5 };
    let summary_height = if frame.area().width >= 110 { 7 } else { 9 };
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(summary_height),
        Constraint::Length(2),
        Constraint::Min(7),
        Constraint::Length(log_height),
        Constraint::Length(2),
    ])
    .split(frame.area());
    header(frame, app, chunks[0], "CONTROL PLANE");
    summary(frame, app, chunks[1]);
    activity(frame, app, chunks[2]);
    runs(frame, app, chunks[3]);
    output(frame, app, chunks[4]);
    footer(frame, app, chunks[5]);
}

fn summary(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let columns = if area.width >= 110 {
        Layout::horizontal([
            Constraint::Percentage(38),
            Constraint::Percentage(24),
            Constraint::Percentage(20),
            Constraint::Percentage(18),
        ])
        .split(area)
        .to_vec()
    } else {
        let rows =
            Layout::vertical([Constraint::Percentage(52), Constraint::Percentage(48)]).split(area);
        let top = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(rows[0]);
        let bottom = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);
        vec![top[0], top[1], bottom[0], bottom[1]]
    };
    let width = usize::from(columns[0].width.saturating_sub(15));
    let mut five = vec![Span::styled("5H  ", Style::default().fg(app.theme.label))];
    five.extend(meter(app.quota.five_hour_pct, width, app.theme));
    five.push(Span::raw(format!(
        " {:>3.0}%",
        app.quota.five_hour_pct.unwrap_or(0.0)
    )));
    let mut week = vec![Span::styled("WK  ", Style::default().fg(app.theme.label))];
    week.extend(meter(app.quota.weekly_pct, width, app.theme));
    week.push(Span::raw(format!(
        " {:>3.0}%",
        app.quota.weekly_pct.unwrap_or(0.0)
    )));
    let history = app
        .quota_history
        .iter()
        .filter_map(|value| value.get("five_hour_pct").and_then(Value::as_f64))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(five),
            Line::from(week),
            Line::from(vec![
                Span::styled(
                    braille(&history, width.min(28)),
                    Style::default().fg(app.theme.accent),
                ),
                Span::styled(
                    format!(
                        "  {} · reset {}m",
                        app.quota.level.to_uppercase(),
                        app.quota.window_resets_in_min.unwrap_or_default()
                    ),
                    Style::default().fg(app.theme.text_dim),
                ),
            ]),
        ])
        .block(panel("quota / minimax", app.theme, true)),
        columns[0],
    );

    let attention = app
        .snapshot
        .runs
        .iter()
        .filter(|run| run.status == "failed" || run.attention.is_some())
        .take(3)
        .collect::<Vec<_>>();
    let lines = if attention.is_empty() {
        vec![
            Line::from(Span::styled(
                "CLEAR",
                Style::default()
                    .fg(app.theme.ok)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "no failed or stalled workers",
                Style::default().fg(app.theme.text_dim),
            )),
        ]
    } else {
        attention
            .iter()
            .map(|run| {
                Line::from(vec![
                    Span::styled("× ", Style::default().fg(app.theme.error)),
                    Span::styled(
                        run.id.chars().take(22).collect::<String>(),
                        Style::default().fg(app.theme.text),
                    ),
                ])
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines).block(panel("needs attention", app.theme, !attention.is_empty())),
        columns[1],
    );
    let stats = worker_stats(&app.snapshot.runs);
    let selected_session = app.current_run().and_then(|run| run.session.as_deref());
    let budget_line = selected_session.map_or_else(
        || "select a session for budget".to_owned(),
        |session| {
            let spend = app
                .snapshot
                .runs
                .iter()
                .filter(|run| run.session.as_deref() == Some(session))
                .map(run_cost)
                .sum::<f64>();
            orc_core::control::session_budget(session).map_or_else(
                || format!("${spend:.2} · no budget set"),
                |budget| format!("${spend:.2} / ${budget:.2} advisory"),
            )
        },
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    stats
                        .by_status
                        .get("running")
                        .copied()
                        .unwrap_or(0)
                        .to_string(),
                    Style::default()
                        .fg(app.theme.running)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" workers live", Style::default().fg(app.theme.text_dim)),
            ]),
            Line::from(vec![
                Span::styled(
                    stats.by_session.len().to_string(),
                    Style::default().fg(app.theme.text),
                ),
                Span::styled(" sessions", Style::default().fg(app.theme.text_dim)),
            ]),
            Line::from(Span::styled(
                budget_line,
                Style::default().fg(app.theme.label),
            )),
        ])
        .block(panel("session state", app.theme, false)),
        columns[2],
    );
    let value = delegated_value(&app.snapshot.runs);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("≈ ${:.2} saved", value.saved_usd),
                Style::default()
                    .fg(app.theme.accent_2)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("{:.1}× vs brain rates", value.multiple),
                Style::default().fg(app.theme.text),
            )),
            Line::from(Span::styled(
                format!("{:.0}% exact basis", value.exact_share * 100.0),
                Style::default().fg(app.theme.text_dim),
            )),
        ])
        .block(panel("value / basis", app.theme, false)),
        columns[3],
    );
}

fn activity(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let now = Utc::now().timestamp() as f64;
    let mut buckets = [0.0_f64; 48];
    let mut estimated = false;
    for run in &app.snapshot.runs {
        let age = now - run.created_ts;
        if (0.0..86_400.0).contains(&age) {
            let index = 47 - (age / 1800.0) as usize;
            buckets[index] += run.tokens.displayed_total() as f64;
            estimated |= !run.tokens.is_exact();
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("  {}  ", spaced("completed volume")),
                Style::default().fg(app.theme.label),
            ),
            Span::styled(
                braille(&buckets, usize::from(area.width.saturating_sub(66))),
                Style::default().fg(app.theme.accent),
            ),
            Span::styled(
                if estimated {
                    "  24h · exact + ~estimated"
                } else {
                    "  24h · exact"
                },
                Style::default().fg(app.theme.text_dim),
            ),
        ]))
        .style(Style::default().bg(app.theme.surface)),
        area,
    );
}

fn row_values(app: &App, row: &DisplayRow) -> [String; 6] {
    match row {
        DisplayRow::Session {
            key,
            members,
            status,
        } => {
            let tokens = members
                .iter()
                .map(|index| app.snapshot.runs[*index].tokens.displayed_total())
                .sum();
            let cost = members
                .iter()
                .map(|index| run_cost(&app.snapshot.runs[*index]))
                .sum();
            [
                format!(
                    "{}  SESSION / {key}",
                    if app.expanded.contains(key) {
                        "▾"
                    } else {
                        "▸"
                    }
                ),
                format!("{} {status}", mark(status)),
                format!("{} workers", members.len()),
                "—".to_owned(),
                fmt_tokens(tokens),
                fmt_cost(cost),
            ]
        }
        DisplayRow::Run { index, child } => {
            let run = &app.snapshot.runs[*index];
            [
                format!(
                    "{}{}",
                    if *child { "   └─ " } else { "" },
                    run.id.chars().take(38).collect::<String>()
                ),
                format!("{} {}", mark(&run.status), run.status),
                run.brain.to_uppercase(),
                fmt_elapsed(run),
                format!(
                    "{}{}",
                    if run.tokens.is_exact() { "" } else { "~" },
                    fmt_tokens(run.tokens.displayed_total())
                ),
                fmt_cost(run_cost(run)),
            ]
        }
    }
}

fn runs(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    app.table_area = area;
    if app.rows.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "NO DELEGATED RUNS YET",
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from("orc run \"audit this repository\" --bg --brain codex"),
                Line::from("orc rpc \"investigate interactively\" --bg"),
                Line::from(Span::styled(
                    "Press n to launch from this console.",
                    Style::default().fg(app.theme.text_dim),
                )),
            ])
            .alignment(Alignment::Center)
            .block(panel("sessions / runs", app.theme, true)),
            area,
        );
        return;
    }
    let capacity = usize::from(area.height.saturating_sub(3)).max(1);
    let start = app
        .selected_row
        .saturating_sub(capacity / 2)
        .min(app.rows.len().saturating_sub(capacity));
    app.viewport_start = start;
    let end = (start + capacity).min(app.rows.len());
    let wide = area.width >= 100;
    let rows = app.rows[start..end]
        .iter()
        .map(|row| {
            let values = row_values(app, row);
            let status = match row {
                DisplayRow::Session { status, .. } => status.as_str(),
                DisplayRow::Run { index, .. } => app.snapshot.runs[*index].status.as_str(),
            };
            let mut cells = vec![
                Cell::from(values[0].clone()),
                Cell::from(Line::from(Span::styled(
                    values[1].clone(),
                    Style::default().fg(app.theme.status(status)),
                ))),
            ];
            if wide {
                cells.push(Cell::from(values[2].clone()));
            }
            cells.extend([
                Cell::from(values[3].clone()),
                Cell::from(values[4].clone()),
                Cell::from(values[5].clone()),
            ]);
            Row::new(cells)
        })
        .collect::<Vec<_>>();
    let headers = if wide {
        vec![
            "SESSION / RUN",
            "STATUS",
            "CONTROLLER",
            "ELAPSED",
            "TOKENS",
            "COST",
        ]
    } else {
        vec!["SESSION / RUN", "STATUS", "ELAPSED", "TOKENS", "COST"]
    };
    let widths = if wide {
        vec![
            Constraint::Min(34),
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(9),
        ]
    } else {
        vec![
            Constraint::Min(30),
            Constraint::Length(13),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(8),
        ]
    };
    let table = Table::new(rows, widths)
        .header(
            Row::new(headers).style(
                Style::default()
                    .fg(app.theme.label)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .row_highlight_style(
            Style::default()
                .bg(app.theme.surface)
                .fg(app.theme.text)
                .add_modifier(Modifier::BOLD),
        )
        .block(panel("sessions / runs", app.theme, true));
    let mut state = TableState::default();
    state.select(Some(app.selected_row - start));
    frame.render_stateful_widget(table, area, &mut state);
}

fn output(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let selected = app.current_run().map(|run| run.id.clone());
    let detail = app.current_detail();
    let lines = if detail.reply.is_empty() {
        vec![Line::from(Span::styled(
            "no worker output yet",
            Style::default().fg(app.theme.text_dim),
        ))]
    } else {
        detail
            .reply
            .lines()
            .rev()
            .take(usize::from(area.height.saturating_sub(2)))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| Line::from(line.to_owned()))
            .collect()
    };
    let title = selected.map_or_else(
        || "live output".to_owned(),
        |id| format!("live output / {}", id.chars().take(30).collect::<String>()),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(&title, app.theme, false))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" j/k ", Style::default().fg(app.theme.accent)),
        Span::styled("navigate  ", Style::default().fg(app.theme.text_dim)),
        Span::styled(" enter ", Style::default().fg(app.theme.accent)),
        Span::styled("open  ", Style::default().fg(app.theme.text_dim)),
        Span::styled(" / ", Style::default().fg(app.theme.accent)),
        Span::styled("search output  ", Style::default().fg(app.theme.text_dim)),
        Span::styled(" n ", Style::default().fg(app.theme.accent)),
        Span::styled("new  ", Style::default().fg(app.theme.text_dim)),
        Span::styled(" , ", Style::default().fg(app.theme.accent)),
        Span::styled("settings  ", Style::default().fg(app.theme.text_dim)),
        Span::styled(" q ", Style::default().fg(app.theme.accent)),
        Span::styled("quit", Style::default().fg(app.theme.text_dim)),
    ]);
    let message = if app.message.is_empty() {
        &app.query
    } else {
        &app.message
    };
    frame.render_widget(
        Paragraph::new(vec![
            line,
            Line::from(Span::styled(
                message.clone(),
                Style::default().fg(app.theme.warn),
            )),
        ])
        .style(Style::default().bg(app.theme.panel)),
        area,
    );
}

fn session(frame: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(2),
    ])
    .split(frame.area());
    header(frame, app, chunks[0], "SESSION WORKSPACE");
    let panes = if chunks[1].width >= 110 {
        Layout::horizontal([Constraint::Percentage(36), Constraint::Percentage(64)])
            .split(chunks[1])
    } else if chunks[1].width >= 80 {
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1])
    } else {
        Layout::vertical([Constraint::Percentage(42), Constraint::Percentage(58)]).split(chunks[1])
    };
    topology(frame, app, panes[0]);
    detail(frame, app, panes[1]);
    session_footer(frame, app, chunks[2]);
}

fn topology(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let current = app.current_run();
    let brain = current.map_or("human", |run| run.brain.as_str());
    let model = current
        .and_then(|run| run.brain_model.as_deref())
        .unwrap_or("model not recorded");
    let session = current
        .and_then(|run| run.session.as_deref())
        .unwrap_or("single run");
    let session_spend = current
        .and_then(|run| run.session.as_deref())
        .map(|session| {
            app.snapshot
                .runs
                .iter()
                .filter(|run| run.session.as_deref() == Some(session))
                .map(run_cost)
                .sum::<f64>()
        })
        .unwrap_or(0.0);
    let budget = current
        .and_then(|run| run.session.as_deref())
        .and_then(orc_core::control::session_budget);
    let mut lines = vec![
        Line::from(Span::styled(
            session.to_owned(),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            budget.map_or_else(
                || format!("${session_spend:.2} spent · budget not set"),
                |budget| format!("${session_spend:.2} / ${budget:.2} advisory budget"),
            ),
            Style::default().fg(if budget.is_some_and(|budget| session_spend > budget) {
                app.theme.warn
            } else {
                app.theme.text_dim
            }),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                brain.to_uppercase(),
                Style::default()
                    .fg(app.theme.controller(brain))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" / CONTROLLER", Style::default().fg(app.theme.label)),
        ]),
        Line::from(Span::styled(
            model.to_owned(),
            Style::default().fg(app.theme.text_dim),
        )),
        Line::from(Span::styled(
            "        │  task dispatch",
            Style::default().fg(app.theme.border_focus),
        )),
    ];
    for (position, index) in app.session_members.iter().enumerate() {
        let run = &app.snapshot.runs[*index];
        let selected = position == app.selected_worker;
        let rail = if position + 1 == app.session_members.len() {
            "        └─"
        } else {
            "        ├─"
        };
        let link = if run.handoff_from.is_some() {
            "HANDOFF"
        } else if run.retry_of.is_some() {
            "RETRY"
        } else {
            "M3"
        };
        lines.push(Line::from(vec![
            Span::styled(
                rail,
                Style::default().fg(if selected {
                    app.theme.accent
                } else {
                    app.theme.border
                }),
            ),
            Span::styled(
                format!(" {link} / MINIMAX M3"),
                Style::default()
                    .fg(if selected {
                        app.theme.accent_2
                    } else {
                        app.theme.text
                    })
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("           "),
            Span::styled(
                format!("{} {}", mark(&run.status), run.status.to_uppercase()),
                Style::default().fg(app.theme.status(&run.status)),
            ),
            Span::styled(
                format!(
                    "  {}  {}  {}",
                    fmt_elapsed(run),
                    fmt_tokens(run.tokens.displayed_total()),
                    fmt_cost(run_cost(run))
                ),
                Style::default().fg(app.theme.text_dim),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            format!("           {}", run.id.chars().take(30).collect::<String>()),
            Style::default().fg(app.theme.text_dim),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("controller / workers", app.theme, true))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn detail(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let run = app.current_run().cloned();
    let content = app.current_detail();
    let sections = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(area);
    let tabs = ["CONVERSATION", "LOG", "TIMELINE", "META"];
    let tab_line = Line::from(
        tabs.iter()
            .enumerate()
            .flat_map(|(index, name)| {
                let style = if index == app.detail_tab {
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(app.theme.text_dim)
                };
                [Span::styled(format!(" {name} "), style), Span::raw("  ")]
            })
            .collect::<Vec<_>>(),
    );
    let status = run.as_ref().map_or("—", |run| run.status.as_str());
    frame.render_widget(
        Paragraph::new(vec![
            tab_line,
            Line::from(vec![
                Span::styled(
                    format!("{} {status}", mark(status)),
                    Style::default().fg(app.theme.status(status)),
                ),
                Span::styled(
                    format!("   {} follow-ups", content.prompt_count),
                    Style::default().fg(app.theme.text_dim),
                ),
            ]),
        ])
        .block(panel("selected worker", app.theme, true)),
        sections[0],
    );
    let text = match app.detail_tab {
        1 => Text::from(content.tail.into_iter().map(Line::from).collect::<Vec<_>>()),
        2 => Text::from(
            content
                .timeline
                .into_iter()
                .map(Line::from)
                .collect::<Vec<_>>(),
        ),
        3 => Text::from(run.as_ref().map_or_else(Vec::new, |run| {
            serde_json::to_string_pretty(run)
                .unwrap_or_default()
                .lines()
                .map(|line| Line::from(line.to_owned()))
                .collect()
        })),
        _ => {
            let mut lines = vec![Line::from(Span::styled(
                "PROMPT",
                Style::default()
                    .fg(app.theme.label)
                    .add_modifier(Modifier::BOLD),
            ))];
            if let Some(run) = &run {
                lines.extend(run.task.lines().map(|line| Line::from(line.to_owned())));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "WORKER REPLY",
                Style::default()
                    .fg(app.theme.label)
                    .add_modifier(Modifier::BOLD),
            )));
            if content.reply.is_empty() {
                lines.push(Line::from(Span::styled(
                    "waiting for output",
                    Style::default().fg(app.theme.text_dim),
                )));
            } else {
                lines.extend(
                    content
                        .reply
                        .lines()
                        .map(|line| Line::from(line.to_owned())),
                );
            }
            Text::from(lines)
        }
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(panel("workspace", app.theme, false))
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0)),
        sections[1],
    );
}

fn session_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let can_send = app
        .current_run()
        .is_some_and(|run| run.is_running() && run.is_rpc());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" esc ", Style::default().fg(app.theme.accent)),
                Span::styled("back  ", Style::default().fg(app.theme.text_dim)),
                Span::styled(" j/k ", Style::default().fg(app.theme.accent)),
                Span::styled("scroll  ", Style::default().fg(app.theme.text_dim)),
                Span::styled(" [ ] ", Style::default().fg(app.theme.accent)),
                Span::styled("workers  ", Style::default().fg(app.theme.text_dim)),
                Span::styled(" tab ", Style::default().fg(app.theme.accent)),
                Span::styled("detail  ", Style::default().fg(app.theme.text_dim)),
                Span::styled(
                    " s ",
                    Style::default().fg(if can_send {
                        app.theme.accent
                    } else {
                        app.theme.border
                    }),
                ),
                Span::styled("send  ", Style::default().fg(app.theme.text_dim)),
                Span::styled(" r ", Style::default().fg(app.theme.accent)),
                Span::styled("retry  ", Style::default().fg(app.theme.text_dim)),
                Span::styled(" h ", Style::default().fg(app.theme.accent)),
                Span::styled("handoff  ", Style::default().fg(app.theme.text_dim)),
                Span::styled(" x ", Style::default().fg(app.theme.error)),
                Span::styled("kill", Style::default().fg(app.theme.text_dim)),
                Span::styled("  + / - ", Style::default().fg(app.theme.accent)),
                Span::styled("budget", Style::default().fg(app.theme.text_dim)),
            ]),
            Line::from(Span::styled(
                app.message.clone(),
                Style::default().fg(app.theme.warn),
            )),
        ])
        .style(Style::default().bg(app.theme.panel)),
        area,
    );
}

fn settings(frame: &mut Frame<'_>, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(2),
    ])
    .split(frame.area());
    header(frame, app, chunks[0], "SETTINGS");
    let value = |key: &str| {
        app.config
            .get(key)
            .map_or_else(|| "—".to_owned(), Value::to_string)
    };
    let lines = vec![
        Line::from(format!(
            "THEME                 {}      t cycle",
            app.theme.name
        )),
        Line::from(format!(
            "NOTIFICATIONS         {}      n cycle",
            value("notifications")
        )),
        Line::from(""),
        Line::from(format!(
            "WARN THRESHOLD        {}      w / W",
            value("warn_pct")
        )),
        Line::from(format!(
            "BLOCK THRESHOLD       {}      b / B",
            value("block_pct")
        )),
        Line::from(""),
        Line::from(format!(
            "ADVISORY BUDGET       ${}      + / -",
            value("advisory_budget_usd")
        )),
        Line::from(Span::styled(
            "Budgets warn and project. They never block, stop, or pre-allocate worker thinking.",
            Style::default().fg(app.theme.text_dim),
        )),
    ];
    let width = frame.area().width.saturating_sub(8).min(96);
    let box_area = Rect::new(
        chunks[1].x + (chunks[1].width.saturating_sub(width)) / 2,
        chunks[1].y + 2,
        width,
        14_u16.min(chunks[1].height),
    );
    frame.render_widget(
        Paragraph::new(lines).block(panel("operator controls", app.theme, true)),
        box_area,
    );
    frame.render_widget(
        Paragraph::new(" esc / ,  back")
            .style(Style::default().bg(app.theme.panel).fg(app.theme.text_dim)),
        chunks[2],
    );
}

fn help(frame: &mut Frame<'_>, app: &App) {
    let area = centered(frame.area(), 68, 18);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                spaced("operator keys"),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("j / k / arrows      navigate stable run identity"),
            Line::from("enter               expand session or open worker"),
            Line::from("/                   cross-run prompt and output search"),
            Line::from("n                   launch a background worker"),
            Line::from("s                   steer selected RPC worker"),
            Line::from("r / h / x           retry / brain-reviewed handoff / kill"),
            Line::from("tab                 conversation / log / timeline / meta"),
            Line::from(",                   settings"),
            Line::from("t                   switch theme"),
            Line::from("q                   quit from dashboard"),
            Line::from(""),
            Line::from(Span::styled(
                "~ marks estimates. Budgets are advisory. Provider quota remains the safety gate.",
                Style::default().fg(app.theme.text_dim),
            )),
        ])
        .block(panel("help / honesty", app.theme, true))
        .style(Style::default().bg(app.theme.panel)),
        area,
    );
}

fn input(frame: &mut Frame<'_>, app: &App) {
    let title = match app.input_mode {
        InputMode::Search => "search prompts and output",
        InputMode::NewTask => "new MiniMax task",
        InputMode::Send => "follow-up to selected RPC worker",
        InputMode::Handoff => "brain-verified remaining work",
        InputMode::None => "input",
    };
    let area = Rect::new(
        frame.area().x + 2,
        frame.area().bottom().saturating_sub(4),
        frame.area().width.saturating_sub(4),
        3,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(format!("> {}_", app.input))
            .block(panel(title, app.theme, true))
            .style(Style::default().bg(app.theme.panel)),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
