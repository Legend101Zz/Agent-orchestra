use std::path::PathBuf;
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use orc_core::control::{self, LaunchOptions};
use orc_core::metrics::{brain_usage, delegated_value, worker_stats};
use orc_core::quota;
use orc_core::registry::list_runs;
use orc_core::runner::Mode;
use orc_core::tasks::{self, NewTask, TaskActor, TaskStatus};

#[derive(Clone, Debug, ValueEnum)]
enum Brain {
    Claude,
    Codex,
    Human,
}

impl Brain {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Human => "human",
        }
    }
}

#[derive(Clone, Debug, ValueEnum)]
enum TaskActorArg {
    Brain,
    Human,
}

impl From<TaskActorArg> for TaskActor {
    fn from(value: TaskActorArg) -> Self {
        match value {
            TaskActorArg::Brain => Self::Brain,
            TaskActorArg::Human => Self::Human,
        }
    }
}

#[derive(Clone, Debug, ValueEnum)]
enum TaskStatusArg {
    Backlog,
    Assigned,
    Running,
    Review,
    Done,
    Dropped,
}

impl From<TaskStatusArg> for TaskStatus {
    fn from(value: TaskStatusArg) -> Self {
        match value {
            TaskStatusArg::Backlog => Self::Backlog,
            TaskStatusArg::Assigned => Self::Assigned,
            TaskStatusArg::Running => Self::Running,
            TaskStatusArg::Review => Self::Review,
            TaskStatusArg::Done => Self::Done,
            TaskStatusArg::Dropped => Self::Dropped,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "orc", about = "pi-orchestra: MiniMax M3 worker delegation")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print the Rust CLI version.
    Version,
    /// Run one registered JSON-mode worker.
    Run {
        task: String,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "human")]
        brain: Brain,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        bg: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        idle_timeout: Option<f64>,
        #[arg(long)]
        brain_model: Option<String>,
    },
    /// Run one registered streaming RPC worker.
    Rpc {
        task: String,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "human")]
        brain: Brain,
        #[arg(long)]
        bg: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        idle_timeout: Option<f64>,
        #[arg(long)]
        brain_model: Option<String>,
    },
    #[command(name = "_exec", hide = true)]
    _Exec {
        run_dir: PathBuf,
        #[arg(long)]
        echo: bool,
        #[arg(long)]
        idle_timeout: Option<f64>,
    },
    /// List registry runs, reconciling dead worker PIDs.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show metadata and the tail of one run.
    Show {
        id: String,
        #[arg(long, default_value_t = 40)]
        tail: usize,
    },
    /// Request termination of one running worker.
    Kill { id: String },
    /// Read coding-plan quota and enforce configured thresholds.
    Quota {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        force: bool,
    },
    /// Report worker usage, delegated value, and brain usage.
    Stats {
        #[arg(long)]
        json: bool,
    },
    /// Send one follow-up to a running RPC worker.
    Send { id: String, message: String },
    /// Start a linked retry without changing the source run.
    Retry {
        id: String,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        foreground: bool,
    },
    /// Continue stopped work from a brain-reviewed remaining-work brief.
    Handoff {
        id: String,
        brief: String,
        #[arg(long)]
        foreground: bool,
    },
    /// Read or edit operator-console configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Set a non-blocking advisory budget for one session.
    Budget { session: String, usd: f64 },
    /// Open the Ratatui operator console.
    Top {
        #[arg(long)]
        theme: Option<String>,
    },
    /// Maintain the durable session task board.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    /// Add one backlog task.
    Add {
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',')]
        depends_on: Vec<String>,
        #[arg(long)]
        isolate: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// List parseable tasks without hiding valid siblings when one is corrupt.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show one task and its append-only history.
    Show {
        id: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Assign a task to a worker or pane.
    Assign {
        id: String,
        assignee: String,
        #[arg(long)]
        run: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Mark an assigned task running.
    Start {
        id: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Move a running task to review.
    Review {
        id: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Mark a reviewed task done.
    Done {
        id: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Drop a task while preserving its audit record.
    Drop {
        id: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Move a task through the documented state machine.
    Move {
        id: String,
        status: TaskStatusArg,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Report a worktree diff once isolation has been materialized.
    Diff {
        id: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Explicitly squash-merge one reviewed isolated task.
    Merge {
        id: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    List,
    Get { key: String },
    Set { key: String, value: String },
}

fn quota_exit(level: &str) -> i32 {
    match level {
        "ok" => 0,
        "warn" => 2,
        "block" => 3,
        _ => 4,
    }
}

fn fmt_tokens(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1e6)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1e3)
    } else {
        value.to_string()
    }
}

fn task_session(explicit: Option<String>) -> Result<String> {
    explicit
        .or_else(|| std::env::var("ORC_SESSION").ok())
        .filter(|session| !session.is_empty())
        .ok_or_else(|| anyhow!("task session is required; pass --session or set ORC_SESSION"))
}

fn print_task(task: &orc_core::tasks::Task, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(task)?);
    } else {
        println!("{}  {:<9}  {}", task.id, task.status, task.title);
    }
    Ok(())
}

fn dispatch_task(command: TaskCommand) -> Result<i32> {
    match command {
        TaskCommand::Add {
            title,
            description,
            depends_on,
            isolate,
            session,
            actor,
            json,
        } => {
            let task = tasks::add_task(
                &task_session(session)?,
                actor.into(),
                NewTask {
                    title,
                    description: description.unwrap_or_default(),
                    depends_on,
                    isolate,
                },
            )?;
            print_task(&task, json)?;
        }
        TaskCommand::List { session, json } => {
            let tasks = tasks::list_tasks(&task_session(session)?)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&tasks)?);
            } else if tasks.is_empty() {
                println!("no tasks yet — try: orc task add \"first task\" --session <session>");
            } else {
                for task in tasks {
                    print_task(&task, false)?;
                }
            }
        }
        TaskCommand::Show { id, session, json } => {
            print_task(&tasks::read_task(&task_session(session)?, &id)?, json)?
        }
        TaskCommand::Assign {
            id,
            assignee,
            run,
            session,
            actor,
            json,
        } => print_task(
            &tasks::assign_task(&task_session(session)?, &id, assignee, run, actor.into())?,
            json,
        )?,
        TaskCommand::Start {
            id,
            session,
            actor,
            json,
        } => print_task(
            &tasks::start_task(&task_session(session)?, &id, actor.into())?,
            json,
        )?,
        TaskCommand::Review {
            id,
            session,
            actor,
            json,
        } => print_task(
            &tasks::review_task(&task_session(session)?, &id, actor.into())?,
            json,
        )?,
        TaskCommand::Done {
            id,
            session,
            actor,
            json,
        } => print_task(
            &tasks::done_task(&task_session(session)?, &id, actor.into())?,
            json,
        )?,
        TaskCommand::Drop {
            id,
            session,
            actor,
            json,
        } => print_task(
            &tasks::drop_task(&task_session(session)?, &id, actor.into())?,
            json,
        )?,
        TaskCommand::Move {
            id,
            status,
            session,
            actor,
            json,
        } => print_task(
            &tasks::move_task(&task_session(session)?, &id, status.into(), actor.into())?,
            json,
        )?,
        TaskCommand::Diff { id, session, json } => {
            let diff = tasks::diff_task(&task_session(session)?, &id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&diff)?);
            } else {
                println!(
                    "+{} -{} · {} files",
                    diff.insertions, diff.deletions, diff.files
                );
            }
        }
        TaskCommand::Merge {
            id,
            session,
            actor,
            json,
        } => print_task(
            &tasks::merge_task(&task_session(session)?, &id, actor.into())?,
            json,
        )?,
    }
    Ok(0)
}

fn dispatch(command: Commands) -> Result<i32> {
    match command {
        Commands::Version => {
            println!("orc 0.4.0");
            Ok(0)
        }
        Commands::Run {
            task,
            cwd,
            brain,
            name,
            bg,
            force,
            session,
            idle_timeout,
            brain_model,
        } => {
            let options = LaunchOptions {
                task,
                cwd,
                brain: brain.as_str().to_owned(),
                name,
                session,
                brain_model,
                force,
                idle_timeout,
                background: bg,
                mode: Mode::Json,
                retry_of: None,
                handoff_from: None,
            };
            let (run, code) = control::launch(&options)?;
            if bg {
                println!("{}", run.file_name().unwrap_or_default().to_string_lossy());
            }
            Ok(code)
        }
        Commands::Rpc {
            task,
            cwd,
            brain,
            bg,
            force,
            session,
            idle_timeout,
            brain_model,
        } => {
            let options = LaunchOptions {
                task,
                cwd,
                brain: brain.as_str().to_owned(),
                name: None,
                session,
                brain_model,
                force,
                idle_timeout,
                background: bg,
                mode: Mode::Rpc,
                retry_of: None,
                handoff_from: None,
            };
            let (run, code) = control::launch(&options)?;
            if bg {
                println!("{}", run.file_name().unwrap_or_default().to_string_lossy());
            }
            Ok(code)
        }
        Commands::_Exec {
            run_dir,
            echo,
            idle_timeout,
        } => control::run_hidden(&run_dir, idle_timeout, echo),
        Commands::List { json } => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&control::runs_as_json(true)?)?
                );
            } else {
                let runs = list_runs(true)?;
                if runs.is_empty() {
                    println!("no runs yet — try: orc run \"hello\"");
                } else {
                    println!(
                        "{:<38} {:<6} {:<9} {:<20} TASK",
                        "ID", "BRAIN", "STATUS", "STARTED"
                    );
                    for run in runs {
                        let task = if run.task.chars().count() > 48 {
                            format!("{}…", run.task.chars().take(47).collect::<String>())
                        } else {
                            run.task
                        };
                        println!(
                            "{:<38} {:<6} {:<9} {:<20} {}",
                            run.id.chars().take(38).collect::<String>(),
                            run.brain.chars().take(6).collect::<String>(),
                            run.status,
                            run.started_at.chars().take(19).collect::<String>(),
                            task
                        );
                    }
                }
            }
            Ok(0)
        }
        Commands::Show { id, tail } => {
            let (meta, lines) = control::show(&id, tail)?;
            println!("{}", serde_json::to_string_pretty(&meta)?);
            if !lines.is_empty() {
                println!("\n--- output.log (last {tail} lines) ---");
                for line in lines {
                    println!("{line}");
                }
            }
            Ok(0)
        }
        Commands::Kill { id } => {
            let meta = control::kill(&id)?;
            println!("{}: {}", meta.id, meta.status);
            Ok(if meta.is_terminal() { 0 } else { 1 })
        }
        Commands::Quota { json, force } => {
            let result = quota::get_quota(force);
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if result.level == "unknown" {
                println!(
                    "MiniMax quota: unknown — {}",
                    result.reason.clone().unwrap_or_default()
                );
            } else {
                println!("MiniMax coding-plan quota (general):");
                println!(
                    "  5-hour window : {}% remaining (resets in ~{} min)",
                    result.five_hour_pct.unwrap_or_default(),
                    result.window_resets_in_min.unwrap_or_default()
                );
                println!(
                    "  weekly window : {}% remaining",
                    result.weekly_pct.unwrap_or_default()
                );
                println!(
                    "  level: {}   [source: {}]",
                    result.level,
                    result.source.clone().unwrap_or_else(|| "?".to_owned())
                );
            }
            Ok(quota_exit(&result.level))
        }
        Commands::Stats { json } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&control::stats_json()?)?);
                return Ok(0);
            }
            let runs = list_runs(false)?;
            let workers = worker_stats(&runs);
            let value = delegated_value(&runs);
            let brains = brain_usage();
            println!("WORKERS (registry — exact where pi reported usage)");
            println!("  runs: {}", workers.runs);
            println!(
                "  exact: {} runs · in {} / out {} / cache {} · ${:.4}",
                workers.exact.runs,
                fmt_tokens(workers.exact.input),
                fmt_tokens(workers.exact.output),
                fmt_tokens(workers.exact.cache_read),
                workers.exact.cost_usd
            );
            if workers.estimated.runs > 0 {
                println!(
                    "  estimated (chars/4): {} runs · ~{} tokens",
                    workers.estimated.runs,
                    fmt_tokens(workers.estimated.total)
                );
            }
            println!("\nDELEGATED VALUE (worker tokens priced at brain API rates)");
            println!(
                "  saved ≈ ${:.2}   ({:.1}x cheaper: ${:.2} brain-equivalent vs ${:.4} MiniMax)",
                value.saved_usd, value.multiple, value.brain_equiv_usd, value.worker_cost_usd
            );
            println!(
                "  exact basis: {:.0}% of tokens are exact",
                value.exact_share * 100.0
            );
            println!(
                "\nBRAINS (local session logs — API-equivalent value; subscriptions are flat-rate)"
            );
            for name in ["claude", "codex"] {
                if let Some(usage) = brains.get(name) {
                    println!(
                        "  {name:<6} today in {} / out {} / cache-read {}",
                        fmt_tokens(usage.today.input),
                        fmt_tokens(usage.today.output),
                        fmt_tokens(usage.today.cache_read)
                    );
                } else {
                    println!("  {name:<6} n/a");
                }
            }
            Ok(0)
        }
        Commands::Send { id, message } => {
            let path = control::send(&id, &message)?;
            println!(
                "queued {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            Ok(0)
        }
        Commands::Retry {
            id,
            task,
            foreground,
        } => {
            let (run, code) = control::retry(&id, task, foreground)?;
            if !foreground {
                println!("{}", run.file_name().unwrap_or_default().to_string_lossy());
            }
            Ok(code)
        }
        Commands::Handoff {
            id,
            brief,
            foreground,
        } => {
            let (run, code) = control::handoff(&id, &brief, foreground)?;
            if !foreground {
                println!("{}", run.file_name().unwrap_or_default().to_string_lossy());
            }
            Ok(code)
        }
        Commands::Config { command } => {
            let config = match command {
                ConfigCommand::List => control::read_config_value(),
                ConfigCommand::Get { key } => control::read_config_value()
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| anyhow!("orc: config key '{key}' is not set"))?,
                ConfigCommand::Set { key, value } => control::set_config(&key, &value)?,
            };
            println!("{}", serde_json::to_string_pretty(&config)?);
            Ok(0)
        }
        Commands::Budget { session, usd } => {
            let record = control::set_session_budget(&session, usd)?;
            println!("{}", serde_json::to_string_pretty(&record)?);
            Ok(0)
        }
        Commands::Top { theme } => {
            let current = std::env::current_exe().context("locate orc binary")?;
            let sibling = current.with_file_name("pi-orchestra");
            let executable = if sibling.is_file() {
                sibling
            } else {
                PathBuf::from("pi-orchestra")
            };
            let mut command = Command::new(executable);
            if let Some(theme) = theme {
                command.args(["--theme", &theme]);
            }
            command.arg("runs");
            let status = command.status().context("open pi-orchestra RUNS shell")?;
            Ok(status.code().unwrap_or(1))
        }
        Commands::Task { command } => dispatch_task(command),
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        println!();
        return Ok(1);
    };
    match dispatch(command) {
        Err(error) if error.to_string() == "quota-blocked" => Ok(3),
        other => other,
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1)),
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(1)
        }
    }
}
