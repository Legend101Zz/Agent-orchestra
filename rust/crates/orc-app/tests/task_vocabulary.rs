#![allow(unsafe_code)]
//! The seam the STAGE message animation hangs off: the words the task board
//! really writes, against the words the UI matches.
//!
//! This lives in its own integration target on purpose. It points `ORC_HOME`
//! at a temp root, and an env var is process-global — doing that from a unit
//! test would race every other test in `orc-app`'s single test binary. The
//! `allow` below is the same one every sibling test that repoints the store
//! carries (`orc-core/tests/bench.rs` and friends).
//!
//!
//! It exists because the first version of `circuit::message_for` matched four
//! action words nothing in the workspace writes, and missed completion
//! entirely: `move_task` records every status transition as `moved` and puts
//! the destination in `to`, so a finished task animated nothing. Every unit
//! test around it passed, because they all built their own history and shared
//! the same wrong assumption. Only the real API can catch that class of bug.

use std::sync::{Mutex, MutexGuard, OnceLock};

use orc_app::circuit::{Direction, Outcome, message_for};
use orc_core::bench::{BenchSession, write_session};
use orc_core::tasks::{
    NewTask, TaskActor, add_task, assign_task, done_task, drop_task, record_delivery, review_task,
    start_task,
};

/// `ORC_HOME` is process-global, so the tests that repoint it take turns.
/// Same pattern as `orc-core/tests/bench.rs`.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Point the store at a private root and put one bench session in it, which is
/// what the task board hangs off.
fn temp_session(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("orc-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create temp home");
    // SAFETY: guarded by `lock()`, so no other test in this binary is reading
    // the variable while it is being set.
    unsafe { std::env::set_var("ORC_HOME", &root) };
    write_session(&BenchSession {
        id: name.to_owned(),
        brain: "codex".to_owned(),
        workers: vec!["pi-m3".to_owned()],
        cwd: root.to_string_lossy().into_owned(),
        created_at: "2026-07-30T09:00:00Z".to_owned(),
        updated_at: "2026-07-30T09:00:00Z".to_owned(),
        base_repo: None,
        base_branch: None,
        base_commit: None,
        isolation: None,
        panes: Vec::new(),
        layout: Vec::new(),
        reorientation: None,
        extra: std::collections::BTreeMap::new(),
    })
    .expect("seed bench session");
    root
}

/// Every entry's classification, in order.
fn classify(task: &orc_core::tasks::Task) -> Vec<Option<(Direction, Outcome)>> {
    task.history
        .iter()
        .map(|entry| message_for(&entry.action, entry.to.as_deref()))
        .collect()
}

fn trace(task: &orc_core::tasks::Task) -> Vec<(String, Option<String>)> {
    task.history
        .iter()
        .map(|entry| (entry.action.clone(), entry.to.clone()))
        .collect()
}

#[test]
fn a_real_task_lifecycle_animates_one_dispatch_out_and_one_confirmation_back() {
    let _guard = lock();
    let session = "vocab-happy";
    let root = temp_session(session);
    let actor = TaskActor::Brain;

    let task = add_task(
        session,
        actor,
        NewTask {
            title: "brief".to_owned(),
            ..Default::default()
        },
    )
    .expect("create");
    assign_task(
        session,
        &task.id,
        "pi-m3".to_owned(),
        Some("pane-1".to_owned()),
        actor,
    )
    .expect("assign");
    start_task(session, &task.id, actor).expect("start");
    review_task(session, &task.id, actor).expect("review");
    let finished = done_task(session, &task.id, actor).expect("done");

    assert_eq!(
        classify(&finished)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        vec![
            (Direction::Outbound, Outcome::Dispatched),
            (Direction::Inbound, Outcome::Confirmed),
        ],
        "create -> assign -> start -> review -> done is one packet out and one \
         back; the real history was {:?}",
        trace(&finished)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_confirmed_delivery_and_a_dropped_task_are_the_two_inbound_outcomes() {
    let _guard = lock();
    let session = "vocab-outcomes";
    let root = temp_session(session);
    let actor = TaskActor::Brain;

    let confirmed = add_task(
        session,
        actor,
        NewTask {
            title: "delivered".to_owned(),
            ..Default::default()
        },
    )
    .expect("create");
    assign_task(
        session,
        &confirmed.id,
        "pi-m3".to_owned(),
        Some("pane-1".to_owned()),
        actor,
    )
    .expect("assign");
    let delivered = record_delivery(
        session,
        &confirmed.id,
        actor,
        Some("run-1".to_owned()),
        "delivered".to_owned(),
    )
    .expect("record delivery");
    assert!(
        classify(&delivered)
            .into_iter()
            .flatten()
            .any(|message| message == (Direction::Inbound, Outcome::Confirmed)),
        "a confirmed delivery comes back confirmed; history was {:?}",
        trace(&delivered)
    );

    let dropped_task = add_task(
        session,
        actor,
        NewTask {
            title: "abandoned".to_owned(),
            ..Default::default()
        },
    )
    .expect("create");
    let dropped = drop_task(session, &dropped_task.id, actor).expect("drop");
    assert!(
        classify(&dropped)
            .into_iter()
            .flatten()
            .any(|message| message == (Direction::Inbound, Outcome::Failed)),
        "a dropped task comes back failed; history was {:?}",
        trace(&dropped)
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Every action word `orc-core` writes, and what STAGE does with it.
///
/// The whole table, not a sample — three separate wrong guesses at this
/// mapping got through before it was written down: matching words nothing
/// writes, missing completion because it is a `moved` transition rather than
/// an action, and missing `dropped` because it records its own word rather
/// than a transition. A new action in `orc-core` should land here as a
/// deliberate row.
type Row = (
    &'static str,
    Option<&'static str>,
    Option<(Direction, Outcome)>,
);

const VOCABULARY: &[Row] = &[
    // Outbound: the brief leaving the conductor.
    (
        "assigned",
        None,
        Some((Direction::Outbound, Outcome::Dispatched)),
    ),
    (
        "reassigned",
        None,
        Some((Direction::Outbound, Outcome::Dispatched)),
    ),
    // Inbound, confirmed.
    (
        "delivery_confirmed",
        None,
        Some((Direction::Inbound, Outcome::Confirmed)),
    ),
    (
        "review_delivery_confirmed",
        None,
        Some((Direction::Inbound, Outcome::Confirmed)),
    ),
    (
        "moved",
        Some("done"),
        Some((Direction::Inbound, Outcome::Confirmed)),
    ),
    // Inbound, failed.
    (
        "delivery_failed",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
    ),
    (
        "review_delivery_failed",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
    ),
    (
        "dropped",
        Some("dropped"),
        Some((Direction::Inbound, Outcome::Failed)),
    ),
    (
        "merge_conflict",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
    ),
    (
        "isolation_unavailable",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
    ),
    // Silent, each for a stated reason.
    ("created", Some("backlog"), None),
    ("isolated", None, None),
    ("report_persisted", None, None),
    ("delivery_queued", None, None),
    ("merged", None, None),
    ("moved", Some("running"), None),
    ("moved", Some("review"), None),
    ("moved", None, None),
];

#[test]
fn every_action_the_board_writes_has_a_decided_classification() {
    for (action, to, want) in VOCABULARY {
        assert_eq!(
            message_for(action, *to),
            *want,
            "action {action:?} to {to:?}"
        );
    }
}

#[test]
fn the_words_nothing_writes_are_not_matched() {
    // A rename in `orc-core` should surface as a failing test here rather than
    // as a silently dead animation. These four were matched by the first
    // version and are written nowhere in the workspace.
    for dead in ["dispatched", "delivery_started", "failed", "done"] {
        assert_eq!(
            message_for(dead, None),
            None,
            "{dead:?} is not an action this workspace writes"
        );
    }
}
