use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use orc_core::model::RunMeta;
use orc_core::quota::{self, QuotaResult};
use orc_core::search::search_runs;
use ratatui::layout::Rect;
use serde_json::Value;

use crate::snapshot::Snapshot;
use crate::theme::Theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Session,
    Settings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputMode {
    None,
    Search,
    NewTask,
    Send,
    Handoff,
}

#[derive(Clone, Debug)]
pub enum DisplayRow {
    Session {
        key: String,
        members: Vec<usize>,
        status: String,
    },
    Run {
        index: usize,
        child: bool,
    },
}

#[derive(Clone, Debug, Default)]
pub struct RunDetail {
    pub reply: String,
    pub thinking: String,
    pub tail: Vec<String>,
    pub timeline: Vec<String>,
    pub prompt_count: usize,
}

#[derive(Clone, Debug)]
struct CachedDetail {
    modified: std::time::SystemTime,
    len: u64,
    detail: RunDetail,
}

impl DisplayRow {
    #[must_use]
    pub fn key(&self, runs: &[RunMeta]) -> String {
        match self {
            Self::Session { key, .. } => format!("session:{key}"),
            Self::Run { index, .. } => format!("run:{}", runs[*index].id),
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub snapshot: Snapshot,
    pub rows: Vec<DisplayRow>,
    pub selected_row: usize,
    pub selected_key: Option<String>,
    pub expanded: HashSet<String>,
    pub view: View,
    pub session_members: Vec<usize>,
    pub selected_worker: usize,
    pub detail_tab: usize,
    pub detail_scroll: u16,
    pub theme: Theme,
    pub quota: QuotaResult,
    pub quota_history: Vec<Value>,
    pub config: Value,
    pub query: String,
    pub search_ids: HashSet<String>,
    pub input_mode: InputMode,
    pub input: String,
    pub message: String,
    pub help: bool,
    pub last_refresh: Instant,
    pub table_area: Rect,
    pub viewport_start: usize,
    detail_cache: HashMap<String, CachedDetail>,
}

impl App {
    pub fn new(theme_override: Option<&str>) -> Result<Self> {
        let config = orc_core::control::read_config_value();
        let theme_name = theme_override
            .or_else(|| config.get("theme").and_then(Value::as_str))
            .unwrap_or("ember");
        let mut snapshot = Snapshot::default();
        snapshot.refresh()?;
        let quota = quota::get_quota(false);
        let quota_history = quota::read_history(96).unwrap_or_default();
        let mut app = Self {
            snapshot,
            rows: Vec::new(),
            selected_row: 0,
            selected_key: None,
            expanded: HashSet::new(),
            view: View::Dashboard,
            session_members: Vec::new(),
            selected_worker: 0,
            detail_tab: 0,
            detail_scroll: 0,
            theme: Theme::named(theme_name),
            quota,
            quota_history,
            config,
            query: String::new(),
            search_ids: HashSet::new(),
            input_mode: InputMode::None,
            input: String::new(),
            message: String::new(),
            help: false,
            last_refresh: Instant::now(),
            table_area: Rect::default(),
            viewport_start: 0,
            detail_cache: HashMap::new(),
        };
        app.rebuild_rows();
        Ok(app)
    }

    #[cfg(test)]
    pub fn with_runs(runs: Vec<RunMeta>, theme: Theme) -> Self {
        let mut app = Self {
            snapshot: Snapshot::from_runs(runs),
            rows: Vec::new(),
            selected_row: 0,
            selected_key: None,
            expanded: HashSet::new(),
            view: View::Dashboard,
            session_members: Vec::new(),
            selected_worker: 0,
            detail_tab: 0,
            detail_scroll: 0,
            theme,
            quota: QuotaResult {
                level: "ok".to_owned(),
                five_hour_pct: Some(71.0),
                weekly_pct: Some(46.0),
                window_resets_in_min: Some(137),
                fetched_at: Some(0.0),
                source: Some("fixture".to_owned()),
                reason: None,
            },
            quota_history: Vec::new(),
            config: serde_json::json!({"advisory_budget_usd": 0.5}),
            query: String::new(),
            search_ids: HashSet::new(),
            input_mode: InputMode::None,
            input: String::new(),
            message: String::new(),
            help: false,
            last_refresh: Instant::now(),
            table_area: Rect::default(),
            viewport_start: 0,
            detail_cache: HashMap::new(),
        };
        app.rebuild_rows();
        app
    }

    pub fn refresh(&mut self) -> Result<bool> {
        if self.last_refresh.elapsed() < Duration::from_millis(500) {
            return Ok(false);
        }
        self.last_refresh = Instant::now();
        let changed = self.snapshot.refresh()?;
        if changed {
            self.rebuild_rows();
            self.rebuild_session_members();
        }
        Ok(changed)
    }

    fn attention_rank(status: &str, attention: Option<&str>) -> usize {
        if attention.is_some() || status == "failed" {
            0
        } else if matches!(status, "running" | "starting") {
            1
        } else if matches!(status, "killed" | "orphaned") {
            2
        } else {
            3
        }
    }

    fn visible_run(&self, run: &RunMeta) -> bool {
        if self.query.is_empty() {
            return true;
        }
        self.search_ids.contains(&run.id)
    }

    pub fn rebuild_rows(&mut self) {
        let preserve = self
            .rows
            .get(self.selected_row)
            .map(|row| row.key(&self.snapshot.runs))
            .or_else(|| self.selected_key.clone());
        let mut sessions: HashMap<String, Vec<usize>> = HashMap::new();
        let mut singles = Vec::new();
        for (index, run) in self.snapshot.runs.iter().enumerate() {
            if !self.visible_run(run) {
                continue;
            }
            if let Some(session) = &run.session {
                sessions.entry(session.clone()).or_default().push(index);
            } else {
                singles.push(index);
            }
        }
        let mut groups = sessions
            .into_iter()
            .map(|(key, members)| {
                let representative = members
                    .iter()
                    .map(|index| &self.snapshot.runs[*index])
                    .min_by_key(|run| Self::attention_rank(&run.status, run.attention.as_deref()))
                    .expect("session has members");
                let status = if members
                    .iter()
                    .any(|index| self.snapshot.runs[*index].is_running())
                {
                    "running".to_owned()
                } else {
                    representative.status.clone()
                };
                (
                    Self::attention_rank(
                        &representative.status,
                        representative.attention.as_deref(),
                    ),
                    representative.created_ts,
                    DisplayRow::Session {
                        key,
                        members,
                        status,
                    },
                )
            })
            .collect::<Vec<_>>();
        groups.extend(singles.into_iter().map(|index| {
            let run = &self.snapshot.runs[index];
            (
                Self::attention_rank(&run.status, run.attention.as_deref()),
                run.created_ts,
                DisplayRow::Run {
                    index,
                    child: false,
                },
            )
        }));
        groups.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| right.1.total_cmp(&left.1))
        });
        self.rows.clear();
        for (_, _, row) in groups {
            if let DisplayRow::Session { key, members, .. } = &row {
                let key = key.clone();
                let members = members.clone();
                self.rows.push(row);
                if self.expanded.contains(&key) {
                    self.rows.extend(
                        members
                            .into_iter()
                            .map(|index| DisplayRow::Run { index, child: true }),
                    );
                }
            } else {
                self.rows.push(row);
            }
        }
        self.selected_row = preserve
            .as_ref()
            .and_then(|key| {
                self.rows
                    .iter()
                    .position(|row| row.key(&self.snapshot.runs) == *key)
            })
            .unwrap_or_else(|| self.selected_row.min(self.rows.len().saturating_sub(1)));
        self.selected_key = self
            .rows
            .get(self.selected_row)
            .map(|row| row.key(&self.snapshot.runs));
    }

    fn rebuild_session_members(&mut self) {
        let Some(selected) = self.current_run().map(|run| run.id.clone()) else {
            return;
        };
        self.session_members =
            if let Some(session) = self.current_run().and_then(|run| run.session.clone()) {
                self.snapshot
                    .runs
                    .iter()
                    .enumerate()
                    .filter(|(_, run)| run.session.as_deref() == Some(&session))
                    .map(|(index, _)| index)
                    .collect()
            } else {
                self.snapshot
                    .runs
                    .iter()
                    .position(|run| run.id == selected)
                    .into_iter()
                    .collect()
            };
        self.selected_worker = self
            .session_members
            .iter()
            .position(|index| self.snapshot.runs[*index].id == selected)
            .unwrap_or(0);
    }

    #[must_use]
    pub fn current_run(&self) -> Option<&RunMeta> {
        if self.view == View::Session {
            return self
                .session_members
                .get(self.selected_worker)
                .and_then(|index| self.snapshot.runs.get(*index));
        }
        match self.rows.get(self.selected_row)? {
            DisplayRow::Run { index, .. } => self.snapshot.runs.get(*index),
            DisplayRow::Session { members, .. } => members
                .iter()
                .map(|index| &self.snapshot.runs[*index])
                .find(|run| run.is_running() || run.attention.is_some())
                .or_else(|| members.first().map(|index| &self.snapshot.runs[*index])),
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.view == View::Session {
            let len = self.session_members.len();
            if len > 0 {
                self.selected_worker = self
                    .selected_worker
                    .saturating_add_signed(delta)
                    .min(len - 1);
                self.detail_scroll = 0;
            }
            return;
        }
        let len = self.rows.len();
        if len > 0 {
            self.selected_row = self.selected_row.saturating_add_signed(delta).min(len - 1);
            self.selected_key = self
                .rows
                .get(self.selected_row)
                .map(|row| row.key(&self.snapshot.runs));
        }
    }

    pub fn open_selected(&mut self) {
        let Some(row) = self.rows.get(self.selected_row).cloned() else {
            return;
        };
        match row {
            DisplayRow::Session { key, .. } => {
                if !self.expanded.insert(key.clone()) {
                    self.expanded.remove(&key);
                }
                self.rebuild_rows();
            }
            DisplayRow::Run { .. } => {
                self.rebuild_session_members();
                self.view = View::Session;
            }
        }
    }

    fn invoke(&mut self, args: &[&str]) {
        let result = std::env::current_exe()
            .context("locate orc")
            .and_then(|exe| {
                Command::new(exe)
                    .args(args)
                    .output()
                    .context("run orc action")
            });
        self.message = match result {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            }
            Ok(output) => String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            Err(error) => error.to_string(),
        };
        self.last_refresh = Instant::now() - Duration::from_secs(1);
    }

    fn begin_input(&mut self, mode: InputMode) {
        self.input_mode = mode;
        self.input.clear();
    }

    fn submit_input(&mut self) {
        let value = self.input.trim().to_owned();
        match self.input_mode {
            InputMode::Search => {
                self.query.clone_from(&value);
                self.search_ids = search_runs(&self.snapshot.runs, &value, 500)
                    .into_iter()
                    .map(|hit| hit.run_id)
                    .collect();
                self.rebuild_rows();
                self.message = if value.is_empty() {
                    String::new()
                } else {
                    format!("{} matching runs", self.search_ids.len())
                };
            }
            InputMode::NewTask if !value.is_empty() => {
                self.invoke(&["run", &value, "--bg", "--brain", "human"]);
            }
            InputMode::Send if !value.is_empty() => {
                if let Some(id) = self.current_run().map(|run| run.id.clone()) {
                    self.invoke(&["send", &id, &value]);
                }
            }
            InputMode::Handoff if !value.is_empty() => {
                if let Some(id) = self.current_run().map(|run| run.id.clone()) {
                    self.invoke(&["handoff", &id, &value]);
                }
            }
            _ => {}
        }
        self.input_mode = InputMode::None;
        self.input.clear();
    }

    fn cycle_theme(&mut self) {
        self.theme = self.theme.other();
        let name = self.theme.name.to_owned();
        self.invoke(&["config", "set", "theme", &name]);
        self.config["theme"] = Value::String(name);
    }

    fn cycle_notifications(&mut self) {
        let current = self
            .config
            .get("notifications")
            .and_then(Value::as_str)
            .unwrap_or("actionable");
        let next = match current {
            "actionable" => "all",
            "all" => "off",
            _ => "actionable",
        };
        self.invoke(&["config", "set", "notifications", next]);
        self.config["notifications"] = Value::String(next.to_owned());
    }

    fn adjust_setting(&mut self, key: &str, delta: f64, floor: f64, ceiling: f64) {
        let current = self
            .config
            .get(key)
            .and_then(Value::as_f64)
            .unwrap_or(floor);
        let next = (current + delta).clamp(floor, ceiling);
        let raw = format!("{next}");
        self.invoke(&["config", "set", key, &raw]);
        self.config[key] = serde_json::json!(next);
    }

    fn adjust_session_budget(&mut self, delta: f64) {
        let Some(session) = self.current_run().and_then(|run| run.session.clone()) else {
            self.message = "selected worker has no session".to_owned();
            return;
        };
        let current = orc_core::control::session_budget(&session).unwrap_or(0.0);
        let next = (current + delta).max(0.0);
        let raw = next.to_string();
        self.invoke(&["budget", &session, &raw]);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.input_mode != InputMode::None {
            match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::None;
                    self.input.clear();
                }
                KeyCode::Enter => self.submit_input(),
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.input.push(character);
                }
                _ => {}
            }
            return false;
        }
        if self.help {
            self.help = false;
            return false;
        }
        if self.view == View::Settings {
            match key.code {
                KeyCode::Esc | KeyCode::Char(',') => self.view = View::Dashboard,
                KeyCode::Char('t') => self.cycle_theme(),
                KeyCode::Char('n') => self.cycle_notifications(),
                KeyCode::Char('w') => self.adjust_setting("warn_pct", 5.0, 5.0, 95.0),
                KeyCode::Char('W') => self.adjust_setting("warn_pct", -5.0, 5.0, 95.0),
                KeyCode::Char('b') => self.adjust_setting("block_pct", 5.0, 0.0, 90.0),
                KeyCode::Char('B') => self.adjust_setting("block_pct", -5.0, 0.0, 90.0),
                KeyCode::Char('+') => self.adjust_setting("advisory_budget_usd", 0.10, 0.0, 1000.0),
                KeyCode::Char('-') => {
                    self.adjust_setting("advisory_budget_usd", -0.10, 0.0, 1000.0)
                }
                KeyCode::Char('q') => return true,
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc if self.view == View::Session => self.view = View::Dashboard,
            KeyCode::Char('j') | KeyCode::Down if self.view == View::Session => {
                self.detail_scroll = self.detail_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up if self.view == View::Session => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
            }
            KeyCode::PageDown if self.view == View::Session => {
                self.detail_scroll = self.detail_scroll.saturating_add(8);
            }
            KeyCode::PageUp if self.view == View::Session => {
                self.detail_scroll = self.detail_scroll.saturating_sub(8);
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Enter if self.view == View::Dashboard => self.open_selected(),
            KeyCode::Char('/') if self.view == View::Dashboard => {
                self.begin_input(InputMode::Search)
            }
            KeyCode::Char('n') if self.view == View::Dashboard => {
                self.begin_input(InputMode::NewTask)
            }
            KeyCode::Char('t') => self.cycle_theme(),
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char(',') => self.view = View::Settings,
            KeyCode::Tab if self.view == View::Session => {
                self.detail_tab = (self.detail_tab + 1) % 4
            }
            KeyCode::BackTab if self.view == View::Session => {
                self.detail_tab = (self.detail_tab + 3) % 4
            }
            KeyCode::Char(']') if self.view == View::Session => self.move_selection(1),
            KeyCode::Char('[') if self.view == View::Session => self.move_selection(-1),
            KeyCode::Char('s') if self.view == View::Session => {
                if self.current_run().is_some_and(RunMeta::is_rpc) {
                    self.begin_input(InputMode::Send);
                } else {
                    self.message = "SEND is available only for RPC workers".to_owned();
                }
            }
            KeyCode::Char('r') if self.view == View::Session => {
                if let Some(id) = self.current_run().map(|run| run.id.clone()) {
                    self.invoke(&["retry", &id]);
                }
            }
            KeyCode::Char('h') if self.view == View::Session => {
                self.begin_input(InputMode::Handoff)
            }
            KeyCode::Char('+') if self.view == View::Session => {
                self.adjust_session_budget(0.10);
            }
            KeyCode::Char('-') if self.view == View::Session => {
                self.adjust_session_budget(-0.10);
            }
            KeyCode::Char('x') => {
                if let Some(id) = self.current_run().map(|run| run.id.clone()) {
                    self.invoke(&["kill", &id]);
                }
            }
            _ => {}
        }
        false
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown if self.view == View::Session => {
                self.detail_scroll = self.detail_scroll.saturating_add(3);
            }
            MouseEventKind::ScrollUp if self.view == View::Session => {
                self.detail_scroll = self.detail_scroll.saturating_sub(3);
            }
            MouseEventKind::ScrollDown => self.move_selection(3),
            MouseEventKind::ScrollUp => self.move_selection(-3),
            MouseEventKind::Down(_) if self.view == View::Dashboard => {
                if mouse.column >= self.table_area.x
                    && mouse.column < self.table_area.right()
                    && mouse.row > self.table_area.y
                    && mouse.row < self.table_area.bottom()
                {
                    let row = self.viewport_start + usize::from(mouse.row - self.table_area.y - 1);
                    if row < self.rows.len() {
                        self.selected_row = row;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn current_detail(&mut self) -> RunDetail {
        let Some(run) = self.current_run().cloned() else {
            return RunDetail::default();
        };
        let Some(run_dir) = run.run_dir.clone() else {
            return RunDetail::default();
        };
        let path = run_dir.join("output.log");
        let stat = fs::metadata(&path).ok();
        let modified = stat
            .as_ref()
            .and_then(|stat| stat.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let len = stat.as_ref().map_or(0, fs::Metadata::len);
        if let Some(cached) = self.detail_cache.get(&run.id)
            && cached.modified == modified
            && cached.len == len
        {
            return cached.detail.clone();
        }
        let mut detail = parse_detail(&run, &run_dir, &path, len);
        detail.timeline.sort();
        self.detail_cache.insert(
            run.id,
            CachedDetail {
                modified,
                len,
                detail: detail.clone(),
            },
        );
        detail
    }
}

fn parse_detail(
    run: &RunMeta,
    run_dir: &std::path::Path,
    path: &std::path::Path,
    len: u64,
) -> RunDetail {
    const MAX_DETAIL_BYTES: u64 = 8 * 1024 * 1024;
    let mut detail = RunDetail::default();
    if let Ok(mut file) = File::open(path) {
        if len > MAX_DETAIL_BYTES {
            let _ = file.seek(SeekFrom::End(-(MAX_DETAIL_BYTES as i64)));
            let mut discard = String::new();
            let _ = BufReader::new(&mut file).read_line(&mut discard);
        }
        let mut bytes = Vec::new();
        let _ = file.take(MAX_DETAIL_BYTES).read_to_end(&mut bytes);
        let text = String::from_utf8_lossy(&bytes);
        let mut plain = Vec::new();
        for line in text.lines() {
            match serde_json::from_str::<Value>(line) {
                Ok(value) => {
                    if let Some(event) = value.get("assistantMessageEvent") {
                        let delta = event
                            .get("delta")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        match event.get("type").and_then(Value::as_str) {
                            Some("text_delta") => detail.reply.push_str(delta),
                            Some(kind) if kind.contains("thinking") => {
                                detail.thinking.push_str(delta)
                            }
                            _ => {}
                        }
                    }
                }
                Err(_) => plain.push(line.to_owned()),
            }
        }
        detail.tail = text.lines().rev().take(120).map(str::to_owned).collect();
        detail.tail.reverse();
        if detail.reply.is_empty() && !plain.is_empty() {
            detail.reply = plain.join("\n");
        }
    }
    detail
        .timeline
        .push(format!("{}  DISPATCHED", run.started_at));
    if let Some(ended) = &run.ended_at {
        detail
            .timeline
            .push(format!("{ended}  {}", run.status.to_uppercase()));
    }
    if let Ok(entries) = fs::read_dir(run_dir.join("inbox")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("prompt-") {
                detail.prompt_count += 1;
                detail.timeline.push(format!("{}  FOLLOW-UP QUEUED", name));
            } else if name.starts_with("kill-") {
                detail.timeline.push(format!("{}  KILL REQUESTED", name));
            }
        }
    }
    if run.retry_of.is_some() {
        detail
            .timeline
            .push("link  RETRY OF PRIOR WORKER".to_owned());
    }
    if run.handoff_from.is_some() {
        detail
            .timeline
            .push("link  HANDOFF FROM PRIOR WORKER".to_owned());
    }
    detail
}
