//! The conductor-independent `orch_*` control surface (issue #8).
//!
//! Seven operations — plan, delegate, status, await, review, cancel, finish —
//! are the entire tool surface every conductor gets, whether it drives them as
//! normalized `pio` CLI verbs or as MCP tools. Both surfaces are thin adapters
//! over the functions defined here, so they can never drift (issue #8, AC3):
//! this module is the single place each operation lives.
//!
//! Each operation takes an owned request that derives `serde` plus `schemars`
//! [`JsonSchema`] — the same schema the MCP tool advertises and the same
//! contract-v2 derives introduced in issue #5 — and returns an [`OrchOutcome`]:
//! the durable tasks and dispatch records the operation touched. The functions
//! are **synchronous** and reuse the existing task-board, dispatch, and control
//! primitives verbatim. No tokio leaks in here (per the #16 crate decision), so
//! the async MCP server simply calls them from `spawn_blocking`.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::contract::{TaskContract, render_brief};
use crate::dispatch::{self, DispatchActor, DispatchRecord, DispatchRequest};
use crate::registry::find_run;
use crate::report;
use crate::tasks::{self, NewTask, Task, TaskActor, TaskStatus};

/// Default bound on how long [`await_delegation`] waits for a terminal delivery.
pub const DEFAULT_AWAIT_TIMEOUT_SEC: u64 = 300;
/// Default gap between delivery-state polls in [`await_delegation`].
pub const DEFAULT_AWAIT_POLL_MS: u64 = 500;

/// The MCP server binary clients spawn over stdio. Installed on PATH beside
/// `pio`; a caller may override it with an absolute path.
pub const MCP_SERVER_BIN: &str = "pio-mcp";

/// One of the seven normalized control verbs, in stable spec order.
///
/// This enum is the single source of truth for the surface's shape: the CLI
/// subcommands, the MCP tool names, and the parity tests all derive from
/// [`Verb::ALL`], so a new verb can never appear on one surface only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verb {
    /// Record a contracted task on the board without delegating it yet.
    Plan,
    /// Hand a task to a worker harness: assign, start, and dispatch its brief.
    Delegate,
    /// Read the durable state of one task (or the whole board) and its dispatches.
    Status,
    /// Block until a delegated task's newest delivery reaches a terminal state.
    Await,
    /// Move a running task into review.
    Review,
    /// Drop a task, preserving its audit record, and stop a linked live worker.
    Cancel,
    /// Mark a reviewed task done.
    Finish,
}

impl Verb {
    /// Every verb in stable spec order.
    pub const ALL: [Self; 7] = [
        Self::Plan,
        Self::Delegate,
        Self::Status,
        Self::Await,
        Self::Review,
        Self::Cancel,
        Self::Finish,
    ];

    /// The MCP tool name, e.g. `orch_delegate`.
    #[must_use]
    pub const fn tool_name(self) -> &'static str {
        match self {
            Self::Plan => "orch_plan",
            Self::Delegate => "orch_delegate",
            Self::Status => "orch_status",
            Self::Await => "orch_await",
            Self::Review => "orch_review",
            Self::Cancel => "orch_cancel",
            Self::Finish => "orch_finish",
        }
    }

    /// The `pio orch` CLI verb, e.g. `delegate`.
    #[must_use]
    pub const fn cli_name(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Delegate => "delegate",
            Self::Status => "status",
            Self::Await => "await",
            Self::Review => "review",
            Self::Cancel => "cancel",
            Self::Finish => "finish",
        }
    }

    /// One-line description advertised identically on both surfaces.
    ///
    /// The `#[tool]` attributes in `orc-mcp` repeat these literals (attribute
    /// arguments must be literals); a parity test asserts they stay equal.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Plan => {
                "Record a contracted task on the board (backlog) without delegating it yet."
            }
            Self::Delegate => {
                "Delegate a task to a worker harness: assign it, start it, and return as soon as the brief is delivered while the worker continues in the background. Poll with orch_status or block for output and exit code with orch_await. --dispatch-timeout bounds the background worker (harness dispatch_timeout_sec, 120s when unset); the contract's --timeout is metadata only."
            }
            Self::Status => {
                "Read the durable state of one task (or the whole board) with its dispatch records."
            }
            Self::Await => {
                "Block until a delegated task's newest worker reaches a terminal state, returning its answer, usage, and exit code, or until the await timeout elapses."
            }
            Self::Review => "Move a running task into review.",
            Self::Cancel => {
                "Drop a task, preserving its audit record, and stop a linked live worker best-effort."
            }
            Self::Finish => "Mark a reviewed task done.",
        }
    }
}

/// Who initiated an `orch_*` operation. Defaults to `brain`, since the MCP
/// surface is driven by a conductor.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OrchActor {
    /// A conductor (brain) drove the operation.
    #[default]
    Brain,
    /// A human drove the operation.
    Human,
}

impl From<OrchActor> for TaskActor {
    fn from(value: OrchActor) -> Self {
        match value {
            OrchActor::Brain => Self::Brain,
            OrchActor::Human => Self::Human,
        }
    }
}

impl From<OrchActor> for DispatchActor {
    fn from(value: OrchActor) -> Self {
        match value {
            OrchActor::Brain => Self::Brain,
            OrchActor::Human => Self::Human,
        }
    }
}

impl From<TaskActor> for OrchActor {
    fn from(value: TaskActor) -> Self {
        match value {
            TaskActor::Brain => Self::Brain,
            TaskActor::Human => Self::Human,
        }
    }
}

/// Create one contracted backlog task. Backs `orch_plan`.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct PlanRequest {
    /// Owning Bench session identifier.
    pub session: String,
    /// Card heading for the new task.
    pub title: String,
    /// Optional durable task detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Prerequisite task identifiers in the same session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Whether the task must own an isolated worktree.
    #[serde(default)]
    pub isolate: bool,
    /// Acceptance-driven contract v2 for the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<TaskContract>,
    /// Attribution for the mutation.
    #[serde(default)]
    pub actor: OrchActor,
}

/// Delegate a task to a worker harness. Backs `orch_delegate`.
///
/// When `task` names an existing task it is delegated as-is; otherwise a new
/// task is created inline from `title` (required) plus the optional contract.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct DelegateRequest {
    /// Owning Bench session identifier.
    pub session: String,
    /// Worker harness registry key to dispatch through.
    pub harness: String,
    /// Existing task identifier to delegate. Omit to create one inline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Card heading when creating the task inline (required if `task` is unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional durable task detail for an inline task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Prerequisite task identifiers for an inline task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Whether an inline task must own an isolated worktree.
    #[serde(default)]
    pub isolate: bool,
    /// Acceptance-driven contract v2 for an inline task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<TaskContract>,
    /// Prompt delivered to the worker. Defaults to the rendered task brief.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Optional explicit worker pane linkage recorded for replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    /// Optional explicit run linkage recorded for replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// Optional bounded timeout override in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    /// Attribution for the mutation.
    #[serde(default)]
    pub actor: OrchActor,
}

/// Read task and dispatch state. Backs `orch_status`.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct StatusRequest {
    /// Owning Bench session identifier.
    pub session: String,
    /// One task to inspect. Omit to read the whole board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// Wait for a delegated task's delivery to finish. Backs `orch_await`.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct AwaitRequest {
    /// Owning Bench session identifier.
    pub session: String,
    /// Task whose newest delivery to wait on.
    pub task: String,
    /// Maximum wait in seconds (default 300).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    /// Gap between polls in milliseconds (default 500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_ms: Option<u64>,
}

/// Address one task by session and id for a lifecycle transition. Backs
/// `orch_review`, `orch_cancel`, and `orch_finish`.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct TaskRef {
    /// Owning Bench session identifier.
    pub session: String,
    /// Task identifier to transition.
    pub task: String,
    /// Attribution for the mutation.
    #[serde(default)]
    pub actor: OrchActor,
}

/// The durable records one operation touched, plus an optional plain note.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrchOutcome {
    /// Canonical verb name, e.g. `orch_delegate`.
    pub verb: String,
    /// Tasks the operation created, transitioned, or read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<Task>,
    /// Dispatch records the operation created or observed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dispatches: Vec<DispatchRecord>,
    /// Plain human-readable note when the operation has something to add.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl OrchOutcome {
    fn task_only(verb: Verb, task: Task) -> Self {
        Self {
            verb: verb.tool_name().to_owned(),
            tasks: vec![task],
            dispatches: Vec::new(),
            note: None,
        }
    }
}

/// Record one contracted backlog task without delegating it.
pub fn plan(request: PlanRequest) -> Result<OrchOutcome> {
    let task = tasks::add_task(
        &request.session,
        request.actor.into(),
        NewTask {
            title: request.title,
            description: request.description.unwrap_or_default(),
            depends_on: request.depends_on,
            isolate: request.isolate,
            contract: request.contract,
        },
    )?;
    Ok(OrchOutcome::task_only(Verb::Plan, task))
}

/// Explain delivery and execution state that a conductor must not miss.
///
/// The CLI signals a non-confirmed delivery or failed execution through its
/// exit code. MCP has no such channel, so the same news has to travel in the
/// outcome itself or a conductor reading only the top-level result would take
/// a failure for success. The task is deliberately left in the status the
/// delegation moved it to, so every failure note says so explicitly.
fn delivery_note(record: &DispatchRecord, task: &Task) -> Option<String> {
    if record.is_confirmed() {
        if record.is_running() {
            return Some(format!(
                "dispatch {} delivered; worker is still running. Poll with orch_status or block with orch_await.",
                record.id
            ));
        }
        return matches!(
            record.execution_status.as_deref(),
            Some("failed" | "orphaned")
        )
        .then(|| {
            let exit = record.exit_code.map_or_else(
                || "no exit code".to_owned(),
                |code| format!("exit code {code}"),
            );
            format!(
                "dispatch {} delivered, but worker execution {} ({}, {}): {}. Task {} is left {} — inspect the captured output, then cancel or re-delegate it.",
                record.id,
                record.execution_status.as_deref().unwrap_or("failed"),
                record.failure_kind.as_deref().unwrap_or("execution_failed"),
                exit,
                record
                    .error
                    .as_deref()
                    .unwrap_or("worker execution did not succeed"),
                task.id,
                task.status
            )
        });
    }
    if record.is_queued() {
        // Queued is not a failure: the brief is still going to be delivered, so
        // the advice is to wait, never to clean up.
        return Some(format!(
            "dispatch {} is queued behind {}'s concurrency cap; no worker has the brief yet. Task {} is {}; call orch_await to wait for a free slot.",
            record.id, record.harness, task.id, task.status
        ));
    }
    Some(format!(
        "dispatch {} did not confirm ({}): {}. Task {} is left {} — cancel or re-delegate it.",
        record.id,
        record.failure_kind.as_deref().unwrap_or("delivery_failed"),
        record
            .error
            .as_deref()
            .unwrap_or("worker did not confirm delivery"),
        task.id,
        task.status
    ))
}

/// Report the newest terminal failure for each returned task.
///
/// Dispatch lists are newest-first. Looking up one record per task prevents an
/// old failed attempt from making a later successful retry look unsuccessful.
fn terminal_failure_note(tasks: &[Task], dispatches: &[DispatchRecord]) -> Option<String> {
    let notes = tasks
        .iter()
        .filter_map(|task| {
            dispatches
                .iter()
                .find(|record| record.task == task.id)
                .filter(|record| record.is_terminal())
                .and_then(|record| delivery_note(record, task))
        })
        .collect::<Vec<_>>();
    (!notes.is_empty()).then(|| notes.join("\n"))
}

/// Delegate a task to a worker harness: assign, start, and dispatch its brief.
///
/// The prompt defaults to the task's rendered contract brief (folding in the
/// `render_brief` wiring flagged by issue #5's review), so a delegated worker
/// always receives the acceptance criteria unless the caller overrides it.
///
/// A delivery that fails or is queued is not an `Err`: the durable records are
/// returned as usual and [`OrchOutcome::note`] carries the reason, so both
/// surfaces report the same news — the CLI through its exit code, MCP through
/// the note.
pub fn delegate(request: DelegateRequest) -> Result<OrchOutcome> {
    if request.harness.trim().is_empty() {
        bail!("orch_delegate requires a worker harness")
    }
    let session = request.session.clone();
    let actor = request.actor;

    // Resolve an existing task or create one inline from the contract.
    let mut task = match &request.task {
        Some(id) => tasks::read_task(&session, id)?,
        None => {
            let title = request
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .ok_or_else(|| {
                    anyhow!("orch_delegate requires a title when no task id is given")
                })?;
            tasks::add_task(
                &session,
                actor.into(),
                NewTask {
                    title,
                    description: request.description.clone().unwrap_or_default(),
                    depends_on: request.depends_on.clone(),
                    isolate: request.isolate,
                    contract: request.contract.clone(),
                },
            )?
        }
    };

    // Assign to the harness while the state machine still permits it.
    let status = TaskStatus::parse(&task.status)?;
    if matches!(status, TaskStatus::Backlog | TaskStatus::Assigned)
        && task.assignee.as_deref() != Some(request.harness.as_str())
    {
        task = tasks::assign_task(
            &session,
            &task.id,
            request.harness.clone(),
            request.run.clone(),
            actor.into(),
        )?;
    }

    // Start it so dispatch's running-state precondition holds.
    if TaskStatus::parse(&task.status)? == TaskStatus::Assigned {
        task = tasks::start_task(&session, &task.id, actor.into())?;
    }

    // Default the prompt to the rendered brief so acceptance checks travel with
    // the delegation; a non-empty explicit prompt overrides it.
    let prompt = match request.prompt {
        Some(prompt) if !prompt.trim().is_empty() => prompt,
        _ => render_brief(&task),
    };

    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: actor.into(),
        harness: request.harness.clone(),
        pane_id: request.pane.clone(),
        run: request.run.clone(),
        prompt,
        timeout_sec: request.timeout_sec,
    })?;

    // Re-read so the outcome reflects the delivery history and any assignee_run.
    let task = tasks::read_task(&session, &task.id)?;
    let note = delivery_note(&record, &task);
    Ok(OrchOutcome {
        verb: Verb::Delegate.tool_name().to_owned(),
        tasks: vec![task],
        dispatches: vec![record],
        note,
    })
}

/// Read one task (with its dispatches) or the whole board.
pub fn status(request: StatusRequest) -> Result<OrchOutcome> {
    let (tasks, dispatches) = match &request.task {
        Some(id) => {
            let task = tasks::read_task(&request.session, id)?;
            let dispatches = dispatch::list_dispatches(&request.session)?
                .into_iter()
                .filter(|record| &record.task == id)
                .collect();
            (vec![task], dispatches)
        }
        None => (
            tasks::list_tasks(&request.session)?,
            dispatch::list_dispatches(&request.session)?,
        ),
    };
    let note = terminal_failure_note(&tasks, &dispatches);
    Ok(OrchOutcome {
        verb: Verb::Status.tool_name().to_owned(),
        tasks,
        dispatches,
        note,
    })
}

/// Whether a dispatch record has reached a terminal delivery state.
fn delivery_is_terminal(record: &DispatchRecord) -> bool {
    record.is_terminal()
}

/// The newest dispatch for one task, if any (`list_dispatches` is newest-first).
fn latest_dispatch(session: &str, task: &str) -> Result<Option<DispatchRecord>> {
    Ok(dispatch::list_dispatches(session)?
        .into_iter()
        .find(|record| record.task == task))
}

/// Block until the task's newest delivery is terminal or the timeout elapses.
///
/// A confirmed or failed delivery, or a task that itself reached a terminal
/// status, ends the wait immediately. A queued dispatch (held at a concurrency
/// cap, issue #7) is nudged with a drain each poll. On timeout the outcome
/// carries a note and the last-observed state rather than erroring, so a caller
/// can decide what to do next.
pub fn await_delegation(request: AwaitRequest) -> Result<OrchOutcome> {
    let timeout = Duration::from_secs(request.timeout_sec.unwrap_or(DEFAULT_AWAIT_TIMEOUT_SEC));
    let interval = Duration::from_millis(
        request
            .poll_interval_ms
            .unwrap_or(DEFAULT_AWAIT_POLL_MS)
            .max(1),
    );
    let deadline = Instant::now() + timeout;
    loop {
        let task = tasks::read_task(&request.session, &request.task)?;
        let task_status = TaskStatus::parse(&task.status)?;
        let latest = latest_dispatch(&request.session, &request.task)?;
        let terminal = latest.as_ref().is_some_and(delivery_is_terminal)
            || latest.is_none()
                && matches!(
                    task_status,
                    TaskStatus::Review | TaskStatus::Done | TaskStatus::Dropped
                );
        if terminal {
            let note = latest
                .as_ref()
                .and_then(|record| delivery_note(record, &task));
            return Ok(OrchOutcome {
                verb: Verb::Await.tool_name().to_owned(),
                tasks: vec![task],
                dispatches: latest.into_iter().collect(),
                note,
            });
        }
        if latest.as_ref().is_some_and(DispatchRecord::is_queued) {
            // A slot may have freed since the dispatch was queued (issue #7).
            let _ = dispatch::drain_queued(&request.session);
        }
        if Instant::now() >= deadline {
            return Ok(OrchOutcome {
                verb: Verb::Await.tool_name().to_owned(),
                tasks: vec![task],
                dispatches: latest.into_iter().collect(),
                note: Some(format!(
                    "await timed out after {}s; delivery is not terminal yet",
                    timeout.as_secs()
                )),
            });
        }
        thread::sleep(interval);
    }
}

/// Move a running task into review.
pub fn review(request: TaskRef) -> Result<OrchOutcome> {
    let task = tasks::read_task(&request.session, &request.task)?;
    if task.contract.is_none() {
        let task = tasks::review_task(&request.session, &request.task, request.actor.into())?;
        return Ok(OrchOutcome::task_only(Verb::Review, task));
    }
    if TaskStatus::parse(&task.status)? != TaskStatus::Running {
        bail!(
            "contracted task {} must be running before acceptance review",
            task.id
        )
    }
    let executor = dispatch::list_dispatches(&request.session)?
        .into_iter()
        .find(|record| record.task == task.id && !record.is_review())
        .ok_or_else(|| anyhow!("task {} has no executor dispatch receipt", task.id))?;
    if !executor.is_terminal()
        || executor.execution_status.as_deref() != Some("succeeded")
        || executor.exit_code.is_some_and(|code| code != 0)
    {
        bail!(
            "task {} executor has not completed successfully; await it before review",
            task.id
        )
    }
    let selection = report::select_reviewer(&task)?;
    let prompt = report::render_review_brief(&task, &selection)?;
    let record = dispatch::dispatch_review(&DispatchRequest {
        session: request.session.clone(),
        task: task.id.clone(),
        actor: request.actor.into(),
        harness: selection.reviewer.clone(),
        pane_id: None,
        run: None,
        prompt,
        timeout_sec: None,
    })?;
    let task = if record.is_confirmed() {
        tasks::review_task(&request.session, &request.task, request.actor.into())?
    } else {
        tasks::read_task(&request.session, &request.task)?
    };
    let mode = if selection.self_review {
        "self-review (only one capable harness)"
    } else {
        "independent review"
    };
    let note = Some(if record.is_confirmed() {
        format!(
            "{} dispatched to {}; await its terminal verdicts before finish",
            mode, selection.reviewer
        )
    } else {
        format!(
            "{} could not confirm delivery to {}; task remains {}",
            mode, selection.reviewer, task.status
        )
    });
    Ok(OrchOutcome {
        verb: Verb::Review.tool_name().to_owned(),
        tasks: vec![task],
        dispatches: vec![record],
        note,
    })
}

/// Mark a reviewed task done.
pub fn finish(request: TaskRef) -> Result<OrchOutcome> {
    let current = tasks::read_task(&request.session, &request.task)?;
    if current.contract.is_none() {
        let task = tasks::done_task(&request.session, &request.task, request.actor.into())?;
        return Ok(OrchOutcome::task_only(Verb::Finish, task));
    }
    if TaskStatus::parse(&current.status)? != TaskStatus::Review {
        bail!(
            "contracted task {} must be in review before finish",
            current.id
        )
    }
    let dispatches = dispatch::list_dispatches(&request.session)?
        .into_iter()
        .filter(|record| record.task == current.id)
        .collect::<Vec<_>>();
    let reviewer = dispatches
        .iter()
        .find(|record| record.is_review())
        .ok_or_else(|| anyhow!("task {} has no reviewer dispatch receipt", current.id))?;
    if !reviewer.is_terminal() {
        bail!(
            "task {} reviewer is still running; await it before finish",
            current.id
        )
    }
    if reviewer.execution_status.as_deref() != Some("succeeded")
        || reviewer.exit_code.is_some_and(|code| code != 0)
    {
        bail!("task {} reviewer did not complete successfully", current.id)
    }
    let (reported, report) = report::persist_report(
        &request.session,
        &request.task,
        reviewer,
        &dispatches,
        request.actor.into(),
    )?;
    let failed = report
        .verdicts
        .iter()
        .filter(|verdict| verdict.verdict == "fail")
        .count();
    if failed > 0 {
        bail!(
            "task {} remains review: {failed} acceptance check(s) failed; report {}",
            reported.id,
            report::report_path(&request.session, &request.task).display()
        )
    }
    let task = tasks::done_task(&request.session, &request.task, request.actor.into())?;
    Ok(OrchOutcome {
        verb: Verb::Finish.tool_name().to_owned(),
        tasks: vec![task],
        dispatches: vec![reviewer.clone()],
        note: Some(format!(
            "all {} acceptance checks passed; report {}",
            report.verdicts.len(),
            report::report_path(&request.session, &request.task).display()
        )),
    })
}

/// Drop a task (auditable) and stop a linked live worker run best-effort.
///
/// Dropping is the durable action; if the task links a real background run we
/// also request its termination. A linkage that is not a run (for example a
/// hosted pane id) is left alone — the daemon owns pane teardown.
///
/// In practice the termination half only fires for a task linked to a real
/// background run (`pio run`, or an explicit `--run`). A task delegated through
/// [`delegate`] gets its `assignee_run` set to the *dispatch* id, and a
/// dispatch is a bounded synchronous invocation that has already exited by the
/// time anyone can cancel it — so there is nothing left to kill. That is why
/// the verb promises "best-effort" and not "stops the worker".
pub fn cancel(request: TaskRef) -> Result<OrchOutcome> {
    let task = tasks::drop_task(&request.session, &request.task, request.actor.into())?;
    let mut note = None;
    if let Some(run) = task.assignee_run.clone()
        && find_run(&run).is_ok()
    {
        let _ = crate::control::kill(&run);
        note = Some(format!(
            "dropped {} and requested termination of linked run {run}",
            task.id
        ));
    }
    Ok(OrchOutcome {
        verb: Verb::Cancel.tool_name().to_owned(),
        tasks: vec![task],
        dispatches: Vec::new(),
        note,
    })
}

/// Claude Code `.mcp.json` snippet registering the pi-orchestra MCP server.
///
/// Emitted for the operator to paste; `pio` never writes protected config
/// files itself (issue #8, AC4).
#[must_use]
pub fn claude_mcp_json(command: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "pi-orchestra": {
                "command": command,
                "args": [],
                "env": {}
            }
        }
    }))
    .unwrap_or_default()
}

/// Codex `config.toml` block registering the pi-orchestra MCP server.
#[must_use]
pub fn codex_mcp_toml(command: &str) -> String {
    // `{command:?}` emits a correctly quoted-and-escaped TOML basic string.
    format!("[mcp_servers.pi-orchestra]\ncommand = {command:?}\nargs = []\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::DeliveryStatus;
    use serde_json::Value;
    use std::collections::BTreeSet;

    #[test]
    fn verbs_have_unique_stable_names_on_both_surfaces() {
        assert_eq!(Verb::ALL.len(), 7);
        let tool_names: BTreeSet<_> = Verb::ALL.iter().map(|verb| verb.tool_name()).collect();
        let cli_names: BTreeSet<_> = Verb::ALL.iter().map(|verb| verb.cli_name()).collect();
        assert_eq!(tool_names.len(), 7, "tool names must be unique");
        assert_eq!(cli_names.len(), 7, "cli names must be unique");
        for verb in Verb::ALL {
            // The MCP tool name is the `orch_` prefix plus the CLI verb.
            assert_eq!(verb.tool_name(), format!("orch_{}", verb.cli_name()));
            assert!(!verb.description().is_empty());
        }
    }

    #[test]
    fn actor_defaults_to_brain_and_round_trips() {
        assert_eq!(OrchActor::default(), OrchActor::Brain);
        assert_eq!(TaskActor::from(OrchActor::Human), TaskActor::Human);
        assert_eq!(DispatchActor::from(OrchActor::Brain), DispatchActor::Brain);
        assert_eq!(OrchActor::from(TaskActor::Human), OrchActor::Human);
    }

    #[test]
    fn claude_config_is_valid_json_naming_the_server_command() {
        let value: Value = serde_json::from_str(&claude_mcp_json("/opt/pio-mcp")).unwrap();
        assert_eq!(
            value["mcpServers"]["pi-orchestra"]["command"],
            Value::String("/opt/pio-mcp".to_owned())
        );
        assert!(value["mcpServers"]["pi-orchestra"]["args"].is_array());
    }

    #[test]
    fn codex_config_quotes_the_command_and_declares_the_table() {
        let toml = codex_mcp_toml("/opt/pio-mcp");
        assert!(toml.contains("[mcp_servers.pi-orchestra]"));
        assert!(toml.contains("command = \"/opt/pio-mcp\""));
        assert!(toml.trim_end().ends_with("args = []"));
    }

    #[test]
    fn delivery_note_distinguishes_receipt_success_and_terminal_failure() {
        // Built through serde so the durable records stay free of a `Default`
        // impl they do not otherwise need.
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": "T0007", "session": "s", "title": "t", "status": "running",
            "created_at": "now", "updated_at": "now",
        }))
        .unwrap();
        let mut record: DispatchRecord = serde_json::from_value(serde_json::json!({
            "id": "D-1", "session": "s", "task": "T0007", "actor": "brain",
            "harness": "hermes", "command_line": "hermes", "prompt": "p",
            "status": DeliveryStatus::Confirmed.as_str(),
            "created_at": "now", "updated_at": "now",
        }))
        .unwrap();
        assert!(
            delivery_note(&record, &task).is_none(),
            "a pre-#30 confirmed record remains a quiet terminal success"
        );

        record.execution_status = Some("running".to_owned());
        let running = delivery_note(&record, &task).expect("a running worker gives guidance");
        assert!(
            running.contains("still running") && running.contains("orch_await"),
            "{running}"
        );

        record.execution_status = Some("succeeded".to_owned());
        assert!(
            delivery_note(&record, &task).is_none(),
            "a successful execution is quiet"
        );

        record.execution_status = Some("failed".to_owned());
        record.failure_kind = Some("timeout".to_owned());
        record.error = Some("DISPATCH TIMEOUT: hermes exceeded 1s".to_owned());
        record.exit_code = Some(124);
        let execution_failed = delivery_note(&record, &task).expect("a failed execution speaks");
        assert!(
            execution_failed.contains("worker execution failed")
                && execution_failed.contains("timeout")
                && execution_failed.contains("exit code 124"),
            "{execution_failed}"
        );
        assert!(
            execution_failed.contains("T0007") && execution_failed.contains("running"),
            "{execution_failed}"
        );

        record.status = DeliveryStatus::Failed.as_str().to_owned();
        record.execution_status = Some("failed".to_owned());
        record.exit_code = None;
        record.failure_kind = Some("unknown_harness".to_owned());
        record.error = Some("UNKNOWN HARNESS: hermes".to_owned());
        let failed = delivery_note(&record, &task).expect("a failed delivery speaks");
        assert!(failed.contains("did not confirm"), "{failed}");
        assert!(failed.contains("unknown_harness"), "{failed}");
        assert!(
            failed.contains("T0007") && failed.contains("running"),
            "{failed}"
        );

        record.status = DeliveryStatus::Queued.as_str().to_owned();
        record.failure_kind = None;
        record.error = None;
        let queued = delivery_note(&record, &task).expect("a queued delivery speaks");
        assert!(
            queued.contains("queued") && queued.contains("orch_await"),
            "{queued}"
        );
        // Queued work is still coming; it must not be described as a failure or
        // sent for cleanup.
        assert!(
            !queued.contains("did not confirm") && !queued.contains("cancel"),
            "a queued delivery must not read as a failure: {queued}"
        );
    }

    #[test]
    fn request_schemas_reuse_the_contract_surface() {
        // The plan/delegate requests carry the contract-v2 schema from #5.
        let schema = schemars::schema_for!(PlanRequest);
        let json = serde_json::to_value(&schema).unwrap();
        let properties = json
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        assert!(properties.contains_key("contract"));
        assert!(properties.contains_key("session"));
        assert!(properties.contains_key("title"));
    }
}
