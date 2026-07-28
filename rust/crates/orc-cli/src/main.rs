mod daemon;

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use orc_core::adapter::summarize_registry;
use orc_core::bench::{
    HarnessConfig, create_session, list_sessions, load_harness_registry, write_harness_registry,
};
use orc_core::contract::{TaskBudget, TaskContract, TaskLimits, render_brief};
use orc_core::control::{self, LaunchOptions};
use orc_core::discovery;
use orc_core::harness_models::{self, ModelProbe};
use orc_core::metrics::{brain_usage, delegated_value, worker_stats};
use orc_core::orch::{
    self, AwaitRequest, DelegateRequest, OrchActor, OrchOutcome, PlanRequest, StatusRequest,
    TaskRef,
};
use orc_core::probe::{self, Capability, DoctorOptions};
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
#[command(name = "pio", about = "pi-orchestra: MiniMax M3 worker delegation")]
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
    #[command(name = "_dispatch_exec", hide = true)]
    _DispatchExec {
        /// Private detached-supervisor specification.
        spec: PathBuf,
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
    /// Dispatch one bounded command through a configured worker harness.
    Dispatch {
        #[command(subcommand)]
        command: DispatchCommand,
    },
    /// Show verified adapter capabilities and explicit degradations.
    Adapter {
        #[command(subcommand)]
        command: AdapterCommand,
    },
    /// Discover known coding harnesses on PATH and show the registry.
    Harness {
        #[command(subcommand)]
        command: HarnessCommand,
    },
    /// Probe each harness's capabilities and print an honest capability report.
    Doctor {
        #[arg(long)]
        json: bool,
        /// Re-probe every available harness, ignoring the binary-identity cache.
        #[arg(long)]
        refresh: bool,
    },
    /// Maintain the durable session task board.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Inspect or restart the per-user piod daemon.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Normalized conductor control surface: plan, delegate, status, await,
    /// review, cancel, finish (the same seven tools the MCP server exposes).
    Orch {
        #[command(subcommand)]
        command: OrchCommand,
    },
    /// MCP (stdio) server integration: print client config or serve the tools.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Create and list durable headless sessions for delegation.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    /// Report daemon liveness, pid, build, socket, and live pane count.
    ///
    /// Exit codes: 0 running on this build, 3 not running, 5 build mismatch.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Stop and relaunch piod on the installed build.
    ///
    /// Refuses while live panes exist unless --force, because daemon-owned
    /// PTYs die with the daemon.
    Restart {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
enum OrchCommand {
    /// Record a contracted backlog task without delegating it (orch_plan).
    Plan {
        title: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',')]
        depends_on: Vec<String>,
        #[arg(long)]
        isolate: bool,
        #[command(flatten)]
        contract: Box<ContractArgs>,
        #[arg(long, value_enum, default_value = "brain")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Delegate a task and return after delivery while the worker runs (orch_delegate).
    ///
    /// Names an existing --task, or creates one inline from --title plus the
    /// contract flags. The worker prompt defaults to the rendered task brief.
    Delegate {
        /// Worker harness registry key, e.g. `hermes`.
        harness: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',')]
        depends_on: Vec<String>,
        #[arg(long)]
        isolate: bool,
        #[command(flatten)]
        contract: Box<ContractArgs>,
        /// Override the delivered prompt (defaults to the rendered brief).
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long)]
        pane: Option<String>,
        #[arg(long)]
        run: Option<String>,
        /// Seconds the background worker may run before the supervisor kills it.
        ///
        /// THIS is the delivery bound, distinct from the contract's `--timeout`
        /// (which is metadata only). Defaults to the harness's
        /// `dispatch_timeout_sec`, or 120s when that is 0. This does not make
        /// delegate block; use `pio orch await` when you want to wait.
        #[arg(long = "dispatch-timeout")]
        dispatch_timeout: Option<u64>,
        #[arg(long, value_enum, default_value = "brain")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Read one task (or the whole board) with its dispatches (orch_status).
    Status {
        /// Task id to inspect; omit for the whole board.
        task: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Block until a task's newest delivery is terminal (orch_await).
    Await {
        task: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        timeout: Option<u64>,
        #[arg(long)]
        poll_interval_ms: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    /// Move a running task into review (orch_review).
    Review {
        task: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "brain")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Drop a task and stop a linked live worker best-effort (orch_cancel).
    Cancel {
        task: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "brain")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// Mark a reviewed task done (orch_finish).
    Finish {
        task: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "brain")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Debug, ValueEnum)]
enum McpFormat {
    /// Claude Code `.mcp.json` object.
    Claude,
    /// Codex `config.toml` block.
    Codex,
}

#[derive(Debug, Subcommand)]
enum McpCommand {
    /// Print MCP client registration snippets; never edits protected config.
    ///
    /// With no --format, prints both the Claude Code and Codex snippets under
    /// commented headers. Pass --format to emit a single machine-parseable block.
    PrintConfig {
        #[arg(long, value_enum)]
        format: Option<McpFormat>,
    },
    /// Serve the seven orch_* tools over stdio (execs the pio-mcp binary).
    Serve,
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    /// Create a durable headless session (brain, workers, working directory).
    Create {
        /// Brain harness key for the session.
        #[arg(long, default_value = "claude")]
        brain: String,
        /// A worker harness key. Repeat per worker.
        #[arg(long = "worker")]
        workers: Vec<String>,
        /// Session working directory (defaults to the current directory).
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// List durable sessions.
    List {
        #[arg(long)]
        json: bool,
    },
}

/// Acceptance-driven contract flags shared by task-creating commands.
///
/// Grouped into one `clap::Args` block so the flags stay cohesive and the
/// contract-building logic lives in one testable place rather than sprawled
/// across a command handler.
#[derive(Debug, Default, clap::Args)]
struct ContractArgs {
    /// Contract objective: what exists when this task is done.
    #[arg(long)]
    objective: Option<String>,
    /// A file or directory this task may modify. Repeat per path.
    #[arg(long = "allowed")]
    allowed: Vec<String>,
    /// A forbidden action or no-go zone. Repeat per rule.
    #[arg(long = "forbidden")]
    forbidden: Vec<String>,
    /// One acceptance check. Repeat per check.
    #[arg(long = "check")]
    check: Vec<String>,
    /// Expected artifact (branch, doc, measurement).
    #[arg(long)]
    artifact: Option<String>,
    /// Reviewer that will judge the delivered artifact.
    #[arg(long)]
    reviewer: Option<String>,
    /// Contract metadata: the per-attempt limit recorded on the task.
    ///
    /// This does NOT bound delivery. A worker that runs longer than this is not
    /// killed by it — pass `--dispatch-timeout` to bound the actual invocation
    /// (issue #28: `--timeout 300` was mistaken for the delivery bound, and the
    /// dispatch still used the 120s default).
    #[arg(long)]
    timeout: Option<u64>,
    /// Maximum automatic retries after a failed attempt.
    #[arg(long)]
    max_retries: Option<u32>,
    /// Maximum worker tokens the task may spend.
    #[arg(long)]
    max_tokens: Option<u64>,
    /// Maximum spend in whole US cents.
    #[arg(long)]
    max_usd_cents: Option<u64>,
}

impl ContractArgs {
    /// Build an optional contract from the flags, returning `None` when no
    /// contract field was supplied so an uncontracted task stays uncontracted.
    fn into_contract(self) -> Option<TaskContract> {
        let contract = TaskContract {
            objective: self.objective.unwrap_or_default(),
            allowed_paths: self.allowed,
            forbidden: self.forbidden,
            expected_artifact: self.artifact,
            acceptance_checks: self.check,
            reviewer: self.reviewer,
            limits: TaskLimits {
                timeout_sec: self.timeout,
                max_retries: self.max_retries,
                ..TaskLimits::default()
            },
            budget: TaskBudget {
                max_tokens: self.max_tokens,
                max_usd_cents: self.max_usd_cents,
                ..TaskBudget::default()
            },
            ..TaskContract::default()
        };
        (!contract.is_empty()).then_some(contract)
    }
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
        #[command(flatten)]
        contract: Box<ContractArgs>,
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
    /// Print the dispatch brief a worker receives for one task.
    Brief {
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

#[derive(Debug, Subcommand)]
enum DispatchCommand {
    /// Dispatch one bounded command to a configured worker harness.
    Send {
        /// Stable task identifier in the same session.
        task: String,
        /// Worker harness key, e.g. `hermes`.
        harness: String,
        /// Bounded prompt body delivered to the harness.
        prompt: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        pane: Option<String>,
        #[arg(long)]
        run: Option<String>,
        #[arg(long)]
        timeout: Option<u64>,
        #[arg(long, value_enum, default_value = "brain")]
        actor: TaskActorArg,
        #[arg(long)]
        json: bool,
    },
    /// List durable dispatch records for one session.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Re-attempt queued dispatches whose harness now has a free slot.
    Drain {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show one durable dispatch record.
    Show {
        id: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AdapterCommand {
    /// List configured harness capabilities without invoking a provider.
    List {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum HarnessCommand {
    /// Scan PATH for known harnesses, persist discovery, and list all of them.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Set (or clear) a harness's concurrent-worker cap in the registry.
    ///
    /// The cap bounds how many workers of this harness run at once, protecting
    /// the provider's rate/seat limits across every session (issue #7). Pass
    /// `--clear` to remove the override and fall back to the adapter default.
    Cap {
        /// Harness registry key, e.g. `hermes`.
        harness: String,
        /// Maximum concurrent workers (omit with --clear to reset).
        max: Option<usize>,
        /// Remove the override and use the adapter default instead.
        #[arg(long)]
        clear: bool,
        #[arg(long)]
        json: bool,
    },
    /// Register a new named harness profile, e.g. another model of `pi`.
    ///
    /// Copies `command`/`adapter`/`roles`/`resume_args`/`dispatch_args`/
    /// `dispatch_uses_stdin`/`dispatch_timeout_sec` from `--like`, then sets
    /// `--provider`/`--model` as the new profile's `args` — supported for
    /// the `pi` adapter today. `--list-models` probes and prints without
    /// registering anything.
    Add {
        /// New registry key, e.g. `pi-claude`. Required unless --list-models.
        key: Option<String>,
        /// Existing registry key to copy command/adapter/roles from.
        #[arg(long)]
        like: String,
        /// Provider name, validated against the harness's own model list
        /// when one can be probed.
        #[arg(long)]
        provider: Option<String>,
        /// Model name, validated the same way as --provider.
        #[arg(long)]
        model: Option<String>,
        /// Probe and print the harness's available models; register nothing.
        #[arg(long)]
        list_models: bool,
        #[arg(long)]
        json: bool,
    },
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

/// Render a compact human-readable view of an attached contract under `show`.
fn print_contract(contract: &TaskContract) {
    if !contract.objective.is_empty() {
        println!("  objective: {}", contract.objective);
    }
    for path in &contract.allowed_paths {
        println!("  allowed:   {path}");
    }
    for rule in &contract.forbidden {
        println!("  forbidden: {rule}");
    }
    if let Some(artifact) = &contract.expected_artifact {
        println!("  artifact:  {artifact}");
    }
    for (index, check) in contract.acceptance_checks.iter().enumerate() {
        println!("  check {}:   {check}", index + 1);
    }
    if let Some(reviewer) = &contract.reviewer {
        println!("  reviewer:  {reviewer}");
    }
    if let Some(timeout) = contract.limits.timeout_sec {
        println!("  timeout:   {timeout}s");
    }
    if let Some(retries) = contract.limits.max_retries {
        println!("  retries:   {retries}");
    }
    if let Some(tokens) = contract.budget.max_tokens {
        println!("  tokens:    {tokens}");
    }
    if let Some(cents) = contract.budget.max_usd_cents {
        println!("  budget:    ${}.{:02}", cents / 100, cents % 100);
    }
}

fn print_report(report: &orc_core::tasks::TaskReportLink) {
    println!(
        "  report:    {} · executor {} · reviewer {}",
        report.review_mode, report.executor, report.reviewer
    );
    for (index, verdict) in report.verdicts.iter().enumerate() {
        println!(
            "  verdict {}: {} · {}",
            index + 1,
            verdict.verdict,
            verdict.check
        );
        println!("    evidence: {}", verdict.evidence);
    }
    if let Some(tokens) = report.tokens_total {
        println!("  usage:     {tokens} tokens");
    }
    if let Some(cost) = &report.cost_usd {
        println!("  cost:      ${cost}");
    }
    println!("  receipt:   {}", report.path);
}

fn dispatch_task(command: TaskCommand) -> Result<i32> {
    match command {
        TaskCommand::Add {
            title,
            description,
            depends_on,
            isolate,
            contract,
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
                    contract: contract.into_contract(),
                },
            )?;
            print_task(&task, json)?;
        }
        TaskCommand::List { session, json } => {
            let tasks = tasks::list_tasks(&task_session(session)?)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&tasks)?);
            } else if tasks.is_empty() {
                println!("no tasks yet — try: pio task add \"first task\" --session <session>");
            } else {
                for task in tasks {
                    print_task(&task, false)?;
                }
            }
        }
        TaskCommand::Show { id, session, json } => {
            let task = tasks::read_task(&task_session(session)?, &id)?;
            print_task(&task, json)?;
            if !json && let Some(contract) = &task.contract {
                print_contract(contract);
            }
            if !json && let Some(report) = &task.report {
                print_report(report);
            }
        }
        TaskCommand::Brief { id, session, json } => {
            let task = tasks::read_task(&task_session(session)?, &id)?;
            let brief = render_brief(&task);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": task.id,
                        "session": task.session,
                        "brief": brief,
                    }))?
                );
            } else {
                print!("{brief}");
            }
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

fn print_dispatch(record: &orc_core::dispatch::DispatchRecord, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(record)?);
    } else {
        let state = record.execution_status.as_deref().map_or_else(
            || record.status.clone(),
            |run| format!("{}/{run}", record.status),
        );
        println!(
            "{}  {:<19}  {:<6}  {:<10}  task={}",
            record.id, state, record.actor, record.harness, record.task,
        );
        if let Some(error) = &record.error {
            println!("    error: {error}");
        }
    }
    Ok(())
}

fn dispatch_session(explicit: Option<String>) -> Result<String> {
    explicit
        .or_else(|| std::env::var("ORC_SESSION").ok())
        .filter(|session| !session.is_empty())
        .ok_or_else(|| anyhow!("dispatch session is required; pass --session or set ORC_SESSION"))
}

fn dispatch_dispatch(command: DispatchCommand) -> Result<i32> {
    match command {
        DispatchCommand::Send {
            task,
            harness,
            prompt,
            session,
            pane,
            run,
            timeout,
            actor,
            json,
        } => {
            let session = dispatch_session(session)?;
            let record = orc_core::dispatch::dispatch(&orc_core::dispatch::DispatchRequest {
                session,
                task,
                actor: orc_core::dispatch::DispatchActor::from(orc_core::tasks::TaskActor::from(
                    actor,
                )),
                harness,
                pane_id: pane,
                run,
                prompt,
                timeout_sec: timeout,
            })?;
            print_dispatch(&record, json)?;
            // ORC WARNING channel: rate-limit backoff and concurrency-cap
            // notices surface on stderr (issue #7), matching the quota guard.
            for warning in &record.warnings {
                eprintln!("{warning}");
            }
            Ok(dispatch_exit(&record))
        }
        DispatchCommand::List { session, json } => {
            let session = dispatch_session(session)?;
            let records = orc_core::dispatch::list_dispatches(&session)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&records)?);
            } else if records.is_empty() {
                println!(
                    "no dispatches yet — try: pio dispatch send <task> <harness> <prompt> --session <session>"
                );
            } else {
                for record in records {
                    print_dispatch(&record, false)?;
                }
            }
            Ok(0)
        }
        DispatchCommand::Show { id, session, json } => {
            let session = dispatch_session(session)?;
            let record = orc_core::dispatch::read_dispatch(&session, &id)?;
            print_dispatch(&record, json)?;
            Ok(if record.is_confirmed() { 0 } else { 1 })
        }
        DispatchCommand::Drain { session, json } => {
            let session = dispatch_session(session)?;
            let drained = orc_core::dispatch::drain_queued(&session)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&drained)?);
            } else if drained.is_empty() {
                println!("no queued dispatches ready to run");
            } else {
                for record in &drained {
                    print_dispatch(record, false)?;
                    for warning in &record.warnings {
                        eprintln!("{warning}");
                    }
                }
            }
            Ok(0)
        }
    }
}

/// Exit code for one `dispatch send`: 0 confirmed, 75 queued (EX_TEMPFAIL —
/// recorded but not spawned, drain when a slot frees), 1 otherwise.
fn dispatch_exit(record: &orc_core::dispatch::DispatchRecord) -> i32 {
    if record.is_confirmed() {
        0
    } else if record.is_queued() {
        75
    } else {
        1
    }
}

fn dispatch_adapter(command: AdapterCommand) -> Result<i32> {
    match command {
        AdapterCommand::List { json } => {
            let adapters = summarize_registry(&load_harness_registry()?);
            if json {
                println!("{}", serde_json::to_string_pretty(&adapters)?);
            } else {
                for adapter in adapters {
                    let executable = adapter.executable.as_deref().unwrap_or("unavailable");
                    println!(
                        "{:<10} pane={} dispatch={} steer={} exact_usage={} executable={}",
                        adapter.harness,
                        adapter.interactive_pane,
                        adapter.headless_delivery,
                        adapter.steerable,
                        adapter.exact_usage,
                        executable,
                    );
                    println!("    {}", adapter.degradation);
                }
            }
            Ok(0)
        }
    }
}

fn dispatch_harness(command: HarnessCommand) -> Result<i32> {
    match command {
        HarnessCommand::List { json } => {
            let harnesses = discovery::discover(true)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&harnesses)?);
            } else {
                for harness in &harnesses {
                    if harness.available {
                        println!(
                            "{:<10} on PATH · available     {}",
                            harness.name,
                            harness.path.as_deref().unwrap_or("?"),
                        );
                        println!(
                            "    {}",
                            harness.version.as_deref().unwrap_or("version unknown")
                        );
                    } else {
                        println!("{:<10} NOT ON PATH · unavailable", harness.name);
                    }
                }
                print_registered_profiles()?;
            }
            Ok(0)
        }
        HarnessCommand::Cap {
            harness,
            max,
            clear,
            json,
        } => {
            let mut registry = load_harness_registry()?;
            if !registry.harnesses.contains_key(&harness) {
                anyhow::bail!("unknown harness: {harness}");
            }
            if clear {
                registry.concurrency.remove(&harness);
            } else {
                let max = max.ok_or_else(|| {
                    anyhow!("provide a max (e.g. `pio harness cap {harness} 2`) or --clear")
                })?;
                if max == 0 {
                    anyhow::bail!(
                        "concurrency cap must be at least 1; use --clear for the default"
                    );
                }
                registry.concurrency.insert(harness.clone(), max);
            }
            write_harness_registry(&registry)?;
            let effective = orc_core::spawn_guard::effective_cap(&registry, &harness);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "harness": harness,
                        "override": registry.concurrency.get(&harness),
                        "effective_cap": effective,
                    }))?
                );
            } else {
                match registry.concurrency.get(&harness) {
                    Some(cap) => println!("{harness} concurrency cap set to {cap}"),
                    None => println!(
                        "{harness} concurrency override cleared; using adapter default {effective}"
                    ),
                }
            }
            Ok(0)
        }
        HarnessCommand::Add {
            key,
            like,
            provider,
            model,
            list_models,
            json,
        } => {
            let mut registry = load_harness_registry()?;
            let source = registry
                .harnesses
                .get(&like)
                .cloned()
                .with_context(|| format!("unknown harness: {like}"))?;

            if list_models {
                let probe = harness_models::probe(&source.adapter, &source.command);
                return print_model_probe(&like, &probe, json);
            }

            let key =
                key.ok_or_else(|| anyhow!("harness add requires <key> unless --list-models"))?;
            let provider = provider.ok_or_else(|| {
                anyhow!("harness add requires --provider (use --list-models to see choices)")
            })?;
            let model = model.ok_or_else(|| {
                anyhow!("harness add requires --model (use --list-models to see choices)")
            })?;

            if source.adapter != "pi" {
                anyhow::bail!(
                    "harness add only supports provider/model profiles for the 'pi' adapter \
                     today; {like} uses adapter '{}'",
                    source.adapter
                );
            }

            if let ModelProbe::Models(models) =
                harness_models::probe(&source.adapter, &source.command)
                && !models.iter().any(|(p, m)| *p == provider && *m == model)
            {
                let choices = models
                    .iter()
                    .map(|(p, m)| format!("{p}/{m}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow::bail!(
                    "'{provider}/{model}' is not a model {like} can run; valid choices: {choices}"
                );
            }

            let mut config = source;
            config.args = vec![
                "--provider".to_owned(),
                provider.clone(),
                "--model".to_owned(),
                model.clone(),
            ];
            registry.harnesses.insert(key.clone(), config);
            write_harness_registry(&registry)?;

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "key": key,
                        "like": like,
                        "provider": provider,
                        "model": model,
                    }))?
                );
            } else {
                println!("registered {key} (like {like}) \u{b7} {provider}/{model}");
            }
            Ok(0)
        }
    }
}

/// Print every registered harness profile with its configured provider/model
/// when one is set (currently only the `pi` adapter's `--provider <p>
/// --model <m>` shape, written by `harness add` and the `pi-m3` default).
fn print_registered_profiles() -> Result<()> {
    let registry = load_harness_registry()?;
    if registry.harnesses.is_empty() {
        return Ok(());
    }
    println!("\nregistered profiles:");
    for (key, config) in &registry.harnesses {
        let roles = config.roles.join("+");
        match provider_model_args(config) {
            Some((provider, model)) => {
                println!(
                    "  {key:<12} {} ({roles}) \u{b7} {provider}/{model}",
                    config.command
                );
            }
            None => {
                println!("  {key:<12} {} ({roles})", config.command);
            }
        }
    }
    Ok(())
}

/// Extract `(provider, model)` from a profile's `args` when they carry the
/// `--provider <p> --model <m>` shape this module writes.
fn provider_model_args(config: &HarnessConfig) -> Option<(&str, &str)> {
    let mut iter = config.args.iter();
    let mut provider = None;
    let mut model = None;
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--provider" => provider = iter.next().map(String::as_str),
            "--model" => model = iter.next().map(String::as_str),
            _ => {}
        }
    }
    provider.zip(model)
}

fn print_model_probe(key: &str, probe: &ModelProbe, json: bool) -> Result<i32> {
    match probe {
        ModelProbe::Models(models) => {
            if json {
                let rows: Vec<_> = models
                    .iter()
                    .map(|(provider, model)| {
                        serde_json::json!({"provider": provider, "model": model})
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                for (provider, model) in models {
                    println!("{provider}/{model}");
                }
            }
            Ok(0)
        }
        ModelProbe::NoProber => {
            eprintln!(
                "no automatic model listing for {key}; run its own model-listing command \
                 yourself (e.g. its --help), then pass --provider/--model to `harness add`"
            );
            Ok(1)
        }
        ModelProbe::Failed(reason) => {
            eprintln!(
                "could not probe {key}'s models ({reason}); pass --provider/--model manually"
            );
            Ok(1)
        }
    }
}

fn dispatch_doctor(json: bool, refresh: bool) -> Result<i32> {
    let reports = probe::doctor(&DoctorOptions { refresh })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
        return Ok(0);
    }
    let width = reports
        .iter()
        .map(|report| report.display.chars().count())
        .max()
        .unwrap_or(0)
        .max(11);
    // Spec report table: display, status, role, one-line capability summary.
    for report in &reports {
        if report.available {
            println!(
                "{:<width$}  installed     {:<11}  {}",
                report.display, report.role, report.summary,
            );
        } else {
            println!("{:<width$}  unavailable", report.display);
        }
    }
    // Capability matrix: every probed capability, glyph + label, never hidden.
    println!("\nCAPABILITIES (\u{2713} probed available \u{b7} \u{2717} not advertised)");
    print!("{:<width$} ", "");
    for capability in Capability::ALL {
        print!(" {:<7}", capability.label());
    }
    println!();
    for report in &reports {
        print!("{:<width$} ", report.display);
        for capability in Capability::ALL {
            let mark = if !report.available {
                "\u{2013}" // en dash: not applicable (harness unavailable)
            } else if report.capabilities.get(capability.slug()).copied() == Some(true) {
                "\u{2713}" // check
            } else {
                "\u{2717}" // cross
            };
            print!(" {mark:<7}");
        }
        println!();
    }
    Ok(0)
}

fn orch_actor(actor: TaskActorArg) -> OrchActor {
    OrchActor::from(TaskActor::from(actor))
}

/// Render one orch outcome: pretty JSON under --json, else a compact summary
/// pairing each task's id, status, and title with any dispatch and note lines.
fn print_outcome(outcome: &OrchOutcome, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(outcome)?);
        return Ok(());
    }
    for task in &outcome.tasks {
        println!("{}  {:<9}  {}", task.id, task.status, task.title);
    }
    for record in &outcome.dispatches {
        let state = record.execution_status.as_deref().map_or_else(
            || record.status.clone(),
            |run| format!("{}/{run}", record.status),
        );
        println!(
            "dispatch {}  {:<19}  {}  task={}",
            record.id, state, record.harness, record.task
        );
    }
    if let Some(note) = &outcome.note {
        println!("note: {note}");
    }
    Ok(())
}

fn dispatch_orch(command: OrchCommand) -> Result<i32> {
    match command {
        OrchCommand::Plan {
            title,
            session,
            description,
            depends_on,
            isolate,
            contract,
            actor,
            json,
        } => {
            let outcome = orch::plan(PlanRequest {
                session: task_session(session)?,
                title,
                description,
                depends_on,
                isolate,
                contract: contract.into_contract(),
                actor: orch_actor(actor),
            })?;
            print_outcome(&outcome, json)?;
            Ok(0)
        }
        OrchCommand::Delegate {
            harness,
            session,
            task,
            title,
            description,
            depends_on,
            isolate,
            contract,
            prompt,
            pane,
            run,
            dispatch_timeout,
            actor,
            json,
        } => {
            let outcome = orch::delegate(DelegateRequest {
                session: task_session(session)?,
                harness,
                task,
                title,
                description,
                depends_on,
                isolate,
                contract: contract.into_contract(),
                prompt,
                pane,
                run,
                timeout_sec: dispatch_timeout,
                actor: orch_actor(actor),
            })?;
            // Mirror `dispatch send`: the exit code reflects the delivery, and
            // rate-limit/concurrency notices surface on the ORC WARNING channel.
            let exit = outcome.dispatches.first().map_or(0, dispatch_exit);
            print_outcome(&outcome, json)?;
            for record in &outcome.dispatches {
                for warning in &record.warnings {
                    eprintln!("{warning}");
                }
            }
            Ok(exit)
        }
        OrchCommand::Status {
            task,
            session,
            json,
        } => {
            let outcome = orch::status(StatusRequest {
                session: task_session(session)?,
                task,
            })?;
            print_outcome(&outcome, json)?;
            Ok(0)
        }
        OrchCommand::Await {
            task,
            session,
            timeout,
            poll_interval_ms,
            json,
        } => {
            let outcome = orch::await_delegation(AwaitRequest {
                session: task_session(session)?,
                task,
                timeout_sec: timeout,
                poll_interval_ms,
            })?;
            // A note means the wait timed out before a terminal delivery; exit
            // 75 (EX_TEMPFAIL) so scripts can retry without treating it as fatal.
            let timed_out = outcome.note.is_some();
            let worker_exit = outcome
                .dispatches
                .first()
                .filter(|record| record.is_terminal())
                .and_then(|record| record.exit_code)
                .filter(|code| *code != 0)
                .unwrap_or(0);
            print_outcome(&outcome, json)?;
            Ok(if timed_out { 75 } else { worker_exit })
        }
        OrchCommand::Review {
            task,
            session,
            actor,
            json,
        } => {
            let outcome = orch::review(TaskRef {
                session: task_session(session)?,
                task,
                actor: orch_actor(actor),
            })?;
            print_outcome(&outcome, json)?;
            Ok(0)
        }
        OrchCommand::Cancel {
            task,
            session,
            actor,
            json,
        } => {
            let outcome = orch::cancel(TaskRef {
                session: task_session(session)?,
                task,
                actor: orch_actor(actor),
            })?;
            print_outcome(&outcome, json)?;
            Ok(0)
        }
        OrchCommand::Finish {
            task,
            session,
            actor,
            json,
        } => {
            let outcome = orch::finish(TaskRef {
                session: task_session(session)?,
                task,
                actor: orch_actor(actor),
            })?;
            print_outcome(&outcome, json)?;
            Ok(0)
        }
    }
}

/// Resolve the `pio-mcp` server command: the sibling binary beside this `pio`
/// when present (an installed pair), else the bare name for a PATH lookup.
fn mcp_server_command() -> String {
    std::env::current_exe()
        .ok()
        .map(|path| path.with_file_name(orch::MCP_SERVER_BIN))
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| orch::MCP_SERVER_BIN.to_owned())
}

fn dispatch_mcp(command: McpCommand) -> Result<i32> {
    match command {
        McpCommand::PrintConfig { format } => {
            let server = mcp_server_command();
            match format {
                Some(McpFormat::Claude) => println!("{}", orch::claude_mcp_json(&server)),
                Some(McpFormat::Codex) => print!("{}", orch::codex_mcp_toml(&server)),
                None => {
                    println!(
                        "# Claude Code \u{2014} add to .mcp.json (project) or ~/.claude.json:"
                    );
                    println!("{}", orch::claude_mcp_json(&server));
                    println!();
                    println!("# Codex \u{2014} add to ~/.codex/config.toml:");
                    print!("{}", orch::codex_mcp_toml(&server));
                }
            }
            Ok(0)
        }
        McpCommand::Serve => {
            let status = Command::new(mcp_server_command())
                .status()
                .context("exec pio-mcp")?;
            Ok(status.code().unwrap_or(1))
        }
    }
}

fn dispatch_session_command(command: SessionCommand) -> Result<i32> {
    match command {
        SessionCommand::Create {
            brain,
            workers,
            cwd,
            json,
        } => {
            let cwd = match cwd {
                Some(cwd) => cwd,
                None => std::env::current_dir().context("resolve current directory")?,
            };
            let session = create_session(&brain, &workers, &cwd)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&session)?);
            } else {
                println!("{}", session.id);
            }
            Ok(0)
        }
        SessionCommand::List { json } => {
            let sessions = list_sessions()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&sessions)?);
            } else if sessions.is_empty() {
                println!(
                    "no sessions yet \u{2014} try: pio session create --brain claude --worker <harness>"
                );
            } else {
                for session in sessions {
                    println!(
                        "{}  brain={}  workers={}  {}",
                        session.id,
                        session.brain,
                        session.workers.join(","),
                        session.cwd
                    );
                }
            }
            Ok(0)
        }
    }
}

fn dispatch(command: Commands) -> Result<i32> {
    match command {
        Commands::Version => {
            println!("pio {}", orc_proto::BUILD_IDENTIFIER);
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
        Commands::_DispatchExec { spec } => orc_core::dispatch_supervisor::execute(&spec),
        Commands::List { json } => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&control::runs_as_json(true)?)?
                );
            } else {
                let runs = list_runs(true)?;
                if runs.is_empty() {
                    println!("no runs yet — try: pio run \"hello\"");
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
                    .ok_or_else(|| anyhow!("pio: config key '{key}' is not set"))?,
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
            let current = std::env::current_exe().context("locate pio binary")?;
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
        Commands::Dispatch { command } => dispatch_dispatch(command),
        Commands::Adapter { command } => dispatch_adapter(command),
        Commands::Harness { command } => dispatch_harness(command),
        Commands::Doctor { json, refresh } => dispatch_doctor(json, refresh),
        Commands::Task { command } => dispatch_task(command),
        Commands::Daemon { command } => match command {
            DaemonCommand::Status { json } => daemon::status(json),
            DaemonCommand::Restart { force } => daemon::restart(force),
        },
        Commands::Orch { command } => dispatch_orch(command),
        Commands::Mcp { command } => dispatch_mcp(command),
        Commands::Session { command } => dispatch_session_command(command),
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
