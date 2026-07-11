#![warn(missing_docs)]
//! Per-user `orcd` process for durable Bench PTY ownership.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use clap::Parser;
use orc_daemon::{
    Daemon, DaemonRecord, PaneProcessRecord, process_identity, reap_recorded_children, serve,
    write_daemon_record,
};
use orc_pty::{HostedPane, update_signal};
use tracing::info;
use tracing_subscriber::EnvFilter;

const MAX_PANES: usize = 16;
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const RETAINED_LOGS: usize = 3;

#[derive(Debug, Parser)]
#[command(about = "Host Bench PTYs behind the per-user Unix socket")]
struct Args {
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long = "pane")]
    panes: Vec<String>,
    #[arg(long, default_value_t = 30)]
    rows: u16,
    #[arg(long, default_value_t = 90)]
    cols: u16,
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
    #[arg(long)]
    home: Option<PathBuf>,
}

struct LogState {
    path: PathBuf,
    file: File,
    bytes: u64,
}

#[derive(Clone)]
struct RotatingLog(Arc<Mutex<LogState>>);

impl RotatingLog {
    fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let bytes = file.metadata()?.len();
        Ok(Self(Arc::new(Mutex::new(LogState {
            path: path.to_owned(),
            file,
            bytes,
        }))))
    }

    fn rotate(state: &mut LogState) -> io::Result<()> {
        state.file.flush()?;
        state.file.sync_all()?;
        for index in (1..RETAINED_LOGS).rev() {
            let source = state.path.with_extension(format!("log.{index}"));
            let target = state.path.with_extension(format!("log.{}", index + 1));
            if source.exists() {
                let _ = fs::remove_file(&target);
                fs::rename(source, target)?;
            }
        }
        let first = state.path.with_extension("log.1");
        let _ = fs::remove_file(&first);
        if state.path.exists() {
            fs::rename(&state.path, first)?;
        }
        state.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&state.path)?;
        state.bytes = 0;
        Ok(())
    }
}

impl Write for RotatingLog {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("orcd log lock poisoned"))?;
        if state.bytes.saturating_add(buffer.len() as u64) > MAX_LOG_BYTES {
            Self::rotate(&mut state)?;
        }
        let written = state.file.write(buffer)?;
        state.bytes = state.bytes.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("orcd log lock poisoned"))?
            .file
            .flush()
    }
}

fn orchestra_home(explicit: Option<PathBuf>) -> PathBuf {
    explicit
        .or_else(|| std::env::var_os("ORC_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".orchestra")))
        .unwrap_or_else(|| PathBuf::from(".orchestra"))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let home = orchestra_home(args.home);
    let socket = args.socket.unwrap_or_else(|| home.join("orcd.sock"));
    let log = RotatingLog::open(&home.join("orcd.log")).context("open bounded orcd log")?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .with_ansi(false)
        .compact()
        .with_writer(move || log.clone())
        .init();

    let record_path = home.join("daemon.json");
    let reaped = reap_recorded_children(&record_path)?;
    if reaped > 0 {
        info!(reaped, "reaped identity-matched children from prior daemon");
    }
    if args.panes.len() > MAX_PANES {
        bail!("at most {MAX_PANES} panes may be hosted");
    }
    let mut panes = Vec::with_capacity(args.panes.len());
    let mut records = Vec::with_capacity(args.panes.len());
    let signal = update_signal();
    for (index, spec) in args.panes.iter().enumerate() {
        let words = shell_words::split(spec).with_context(|| format!("parse pane: {spec}"))?;
        let Some((program, program_args)) = words.split_first() else {
            bail!("pane command cannot be empty");
        };
        let id = format!("pane-{}", index + 1);
        info!(%id, command = %spec, "spawning pane");
        let pane = HostedPane::spawn_with_signal(
            &id,
            program,
            program,
            program_args,
            &args.cwd,
            args.rows,
            args.cols,
            signal.clone(),
        )?;
        if let Some(pid) = pane.process_id() {
            records.push(PaneProcessRecord {
                pane_id: id,
                session_id: "legacy-cli".to_owned(),
                process: process_identity(pid)?,
            });
        }
        panes.push(pane);
    }
    write_daemon_record(
        &record_path,
        &DaemonRecord {
            version: 1,
            panes: records,
            extra: Default::default(),
        },
    )?;
    serve(&socket, Arc::new(Daemon::new(panes, signal)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::{MAX_LOG_BYTES, RotatingLog};

    #[test]
    fn daemon_log_rotates_before_exceeding_the_bound() {
        let root = std::env::temp_dir().join(format!("orcd-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let path = root.join("orcd.log");
        let mut writer = RotatingLog::open(&path).expect("open rotating test log");
        let chunk = vec![b'x'; (MAX_LOG_BYTES / 2 + 1) as usize];
        writer.write_all(&chunk).expect("write first log chunk");
        writer.write_all(&chunk).expect("rotate second log chunk");
        writer.flush().expect("flush rotating log");
        assert!(path.with_extension("log.1").is_file());
        assert!(fs::metadata(&path).expect("current log metadata").len() <= MAX_LOG_BYTES);
        let _ = fs::remove_dir_all(root);
    }
}
