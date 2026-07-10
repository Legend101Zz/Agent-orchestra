use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::inbox::{acknowledge_prompt, has_kill, pending_prompts, read_prompt};
use crate::model::RunMeta;
use crate::registry::{now_iso, read_meta, write_meta};

const JSON_ARGS: &[&str] = &[
    "-p",
    "--mode",
    "json",
    "--offline",
    "--provider",
    "minimax",
    "--model",
    "MiniMax-M3",
    "--no-session",
];

const RPC_ARGS: &[&str] = &[
    "--mode",
    "rpc",
    "--offline",
    "--provider",
    "minimax",
    "--model",
    "MiniMax-M3",
    "--no-session",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Json,
    Rpc,
}

impl Mode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Rpc => "rpc",
        }
    }

    #[must_use]
    pub fn from_meta(meta: &RunMeta) -> Self {
        if meta.mode.as_deref() == Some("rpc") {
            Self::Rpc
        } else {
            Self::Json
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub total: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug)]
struct LineEvent {
    bytes: Vec<u8>,
}

fn spawn_pi(meta: &RunMeta, mode: Mode) -> Result<Child> {
    let mut command = Command::new("pi");
    command
        .args(match mode {
            Mode::Json => JSON_ARGS,
            Mode::Rpc => RPC_ARGS,
        })
        .current_dir(&meta.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if mode == Mode::Json {
        command.arg(&meta.task).stdin(Stdio::null());
    } else {
        command.stdin(Stdio::piped());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().context("spawn pi")
}

fn pump_reader<R: std::io::Read + Send + 'static>(reader: R, sender: mpsc::Sender<LineEvent>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        loop {
            let mut bytes = Vec::new();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if bytes.last() == Some(&b'\n') {
                        bytes.pop();
                        if bytes.last() == Some(&b'\r') {
                            bytes.pop();
                        }
                    }
                    if sender.send(LineEvent { bytes }).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

fn compact_log_line(bytes: &[u8]) -> Vec<u8> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return bytes.to_vec();
    };
    if value.get("type").and_then(Value::as_str) == Some("message_update")
        && let Some(event) = value.get("assistantMessageEvent")
    {
        return serde_json::to_vec(&serde_json::json!({
            "type": "message_update",
            "assistantMessageEvent": event,
        }))
        .unwrap_or_else(|_| bytes.to_vec());
    }
    bytes.to_vec()
}

#[must_use]
pub fn extract_text(value: &Value) -> Option<&str> {
    let event = value.get("assistantMessageEvent")?;
    (event.get("type")?.as_str()? == "text_delta").then(|| event.get("delta")?.as_str())?
}

#[must_use]
pub fn extract_usage(value: &Value) -> Option<Usage> {
    let messages = value.get("messages")?.as_array()?;
    let mut best = None;
    for message in messages {
        let Some(usage) = message.get("usage").and_then(Value::as_object) else {
            continue;
        };
        let Some(total) = usage.get("totalTokens").and_then(Value::as_u64) else {
            continue;
        };
        if total == 0 {
            continue;
        }
        best = Some(Usage {
            input: usage.get("input").and_then(Value::as_u64).unwrap_or(0),
            output: usage.get("output").and_then(Value::as_u64).unwrap_or(0),
            cache_read: usage.get("cacheRead").and_then(Value::as_u64).unwrap_or(0),
            total,
            cost_usd: usage
                .get("cost")
                .and_then(|cost| cost.get("total"))
                .and_then(Value::as_f64)
                .map(|cost| (cost * 1_000_000.0).round() / 1_000_000.0),
        });
    }
    best
}

fn write_rpc_prompt(stdin: &mut ChildStdin, message: &str) -> Result<()> {
    serde_json::to_writer(
        &mut *stdin,
        &serde_json::json!({
            "type": "prompt",
            "message": message,
        }),
    )?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

#[cfg(unix)]
pub fn terminate_pid(pid: u32) {
    #[allow(unsafe_code)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
}

#[cfg(not(unix))]
pub fn terminate_pid(_pid: u32) {}

#[cfg(unix)]
fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| -status.signal().unwrap_or(1))
}

#[cfg(not(unix))]
fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

fn context_exhausted(bytes: &[u8]) -> bool {
    let line = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    [
        "context_length_exceeded",
        "maximum context length",
        "context window exceeded",
        "context limit exceeded",
        "token limit exceeded",
        "\"finish_reason\":\"length\"",
        "\"finishreason\":\"length\"",
        "\"stop_reason\":\"max_tokens\"",
        "\"stopreason\":\"max_tokens\"",
    ]
    .iter()
    .any(|signature| line.contains(signature))
}

fn finalize(
    run_dir: &Path,
    code: i32,
    usage: Option<&Usage>,
    failure_kind: Option<&str>,
) -> Result<()> {
    let mut meta = read_meta(run_dir)?;
    meta.status = if code == 0 {
        "done"
    } else if code < 0 {
        "killed"
    } else {
        "failed"
    }
    .to_owned();
    meta.exit_code = Some(code);
    meta.ended_at = Some(now_iso());
    let log_size = fs::metadata(run_dir.join("output.log")).map_or(0, |m| m.len());
    meta.tokens.estimated_total = ((meta.task.len() as u64) + log_size) / 4;
    if let Some(usage) = usage {
        meta.tokens.input = Some(usage.input);
        meta.tokens.output = Some(usage.output);
        meta.tokens.cache_read = Some(usage.cache_read);
        meta.tokens.total = Some(usage.total);
        meta.tokens.cost_usd = usage.cost_usd;
        meta.tokens.estimated_total = usage.total;
    }
    if code == 124 {
        meta.attention = Some("handoff_needed".to_owned());
        meta.failure_kind = Some("idle_timeout".to_owned());
    } else if let Some(failure_kind) = failure_kind {
        meta.attention = Some("handoff_needed".to_owned());
        meta.failure_kind = Some(failure_kind.to_owned());
    }
    write_meta(run_dir, &meta)?;
    crate::notification::run_finished(&meta);
    Ok(())
}

fn deliver_pending_prompts(
    run_dir: &Path,
    stdin: &mut ChildStdin,
    delivered: &mut HashSet<PathBuf>,
) -> Result<()> {
    for path in pending_prompts(run_dir, delivered)? {
        let message = read_prompt(&path)?;
        if message.kind != "prompt" {
            delivered.insert(path);
            continue;
        }
        write_rpc_prompt(stdin, &message.message)?;
        acknowledge_prompt(run_dir, &path)?;
        delivered.insert(path);
    }
    Ok(())
}

pub fn execute(run_dir: &Path, echo: bool, idle_timeout: f64) -> Result<i32> {
    let mut meta = read_meta(run_dir)?;
    let mode = Mode::from_meta(&meta);
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("output.log"))?;
    let mut log = BufWriter::new(log);
    let mut child = match spawn_pi(&meta, mode) {
        Ok(child) => child,
        Err(error) => {
            let message = "orc: pi executable not found on PATH\n";
            log.write_all(message.as_bytes())?;
            log.flush()?;
            if echo {
                eprint!("{message}");
            }
            finalize(run_dir, 127, None, None)?;
            return if error.downcast_ref::<std::io::Error>().is_some() {
                Ok(127)
            } else {
                Err(error)
            };
        }
    };
    let child_pid = child.id();
    meta.pid = Some(child_pid);
    meta.status = "running".to_owned();
    write_meta(run_dir, &meta)?;

    let mut stdin = child.stdin.take();
    if mode == Mode::Rpc
        && let Some(stdin) = stdin.as_mut()
    {
        write_rpc_prompt(stdin, &meta.task)?;
    }

    let (sender, receiver) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        pump_reader(stdout, sender.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        pump_reader(stderr, sender);
    }

    let interrupted = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted))?;
    let mut delivered = HashSet::new();
    let mut last_output = Instant::now();
    let mut usage = None;
    let mut killed = false;
    let mut timed_out = false;
    let mut saw_delta = false;
    let mut exhausted_context = false;
    let poll = Duration::from_millis(if mode == Mode::Rpc { 300 } else { 500 });

    loop {
        if interrupted.load(Ordering::Relaxed) {
            terminate_pid(child_pid);
            killed = true;
            break;
        }
        if mode == Mode::Rpc {
            if has_kill(run_dir) {
                terminate_pid(child_pid);
                killed = true;
                break;
            }
            if let Some(stdin) = stdin.as_mut() {
                deliver_pending_prompts(run_dir, stdin, &mut delivered)?;
            }
        }
        match receiver.recv_timeout(poll) {
            Ok(line) => {
                last_output = Instant::now();
                exhausted_context |= context_exhausted(&line.bytes);
                let compact = compact_log_line(&line.bytes);
                log.write_all(&compact)?;
                log.write_all(b"\n")?;
                log.flush()?;
                match serde_json::from_slice::<Value>(&line.bytes) {
                    Ok(value) => {
                        if let Some(text) = extract_text(&value) {
                            if echo {
                                print!("{text}");
                                std::io::stdout().flush()?;
                            }
                            saw_delta = true;
                        }
                        if value.get("type").and_then(Value::as_str) == Some("agent_end") {
                            usage = extract_usage(&value).or(usage);
                            if mode == Mode::Rpc {
                                break;
                            }
                        }
                    }
                    Err(_) if echo && !line.bytes.is_empty() => {
                        println!("{}", String::from_utf8_lossy(&line.bytes));
                    }
                    Err(_) => {}
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(status) = child.try_wait()? {
                    let mut code = exit_code(status);
                    if code == 143 || has_kill(run_dir) {
                        code = -libc::SIGTERM;
                    }
                    if saw_delta && echo {
                        println!();
                    }
                    finalize(
                        run_dir,
                        code,
                        usage.as_ref(),
                        exhausted_context.then_some("context_exhausted"),
                    )?;
                    return Ok(if code < 0 { 130 } else { code });
                }
                if idle_timeout > 0.0
                    && last_output.elapsed() > Duration::from_secs_f64(idle_timeout)
                {
                    let message = format!(
                        "\norc: idle timeout after {}s — killing worker\n",
                        idle_timeout as u64
                    );
                    log.write_all(message.as_bytes())?;
                    log.flush()?;
                    if echo {
                        eprint!("{message}");
                    }
                    terminate_pid(child_pid);
                    timed_out = true;
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    drop(stdin);
    if killed || timed_out {
        terminate_pid(child_pid);
    }
    let status = child.wait()?;
    let code = if timed_out {
        124
    } else if killed || exit_code(status) == 143 || has_kill(run_dir) {
        -libc::SIGTERM
    } else {
        exit_code(status)
    };
    if saw_delta && echo {
        println!();
    }
    finalize(
        run_dir,
        code,
        usage.as_ref(),
        exhausted_context.then_some("context_exhausted"),
    )?;
    Ok(if code < 0 { 130 } else { code })
}

#[must_use]
pub fn default_output_log(run_dir: &Path) -> PathBuf {
    run_dir.join("output.log")
}

pub fn create_empty_log(run_dir: &Path) -> Result<File> {
    File::create(default_output_log(run_dir)).context("create output.log")
}

pub fn spawn_background(run_dir: &Path, idle_timeout: Option<f64>) -> Result<()> {
    let executable = std::env::current_exe().context("locate Rust orc binary")?;
    let mut command = Command::new(executable);
    command
        .arg("_exec")
        .arg(run_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(timeout) = idle_timeout {
        command.args(["--idle-timeout", &timeout.to_string()]);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().context("launch background runner")?;
    Ok(())
}
