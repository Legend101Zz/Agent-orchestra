use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::Config;
use crate::registry::{atomic_write_json, home};

pub const REMAINS_URL: &str = "https://api.minimax.io/v1/token_plan/remains";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct QuotaSample {
    pub five_hour_pct: f64,
    pub weekly_pct: f64,
    pub window_resets_in_min: i64,
    pub fetched_at: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct QuotaResult {
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_hour_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_resets_in_min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Gate {
    Proceed,
    Warn(String),
    Block(String),
    Unknown(String),
}

fn epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64())
}

pub fn load_config() -> Config {
    let path = home().join("config.json");
    fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn get_key() -> Option<String> {
    let keychain = Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            &std::env::var("USER").unwrap_or_default(),
            "-s",
            "minimax_api_key",
            "-w",
        ])
        .output();
    if let Ok(output) = keychain
        && output.status.success()
    {
        let key = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !key.is_empty() {
            return Some(key);
        }
    }
    let auth = std::env::var_os("HOME")
        .map(PathBuf::from)?
        .join(".pi/agent/auth.json");
    let raw: Value = serde_json::from_slice(&fs::read(auth).ok()?).ok()?;
    let entry = raw.get("minimax")?.as_object()?;
    entry
        .get("key")
        .or_else(|| entry.get("apiKey"))
        .or_else(|| entry.get("api_key"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn parse_remains(raw: &Value, fetched_at: f64) -> Option<QuotaSample> {
    let entry = raw
        .get("model_remains")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("model_name").and_then(Value::as_str) == Some("general"))?;
    Some(QuotaSample {
        five_hour_pct: entry.get("current_interval_remaining_percent")?.as_f64()?,
        weekly_pct: entry.get("current_weekly_remaining_percent")?.as_f64()?,
        window_resets_in_min: (entry
            .get("remains_time")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            / 60_000.0)
            .round() as i64,
        fetched_at,
    })
}

pub fn fetch_remains(key: &str) -> Result<Value> {
    let mut response = ureq::get(REMAINS_URL)
        .header("Authorization", &format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .call()
        .context("MiniMax quota request")?;
    response
        .body_mut()
        .read_json()
        .context("decode MiniMax quota response")
}

#[must_use]
pub fn level_for(sample: &QuotaSample, config: &Config) -> String {
    let remaining = sample.five_hour_pct.min(sample.weekly_pct);
    if remaining <= config.block_pct {
        "block"
    } else if remaining <= config.warn_pct {
        "warn"
    } else {
        "ok"
    }
    .to_owned()
}

fn result_from_sample(sample: &QuotaSample, config: &Config, source: &str) -> QuotaResult {
    QuotaResult {
        level: level_for(sample, config),
        five_hour_pct: Some(sample.five_hour_pct),
        weekly_pct: Some(sample.weekly_pct),
        window_resets_in_min: Some(sample.window_resets_in_min),
        fetched_at: Some(sample.fetched_at),
        source: Some(source.to_owned()),
        reason: None,
    }
}

fn unknown(reason: impl Into<String>) -> QuotaResult {
    QuotaResult {
        level: "unknown".to_owned(),
        five_hour_pct: None,
        weekly_pct: None,
        window_resets_in_min: None,
        fetched_at: None,
        source: None,
        reason: Some(reason.into()),
    }
}

pub fn get_quota(force: bool) -> QuotaResult {
    let config = load_config();
    let root = home();
    if let Err(error) = fs::create_dir_all(&root) {
        return unknown(error.to_string());
    }
    let cache = root.join("quota.json");
    if !force
        && let Ok(bytes) = fs::read(&cache)
        && let Ok(sample) = serde_json::from_slice::<QuotaSample>(&bytes)
        && epoch_seconds() - sample.fetched_at < config.cache_ttl_sec as f64
    {
        return result_from_sample(&sample, &config, "cache");
    }
    let Some(key) = get_key() else {
        return unknown("no MiniMax key in Keychain or auth.json");
    };
    let sample = match fetch_remains(&key).and_then(|raw| {
        parse_remains(&raw, epoch_seconds())
            .ok_or_else(|| anyhow!("no 'general' entry — key may not be a coding-plan key"))
    }) {
        Ok(sample) => sample,
        Err(error) => return unknown(error.to_string()),
    };
    if let Err(error) = atomic_write_json(&cache, &sample) {
        return unknown(error.to_string());
    }
    let _ = append_history(&sample);
    result_from_sample(&sample, &config, "api")
}

#[must_use]
pub fn gate(force: bool) -> Gate {
    let quota = get_quota(false);
    match quota.level.as_str() {
        "warn" => Gate::Warn(format!(
            "ORC WARNING: MiniMax quota low — 5h window {}% / weekly {}% remaining. Consider pausing delegation.",
            display_pct(quota.five_hour_pct),
            display_pct(quota.weekly_pct)
        )),
        "block" if !force => Gate::Block(format!(
            "ORC BLOCKED: MiniMax quota below block threshold (5h {}%, weekly {}%). Use --force to override.",
            display_pct(quota.five_hour_pct),
            display_pct(quota.weekly_pct)
        )),
        "block" => Gate::Proceed,
        "unknown" => Gate::Unknown(format!(
            "ORC NOTE: quota unknown ({}) — proceeding.",
            quota.reason.unwrap_or_default()
        )),
        _ => Gate::Proceed,
    }
}

fn display_pct(value: Option<f64>) -> String {
    value.map_or_else(|| "?".to_owned(), |value| format!("{value}"))
}

pub fn append_history(sample: &QuotaSample) -> Result<()> {
    let path = home().join("quota_history.jsonl");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({
            "ts": epoch_seconds(),
            "five_hour_pct": sample.five_hour_pct,
            "weekly_pct": sample.weekly_pct,
        }),
    )?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn read_history(limit: usize) -> Result<Vec<Value>> {
    let path = home().join("quota_history.jsonl");
    let file = File::open(path)?;
    let mut values = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter(|value| value.get("five_hour_pct").is_some())
        .collect::<Vec<_>>();
    if values.len() > limit {
        values.drain(..values.len() - limit);
    }
    Ok(values)
}
