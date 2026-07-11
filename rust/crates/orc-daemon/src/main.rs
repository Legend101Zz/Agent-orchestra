#![warn(missing_docs)]
//! `orcd` phase-one daemon spike.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;
use orc_daemon::{Daemon, serve};
use orc_pty::{HostedPane, update_signal};
use tracing::info;
use tracing_subscriber::EnvFilter;

const MAX_PANES: usize = 16;

#[derive(Debug, Parser)]
#[command(about = "Host Bench spike PTYs behind a Unix socket")]
struct Args {
    #[arg(long, default_value = "/tmp/orcd-spike.sock")]
    socket: PathBuf,
    #[arg(long = "pane")]
    panes: Vec<String>,
    #[arg(long, default_value_t = 30)]
    rows: u16,
    #[arg(long, default_value_t = 90)]
    cols: u16,
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();
    let args = Args::parse();
    let pane_specs = if args.panes.is_empty() {
        vec!["claude".to_owned(), "hermes --tui".to_owned()]
    } else {
        args.panes
    };
    if pane_specs.len() > MAX_PANES {
        bail!("at most {MAX_PANES} panes may be hosted by the spike");
    }
    let mut panes = Vec::with_capacity(pane_specs.len());
    let signal = update_signal();
    for (index, spec) in pane_specs.iter().enumerate() {
        let words = shell_words::split(spec).with_context(|| format!("parse pane: {spec}"))?;
        let Some((program, program_args)) = words.split_first() else {
            bail!("pane command cannot be empty");
        };
        let id = format!("pane-{}", index + 1);
        info!(%id, command = %spec, "spawning pane");
        panes.push(HostedPane::spawn_with_signal(
            id,
            program.clone(),
            program,
            program_args,
            &args.cwd,
            args.rows,
            args.cols,
            signal.clone(),
        )?);
    }
    serve(&args.socket, Arc::new(Daemon::new(panes, signal)))?;
    Ok(())
}
