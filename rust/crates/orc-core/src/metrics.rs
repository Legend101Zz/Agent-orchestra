use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::RunMeta;

pub const WORKER_PRICE: (f64, f64) = (0.30, 1.20);

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ExactStats {
    pub runs: usize,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub total: u64,
    pub cost_usd: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct EstimatedStats {
    pub runs: usize,
    pub total: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GroupStats {
    pub runs: usize,
    pub total: u64,
    pub cost_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct DayStats {
    pub runs: usize,
    pub total: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct WorkerStats {
    pub runs: usize,
    pub by_status: BTreeMap<String, usize>,
    pub exact: ExactStats,
    pub estimated: EstimatedStats,
    pub by_brain: BTreeMap<String, GroupStats>,
    pub by_session: BTreeMap<String, GroupStats>,
    pub by_day: BTreeMap<String, DayStats>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct DelegatedValue {
    pub worker_cost_usd: f64,
    pub brain_equiv_usd: f64,
    pub saved_usd: f64,
    pub multiple: f64,
    pub exact_share: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct UsagePeriod {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_create: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct BrainUsage {
    pub today: UsagePeriod,
    pub week: UsagePeriod,
    pub by_model: BTreeMap<String, u64>,
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[must_use]
pub fn run_cost(meta: &RunMeta) -> f64 {
    if let Some(cost) = meta.tokens.cost_usd {
        return cost;
    }
    if meta.tokens.is_exact() {
        meta.tokens.input.unwrap_or(0) as f64 / 1e6 * WORKER_PRICE.0
            + meta.tokens.output.unwrap_or(0) as f64 / 1e6 * WORKER_PRICE.1
    } else {
        0.0
    }
}

fn status_rank(status: &str) -> usize {
    match status {
        "running" => 0,
        "starting" => 1,
        "failed" => 2,
        "killed" => 3,
        "orphaned" => 4,
        "done" => 5,
        _ => 9,
    }
}

#[must_use]
pub fn worst_status<'a>(statuses: impl IntoIterator<Item = &'a str>) -> String {
    statuses
        .into_iter()
        .min_by_key(|status| status_rank(status))
        .unwrap_or("done")
        .to_owned()
}

#[must_use]
pub fn worker_stats(runs: &[RunMeta]) -> WorkerStats {
    let mut stats = WorkerStats::default();
    let mut session_statuses: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for meta in runs {
        stats.runs += 1;
        *stats.by_status.entry(meta.status.clone()).or_default() += 1;
        let exact = meta.tokens.is_exact();
        let total = meta.tokens.displayed_total();
        let cost = run_cost(meta);
        if exact {
            stats.exact.runs += 1;
            stats.exact.input += meta.tokens.input.unwrap_or(0);
            stats.exact.output += meta.tokens.output.unwrap_or(0);
            stats.exact.cache_read += meta.tokens.cache_read.unwrap_or(0);
            stats.exact.total += total;
            stats.exact.cost_usd = round6(stats.exact.cost_usd + cost);
        } else {
            stats.estimated.runs += 1;
            stats.estimated.total += total;
        }
        let brain = stats.by_brain.entry(meta.brain.clone()).or_default();
        brain.runs += 1;
        brain.total += total;
        brain.cost_usd = round6(brain.cost_usd + cost);
        if let Some(session) = &meta.session {
            let group = stats.by_session.entry(session.clone()).or_default();
            group.runs += 1;
            group.total += total;
            group.cost_usd = round6(group.cost_usd + cost);
            session_statuses
                .entry(session.clone())
                .or_default()
                .push(meta.status.clone());
        }
        let day = meta.started_at.get(..10).unwrap_or_default();
        if !day.is_empty() {
            let group = stats.by_day.entry(day.to_owned()).or_default();
            group.runs += 1;
            group.total += total;
        }
    }
    for (session, statuses) in session_statuses {
        if let Some(group) = stats.by_session.get_mut(&session) {
            group.status = Some(worst_status(statuses.iter().map(String::as_str)));
        }
    }
    stats
}

fn brain_price(brain: &str) -> (f64, f64) {
    match brain {
        "codex" => (1.25, 10.0),
        _ => (3.0, 15.0),
    }
}

#[must_use]
pub fn delegated_value(runs: &[RunMeta]) -> DelegatedValue {
    let mut worker_cost = 0.0;
    let mut brain_equiv = 0.0;
    let mut exact_tokens = 0_u64;
    let mut all_tokens = 0_u64;
    for meta in runs {
        let price = brain_price(&meta.brain);
        worker_cost += run_cost(meta);
        if meta.tokens.is_exact() {
            brain_equiv += meta.tokens.input.unwrap_or(0) as f64 / 1e6 * price.0
                + meta.tokens.output.unwrap_or(0) as f64 / 1e6 * price.1;
            let total = meta.tokens.total.unwrap_or(0);
            exact_tokens += total;
            all_tokens += total;
        } else {
            let estimate = meta.tokens.estimated_total;
            brain_equiv += estimate as f64 / 1e6 * price.0;
            worker_cost += estimate as f64 / 1e6 * WORKER_PRICE.0;
            all_tokens += estimate;
        }
    }
    let saved = if all_tokens == 0 {
        0.0
    } else {
        ((brain_equiv - worker_cost) * 10_000.0).round() / 10_000.0
    };
    DelegatedValue {
        worker_cost_usd: round6(worker_cost),
        brain_equiv_usd: round6(brain_equiv),
        saved_usd: saved,
        multiple: if worker_cost > 0.0 {
            (brain_equiv / worker_cost * 10.0).round() / 10.0
        } else {
            0.0
        },
        exact_share: if all_tokens == 0 {
            0.0
        } else {
            (exact_tokens as f64 / all_tokens as f64 * 1000.0).round() / 1000.0
        },
    }
}

fn jsonl_paths(root: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            jsonl_paths(&path, output);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            output.push(path);
        }
    }
}

fn add_period(target: &mut UsagePeriod, source: &UsagePeriod) {
    target.input += source.input;
    target.output += source.output;
    target.cache_read += source.cache_read;
    target.cache_create += source.cache_create;
}

fn parse_claude(path: &Path) -> (BTreeMap<String, UsagePeriod>, BTreeMap<String, u64>) {
    let Ok(file) = File::open(path) else {
        return (BTreeMap::new(), BTreeMap::new());
    };
    let mut days = BTreeMap::new();
    let mut models = BTreeMap::new();
    let mut seen = HashSet::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(message) = record.get("message") else {
            continue;
        };
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let dedupe = (
            message.get("id").and_then(Value::as_str).map(str::to_owned),
            record
                .get("requestId")
                .and_then(Value::as_str)
                .map(str::to_owned),
        );
        if dedupe != (None, None) && !seen.insert(dedupe) {
            continue;
        }
        let day = record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|value| value.get(..10))
            .unwrap_or_default()
            .to_owned();
        let period = days.entry(day).or_insert_with(UsagePeriod::default);
        let input = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        period.input += input;
        period.output += output;
        period.cache_read += usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        period.cache_create += usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let model = message.get("model").and_then(Value::as_str).unwrap_or("?");
        *models.entry(model.to_owned()).or_default() += input + output;
    }
    (days, models)
}

fn parse_codex(path: &Path) -> BTreeMap<String, UsagePeriod> {
    let Ok(file) = File::open(path) else {
        return BTreeMap::new();
    };
    let mut last = None;
    let mut last_day = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(payload) = record.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        let Some(info) = payload.get("info") else {
            continue;
        };
        let Some(total) = info
            .get("total_token_usage")
            .or_else(|| info.get("last_token_usage"))
        else {
            continue;
        };
        last = Some(total.clone());
        last_day = record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|value| value.get(..10))
            .map(str::to_owned);
    }
    let Some(last) = last else {
        return BTreeMap::new();
    };
    let day = last_day.unwrap_or_default();
    BTreeMap::from([(
        day,
        UsagePeriod {
            input: last
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output: last
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_read: last
                .get("cached_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_create: 0,
        },
    )])
}

#[must_use]
pub fn brain_usage_at(
    claude_root: &Path,
    codex_root: &Path,
    today: &str,
) -> BTreeMap<String, BrainUsage> {
    let week_floor = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.checked_sub_days(chrono::Days::new(7)))
        .map_or_else(String::new, |date| date.format("%Y-%m-%d").to_string());
    let mut result = BTreeMap::new();
    let mut paths = Vec::new();
    jsonl_paths(claude_root, &mut paths);
    if !paths.is_empty() {
        let mut usage = BrainUsage::default();
        for path in paths {
            let (days, models) = parse_claude(&path);
            for (day, period) in days {
                if day == today {
                    add_period(&mut usage.today, &period);
                }
                if day >= week_floor {
                    add_period(&mut usage.week, &period);
                }
            }
            for (model, tokens) in models {
                *usage.by_model.entry(model).or_default() += tokens;
            }
        }
        result.insert("claude".to_owned(), usage);
    }
    let mut paths = Vec::new();
    jsonl_paths(codex_root, &mut paths);
    if !paths.is_empty() {
        let mut usage = BrainUsage::default();
        for path in paths {
            for (day, period) in parse_codex(&path) {
                if day == today {
                    add_period(&mut usage.today, &period);
                }
                if day >= week_floor {
                    add_period(&mut usage.week, &period);
                }
            }
        }
        result.insert("codex".to_owned(), usage);
    }
    result
}

#[must_use]
pub fn brain_usage() -> BTreeMap<String, BrainUsage> {
    let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    brain_usage_at(
        &home.join(".claude/projects"),
        &home.join(".codex/sessions"),
        &today,
    )
}
