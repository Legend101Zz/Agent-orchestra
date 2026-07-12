//! `orc daemon status` and `orc daemon restart` for the per-user `orcd`.
//!
//! Status must keep working against a daemon running an older build — that is
//! exactly the situation it exists to diagnose — so this module speaks the
//! wire protocol directly instead of using the strict Bench client handshake.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use orc_proto::{
    BUILD_IDENTIFIER, ClientRequest, DaemonPaneStatus, PROTOCOL_VERSION, ServerResponse,
};

const MAX_STATUS_RESPONSE_BYTES: u64 = 1024 * 1024;

/// Socket path of the per-user daemon.
#[must_use]
pub fn socket_path() -> PathBuf {
    orc_core::registry::home().join("orcd.sock")
}

/// What a status probe learned about the running daemon, if any.
struct DaemonProbe {
    /// Build reported in `Welcome`; empty when the daemon predates the field.
    build: String,
    /// Process identifier; `None` when the daemon predates `daemon status`.
    pid: Option<u32>,
    /// Hosted panes; `None` when the daemon predates `daemon status`.
    panes: Option<Vec<DaemonPaneStatus>>,
    /// Attached clients; `None` when the daemon predates `daemon status`.
    attached_clients: Option<usize>,
}

fn request(
    stream: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    message: &ClientRequest,
) -> Result<ServerResponse> {
    serde_json::to_writer(&mut *stream, message)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut bytes = Vec::new();
    let read = reader
        .by_ref()
        .take(MAX_STATUS_RESPONSE_BYTES + 1)
        .read_until(b'\n', &mut bytes)?;
    if read == 0 || read as u64 > MAX_STATUS_RESPONSE_BYTES || !bytes.ends_with(b"\n") {
        bail!("daemon status probe received an invalid response");
    }
    serde_json::from_slice(&bytes).context("parse daemon status response")
}

/// Probe the daemon at `socket`; `Ok(None)` means no daemon is listening.
fn probe(socket: &Path) -> Result<Option<DaemonProbe>> {
    let mut stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let build = match request(
        &mut stream,
        &mut reader,
        &ClientRequest::Hello {
            version: PROTOCOL_VERSION,
        },
    )? {
        ServerResponse::Welcome { build, .. } => build,
        ServerResponse::Error { message } => bail!("daemon refused hello: {message}"),
        response => bail!("unexpected hello response: {response:?}"),
    };
    match request(&mut stream, &mut reader, &ClientRequest::DaemonStatus)? {
        ServerResponse::DaemonStatus {
            pid,
            build,
            panes,
            attached_clients,
            ..
        } => Ok(Some(DaemonProbe {
            build,
            pid: Some(pid),
            panes: Some(panes),
            attached_clients: Some(attached_clients),
        })),
        // A daemon predating the status command answers with a protocol
        // error; the connection stays usable but reveals nothing more.
        ServerResponse::Error { .. } => Ok(Some(DaemonProbe {
            build,
            pid: None,
            panes: None,
            attached_clients: None,
        })),
        response => bail!("unexpected status response: {response:?}"),
    }
}

fn live_panes(panes: &[DaemonPaneStatus]) -> Vec<&DaemonPaneStatus> {
    panes.iter().filter(|pane| pane.live).collect()
}

fn print_status(probe: &DaemonProbe, socket: &Path) {
    println!("orcd: running");
    match probe.pid {
        Some(pid) => println!("  pid: {pid}"),
        None => println!("  pid: unknown (daemon predates the status protocol)"),
    }
    if probe.build.is_empty() {
        println!("  build: unknown (older than client {BUILD_IDENTIFIER})");
    } else {
        println!("  build: {}", probe.build);
    }
    println!("  client build: {BUILD_IDENTIFIER}");
    println!("  socket: {}", socket.display());
    match &probe.panes {
        Some(panes) => {
            let live = live_panes(panes);
            println!("  live panes: {} (of {} hosted)", live.len(), panes.len());
            for pane in live {
                println!(
                    "    {} · session {} · {} {}",
                    pane.id,
                    pane.session_id.as_deref().unwrap_or("none"),
                    pane.harness.as_deref().unwrap_or("unknown"),
                    pane.role.as_deref().unwrap_or(""),
                );
            }
        }
        None => println!("  live panes: unknown (daemon predates the status protocol)"),
    }
    if let Some(clients) = probe.attached_clients {
        println!("  attached clients: {clients}");
    }
    if probe.build != BUILD_IDENTIFIER {
        println!(
            "  BUILD MISMATCH — detach clients, then run `orc daemon restart` (live panes die with the daemon)"
        );
    }
}

fn status_json(probe: Option<&DaemonProbe>, socket: &Path) -> serde_json::Value {
    match probe {
        None => serde_json::json!({
            "running": false,
            "socket": socket.display().to_string(),
            "client_build": BUILD_IDENTIFIER,
        }),
        Some(probe) => serde_json::json!({
            "running": true,
            "pid": probe.pid,
            "build": probe.build,
            "client_build": BUILD_IDENTIFIER,
            "build_matches": probe.build == BUILD_IDENTIFIER,
            "socket": socket.display().to_string(),
            "live_panes": probe.panes.as_ref().map(|panes| live_panes(panes).len()),
            "hosted_panes": probe.panes.as_ref().map(Vec::len),
            "attached_clients": probe.attached_clients,
        }),
    }
}

/// `orc daemon status`: 0 running+matching, 3 not running, 5 build mismatch.
pub fn status(json: bool) -> Result<i32> {
    let socket = socket_path();
    let probe = probe(&socket)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&status_json(probe.as_ref(), &socket))?
        );
    } else if let Some(probe) = &probe {
        print_status(probe, &socket);
    } else {
        println!("orcd: not running (socket: {})", socket.display());
        println!("  client build: {BUILD_IDENTIFIER}");
    }
    Ok(match probe {
        None => 3,
        Some(probe) if probe.build == BUILD_IDENTIFIER => 0,
        Some(_) => 5,
    })
}

fn pgrep(args: &[&str]) -> Result<Vec<u32>> {
    let output = Command::new("pgrep").args(args).output()?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|pid| pid.parse::<u32>().ok())
        .collect())
}

/// Find the orcd owning `socket`; one user may run several isolated daemons.
fn discover_pid(socket: &Path) -> Result<u32> {
    // A daemon started on demand always carries `--socket <path>`.
    let by_socket = pgrep(&["-f", &format!("orcd .*--socket {}", socket.display())])?;
    if let [pid] = by_socket.as_slice() {
        return Ok(*pid);
    }
    let all = pgrep(&["-x", "orcd"])?;
    match all.as_slice() {
        [pid] => Ok(*pid),
        [] => bail!("could not find the orcd process; stop it manually and rerun"),
        pids => bail!(
            "multiple orcd processes are running ({pids:?}) and none names {} on its command line; stop the right one manually and rerun",
            socket.display()
        ),
    }
}

fn stop_daemon(pid: u32, socket: &Path) -> Result<()> {
    let killed = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .context("send SIGTERM to orcd")?;
    if !killed.success() {
        bail!("kill -TERM {pid} failed; stop orcd manually and rerun");
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match UnixStream::connect(socket) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ) =>
            {
                return Ok(());
            }
            _ => thread::sleep(Duration::from_millis(50)),
        }
    }
    bail!("orcd (pid {pid}) did not exit within 5 seconds; stop it manually and rerun")
}

fn start_daemon(socket: &Path) -> Result<()> {
    let current = std::env::current_exe().context("locate orc")?;
    let sibling = current.with_file_name("orcd");
    let executable = if sibling.is_file() {
        sibling
    } else {
        PathBuf::from("orcd")
    };
    let mut command = Command::new(executable);
    command
        .arg("--home")
        .arg(orc_core::registry::home())
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
    let mut child = command.spawn().context("start orcd")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if UnixStream::connect(socket).is_ok() {
            return Ok(());
        }
        if let Some(exit) = child.try_wait().context("poll starting orcd")?
            && !exit.success()
        {
            bail!("orcd exited before its socket became ready: {exit}");
        }
        thread::sleep(Duration::from_millis(25));
    }
    bail!("orcd did not create {} within 3 seconds", socket.display())
}

/// `orc daemon restart`: refuse while live panes exist unless `--force`.
pub fn restart(force: bool) -> Result<i32> {
    let socket = socket_path();
    let Some(current) = probe(&socket)? else {
        println!("orcd is not running — starting it");
        start_daemon(&socket)?;
        return status(false);
    };
    match &current.panes {
        Some(panes) => {
            let live = live_panes(panes);
            if !live.is_empty() && !force {
                println!(
                    "refusing to restart: {} live pane(s) die with the daemon (PTYs are daemon-owned):",
                    live.len()
                );
                for pane in live {
                    println!(
                        "  {} · session {} · {} {}",
                        pane.id,
                        pane.session_id.as_deref().unwrap_or("none"),
                        pane.harness.as_deref().unwrap_or("unknown"),
                        pane.role.as_deref().unwrap_or(""),
                    );
                }
                println!("re-run with --force to kill them and restart");
                return Ok(1);
            }
        }
        None if !force => {
            println!(
                "refusing to restart: this daemon predates pane reporting, so live panes cannot be verified and would die silently"
            );
            println!("re-run with --force to restart anyway");
            return Ok(1);
        }
        None => {}
    }
    let pid = match current.pid {
        Some(pid) => pid,
        None => discover_pid(&socket)?,
    };
    println!("stopping orcd (pid {pid})");
    stop_daemon(pid, &socket)?;
    println!("starting orcd on build {BUILD_IDENTIFIER}");
    start_daemon(&socket)?;
    status(false)
}
