use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_warn_pct() -> f64 {
    25.0
}

fn default_block_pct() -> f64 {
    10.0
}

fn default_cache_ttl() -> u64 {
    60
}

fn default_parallel() -> usize {
    3
}

fn default_idle_timeout() -> f64 {
    300.0
}

fn default_theme() -> String {
    "ember".to_owned()
}

fn default_notifications() -> String {
    "actionable".to_owned()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Config {
    #[serde(default = "default_warn_pct")]
    pub warn_pct: f64,
    #[serde(default = "default_block_pct")]
    pub block_pct: f64,
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_sec: u64,
    #[serde(default = "default_parallel")]
    pub max_parallel_workers: usize,
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_sec: f64,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_notifications")]
    pub notifications: String,
    #[serde(default)]
    pub advisory_budget_usd: Option<f64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            warn_pct: default_warn_pct(),
            block_pct: default_block_pct(),
            cache_ttl_sec: default_cache_ttl(),
            max_parallel_workers: default_parallel(),
            idle_timeout_sec: default_idle_timeout(),
            theme: default_theme(),
            notifications: default_notifications(),
            advisory_budget_usd: None,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Tokens {
    #[serde(default)]
    pub estimated_total: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Tokens {
    #[must_use]
    pub fn displayed_total(&self) -> u64 {
        self.total.unwrap_or(self.estimated_total)
    }

    #[must_use]
    pub const fn is_exact(&self) -> bool {
        self.total.is_some()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RunMeta {
    pub id: String,
    pub task: String,
    pub brain: String,
    pub cwd: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub pid: Option<u32>,
    pub status: String,
    pub started_at: String,
    #[serde(default)]
    pub created_ts: f64,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub tokens: Tokens,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brain_model: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
    #[serde(skip)]
    pub run_dir: Option<PathBuf>,
}

impl RunMeta {
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self.status.as_str(), "starting" | "running")
    }

    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "done" | "failed" | "killed" | "orphaned"
        )
    }

    #[must_use]
    pub fn is_rpc(&self) -> bool {
        self.mode.as_deref() == Some("rpc")
    }
}
