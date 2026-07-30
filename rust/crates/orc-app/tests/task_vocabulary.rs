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

// --- Issue #45 check 1: a delegation reaches the pane it is sitting in -------

/// A registry whose one worker is a shell script that exits immediately.
fn seated_registry(root: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("create bin");
    let script = bin.join("seated-worker.sh");
    std::fs::write(&script, "#!/bin/sh\necho 'hello from hermes'\nexit 0\n").expect("write worker");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut registry = orc_core::bench::HarnessRegistry::default();
    for config in registry.harnesses.values_mut() {
        config.roles.retain(|role| role == "brain");
    }
    registry.harnesses.insert(
        "seated-worker".to_owned(),
        orc_core::bench::HarnessConfig {
            command: "/bin/sh".to_owned(),
            args: vec![script.to_string_lossy().into_owned()],
            resume_args: Vec::new(),
            roles: vec!["worker".to_owned()],
            adapter: "seated-worker".to_owned(),
            dispatch_args: vec!["--oneshot".to_owned()],
            dispatch_uses_stdin: false,
            dispatch_timeout_sec: 30,
            extra: Default::default(),
        },
    );
    orc_core::bench::write_harness_registry(&registry).expect("write registry");
}

#[test]
fn a_delegation_into_a_seated_session_lands_on_the_pane_on_screen() {
    // The headline of issue #45, driven through the real `orch::delegate` the
    // CLI calls — not a hand-built board.
    //
    // The reported failure was that a conductor sitting in a pane created a
    // *second*, pane-less session and dispatched into it. That still worked,
    // invisibly: dispatch's headless fallback ran a background worker nobody
    // could see, the attached session's board never changed, and STAGE never
    // moved. What must be true instead is all of the following at once —
    // no new session, the seated pane selected without anyone naming it, and
    // a board whose history animates onto *that* pane.
    let _guard = lock();
    let root = temp_session("seated-delegation");
    seated_registry(&root);

    // Seat two workers, as `n` does. Only one matches the harness we delegate
    // to, so "it picked the right one" is a real claim and not a coin toss.
    let mut session = orc_core::bench::read_session("seated-delegation").expect("read session");
    for (id, harness, state) in [
        ("seated-delegation-brain", "claude", "running"),
        ("seated-delegation-worker-1", "other-worker", "running"),
        ("seated-delegation-worker-2", "seated-worker", "running"),
    ] {
        session.panes.push(orc_core::bench::SessionPaneRecord {
            id: id.to_owned(),
            harness: harness.to_owned(),
            role: if id.ends_with("brain") {
                "brain"
            } else {
                "worker"
            }
            .to_owned(),
            state: state.to_owned(),
            pid: None,
            down_at: None,
            extra: Default::default(),
        });
    }
    write_session(&session).expect("seat the panes");
    let before = orc_core::bench::list_sessions()
        .expect("list sessions")
        .len();

    let outcome = orc_core::orch::delegate(orc_core::orch::DelegateRequest {
        session: "seated-delegation".to_owned(),
        harness: "seated-worker".to_owned(),
        task: None,
        title: Some("say hello".to_owned()),
        description: None,
        depends_on: Vec::new(),
        isolate: false,
        contract: None,
        prompt: Some("reply with exactly \"hello from hermes\"".to_owned()),
        // Deliberately NOT naming a pane: selecting the seated worker without
        // being told which one is the whole affordance.
        pane: None,
        run: None,
        timeout_sec: None,
        actor: orc_core::orch::OrchActor::Brain,
    })
    .expect("delegate into the seated session");

    // 1. No second session. This is the regression that started it all.
    let after = orc_core::bench::list_sessions()
        .expect("list sessions")
        .len();
    assert_eq!(before, after, "delegating must not create a second session");

    // 2. The dispatch went to the seated pane whose harness matched.
    let dispatch = &outcome.dispatches[0];
    assert_eq!(
        dispatch.status, "confirmed",
        "delivery confirmed: {dispatch:?}"
    );
    assert_eq!(
        dispatch.pane_id.as_deref(),
        Some("seated-delegation-worker-2"),
        "dispatch selected the seated worker whose harness matched"
    );

    // 3. The board links the task to that pane, which is what STAGE aims at.
    //
    // Read from the durable board rather than the returned outcome, and poll
    // for it: the confirmation is written by the detached supervisor, so
    // `delegate` can return a snapshot taken a moment before it lands. That
    // is not a defect — STAGE re-reads the board on every snapshot and picks
    // the confirmation up on the next one — but a test that read the returned
    // value once would be racing the supervisor, and did.
    let task_id = outcome.tasks[0].id.clone();
    let mut task = orc_core::tasks::read_task("seated-delegation", &task_id).expect("read task");
    for _ in 0..100 {
        if task.assignee_run.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        task = orc_core::tasks::read_task("seated-delegation", &task_id).expect("read task");
    }
    assert_eq!(
        task.assignee_run.as_deref(),
        Some("seated-delegation-worker-2"),
        "the task's run linkage is the pane id, not a dispatch id: {:?}",
        task.history
    );

    // 4. That history animates: an outbound packet and an inbound confirm,
    //    which is exactly what `note_task_events` turns into two InFlights.
    let animated: Vec<(Direction, Outcome)> = task
        .history
        .iter()
        .filter_map(|entry| message_for(&entry.action, entry.to.as_deref()))
        .collect();
    assert!(
        animated.contains(&(Direction::Outbound, Outcome::Dispatched)),
        "the brief crossing to the worker animates: {:?}",
        task.history
    );
    assert!(
        animated.contains(&(Direction::Inbound, Outcome::Confirmed)),
        "and so does its confirmed receipt: {:?}",
        task.history
    );

    let _ = std::fs::remove_dir_all(&root);
}
