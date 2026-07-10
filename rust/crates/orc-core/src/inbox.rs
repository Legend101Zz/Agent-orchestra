use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::registry::{atomic_write_json, now_iso};

static MESSAGE_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PromptMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub at: Option<String>,
}

fn message_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    format!(
        "{millis}-{:04x}",
        MESSAGE_NONCE.fetch_add(1, Ordering::Relaxed) & 0xffff
    )
}

pub fn publish_prompt(run_dir: &Path, message: &str) -> Result<PathBuf> {
    let path = run_dir
        .join("inbox")
        .join(format!("prompt-{}.json", message_id()));
    atomic_write_json(
        &path,
        &PromptMessage {
            kind: "prompt".to_owned(),
            message: message.to_owned(),
            at: Some(now_iso()),
        },
    )?;
    Ok(path)
}

pub fn publish_kill(run_dir: &Path) -> Result<PathBuf> {
    let path = run_dir
        .join("inbox")
        .join(format!("kill-{}.json", message_id()));
    atomic_write_json(&path, &serde_json::json!({"type": "kill", "at": now_iso()}))?;
    Ok(path)
}

#[must_use]
pub fn has_kill(run_dir: &Path) -> bool {
    fs::read_dir(run_dir.join("inbox")).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("kill-") && name.ends_with(".json"))
        })
    })
}

pub fn pending_prompts(run_dir: &Path, delivered: &HashSet<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut prompts = fs::read_dir(run_dir.join("inbox"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.starts_with("prompt-") && name.ends_with(".json")
            }) && !delivered.contains(path)
        })
        .collect::<Vec<_>>();
    prompts.sort();
    Ok(prompts)
}

pub fn read_prompt(path: &Path) -> Result<PromptMessage> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

pub fn acknowledge_prompt(run_dir: &Path, prompt_path: &Path) -> Result<PathBuf> {
    let processed = run_dir.join("inbox").join("processed");
    fs::create_dir_all(&processed)?;
    let name = prompt_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("prompt.json");
    let ack = processed.join(format!("ack-{name}"));
    atomic_write_json(
        &ack,
        &serde_json::json!({"type": "ack", "of": name, "at": now_iso()}),
    )?;
    Ok(ack)
}
