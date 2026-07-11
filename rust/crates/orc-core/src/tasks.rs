//! Durable task-board records and their single-writer mutations.
//!
//! Tasks are stored one JSON document per task under the owning Bench session.
//! The module takes a bounded filesystem lock for every mutation, writes via
//! the registry's flush-and-rename primitive, and keeps unknown JSON fields.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::bench::read_session;
use crate::registry::{atomic_write_json, home, now_iso};

const LOCK_ATTEMPTS: usize = 100;
const LOCK_WAIT: Duration = Duration::from_millis(5);

/// Errors specific to the durable task domain.
#[derive(Debug, Error)]
pub enum TaskError {
    /// A task status word is not part of the stable contract.
    #[error("invalid task status '{0}'")]
    InvalidStatus(String),
    /// A mutation attempts a transition not allowed by the state machine.
    #[error("invalid task transition: {from} -> {to}")]
    InvalidTransition {
        /// Status before the mutation.
        from: String,
        /// Requested status.
        to: String,
    },
    /// The task lock could not be acquired within the bounded wait.
    #[error("task board is busy; retry the command")]
    Busy,
}

/// Stable lifecycle words for a task card.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Work exists but has no owner.
    Backlog,
    /// An owner is selected but work has not begun.
    Assigned,
    /// The owner is actively working.
    Running,
    /// Work is ready for an explicit human or brain review.
    Review,
    /// Reviewed work is complete.
    Done,
    /// Work was intentionally discarded.
    Dropped,
}

impl TaskStatus {
    /// Parse one contract status word.
    pub fn parse(value: &str) -> std::result::Result<Self, TaskError> {
        match value {
            "backlog" => Ok(Self::Backlog),
            "assigned" => Ok(Self::Assigned),
            "running" => Ok(Self::Running),
            "review" => Ok(Self::Review),
            "done" => Ok(Self::Done),
            "dropped" => Ok(Self::Dropped),
            _ => Err(TaskError::InvalidStatus(value.to_owned())),
        }
    }

    /// Return the durable lowercase status word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Assigned => "assigned",
            Self::Running => "running",
            Self::Review => "review",
            Self::Done => "done",
            Self::Dropped => "dropped",
        }
    }

    fn permits(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Backlog, Self::Assigned | Self::Dropped)
                | (
                    Self::Assigned,
                    Self::Backlog | Self::Running | Self::Dropped
                )
                | (Self::Running, Self::Assigned | Self::Review | Self::Dropped)
                | (
                    Self::Review,
                    Self::Assigned | Self::Running | Self::Done | Self::Dropped
                )
        )
    }
}

/// Attribution for a durable task mutation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskActor {
    /// A human invoked the command.
    Human,
    /// A brain invoked the command path.
    Brain,
}

impl TaskActor {
    /// Parse the actor contract word.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "human" => Ok(Self::Human),
            "brain" => Ok(Self::Brain),
            _ => bail!("invalid task actor '{value}'; expected brain or human"),
        }
    }

    /// Return the durable actor word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Brain => "brain",
        }
    }
}

/// One append-only, actor-attributed task event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskHistory {
    /// Event timestamp.
    pub at: String,
    /// `human` or `brain`.
    pub actor: String,
    /// Plain action word such as `created`, `assigned`, or `moved`.
    pub action: String,
    /// Previous status when this was a state transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Resulting status when this was a state transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Human-readable mutation detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Requested or materialized worktree metadata owned by a task.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskWorktree {
    /// Lifecycle word: `requested`, `ready`, `merged`, or `pruned`.
    pub state: String,
    /// Worktree path after isolation is materialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Owned branch after isolation is materialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Plain additive JSON task record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Task {
    /// Stable `T`-prefixed identifier.
    pub id: String,
    /// Owning Bench session.
    pub session: String,
    /// Short card heading.
    pub title: String,
    /// Optional durable task detail.
    #[serde(default)]
    pub description: String,
    /// Current lifecycle word.
    pub status: String,
    /// Prerequisite task identifiers in the same session.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Named assignee or worker key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Linked run or pane identifier when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_run: Option<String>,
    /// Isolation request or owned worktree lifecycle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<TaskWorktree>,
    /// Creation timestamp.
    pub created_at: String,
    /// Last mutation timestamp.
    pub updated_at: String,
    /// Append-only actor-attributed event history.
    #[serde(default)]
    pub history: Vec<TaskHistory>,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Values supplied when adding a new task.
#[derive(Clone, Debug, Default)]
pub struct NewTask {
    /// Required card heading.
    pub title: String,
    /// Optional durable detail.
    pub description: String,
    /// Prerequisite IDs in the same session.
    pub depends_on: Vec<String>,
    /// Whether Phase 3B must materialize a worktree.
    pub isolate: bool,
}

struct BoardLock {
    path: PathBuf,
    _file: File,
}

impl Drop for BoardLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn session_key(session: &str) -> String {
    session
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn validate_session_id(session: &str) -> Result<()> {
    if session.is_empty() {
        bail!("task session is required; pass --session or set ORC_SESSION")
    }
    Ok(())
}

fn board_dir(session: &str) -> PathBuf {
    home().join("tasks").join(session_key(session))
}

/// Return the task JSON path for a known stable task and session.
#[must_use]
pub fn task_path(session: &str, id: &str) -> PathBuf {
    board_dir(session).join(format!("{id}.json"))
}

fn task_id_is_valid(id: &str) -> bool {
    id.starts_with('T') && id[1..].bytes().all(|byte| byte.is_ascii_digit()) && id.len() > 1
}

fn validate_id(id: &str) -> Result<()> {
    if !task_id_is_valid(id) {
        bail!("invalid task id '{id}'; expected a T-prefixed numeric id")
    }
    Ok(())
}

fn lock_board(session: &str) -> Result<BoardLock> {
    validate_session_id(session)?;
    let dir = board_dir(session);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(".board.lock");
    for _ in 0..LOCK_ATTEMPTS {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok(BoardLock { path, _file: file }),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => thread::sleep(LOCK_WAIT),
            Err(error) => return Err(error).with_context(|| format!("lock {}", path.display())),
        }
    }
    Err(TaskError::Busy.into())
}

fn task_files(session: &str) -> Result<Vec<PathBuf>> {
    let dir = board_dir(session);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut paths = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn read_all_strict(session: &str) -> Result<Vec<Task>> {
    task_files(session)?
        .into_iter()
        .map(|path| {
            serde_json::from_slice::<Task>(&fs::read(&path)?)
                .with_context(|| format!("parse task {}", path.display()))
        })
        .collect()
}

fn write_task(task: &Task) -> Result<()> {
    atomic_write_json(&task_path(&task.session, &task.id), task)
}

fn append_history(
    task: &mut Task,
    actor: TaskActor,
    action: &str,
    from: Option<TaskStatus>,
    to: Option<TaskStatus>,
    detail: Option<String>,
) {
    let now = now_iso();
    task.updated_at = now.clone();
    task.history.push(TaskHistory {
        at: now,
        actor: actor.as_str().to_owned(),
        action: action.to_owned(),
        from: from.map(|status| status.as_str().to_owned()),
        to: to.map(|status| status.as_str().to_owned()),
        detail,
        extra: BTreeMap::new(),
    });
}

fn dependencies_are_done(task: &Task, all: &[Task]) -> Result<bool> {
    for dependency in &task.depends_on {
        let dependency_task = all
            .iter()
            .find(|candidate| candidate.id == *dependency)
            .ok_or_else(|| anyhow!("task {} depends on missing task {dependency}", task.id))?;
        if TaskStatus::parse(&dependency_task.status)? != TaskStatus::Done {
            return Ok(false);
        }
    }
    Ok(true)
}

fn validate_dependencies(id: &str, dependencies: &[String], all: &[Task]) -> Result<()> {
    let mut unique = BTreeSet::new();
    for dependency in dependencies {
        validate_id(dependency)?;
        if dependency == id {
            bail!("task {id} cannot depend on itself")
        }
        if !unique.insert(dependency) {
            bail!("task {id} repeats dependency {dependency}")
        }
        if !all.iter().any(|task| task.id == *dependency) {
            bail!("task {id} depends on missing task {dependency}")
        }
    }
    let by_id = all
        .iter()
        .map(|task| (&task.id, &task.depends_on))
        .collect::<BTreeMap<_, _>>();
    fn reaches(
        current: &str,
        target: &str,
        by_id: &BTreeMap<&String, &Vec<String>>,
        seen: &mut BTreeSet<String>,
    ) -> bool {
        if current == target {
            return true;
        }
        if !seen.insert(current.to_owned()) {
            return false;
        }
        by_id.get(&current.to_owned()).is_some_and(|children| {
            children
                .iter()
                .any(|child| reaches(child, target, by_id, seen))
        })
    }
    for dependency in dependencies {
        if reaches(dependency, id, &by_id, &mut BTreeSet::new()) {
            bail!("task dependency cycle includes {id} and {dependency}")
        }
    }
    Ok(())
}

fn next_id(all: &[Task]) -> String {
    let highest = all
        .iter()
        .filter_map(|task| task.id.strip_prefix('T'))
        .filter_map(|number| number.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("T{:04}", highest.saturating_add(1))
}

/// Read every parseable task newest first, ignoring corrupt sibling files.
pub fn list_tasks(session: &str) -> Result<Vec<Task>> {
    validate_session_id(session)?;
    let mut tasks = task_files(session)?
        .into_iter()
        .filter_map(|path| fs::read(path).ok())
        .filter_map(|bytes| serde_json::from_slice::<Task>(&bytes).ok())
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(tasks)
}

/// Read one task, returning explicit errors for an invalid ID, missing file, or corrupt JSON.
pub fn read_task(session: &str, id: &str) -> Result<Task> {
    validate_session_id(session)?;
    validate_id(id)?;
    let path = task_path(session, id);
    serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("parse task {}", path.display()))
}

/// Add a backlog task after validating its session and dependency graph.
pub fn add_task(session: &str, actor: TaskActor, new: NewTask) -> Result<Task> {
    validate_session_id(session)?;
    if new.title.trim().is_empty() {
        bail!("task title cannot be empty")
    }
    let _lock = lock_board(session)?;
    read_session(session).with_context(|| format!("missing task session {session}"))?;
    let all = read_all_strict(session)?;
    let id = next_id(&all);
    validate_dependencies(&id, &new.depends_on, &all)?;
    let now = now_iso();
    let mut task = Task {
        id,
        session: session.to_owned(),
        title: new.title,
        description: new.description,
        status: TaskStatus::Backlog.as_str().to_owned(),
        depends_on: new.depends_on,
        assignee: None,
        assignee_run: None,
        worktree: new.isolate.then(|| TaskWorktree {
            state: "requested".to_owned(),
            path: None,
            branch: None,
            extra: BTreeMap::new(),
        }),
        created_at: now.clone(),
        updated_at: now,
        history: Vec::new(),
        extra: BTreeMap::new(),
    };
    append_history(
        &mut task,
        actor,
        "created",
        None,
        Some(TaskStatus::Backlog),
        None,
    );
    write_task(&task)?;
    Ok(task)
}

/// Assign a backlog task to a named worker and optional run link.
pub fn assign_task(
    session: &str,
    id: &str,
    assignee: String,
    run: Option<String>,
    actor: TaskActor,
) -> Result<Task> {
    if assignee.trim().is_empty() {
        bail!("task assignee cannot be empty")
    }
    let _lock = lock_board(session)?;
    let _all = read_all_strict(session)?;
    let mut task = read_task(session, id)?;
    let from = TaskStatus::parse(&task.status)?;
    if !matches!(from, TaskStatus::Backlog | TaskStatus::Assigned) {
        return Err(TaskError::InvalidTransition {
            from: from.as_str().to_owned(),
            to: TaskStatus::Assigned.as_str().to_owned(),
        }
        .into());
    }
    task.status = TaskStatus::Assigned.as_str().to_owned();
    task.assignee = Some(assignee.clone());
    task.assignee_run = run;
    let action = if from == TaskStatus::Assigned {
        "reassigned"
    } else {
        "assigned"
    };
    append_history(
        &mut task,
        actor,
        action,
        Some(from),
        Some(TaskStatus::Assigned),
        Some(assignee),
    );
    write_task(&task)?;
    Ok(task)
}

/// Move a task through the documented state machine.
pub fn move_task(session: &str, id: &str, next: TaskStatus, actor: TaskActor) -> Result<Task> {
    let _lock = lock_board(session)?;
    let all = read_all_strict(session)?;
    let mut task = read_task(session, id)?;
    let from = TaskStatus::parse(&task.status)?;
    if !from.permits(next) {
        return Err(TaskError::InvalidTransition {
            from: from.as_str().to_owned(),
            to: next.as_str().to_owned(),
        }
        .into());
    }
    if next == TaskStatus::Running {
        if task.assignee.is_none() {
            bail!("task {id} must be assigned before it can start")
        }
        if !dependencies_are_done(&task, &all)? {
            bail!("task {id} is blocked by unfinished dependencies")
        }
    }
    task.status = next.as_str().to_owned();
    append_history(&mut task, actor, "moved", Some(from), Some(next), None);
    write_task(&task)?;
    Ok(task)
}

/// Start an assigned task after checking its dependencies.
pub fn start_task(session: &str, id: &str, actor: TaskActor) -> Result<Task> {
    move_task(session, id, TaskStatus::Running, actor)
}

/// Move a running task to review.
pub fn review_task(session: &str, id: &str, actor: TaskActor) -> Result<Task> {
    move_task(session, id, TaskStatus::Review, actor)
}

/// Mark a reviewed task done.
pub fn done_task(session: &str, id: &str, actor: TaskActor) -> Result<Task> {
    move_task(session, id, TaskStatus::Done, actor)
}

/// Drop a non-terminal task without deleting its durable record.
pub fn drop_task(session: &str, id: &str, actor: TaskActor) -> Result<Task> {
    move_task(session, id, TaskStatus::Dropped, actor)
}
