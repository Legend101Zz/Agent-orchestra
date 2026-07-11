use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::inbox::{publish_kill, publish_prompt};
use crate::metrics::{brain_usage, delegated_value, worker_stats};
use crate::model::RunMeta;
use crate::quota::{self, Gate};
use crate::registry::{
    NewRunOptions, atomic_write_json, find_run, home, list_runs, new_run, now_iso, pid_alive,
    read_meta, write_meta,
};
use crate::runner::{Mode, execute, spawn_background, terminate_pid};

#[derive(Clone, Debug)]
pub struct LaunchOptions {
    pub task: String,
    pub cwd: Option<PathBuf>,
    pub brain: String,
    pub name: Option<String>,
    pub session: Option<String>,
    pub brain_model: Option<String>,
    pub force: bool,
    pub idle_timeout: Option<f64>,
    pub background: bool,
    pub mode: Mode,
    pub retry_of: Option<String>,
    pub handoff_from: Option<String>,
}

impl LaunchOptions {
    #[must_use]
    pub fn session(&self) -> Option<String> {
        self.session
            .clone()
            .or_else(|| std::env::var("ORC_SESSION").ok())
            .filter(|session| !session.is_empty())
    }
}

fn apply_gate(force: bool) -> Result<()> {
    match quota::gate(force) {
        Gate::Proceed => Ok(()),
        Gate::Warn(message) | Gate::Unknown(message) => {
            eprintln!("{message}");
            Ok(())
        }
        Gate::Block(message) => {
            eprintln!("{message}");
            bail!("quota-blocked")
        }
    }
}

pub fn launch(options: &LaunchOptions) -> Result<(PathBuf, i32)> {
    apply_gate(options.force)?;
    let run = new_run(
        &options.task,
        &NewRunOptions {
            brain: options.brain.clone(),
            cwd: options.cwd.clone(),
            provider: "minimax".to_owned(),
            model: "MiniMax-M3".to_owned(),
            session: options.session(),
            name: options.name.clone(),
            mode: Some(options.mode.as_str().to_owned()),
            retry_of: options.retry_of.clone(),
            handoff_from: options.handoff_from.clone(),
            brain_model: options.brain_model.clone(),
        },
    )?;
    if options.background {
        spawn_background(&run, options.idle_timeout)?;
        Ok((run, 0))
    } else {
        let timeout = options
            .idle_timeout
            .unwrap_or_else(|| quota::load_config().idle_timeout_sec);
        let code = execute(&run, true, timeout)?;
        Ok((run, code))
    }
}

pub fn run_hidden(run_dir: &Path, idle_timeout: Option<f64>, echo: bool) -> Result<i32> {
    let timeout = idle_timeout.unwrap_or_else(|| quota::load_config().idle_timeout_sec);
    execute(run_dir, echo, timeout)
}

pub fn runs_as_json(reconcile: bool) -> Result<Value> {
    let values = list_runs(reconcile)?
        .into_iter()
        .map(|meta| {
            let run_dir = meta.run_dir.clone();
            let mut value = serde_json::to_value(meta)?;
            if let (Some(object), Some(run_dir)) = (value.as_object_mut(), run_dir) {
                object.insert(
                    "_dir".to_owned(),
                    Value::String(run_dir.to_string_lossy().into_owned()),
                );
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Array(values))
}

pub fn show(prefix: &str, tail: usize) -> Result<(RunMeta, Vec<String>)> {
    let run = find_run(prefix)?;
    let meta = read_meta(&run)?;
    let mut lines = VecDeque::with_capacity(tail);
    if tail > 0
        && let Ok(file) = File::open(run.join("output.log"))
    {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if lines.len() == tail {
                lines.pop_front();
            }
            lines.push_back(line);
        }
    }
    Ok((meta, lines.into()))
}

pub fn kill(prefix: &str) -> Result<RunMeta> {
    let run = find_run(prefix)?;
    let mut meta = read_meta(&run)?;
    publish_kill(&run)?;
    if let Some(pid) = meta.pid.filter(|pid| pid_alive(Some(*pid))) {
        terminate_pid(pid);
    }
    for _ in 0..25 {
        meta = read_meta(&run)?;
        if meta.is_terminal() {
            return Ok(meta);
        }
        if !pid_alive(meta.pid) {
            "killed".clone_into(&mut meta.status);
            meta.ended_at = Some(now_iso());
            meta.exit_code = Some(-libc::SIGTERM);
            write_meta(&run, &meta)?;
            return Ok(meta);
        }
        thread::sleep(Duration::from_millis(200));
    }
    Ok(meta)
}

pub fn send(prefix: &str, message: &str) -> Result<PathBuf> {
    let run = find_run(prefix)?;
    let meta = read_meta(&run)?;
    if !meta.is_running() {
        bail!("orc: {} is not running", meta.id);
    }
    if !meta.is_rpc() {
        bail!(
            "orc: {} is a one-shot JSON run; only RPC workers can be steered",
            meta.id
        );
    }
    publish_prompt(&run, message)
}

pub fn retry(
    prefix: &str,
    edited_task: Option<String>,
    foreground: bool,
) -> Result<(PathBuf, i32)> {
    let prior_dir = find_run(prefix)?;
    let prior = read_meta(&prior_dir)?;
    launch(&LaunchOptions {
        task: edited_task.unwrap_or_else(|| prior.task.clone()),
        cwd: Some(PathBuf::from(&prior.cwd)),
        brain: prior.brain.clone(),
        name: prior.name.clone(),
        session: prior.session.clone(),
        brain_model: prior.brain_model.clone(),
        force: false,
        idle_timeout: None,
        background: !foreground,
        mode: Mode::from_meta(&prior),
        retry_of: Some(prior.id),
        handoff_from: None,
    })
}

pub fn handoff(prefix: &str, brief: &str, foreground: bool) -> Result<(PathBuf, i32)> {
    let prior_dir = find_run(prefix)?;
    let prior = read_meta(&prior_dir)?;
    let task = format!(
        "Continue an existing delegated task after brain review.\n\nOriginal objective:\n{}\n\nPrevious run: {}\nPrevious output log: {}\n\nVerified remaining work:\n{}\n\nInspect the existing repository state and previous output first. Preserve completed work, do not restart the task, finish the remaining scope, and report verification evidence.",
        prior.task,
        prior.id,
        prior_dir.join("output.log").display(),
        brief
    );
    launch(&LaunchOptions {
        task,
        cwd: Some(PathBuf::from(&prior.cwd)),
        brain: prior.brain.clone(),
        name: prior.name.clone(),
        session: prior.session.clone(),
        brain_model: prior.brain_model.clone(),
        force: false,
        idle_timeout: None,
        background: !foreground,
        mode: Mode::Json,
        retry_of: None,
        handoff_from: Some(prior.id),
    })
}

pub fn stats_json() -> Result<Value> {
    let runs = list_runs(false)?;
    let brains = brain_usage();
    Ok(serde_json::json!({
        "workers": worker_stats(&runs),
        "delegated_value": delegated_value(&runs),
        "brains": {
            "claude": brains.get("claude"),
            "codex": brains.get("codex"),
        },
    }))
}

pub fn read_config_value() -> Value {
    fs::read(home().join("config.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::to_value(quota::load_config()).unwrap_or_default())
}

pub fn set_config(key: &str, raw_value: &str) -> Result<Value> {
    let mut config = read_config_value();
    let object = config
        .as_object_mut()
        .context("config.json must contain a JSON object")?;
    let value =
        serde_json::from_str(raw_value).unwrap_or_else(|_| Value::String(raw_value.to_owned()));
    object.insert(key.to_owned(), value);
    atomic_write_json(&home().join("config.json"), &config)?;
    Ok(config)
}

fn session_key(session: &str) -> String {
    session
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn session_record_path(session: &str) -> PathBuf {
    home()
        .join("sessions")
        .join(session_key(session))
        .join("session.json")
}

#[must_use]
pub fn session_budget(session: &str) -> Option<f64> {
    fs::read(session_record_path(session))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|record| record.get("advisory_budget_usd").and_then(Value::as_f64))
        .or_else(|| quota::load_config().advisory_budget_usd)
}

pub fn set_session_budget(session: &str, budget: f64) -> Result<Value> {
    let record = serde_json::json!({
        "id": session,
        "advisory_budget_usd": budget.max(0.0),
        "updated_at": now_iso(),
    });
    atomic_write_json(&session_record_path(session), &record)?;
    Ok(record)
}
