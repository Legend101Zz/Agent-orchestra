#![warn(missing_docs)]
//! `pi-orchestra` HOME, STAGE, and attach client.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use orc_app::{BenchClient, ThemeName, benchmark, visible_input_benchmark};

#[derive(Debug, Parser)]
#[command(
    about = "Open or attach to the pi-orchestra Bench",
    version = orc_proto::BUILD_IDENTIFIER
)]
struct Args {
    #[command(subcommand)]
    command: Option<AppCommand>,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long, default_value = "ember")]
    theme: String,
    #[arg(long)]
    bench: bool,
    #[arg(long, default_value_t = 1_000)]
    iterations: usize,
    #[arg(long)]
    metrics: bool,
    #[arg(long)]
    snapshot_once: bool,
    #[arg(long)]
    visible_bench: bool,
    #[arg(long, default_value = "pane-1")]
    pane_id: String,
}

#[derive(Debug, Subcommand)]
enum AppCommand {
    /// Open HOME.
    Home,
    /// Reconnect to a durable session, defaulting to the newest.
    Attach {
        /// Stable session identifier.
        session: Option<String>,
    },
    /// Open the event-driven RUNS ledger.
    Runs,
}

fn orchestra_home() -> PathBuf {
    std::env::var_os("ORC_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".orchestra")))
        .unwrap_or_else(|| PathBuf::from(".orchestra"))
}

fn ensure_daemon(socket: &std::path::Path, home: &std::path::Path) -> Result<()> {
    if std::os::unix::net::UnixStream::connect(socket).is_ok() {
        return Ok(());
    }
    let current = std::env::current_exe().context("locate pi-orchestra")?;
    let sibling = current.with_file_name("orcd");
    let executable = if sibling.is_file() {
        sibling
    } else {
        PathBuf::from("orcd")
    };
    let mut command = Command::new(executable);
    command
        .arg("--home")
        .arg(home)
        .arg("--socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().context("start orcd on demand")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if std::os::unix::net::UnixStream::connect(socket).is_ok() {
            return Ok(());
        }
        if let Some(status) = child.try_wait().context("poll starting orcd")?
            && !status.success()
        {
            bail!("orcd exited before its socket became ready: {status}");
        }
        thread::sleep(Duration::from_millis(25));
    }
    bail!("orcd did not create {} within 3 seconds", socket.display())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let home = orchestra_home();
    let socket = args.socket.unwrap_or_else(|| home.join("orcd.sock"));
    ensure_daemon(&socket, &home)?;
    if args.bench {
        let summary = benchmark(&socket, args.iterations)?;
        println!(
            "{{\"iterations\":{},\"p50_us\":{},\"p95_us\":{},\"p99_us\":{},\"max_us\":{}}}",
            args.iterations, summary.p50_us, summary.p95_us, summary.p99_us, summary.max_us
        );
        return Ok(());
    }
    if args.visible_bench {
        let summary = visible_input_benchmark(&socket, &args.pane_id, args.iterations)?;
        println!(
            "{{\"iterations\":{},\"p50_us\":{},\"p95_us\":{},\"p99_us\":{},\"max_us\":{}}}",
            args.iterations, summary.p50_us, summary.p95_us, summary.p99_us, summary.max_us
        );
        return Ok(());
    }
    if args.metrics {
        let metrics = BenchClient::connect(&socket)?.metrics()?;
        println!("{}", serde_json::to_string(&metrics)?);
        return Ok(());
    }
    if args.snapshot_once {
        let panes = BenchClient::connect(&socket)?.snapshot(None)?;
        let sequences = panes
            .iter()
            .map(|pane| (&pane.id, pane.sequence))
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string(&sequences)?);
        return Ok(());
    }
    let (initial_session, runs) = match args.command {
        Some(AppCommand::Attach { session }) => {
            let session = if session.is_some() {
                session
            } else {
                BenchClient::connect(&socket)?
                    .home()?
                    .sessions
                    .first()
                    .map(|session| session.id.clone())
            };
            (session, false)
        }
        Some(AppCommand::Runs) => (None, true),
        Some(AppCommand::Home) | None => (None, false),
    };
    orc_app::run_initial(socket, ThemeName::named(&args.theme), initial_session, runs)?;
    Ok(())
}
