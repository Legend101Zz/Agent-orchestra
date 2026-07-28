//! Independent acceptance review and durable final task reports.
//!
//! Contracted work is not complete until a reviewer returns one structured
//! verdict per acceptance check. Reports are additive JSON under
//! `~/.orchestra/reports/<session>/<task>.json`; a compact link is copied into
//! the task record so older readers can ignore it and newer surfaces agree.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::locate_executable;
use crate::bench::{load_harness_registry, read_session};
use crate::dispatch::DispatchRecord;
use crate::invocation::resolve_worker_invocation;
use crate::probe::probed_from;
use crate::registry::{atomic_write_json, home, now_iso};
use crate::tasks::{Task, TaskActor, TaskCheckVerdict, TaskReportLink, attach_report, read_task};

/// One independently judged acceptance check.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AcceptanceVerdict {
    /// Contract check, preserved verbatim and in contract order.
    pub check: String,
    /// Stable lowercase verdict: `pass` or `fail`.
    pub verdict: String,
    /// Concise command/output or inspection evidence.
    pub evidence: String,
}

/// Aggregated exact usage from executor and reviewer receipts.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ReportUsage {
    /// Exact input tokens when every dispatch reported usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    /// Exact output tokens when every dispatch reported usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    /// Exact cache-read tokens when every dispatch reported usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    /// Exact total tokens when every dispatch reported usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Exact summed cost when every dispatch reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Receipts that did not expose exact token usage.
    pub unreported_receipts: usize,
}

/// Durable final report for one contracted task.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FinalReport {
    /// Additive report schema version.
    pub version: u32,
    /// Owning Bench session.
    pub session: String,
    /// Stable task identifier.
    pub task: String,
    /// Task title at completion.
    pub title: String,
    /// Harness that executed the task.
    pub executor: String,
    /// Harness that reviewed the acceptance checks.
    pub reviewer: String,
    /// `independent` or `self_review`.
    pub review_mode: String,
    /// One verdict for every acceptance check.
    pub verdicts: Vec<AcceptanceVerdict>,
    /// Aggregated exact usage when reported by the harnesses.
    pub usage: ReportUsage,
    /// Durable dispatch/worktree receipt identifiers.
    pub receipts: Vec<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Unknown future fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Chosen review harness plus the honest diversity mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewerSelection {
    /// Configured harness key.
    pub reviewer: String,
    /// True only when the executor must review its own work.
    pub self_review: bool,
}

#[derive(Deserialize)]
struct ReviewResponse {
    verdicts: Vec<AcceptanceVerdict>,
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

/// Stable report path for one task.
#[must_use]
pub fn report_path(session: &str, task: &str) -> PathBuf {
    home()
        .join("reports")
        .join(session_key(session))
        .join(format!("{task}.json"))
}

/// Read one final report.
pub fn read_report(session: &str, task: &str) -> Result<FinalReport> {
    let path = report_path(session, task);
    serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("parse final report {}", path.display()))
}

/// Read every parseable final report, newest first.
pub fn list_reports(session: Option<&str>) -> Result<Vec<FinalReport>> {
    let root = home().join("reports");
    let mut paths = Vec::new();
    if let Some(session) = session {
        let dir = root.join(session_key(session));
        if let Ok(entries) = fs::read_dir(dir) {
            paths.extend(entries.filter_map(Result::ok).map(|entry| entry.path()));
        }
    } else if let Ok(sessions) = fs::read_dir(root) {
        for dir in sessions.filter_map(Result::ok).map(|entry| entry.path()) {
            if let Ok(entries) = fs::read_dir(dir) {
                paths.extend(entries.filter_map(Result::ok).map(|entry| entry.path()));
            }
        }
    }
    let mut reports = paths
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| fs::read(path).ok())
        .filter_map(|bytes| serde_json::from_slice::<FinalReport>(&bytes).ok())
        .collect::<Vec<_>>();
    reports.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.task.cmp(&left.task))
    });
    Ok(reports)
}

/// Choose a preferred independent reviewer, falling back honestly to self-review.
///
/// `capable` must contain only harnesses whose executable and non-interactive
/// invocation were verified locally. Input order is the routing preference.
pub fn choose_reviewer(
    executor: &str,
    preferred: Option<&str>,
    capable: &[String],
) -> Result<ReviewerSelection> {
    let unique = capable
        .iter()
        .filter(|key| !key.trim().is_empty())
        .cloned()
        .collect::<BTreeSet<_>>();
    if unique.is_empty() {
        bail!("REVIEW UNAVAILABLE: no capable non-interactive harness exists")
    }
    if let Some(preferred) = preferred
        && preferred != executor
        && unique.contains(preferred)
    {
        return Ok(ReviewerSelection {
            reviewer: preferred.to_owned(),
            self_review: false,
        });
    }
    if let Some(reviewer) = capable
        .iter()
        .find(|candidate| candidate.as_str() != executor && unique.contains(*candidate))
    {
        return Ok(ReviewerSelection {
            reviewer: reviewer.clone(),
            self_review: false,
        });
    }
    if unique.contains(executor) {
        return Ok(ReviewerSelection {
            reviewer: executor.to_owned(),
            self_review: true,
        });
    }
    let reviewer = capable
        .iter()
        .find(|candidate| unique.contains(*candidate))
        .cloned()
        .ok_or_else(|| anyhow!("REVIEW UNAVAILABLE: no capable reviewer"))?;
    Ok(ReviewerSelection {
        reviewer,
        self_review: false,
    })
}

/// Resolve a task's reviewer from real registry capabilities.
pub fn select_reviewer(task: &Task) -> Result<ReviewerSelection> {
    let executor = task
        .assignee
        .as_deref()
        .ok_or_else(|| anyhow!("task {} has no executor", task.id))?;
    let session = read_session(&task.session)?;
    let registry = load_harness_registry()?;
    let cwd = task
        .worktree
        .as_ref()
        .and_then(|worktree| worktree.path.as_deref())
        .map(Path::new)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| Path::new(&session.cwd));

    let mut order = Vec::new();
    order.extend(session.workers.iter().cloned());
    order.push(session.brain.clone());
    order.extend(registry.default_workers.iter().cloned());
    order.extend(registry.harnesses.keys().cloned());
    let mut seen = BTreeSet::new();
    order.retain(|key| seen.insert(key.clone()));
    let capable = order
        .into_iter()
        .filter(|key| {
            let Some(config) = registry.harnesses.get(key) else {
                return false;
            };
            config.roles.iter().any(|role| role == "worker")
                && locate_executable(&config.command).is_some()
                && resolve_worker_invocation(
                    config,
                    &probed_from(&registry, &config.adapter),
                    Some(cwd),
                )
                .is_ok()
        })
        .collect::<Vec<_>>();
    choose_reviewer(
        executor,
        task.contract
            .as_ref()
            .and_then(|contract| contract.reviewer.as_deref()),
        &capable,
    )
}

/// Strict reviewer brief: inspect the isolated worktree and return JSON only.
pub fn render_review_brief(task: &Task, selection: &ReviewerSelection) -> Result<String> {
    let contract = task
        .contract
        .as_ref()
        .ok_or_else(|| anyhow!("task {} is not contracted", task.id))?;
    let worktree = task
        .worktree
        .as_ref()
        .and_then(|worktree| worktree.path.as_deref())
        .ok_or_else(|| anyhow!("task {} has no isolated worktree", task.id))?;
    let mut brief = format!(
        "Review task {} independently in the assigned worktree.\nExecutor: {}\nReviewer: {}\nWorktree: {}\n",
        task.id,
        task.assignee.as_deref().unwrap_or("unknown"),
        selection.reviewer,
        worktree
    );
    if selection.self_review {
        brief.push_str(
            "Only one capable harness exists. This is an explicit self-review; do not claim independent diversity.\n",
        );
    }
    brief.push_str(
        "Evaluate every acceptance check using commands or direct inspection. Return JSON only, with no markdown fences, in this exact shape:\n{\"verdicts\":[{\"check\":\"exact check text\",\"verdict\":\"pass|fail\",\"evidence\":\"concise command/output or inspection evidence\"}]}\nAcceptance checks:\n",
    );
    for (index, check) in contract.acceptance_checks.iter().enumerate() {
        brief.push_str(&format!("{}. {}\n", index + 1, check));
    }
    Ok(brief)
}

/// Parse and validate one review response against the task contract.
pub fn parse_review_verdicts(task: &Task, output: &str) -> Result<Vec<AcceptanceVerdict>> {
    let checks = &task
        .contract
        .as_ref()
        .ok_or_else(|| anyhow!("task {} is not contracted", task.id))?
        .acceptance_checks;
    let trimmed = output.trim();
    let json = if trimmed.starts_with('{') && trimmed.ends_with('}') {
        trimmed
    } else {
        let start = trimmed
            .find('{')
            .ok_or_else(|| anyhow!("review output did not contain JSON"))?;
        let end = trimmed
            .rfind('}')
            .ok_or_else(|| anyhow!("review output did not contain complete JSON"))?;
        &trimmed[start..=end]
    };
    let response: ReviewResponse =
        serde_json::from_str(json).context("parse reviewer verdict JSON")?;
    if response.verdicts.len() != checks.len() {
        bail!(
            "review returned {} verdicts for {} acceptance checks",
            response.verdicts.len(),
            checks.len()
        )
    }
    for (index, (verdict, check)) in response.verdicts.iter().zip(checks).enumerate() {
        if verdict.check != *check {
            bail!(
                "review verdict {} changed the acceptance check text",
                index + 1
            )
        }
        if !matches!(verdict.verdict.as_str(), "pass" | "fail") {
            bail!(
                "review verdict {} must be pass or fail, got '{}'",
                index + 1,
                verdict.verdict
            )
        }
        if verdict.evidence.trim().is_empty() {
            bail!("review verdict {} has no evidence", index + 1)
        }
    }
    Ok(response.verdicts)
}

fn aggregate_usage(dispatches: &[DispatchRecord]) -> ReportUsage {
    let reported = dispatches
        .iter()
        .filter_map(|record| record.usage.as_ref())
        .collect::<Vec<_>>();
    let unreported = dispatches.len().saturating_sub(reported.len());
    let all_reported = !dispatches.is_empty() && unreported == 0;
    let all_costed = all_reported && reported.iter().all(|usage| usage.cost_usd.is_some());
    ReportUsage {
        input: all_reported.then(|| reported.iter().map(|usage| usage.input).sum()),
        output: all_reported.then(|| reported.iter().map(|usage| usage.output).sum()),
        cache_read: all_reported.then(|| reported.iter().map(|usage| usage.cache_read).sum()),
        total: all_reported.then(|| reported.iter().map(|usage| usage.total).sum()),
        cost_usd: all_costed.then(|| {
            reported
                .iter()
                .filter_map(|usage| usage.cost_usd)
                .sum::<f64>()
        }),
        unreported_receipts: unreported,
    }
}

/// Persist a validated report and attach its compact link to the task.
pub fn persist_report(
    session: &str,
    task_id: &str,
    reviewer_record: &DispatchRecord,
    dispatches: &[DispatchRecord],
    actor: TaskActor,
) -> Result<(Task, FinalReport)> {
    let task = read_task(session, task_id)?;
    let executor = task
        .assignee
        .clone()
        .ok_or_else(|| anyhow!("task {task_id} has no executor"))?;
    let verdicts = parse_review_verdicts(&task, &reviewer_record.stdout)?;
    let reviewer = reviewer_record.harness.clone();
    let review_mode = if reviewer == executor {
        "self_review"
    } else {
        "independent"
    }
    .to_owned();
    let usage = aggregate_usage(dispatches);
    let mut receipts = dispatches
        .iter()
        .map(|record| format!("dispatch:{}", record.id))
        .collect::<Vec<_>>();
    if let Some(worktree) = &task.worktree {
        if let Some(branch) = &worktree.branch {
            receipts.push(format!("worktree-branch:{branch}"));
        }
        if let Some(commit) = &worktree.result_commit {
            receipts.push(format!("result-commit:{commit}"));
        }
    }
    let report = FinalReport {
        version: 1,
        session: session.to_owned(),
        task: task_id.to_owned(),
        title: task.title.clone(),
        executor: executor.clone(),
        reviewer: reviewer.clone(),
        review_mode: review_mode.clone(),
        verdicts,
        usage,
        receipts,
        created_at: now_iso(),
        extra: BTreeMap::new(),
    };
    let path = report_path(session, task_id);
    atomic_write_json(&path, &report)?;
    let link = TaskReportLink {
        path: path.to_string_lossy().into_owned(),
        executor,
        reviewer,
        review_mode,
        verdicts: report
            .verdicts
            .iter()
            .map(|verdict| TaskCheckVerdict {
                check: verdict.check.clone(),
                verdict: verdict.verdict.clone(),
                evidence: verdict.evidence.clone(),
            })
            .collect(),
        tokens_total: report.usage.total,
        cost_usd: report.usage.cost_usd.map(|cost| format!("{cost:.6}")),
        extra: BTreeMap::new(),
    };
    let task = attach_report(session, task_id, actor, link)?;
    Ok((task, report))
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;
    use crate::contract::TaskContract;
    use crate::dispatch::DispatchRecord;
    use crate::registry::atomic_write_json;
    use crate::tasks::{TaskHistory, TaskWorktree, task_path};

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn contracted_task() -> Task {
        Task {
            id: "T0001".to_owned(),
            session: "review-session".to_owned(),
            title: "Ship isolated change".to_owned(),
            description: String::new(),
            status: "review".to_owned(),
            depends_on: Vec::new(),
            assignee: Some("hermes".to_owned()),
            assignee_run: Some("D-executor".to_owned()),
            worktree: Some(TaskWorktree {
                state: "ready".to_owned(),
                path: Some("/tmp/review-worktree".to_owned()),
                branch: Some("orc/review-session/T0001".to_owned()),
                reason: None,
                result_commit: None,
                extra: BTreeMap::new(),
            }),
            contract: Some(TaskContract {
                objective: "Ship the change.".to_owned(),
                allowed_paths: vec!["src/".to_owned()],
                forbidden: vec!["do not edit main".to_owned()],
                acceptance_checks: vec!["main stays untouched".to_owned(), "tests pass".to_owned()],
                ..TaskContract::default()
            }),
            report: None,
            created_at: "2026-07-28T00:00:00Z".to_owned(),
            updated_at: "2026-07-28T00:00:00Z".to_owned(),
            history: vec![TaskHistory {
                at: "2026-07-28T00:00:00Z".to_owned(),
                actor: "brain".to_owned(),
                action: "created".to_owned(),
                from: None,
                to: Some("backlog".to_owned()),
                detail: None,
                extra: BTreeMap::new(),
            }],
            extra: BTreeMap::new(),
        }
    }

    fn dispatch(id: &str, harness: &str, purpose: Option<&str>, stdout: &str) -> DispatchRecord {
        serde_json::from_value(json!({
            "id": id,
            "session": "review-session",
            "task": "T0001",
            "actor": "brain",
            "harness": harness,
            "command_line": format!("{harness} review"),
            "cwd": "/tmp/review-worktree",
            "prompt": "brief",
            "purpose": purpose,
            "status": "confirmed",
            "execution_status": "succeeded",
            "exit_code": 0,
            "stdout": stdout,
            "stderr": "",
            "warnings": [],
            "usage": {
                "input": 10,
                "output": 5,
                "cache_read": 0,
                "total": 15,
                "cost_usd": 0.001
            },
            "created_at": "2026-07-28T00:00:00Z",
            "updated_at": "2026-07-28T00:01:00Z"
        }))
        .expect("dispatch fixture")
    }

    #[test]
    fn reviewer_selection_prefers_diversity_and_names_self_review_honestly() {
        let two = vec!["hermes".to_owned(), "codex".to_owned()];
        assert_eq!(
            choose_reviewer("hermes", Some("hermes"), &two).unwrap(),
            ReviewerSelection {
                reviewer: "codex".to_owned(),
                self_review: false,
            }
        );
        assert_eq!(
            choose_reviewer("hermes", None, &["hermes".to_owned()]).unwrap(),
            ReviewerSelection {
                reviewer: "hermes".to_owned(),
                self_review: true,
            }
        );
    }

    #[test]
    fn report_json_persists_one_evidenced_verdict_per_check() {
        let _guard = lock();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let home =
            std::env::temp_dir().join(format!("orc-final-report-{}-{nonce}", std::process::id()));
        // SAFETY: this test serializes the process-wide registry root.
        unsafe { std::env::set_var("ORC_HOME", &home) };
        let task = contracted_task();
        atomic_write_json(&task_path(&task.session, &task.id), &task).unwrap();
        let review_json = json!({
            "verdicts": [
                {
                    "check": "main stays untouched",
                    "verdict": "pass",
                    "evidence": "git status --short produced no output"
                },
                {
                    "check": "tests pass",
                    "verdict": "pass",
                    "evidence": "cargo test --workspace: ok"
                }
            ]
        })
        .to_string();
        let executor = dispatch("D-executor", "hermes", None, "implementation complete");
        let reviewer = dispatch("D-reviewer", "codex", Some("review"), &review_json);
        let (saved_task, report) = persist_report(
            &task.session,
            &task.id,
            &reviewer,
            &[reviewer.clone(), executor],
            TaskActor::Brain,
        )
        .unwrap();
        assert_eq!(report.verdicts.len(), 2);
        assert!(
            report
                .verdicts
                .iter()
                .all(|verdict| verdict.verdict == "pass")
        );
        assert_eq!(report.review_mode, "independent");
        assert_eq!(report.usage.total, Some(30));
        let stored = read_report(&task.session, &task.id).unwrap();
        assert_eq!(stored, report);
        assert_eq!(saved_task.report.as_ref().unwrap().verdicts.len(), 2);
        let raw = fs::read_to_string(report_path(&task.session, &task.id)).unwrap();
        assert!(raw.contains("\"main stays untouched\""));
        assert!(raw.contains("\"tests pass\""));
        let _ = fs::remove_dir_all(home);
    }
}
