//! Durable task-board records and their single-writer mutations.
//!
//! Tasks are stored one JSON document per task under the owning Bench session.
//! The module takes a bounded filesystem lock for every mutation, writes via
//! the registry's flush-and-rename primitive, and keeps unknown JSON fields.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::bench::{BenchSession, read_session, write_session};
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
    /// Human-readable reason when isolation cannot be materialized safely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Squash-merge commit after an explicit successful merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_commit: Option<String>,
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

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        bail!("git {} failed: {detail}", args.join(" "))
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn expected_branch(session: &str, id: &str) -> String {
    let slug = crate::registry::make_slug(session);
    format!("orc/{slug}/{id}")
}

fn expected_worktree_path(session: &str, id: &str) -> PathBuf {
    home().join("worktrees").join(session_key(session)).join(id)
}

fn worktree_parent_is_safe(path: &Path) -> Result<()> {
    let root = home().join("worktrees");
    if !path.starts_with(&root) {
        bail!("ISOLATION UNAVAILABLE: worktree path escapes its owned root")
    }
    fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    let root_metadata = fs::symlink_metadata(&root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        bail!("ISOLATION UNAVAILABLE: worktree root is not an owned directory")
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("worktree path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let metadata = fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("ISOLATION UNAVAILABLE: worktree parent is not an owned directory")
    }
    let canonical_root = fs::canonicalize(&root)?;
    if !fs::canonicalize(parent)?.starts_with(canonical_root) {
        bail!("ISOLATION UNAVAILABLE: worktree parent escapes its owned root")
    }
    if path.exists() || fs::symlink_metadata(path).is_ok() {
        bail!("ISOLATION UNAVAILABLE: owned worktree path already exists")
    }
    Ok(())
}

fn session_base(session: &str) -> Result<(BenchSession, PathBuf, String, String)> {
    let mut record =
        read_session(session).with_context(|| format!("missing task session {session}"))?;
    if record.base_repo.is_none() || record.base_branch.is_none() || record.base_commit.is_none() {
        let cwd = PathBuf::from(&record.cwd);
        let repo = match git(&cwd, &["rev-parse", "--show-toplevel"]) {
            Ok(repo) => repo,
            Err(_) => bail!("ISOLATION UNAVAILABLE: session cwd is not a Git work tree"),
        };
        let branch = git(&cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .map_err(|_| anyhow!("ISOLATION UNAVAILABLE: session base has detached HEAD"))?;
        let commit = git(&cwd, &["rev-parse", "HEAD"])?;
        record.base_repo = Some(repo);
        record.base_branch = Some(branch);
        record.base_commit = Some(commit);
        record.updated_at = now_iso();
        write_session(&record)?;
    }
    let repo = PathBuf::from(record.base_repo.clone().unwrap_or_default());
    let branch = record.base_branch.clone().unwrap_or_default();
    let commit = record.base_commit.clone().unwrap_or_default();
    if repo.as_os_str().is_empty() || branch.is_empty() || commit.is_empty() {
        bail!("ISOLATION UNAVAILABLE: session has no complete recorded Git base")
    }
    let actual = git(&repo, &["rev-parse", "--show-toplevel"])?;
    if fs::canonicalize(&repo)? != fs::canonicalize(actual)? {
        bail!("ISOLATION UNAVAILABLE: recorded repository root is inconsistent")
    }
    Ok((record, repo, branch, commit))
}

fn base_is_clean(repo: &Path, branch: &str, commit: &str) -> Result<()> {
    let current_branch = git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|_| anyhow!("ISOLATION UNAVAILABLE: base checkout has detached HEAD"))?;
    if current_branch != branch {
        bail!("ISOLATION UNAVAILABLE: base branch is '{current_branch}', expected '{branch}'")
    }
    let current_commit = git(repo, &["rev-parse", "HEAD"])?;
    if current_commit != commit {
        bail!("ISOLATION UNAVAILABLE: base commit changed; create a fresh session before isolation")
    }
    if !git(repo, &["status", "--porcelain"])?.is_empty() {
        bail!("ISOLATION UNAVAILABLE: base checkout is dirty")
    }
    Ok(())
}

fn mark_unavailable(task: &mut Task, actor: TaskActor, reason: String) {
    if let Some(worktree) = task.worktree.as_mut() {
        worktree.state = "unavailable".to_owned();
        worktree.reason = Some(reason.clone());
        append_history(
            task,
            actor,
            "isolation_unavailable",
            None,
            None,
            Some(reason),
        );
    }
}

fn materialize_worktree(task: &mut Task, actor: TaskActor) -> Result<()> {
    if task.worktree.is_none() {
        return Ok(());
    }
    let result = (|| -> Result<()> {
        let (_session, repo, branch, commit) = session_base(&task.session)?;
        base_is_clean(&repo, &branch, &commit)?;
        let path = expected_worktree_path(&task.session, &task.id);
        worktree_parent_is_safe(&path)?;
        let owned_branch = expected_branch(&task.session, &task.id);
        if git(
            &repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{owned_branch}"),
            ],
        )
        .is_ok()
        {
            bail!("ISOLATION UNAVAILABLE: owned branch name already exists")
        }
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                &owned_branch,
                &path.to_string_lossy(),
                &commit,
            ],
        )?;
        let worktree = task
            .worktree
            .as_mut()
            .ok_or_else(|| anyhow!("missing worktree metadata"))?;
        worktree.state = "ready".to_owned();
        worktree.path = Some(path.to_string_lossy().into_owned());
        worktree.branch = Some(owned_branch);
        worktree.reason = None;
        append_history(task, actor, "isolated", None, None, None);
        Ok(())
    })();
    if let Err(error) = result {
        mark_unavailable(task, actor, error.to_string());
    }
    Ok(())
}

fn owned_worktree(task: &Task) -> Result<(PathBuf, String, PathBuf)> {
    let worktree = task
        .worktree
        .as_ref()
        .ok_or_else(|| anyhow!("ISOLATION UNAVAILABLE: task is not isolated"))?;
    if worktree.state == "unavailable" {
        bail!(
            "ISOLATION UNAVAILABLE: {}",
            worktree.reason.as_deref().unwrap_or("no usable Git base")
        )
    }
    if worktree.state != "ready" && worktree.state != "conflict" {
        bail!("ISOLATION UNAVAILABLE: task worktree is {}", worktree.state)
    }
    let path = PathBuf::from(
        worktree
            .path
            .as_deref()
            .ok_or_else(|| anyhow!("ISOLATION UNAVAILABLE: task has no worktree path"))?,
    );
    let branch = worktree
        .branch
        .clone()
        .ok_or_else(|| anyhow!("ISOLATION UNAVAILABLE: task has no worktree branch"))?;
    if path != expected_worktree_path(&task.session, &task.id)
        || branch != expected_branch(&task.session, &task.id)
    {
        bail!("ISOLATION UNAVAILABLE: task worktree ownership cannot be proven")
    }
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("ISOLATION UNAVAILABLE: missing worktree {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("ISOLATION UNAVAILABLE: worktree path is unsafe")
    }
    let (_session, repo, _base_branch, _base_commit) = session_base(&task.session)?;
    let list = git(&repo, &["worktree", "list", "--porcelain"])?;
    let target = fs::canonicalize(&path)?;
    let found = list.split("\n\n").any(|record| {
        let path_match = record
            .lines()
            .find_map(|line| line.strip_prefix("worktree "))
            .and_then(|value| fs::canonicalize(value).ok())
            .is_some_and(|value| value == target);
        let branch_match = record
            .lines()
            .any(|line| line == format!("branch refs/heads/{branch}"));
        path_match && branch_match
    });
    if !found || git(&path, &["symbolic-ref", "--quiet", "--short", "HEAD"])? != branch {
        bail!("ISOLATION UNAVAILABLE: worktree and branch ownership cannot be proven")
    }
    Ok((path, branch, repo))
}

/// Real changed-line and file counts for an isolated task worktree.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskDiff {
    /// Added text lines, excluding binary-file byte markers.
    pub insertions: u64,
    /// Deleted text lines, excluding binary-file byte markers.
    pub deletions: u64,
    /// Changed paths.
    pub files: u64,
}

/// Read the real Git diff for a task without mutating its worktree.
pub fn diff_task(session: &str, id: &str) -> Result<TaskDiff> {
    let task = read_task(session, id)?;
    let (path, _branch, _repo) = owned_worktree(&task)?;
    let base = session_base(session)?.3;
    let text = git(&path, &["diff", "--numstat", &base])?;
    let mut diff = TaskDiff::default();
    for line in text.lines() {
        let mut fields = line.splitn(3, '\t');
        let additions = fields.next().unwrap_or_default();
        let deletions = fields.next().unwrap_or_default();
        if fields.next().is_none() {
            continue;
        }
        diff.files = diff.files.saturating_add(1);
        diff.insertions = diff
            .insertions
            .saturating_add(additions.parse::<u64>().unwrap_or(0));
        diff.deletions = diff
            .deletions
            .saturating_add(deletions.parse::<u64>().unwrap_or(0));
    }
    Ok(diff)
}

fn prune_owned_worktree(task: &mut Task) -> Result<()> {
    let (path, branch, repo) = owned_worktree(task)?;
    git(&repo, &["worktree", "remove", &path.to_string_lossy()])?;
    git(&repo, &["branch", "-D", &branch])?;
    if let Some(worktree) = task.worktree.as_mut() {
        worktree.state = "pruned".to_owned();
        worktree.reason = None;
    }
    Ok(())
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
            reason: None,
            result_commit: None,
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
    if new.isolate {
        materialize_worktree(&mut task, actor)?;
        write_task(&task)?;
    }
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

/// Record a confirmed or failed worker delivery on the durable task history.
///
/// Only confirmed delivery may set `assignee_run`; failures remain visible in
/// history without claiming that a worker received the brief.
pub fn record_delivery(
    session: &str,
    id: &str,
    actor: TaskActor,
    confirmed_link: Option<String>,
    detail: String,
) -> Result<Task> {
    let _lock = lock_board(session)?;
    let _all = read_all_strict(session)?;
    let mut task = read_task(session, id)?;
    let action = if confirmed_link.is_some() {
        "delivery_confirmed"
    } else {
        "delivery_failed"
    };
    if let Some(link) = confirmed_link {
        task.assignee_run = Some(link);
    }
    append_history(&mut task, actor, action, None, None, Some(detail));
    write_task(&task)?;
    Ok(task)
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
    let _lock = lock_board(session)?;
    let _all = read_all_strict(session)?;
    let mut task = read_task(session, id)?;
    let from = TaskStatus::parse(&task.status)?;
    if !from.permits(TaskStatus::Dropped) {
        return Err(TaskError::InvalidTransition {
            from: from.as_str().to_owned(),
            to: TaskStatus::Dropped.as_str().to_owned(),
        }
        .into());
    }
    if task
        .worktree
        .as_ref()
        .is_some_and(|worktree| matches!(worktree.state.as_str(), "ready" | "conflict"))
    {
        prune_owned_worktree(&mut task)?;
    }
    task.status = TaskStatus::Dropped.as_str().to_owned();
    append_history(
        &mut task,
        actor,
        "dropped",
        Some(from),
        Some(TaskStatus::Dropped),
        None,
    );
    write_task(&task)?;
    Ok(task)
}

/// Explicitly squash-merge a reviewed clean isolated task onto its recorded base.
///
/// The operation refuses dirty, moved, detached, or unowned bases. A merge
/// conflict is recorded on the task and left for a human to resolve; it is
/// never auto-resolved or marked done.
pub fn merge_task(session: &str, id: &str, actor: TaskActor) -> Result<Task> {
    let _lock = lock_board(session)?;
    let _all = read_all_strict(session)?;
    let mut task = read_task(session, id)?;
    if TaskStatus::parse(&task.status)? != TaskStatus::Review {
        bail!("task {id} must be in review before an explicit merge")
    }
    let (path, branch, repo) = owned_worktree(&task)?;
    let (mut record, _recorded_repo, base_branch, base_commit) = session_base(session)?;
    base_is_clean(&repo, &base_branch, &base_commit)?;
    if !git(&path, &["status", "--porcelain"])?.is_empty() {
        bail!("task {id} worktree is dirty; commit its changes before merge")
    }
    if let Err(error) = git(&repo, &["merge", "--squash", "--no-commit", &branch]) {
        if let Some(worktree) = task.worktree.as_mut() {
            worktree.state = "conflict".to_owned();
            worktree.reason = Some(error.to_string());
        }
        append_history(
            &mut task,
            actor,
            "merge_conflict",
            Some(TaskStatus::Review),
            Some(TaskStatus::Review),
            Some(error.to_string()),
        );
        write_task(&task)?;
        return Err(error);
    }
    if git(&repo, &["diff", "--cached", "--quiet"]).is_ok() {
        bail!("task {id} has no committed branch changes to merge")
    }
    git(
        &repo,
        &["commit", "-m", &format!("orc task {id}: {}", task.title)],
    )?;
    let result_commit = git(&repo, &["rev-parse", "HEAD"])?;
    record.base_commit = Some(result_commit.clone());
    record.updated_at = now_iso();
    write_session(&record)?;
    prune_owned_worktree(&mut task)?;
    if let Some(worktree) = task.worktree.as_mut() {
        worktree.state = "merged".to_owned();
        worktree.result_commit = Some(result_commit.clone());
        worktree.reason = None;
    }
    append_history(
        &mut task,
        actor,
        "merged",
        Some(TaskStatus::Review),
        Some(TaskStatus::Review),
        Some(result_commit),
    );
    write_task(&task)?;
    Ok(task)
}
