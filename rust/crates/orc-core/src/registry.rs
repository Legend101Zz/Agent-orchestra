use std::cmp::Ordering;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Local, Utc};
use serde::Serialize;

use crate::model::{RunMeta, Tokens};

static NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default)]
pub struct NewRunOptions {
    pub brain: String,
    pub cwd: Option<PathBuf>,
    pub provider: String,
    pub model: String,
    pub session: Option<String>,
    pub name: Option<String>,
    pub mode: Option<String>,
    pub retry_of: Option<String>,
    pub handoff_from: Option<String>,
    pub brain_model: Option<String>,
}

impl NewRunOptions {
    #[must_use]
    pub fn python_defaults() -> Self {
        Self {
            brain: "human".to_owned(),
            cwd: None,
            provider: "minimax".to_owned(),
            model: "MiniMax-M3".to_owned(),
            session: None,
            name: None,
            mode: None,
            retry_of: None,
            handoff_from: None,
            brain_model: None,
        }
    }
}

#[must_use]
pub fn home() -> PathBuf {
    std::env::var_os("ORC_HOME").map_or_else(
        || {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".orchestra")
        },
        PathBuf::from,
    )
}

pub fn runs_dir() -> Result<PathBuf> {
    let path = home().join("runs");
    fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
    Ok(path)
}

pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let tmp = parent.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, AtomicOrdering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        fs::rename(&tmp, path)
            .with_context(|| format!("rename {} to {}", tmp.display(), path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[must_use]
pub fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

#[must_use]
pub fn make_slug(task: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for ch in task.chars().take(24) {
        if ch.is_ascii_alphanumeric() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(ch.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    if slug.is_empty() {
        "task".to_owned()
    } else {
        slug
    }
}

fn suffix() -> u16 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    (nanos ^ u64::from(std::process::id()) ^ NONCE.fetch_add(1, AtomicOrdering::Relaxed)) as u16
}

pub fn new_run(task: &str, options: &NewRunOptions) -> Result<PathBuf> {
    let root = runs_dir()?;
    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let slug = make_slug(task);
    for _ in 0..16 {
        let id = format!("{timestamp}-{slug}-{:04x}", suffix());
        let run_dir = root.join(&id);
        match fs::create_dir(&run_dir) {
            Ok(()) => {
                fs::create_dir(run_dir.join("inbox"))?;
                let cwd = options.cwd.clone().unwrap_or(std::env::current_dir()?);
                let meta = RunMeta {
                    id,
                    task: task.to_owned(),
                    brain: if options.brain.is_empty() {
                        "human".to_owned()
                    } else {
                        options.brain.clone()
                    },
                    cwd: cwd.to_string_lossy().into_owned(),
                    provider: if options.provider.is_empty() {
                        "minimax".to_owned()
                    } else {
                        options.provider.clone()
                    },
                    model: if options.model.is_empty() {
                        "MiniMax-M3".to_owned()
                    } else {
                        options.model.clone()
                    },
                    pid: None,
                    status: "starting".to_owned(),
                    started_at: now_iso(),
                    created_ts: now_epoch(),
                    ended_at: None,
                    exit_code: None,
                    tokens: Tokens::default(),
                    session: options.session.clone(),
                    name: options.name.clone(),
                    mode: options.mode.clone(),
                    retry_of: options.retry_of.clone(),
                    handoff_from: options.handoff_from.clone(),
                    attention: None,
                    failure_kind: None,
                    brain_model: options.brain_model.clone(),
                    extra: Default::default(),
                    run_dir: None,
                };
                write_meta(&run_dir, &meta)?;
                return Ok(run_dir);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("create run directory"),
        }
    }
    bail!("orc: could not allocate a unique run id")
}

pub fn read_meta(run_dir: &Path) -> Result<RunMeta> {
    let path = run_dir.join("meta.json");
    let file = File::open(&path).with_context(|| format!("open {}", path.display()))?;
    serde_json::from_reader(BufReader::new(file))
        .with_context(|| format!("parse {}", path.display()))
}

pub fn write_meta(run_dir: &Path, meta: &RunMeta) -> Result<()> {
    atomic_write_json(&run_dir.join("meta.json"), meta)
}

#[must_use]
pub fn pid_alive(pid: Option<u32>) -> bool {
    let Some(pid) = pid.filter(|pid| *pid > 0) else {
        return false;
    };
    #[allow(unsafe_code)]
    let result = unsafe { libc::kill(pid.cast_signed(), 0) };
    if result == 0 {
        return true;
    }
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    )
}

pub fn find_run(prefix: &str) -> Result<PathBuf> {
    let matches = fs::read_dir(runs_dir()?)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.starts_with(prefix) || name.contains(prefix)
            })
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => bail!("orc: no runs match '{prefix}'"),
        _ => bail!("orc: {} runs match '{prefix}'", matches.len()),
    }
}

pub fn list_runs(reconcile: bool) -> Result<Vec<RunMeta>> {
    let mut runs = Vec::new();
    for entry in fs::read_dir(runs_dir()?)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.join("meta.json").is_file() {
            continue;
        }
        let Ok(mut meta) = read_meta(&path) else {
            continue;
        };
        if reconcile && matches!(meta.status.as_str(), "starting" | "running") {
            let should_reconcile =
                !pid_alive(meta.pid) && (meta.status == "running" || meta.pid.is_some());
            if should_reconcile {
                meta.status = "orphaned".to_owned();
                if meta.ended_at.is_none() {
                    meta.ended_at = Some(now_iso());
                }
                write_meta(&path, &meta)?;
            }
        }
        meta.run_dir = Some(path);
        runs.push(meta);
    }
    runs.sort_by(|left, right| {
        right
            .created_ts
            .partial_cmp(&left.created_ts)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(runs)
}
