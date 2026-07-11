#![warn(missing_docs)]
//! `pi-orchestra` phase-one Bench client spike.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use orc_app::{ThemeName, benchmark, run};

#[derive(Debug, Parser)]
#[command(about = "Attach to the Bench daemon spike")]
struct Args {
    #[arg(long, default_value = "/tmp/orcd-spike.sock")]
    socket: PathBuf,
    #[arg(long, default_value = "ember")]
    theme: String,
    #[arg(long)]
    bench: bool,
    #[arg(long, default_value_t = 1_000)]
    iterations: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.bench {
        let summary = benchmark(&args.socket, args.iterations)?;
        println!(
            "{{\"iterations\":{},\"p50_us\":{},\"p95_us\":{},\"p99_us\":{},\"max_us\":{}}}",
            args.iterations, summary.p50_us, summary.p95_us, summary.p99_us, summary.max_us
        );
        return Ok(());
    }
    run(args.socket, ThemeName::named(&args.theme))?;
    Ok(())
}
