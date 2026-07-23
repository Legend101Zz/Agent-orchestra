//! Acceptance-driven task contract v2 and its dispatch brief.
//!
//! A [`TaskContract`] captures everything a delegated task carries beyond its
//! title: the objective, the allowed files or directories, forbidden actions,
//! the expected artifact, the acceptance checks, the bounded execution limits,
//! a reviewer, and a token or monetary budget. Dependencies are not stored
//! here — the owning [`Task`] already tracks them in its
//! validated `depends_on` graph, so the brief reads them from there instead of
//! keeping a second copy that could drift.
//!
//! The types derive both `serde` and `schemars` `JsonSchema`. serde gives the
//! durable additive JSON representation (every struct keeps an `extra` map so
//! unknown future fields survive a read→write cycle); the same derive is the
//! single schema source that later powers the normalized MCP tool surface.
//!
//! [`render_brief`] turns a contract into the plain-text brief handed to a
//! worker. It degrades honestly: a task with no contract still produces every
//! section header, each marked `(none specified)` rather than hidden, so a
//! reader can never mistake an empty contract for a satisfied one.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tasks::Task;

/// Bounded execution limits for one delegated task attempt.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct TaskLimits {
    /// Bounded wall-clock timeout in seconds for a single attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    /// Maximum automatic retries permitted after a failed attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    /// Unknown future fields, preserved across a read→write cycle.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl TaskLimits {
    /// Whether no limit is set, so the brief can mark the section honestly.
    #[must_use]
    pub fn is_unset(&self) -> bool {
        self.timeout_sec.is_none() && self.max_retries.is_none() && self.extra.is_empty()
    }
}

/// Token or monetary ceiling for one delegated task.
///
/// Money is stored as whole US cents (an integer) to keep the durable record
/// exact and comparable; no currency math ever rounds a float.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct TaskBudget {
    /// Maximum worker tokens the task may spend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Maximum spend in whole US cents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_usd_cents: Option<u64>,
    /// Unknown future fields, preserved across a read→write cycle.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl TaskBudget {
    /// Whether no budget is set, so the brief can mark the section honestly.
    #[must_use]
    pub fn is_unset(&self) -> bool {
        self.max_tokens.is_none() && self.max_usd_cents.is_none() && self.extra.is_empty()
    }
}

/// Acceptance-driven contract carried by a delegated task.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct TaskContract {
    /// One paragraph: what exists when this task is done that does not now.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub objective: String,
    /// Files or directories this task may create or modify. Everything else
    /// is forbidden by omission.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_paths: Vec<String>,
    /// Explicit no-go zones or actions (for example "no new dependencies").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden: Vec<String>,
    /// What the task produces: a branch, a document, a recorded measurement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_artifact: Option<String>,
    /// Numbered, independently verifiable acceptance checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_checks: Vec<String>,
    /// Who or what reviews the delivered artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    /// Timeout and retry policy for one attempt.
    #[serde(default, skip_serializing_if = "TaskLimits::is_unset")]
    pub limits: TaskLimits,
    /// Token or monetary ceiling for the task.
    #[serde(default, skip_serializing_if = "TaskBudget::is_unset")]
    pub budget: TaskBudget,
    /// Unknown future fields, preserved across a read→write cycle.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl TaskContract {
    /// Whether every contract field is empty, so a caller can avoid attaching
    /// a hollow contract to a task.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objective.trim().is_empty()
            && self.allowed_paths.is_empty()
            && self.forbidden.is_empty()
            && self.expected_artifact.is_none()
            && self.acceptance_checks.is_empty()
            && self.reviewer.is_none()
            && self.limits.is_unset()
            && self.budget.is_unset()
            && self.extra.is_empty()
    }

    /// Trim surrounding whitespace and drop blank list entries in place so the
    /// durable record and the rendered brief never carry accidental padding.
    pub fn normalize(&mut self) {
        self.objective = self.objective.trim().to_owned();
        normalize_list(&mut self.allowed_paths);
        normalize_list(&mut self.forbidden);
        normalize_list(&mut self.acceptance_checks);
        normalize_opt(&mut self.expected_artifact);
        normalize_opt(&mut self.reviewer);
    }

    /// Enforce the minimum an acceptance-driven contract must state.
    ///
    /// A contract that names *any* field must state an objective and at least
    /// one acceptance check — a delegated task with nothing to accept is not a
    /// contract. This runs only at construction time; loading an older or
    /// partial durable record never re-validates, so history stays readable.
    pub fn validate(&self) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        if self.objective.trim().is_empty() {
            bail!("task contract requires an --objective")
        }
        if self.acceptance_checks.is_empty() {
            bail!("task contract requires at least one --check (acceptance check)")
        }
        Ok(())
    }
}

fn normalize_list(values: &mut Vec<String>) {
    values.iter_mut().for_each(|value| {
        *value = value.trim().to_owned();
    });
    values.retain(|value| !value.is_empty());
}

fn normalize_opt(value: &mut Option<String>) {
    if let Some(inner) = value {
        let trimmed = inner.trim().to_owned();
        *value = (!trimmed.is_empty()).then_some(trimmed);
    }
}

/// Render the plain-text dispatch brief for one task.
///
/// The brief is the artifact handed to a worker. Every section is always
/// present; an unset section reads `(none specified)` so a worker can never
/// confuse "no constraint recorded" with "constraint satisfied". Contract
/// text is reproduced verbatim — the objective, each allowed path, each
/// forbidden action, and each acceptance check appear exactly as stored.
#[must_use]
pub fn render_brief(task: &Task) -> String {
    let contract = task.contract.clone().unwrap_or_default();
    let mut brief = String::new();
    // Header: the stable identity a worker echoes back on delivery.
    let _ = writeln!(brief, "# Task {} — {}", task.id, task.title);
    let _ = writeln!(brief, "session: {}", task.session);
    let _ = writeln!(brief, "status: {}", task.status);
    brief.push('\n');

    section_text(&mut brief, "Objective", &contract.objective);
    if !task.description.trim().is_empty() {
        section_text(&mut brief, "Description", &task.description);
    }
    section_bullets(&mut brief, "Allowed paths", &contract.allowed_paths);
    section_bullets(&mut brief, "Forbidden", &contract.forbidden);
    section_bullets(&mut brief, "Dependencies", &task.depends_on);
    section_opt(
        &mut brief,
        "Expected artifact",
        contract.expected_artifact.as_deref(),
    );
    section_numbered(&mut brief, "Acceptance checks", &contract.acceptance_checks);
    section_text(&mut brief, "Limits", &render_limits(&contract.limits));
    section_opt(&mut brief, "Reviewer", contract.reviewer.as_deref());
    section_text(&mut brief, "Budget", &render_budget(&contract.budget));

    brief
}

fn heading(brief: &mut String, title: &str) {
    let _ = writeln!(brief, "## {title}");
}

fn section_text(brief: &mut String, title: &str, body: &str) {
    heading(brief, title);
    if body.trim().is_empty() {
        let _ = writeln!(brief, "(none specified)");
    } else {
        let _ = writeln!(brief, "{body}");
    }
    brief.push('\n');
}

fn section_opt(brief: &mut String, title: &str, body: Option<&str>) {
    section_text(brief, title, body.unwrap_or_default());
}

fn section_bullets(brief: &mut String, title: &str, items: &[String]) {
    heading(brief, title);
    if items.is_empty() {
        let _ = writeln!(brief, "(none specified)");
    } else {
        for item in items {
            let _ = writeln!(brief, "- {item}");
        }
    }
    brief.push('\n');
}

fn section_numbered(brief: &mut String, title: &str, items: &[String]) {
    heading(brief, title);
    if items.is_empty() {
        let _ = writeln!(brief, "(none specified)");
    } else {
        for (index, item) in items.iter().enumerate() {
            let _ = writeln!(brief, "{}. {item}", index + 1);
        }
    }
    brief.push('\n');
}

fn render_limits(limits: &TaskLimits) -> String {
    if limits.is_unset() {
        return String::new();
    }
    let mut parts = Vec::new();
    if let Some(timeout) = limits.timeout_sec {
        parts.push(format!("timeout {timeout}s"));
    }
    if let Some(retries) = limits.max_retries {
        parts.push(format!("max {retries} retries"));
    }
    parts.join(" · ")
}

fn render_budget(budget: &TaskBudget) -> String {
    if budget.is_unset() {
        return String::new();
    }
    let mut parts = Vec::new();
    if let Some(tokens) = budget.max_tokens {
        parts.push(format!("{tokens} tokens"));
    }
    if let Some(cents) = budget.max_usd_cents {
        parts.push(format!("${}.{:02}", cents / 100, cents % 100));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::Task;

    fn contracted_task() -> Task {
        Task {
            id: "T0007".to_owned(),
            session: "bench-demo".to_owned(),
            title: "Ship the widget".to_owned(),
            description: String::new(),
            status: "backlog".to_owned(),
            depends_on: vec!["T0003".to_owned()],
            assignee: None,
            assignee_run: None,
            worktree: None,
            contract: Some(TaskContract {
                objective: "A working widget exists.".to_owned(),
                allowed_paths: vec!["src/widget.rs".to_owned()],
                forbidden: vec!["no new dependencies".to_owned()],
                expected_artifact: Some("branch with code + tests".to_owned()),
                acceptance_checks: vec!["widget renders".to_owned(), "tests pass".to_owned()],
                reviewer: Some("claude".to_owned()),
                limits: TaskLimits {
                    timeout_sec: Some(600),
                    max_retries: Some(2),
                    extra: BTreeMap::new(),
                },
                budget: TaskBudget {
                    max_tokens: Some(50_000),
                    max_usd_cents: Some(250),
                    extra: BTreeMap::new(),
                },
                extra: BTreeMap::new(),
            }),
            created_at: "2026-07-24T00:00:00Z".to_owned(),
            updated_at: "2026-07-24T00:00:00Z".to_owned(),
            history: Vec::new(),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn brief_reproduces_every_contract_section_verbatim() {
        let task = contracted_task();
        let brief = render_brief(&task);
        for needle in [
            "# Task T0007 — Ship the widget",
            "## Objective",
            "A working widget exists.",
            "## Allowed paths",
            "- src/widget.rs",
            "## Forbidden",
            "- no new dependencies",
            "## Dependencies",
            "- T0003",
            "## Expected artifact",
            "branch with code + tests",
            "## Acceptance checks",
            "1. widget renders",
            "2. tests pass",
            "## Limits",
            "timeout 600s · max 2 retries",
            "## Reviewer",
            "claude",
            "## Budget",
            "50000 tokens · $2.50",
        ] {
            assert!(brief.contains(needle), "brief missing {needle:?}:\n{brief}");
        }
    }

    #[test]
    fn brief_marks_missing_sections_without_hiding_them() {
        let mut task = contracted_task();
        task.contract = None;
        task.depends_on.clear();
        let brief = render_brief(&task);
        for header in [
            "## Objective",
            "## Allowed paths",
            "## Forbidden",
            "## Dependencies",
            "## Expected artifact",
            "## Acceptance checks",
            "## Limits",
            "## Reviewer",
            "## Budget",
        ] {
            assert!(brief.contains(header), "missing header {header}");
        }
        assert!(brief.contains("(none specified)"));
    }

    #[test]
    fn validate_requires_objective_and_a_check_once_any_field_is_set() {
        assert!(TaskContract::default().validate().is_ok());
        let only_objective = TaskContract {
            objective: "do a thing".to_owned(),
            ..TaskContract::default()
        };
        assert!(only_objective.validate().is_err());
        let full = TaskContract {
            objective: "do a thing".to_owned(),
            acceptance_checks: vec!["it is done".to_owned()],
            ..TaskContract::default()
        };
        assert!(full.validate().is_ok());
    }

    #[test]
    fn normalize_trims_and_drops_blank_entries() {
        let mut contract = TaskContract {
            objective: "  spaced  ".to_owned(),
            allowed_paths: vec!["  src/ ".to_owned(), "   ".to_owned()],
            acceptance_checks: vec![" check ".to_owned()],
            reviewer: Some("   ".to_owned()),
            ..TaskContract::default()
        };
        contract.normalize();
        assert_eq!(contract.objective, "spaced");
        assert_eq!(contract.allowed_paths, vec!["src/".to_owned()]);
        assert_eq!(contract.acceptance_checks, vec!["check".to_owned()]);
        assert_eq!(contract.reviewer, None);
    }

    #[test]
    fn contract_round_trips_through_additive_json_with_unknown_fields() {
        let mut contract = contracted_task().contract.unwrap();
        contract
            .extra
            .insert("future".to_owned(), Value::String("kept".to_owned()));
        let text = serde_json::to_string(&contract).unwrap();
        let restored: TaskContract = serde_json::from_str(&text).unwrap();
        assert_eq!(restored, contract);
        assert_eq!(restored.extra["future"], Value::String("kept".to_owned()));
    }

    #[test]
    fn json_schema_exposes_the_contract_surface() {
        let schema = schemars::schema_for!(TaskContract);
        let json = serde_json::to_value(&schema).unwrap();
        let properties = json
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        for field in [
            "objective",
            "allowed_paths",
            "forbidden",
            "expected_artifact",
            "acceptance_checks",
            "reviewer",
            "limits",
            "budget",
        ] {
            assert!(properties.contains_key(field), "schema missing {field}");
        }
    }
}
