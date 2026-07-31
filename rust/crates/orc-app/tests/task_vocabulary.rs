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

use orc_app::circuit::{Direction, Lane, Outcome, lane_for, message_for};
use orc_core::bench::{BenchSession, write_session};
use orc_core::tasks::{
    NewTask, TaskActor, add_task, assign_task, done_task, drop_task, record_delivery,
    record_execution, review_task, start_task,
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
fn a_delivery_receipt_travels_out_and_only_a_finished_worker_comes_back() {
    // Issue #49's defect 1, at the vocabulary seam. `record_delivery` is
    // written the instant the worker *process starts* — its own detail says
    // "worker running" — so classifying it as a return meant the answer
    // appeared to arrive milliseconds after the brief left, always. The event
    // that genuinely means the answer arrived is `record_execution`, and the
    // two must be tellable apart by a reader that has only `(action, to)`.
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
        !classify(&delivered)
            .into_iter()
            .flatten()
            .any(|(direction, _)| direction == Direction::Inbound),
        "a brief being taken by a worker is not the answer coming back; \
         history was {:?}",
        trace(&delivered)
    );

    let finished = record_execution(session, &confirmed.id, actor, true, "answered".to_owned())
        .expect("record execution");
    assert_eq!(
        classify(&finished)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        vec![
            (Direction::Outbound, Outcome::Dispatched),
            (Direction::Outbound, Outcome::Confirmed),
            (Direction::Inbound, Outcome::Confirmed),
        ],
        "aimed, taken, answered — in that order and in those directions; \
         history was {:?}",
        trace(&finished)
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
/// `(action, to, classification, lane)`. The lane says which of a reviewed
/// task's two workers the entry is about, which is what `note_task_events`
/// reads to choose between `assignee_run` and `reviewer_run` (issue #51
/// defect 3). Silent rows carry a lane too: `lane_for` is total, and a word
/// STAGE does not animate still belongs to somebody.
type Row = (
    &'static str,
    Option<&'static str>,
    Option<(Direction, Outcome)>,
    Lane,
);

const VOCABULARY: &[Row] = &[
    // Outbound: the brief leaving the conductor.
    (
        "assigned",
        None,
        Some((Direction::Outbound, Outcome::Dispatched)),
        Lane::Executor,
    ),
    (
        "reassigned",
        None,
        Some((Direction::Outbound, Outcome::Dispatched)),
        Lane::Executor,
    ),
    // Outbound, landing: the worker took the brief and started. Written at
    // `command.spawn()`, which is why issue #49 reclassified it — it used to
    // be the return packet, and meant the answer appeared to come back
    // milliseconds after the brief left, for every dispatch ever made.
    (
        "delivery_confirmed",
        None,
        Some((Direction::Outbound, Outcome::Confirmed)),
        Lane::Executor,
    ),
    (
        "review_delivery_confirmed",
        None,
        Some((Direction::Outbound, Outcome::Confirmed)),
        Lane::Reviewer,
    ),
    // Inbound, confirmed: the worker finished and its answer is durable.
    (
        "execution_succeeded",
        None,
        Some((Direction::Inbound, Outcome::Confirmed)),
        Lane::Executor,
    ),
    (
        "review_execution_succeeded",
        None,
        Some((Direction::Inbound, Outcome::Confirmed)),
        Lane::Reviewer,
    ),
    (
        "moved",
        Some("done"),
        Some((Direction::Inbound, Outcome::Confirmed)),
        Lane::Executor,
    ),
    // Inbound, failed.
    (
        "execution_failed",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
        Lane::Executor,
    ),
    (
        "review_execution_failed",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
        Lane::Reviewer,
    ),
    (
        "delivery_failed",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
        Lane::Executor,
    ),
    (
        "review_delivery_failed",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
        Lane::Reviewer,
    ),
    (
        "dropped",
        Some("dropped"),
        Some((Direction::Inbound, Outcome::Failed)),
        Lane::Executor,
    ),
    (
        "merge_conflict",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
        Lane::Executor,
    ),
    (
        "isolation_unavailable",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
        Lane::Executor,
    ),
    // Issue #51 defect 2. `Failed` on the wire, and deliberately not
    // `execution_failed` on the board: a supervisor that died mid-flight took
    // the answer with it, so nobody knows whether the work succeeded. STAGE
    // has three outcome slots and this is the honest one of the three — what
    // came back is that nothing is coming back.
    (
        "execution_orphaned",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
        Lane::Executor,
    ),
    (
        "review_execution_orphaned",
        None,
        Some((Direction::Inbound, Outcome::Failed)),
        Lane::Reviewer,
    ),
    // Silent, each for a stated reason.
    ("created", Some("backlog"), None, Lane::Executor),
    ("isolated", None, None, Lane::Executor),
    ("report_persisted", None, None, Lane::Executor),
    ("delivery_queued", None, None, Lane::Executor),
    ("merged", None, None, Lane::Executor),
    ("moved", Some("running"), None, Lane::Executor),
    ("moved", Some("review"), None, Lane::Executor),
    ("moved", None, None, Lane::Executor),
];

#[test]
fn every_action_the_board_writes_has_a_decided_classification() {
    for (action, to, want, lane) in VOCABULARY {
        assert_eq!(
            message_for(action, *to),
            *want,
            "action {action:?} to {to:?}"
        );
        assert_eq!(lane_for(action), *lane, "action {action:?} lane");
    }
}

#[test]
fn every_word_the_reviewer_branch_writes_is_in_the_reviewer_lane() {
    // Issue #51 defect 3. `dispatch_supervisor` picks its reviewer branch with
    // `record.is_review()` and writes exactly these five words; every one of
    // them must aim at `reviewer_run` rather than at the executor's pane. A
    // sixth added in `orc-core` and forgotten here would silently fly down the
    // executor's wire again, which is the defect.
    for action in [
        "review_delivery_confirmed",
        "review_delivery_failed",
        "review_execution_succeeded",
        "review_execution_failed",
        "review_execution_orphaned",
    ] {
        assert_eq!(lane_for(action), Lane::Reviewer, "{action:?}");
        assert!(
            VOCABULARY.iter().any(|(word, ..)| *word == action),
            "{action:?} is missing from the vocabulary table"
        );
    }
    // And the executor's own words are not swept in by a prefix rule.
    for action in [
        "execution_orphaned",
        "execution_failed",
        "delivery_confirmed",
    ] {
        assert_eq!(lane_for(action), Lane::Executor, "{action:?}");
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
    let task = poll_task("seated-delegation", &task_id, |task| {
        task.history
            .iter()
            .any(|entry| entry.action == "execution_succeeded")
    });
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
        "and so does the answer coming back: {:?}",
        task.history
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Poll the durable board until `ready`, or fail with the history it saw.
///
/// The supervisor is a detached process, so everything it writes lands after
/// `delegate` returns. Polling is the honest shape; asserting once races it.
fn poll_task(
    session: &str,
    id: &str,
    ready: impl Fn(&orc_core::tasks::Task) -> bool,
) -> orc_core::tasks::Task {
    let mut task = orc_core::tasks::read_task(session, id).expect("read task");
    for _ in 0..200 {
        if ready(&task) {
            return task;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        task = orc_core::tasks::read_task(session, id).expect("read task");
    }
    panic!(
        "task {id} never reached the expected state; history was {trace:?}",
        trace = trace(&task)
    )
}

#[test]
fn a_real_dispatch_writes_delivery_then_completion_and_the_gap_is_the_worker() {
    // Issue #49, acceptance checks 1 and 2, driven end to end through the real
    // `orch::delegate` and a real worker process.
    //
    // Check 1: the success path appends durable task history when the worker
    // *finishes*, distinct from "spawned". Before this the board's last word
    // was `delivery_confirmed` — written from the `on_started` callback right
    // after `command.spawn()` — and `persist_terminal`'s success arm appended
    // nothing at all.
    //
    // Check 2: the return packet means the answer. The worker here sleeps for
    // a measured interval, and the assertion is that the two events are that
    // far apart on the wall clock rather than in the same millisecond. The
    // bound is deliberately loose in both directions: this repo has documented
    // storage-dependent wall-clock flakes, so the claim is "the gap is the
    // worker's own runtime", not a tight number.
    const WORKER_SLEEP: std::time::Duration = std::time::Duration::from_millis(1_500);

    let _guard = lock();
    let root = temp_session("timed-delegation");
    slow_registry(&root, WORKER_SLEEP);

    let mut session = orc_core::bench::read_session("timed-delegation").expect("read session");
    session.panes.push(orc_core::bench::SessionPaneRecord {
        id: "timed-delegation-worker-1".to_owned(),
        harness: "slow-worker".to_owned(),
        role: "worker".to_owned(),
        state: "running".to_owned(),
        pid: None,
        down_at: None,
        extra: Default::default(),
    });
    write_session(&session).expect("seat the worker");

    let started = std::time::Instant::now();
    let outcome = orc_core::orch::delegate(orc_core::orch::DelegateRequest {
        session: "timed-delegation".to_owned(),
        harness: "slow-worker".to_owned(),
        task: None,
        title: Some("take your time".to_owned()),
        description: None,
        depends_on: Vec::new(),
        isolate: false,
        contract: None,
        prompt: Some("think".to_owned()),
        pane: None,
        run: None,
        timeout_sec: None,
        actor: orc_core::orch::OrchActor::Brain,
    })
    .expect("delegate");
    let delegate_returned = started.elapsed();

    let task_id = outcome.tasks[0].id.clone();
    let delivered = poll_task("timed-delegation", &task_id, |task| {
        task.history
            .iter()
            .any(|entry| entry.action == "delivery_confirmed")
    });
    let delivery_seen = started.elapsed();
    let finished = poll_task("timed-delegation", &task_id, |task| {
        task.history
            .iter()
            .any(|entry| entry.action == "execution_succeeded")
    });
    let completion_seen = started.elapsed();

    // Check 1: both events are durable, in order, and mean different things.
    let words = trace(&finished)
        .into_iter()
        .map(|(action, _)| action)
        .collect::<Vec<_>>();
    let delivery_at = words
        .iter()
        .position(|action| action == "delivery_confirmed")
        .expect("the brief was handed over");
    let completion_at = words
        .iter()
        .position(|action| action == "execution_succeeded")
        .expect("and the worker finished");
    assert!(
        delivery_at < completion_at,
        "the worker finishes after it starts: {words:?}"
    );
    assert_eq!(
        message_for("delivery_confirmed", None),
        Some((Direction::Outbound, Outcome::Confirmed)),
        "and the two are distinguishable to the reader that animates them"
    );
    assert_eq!(
        message_for("execution_succeeded", None),
        Some((Direction::Inbound, Outcome::Confirmed))
    );
    assert!(
        delivered
            .history
            .iter()
            .all(|entry| entry.action != "execution_succeeded"),
        "the completion event is not written at spawn: {:?}",
        trace(&delivered)
    );

    // Check 2: the gap between them is the worker's own runtime, and
    // `delegate` returned long before either.
    assert!(
        completion_seen >= WORKER_SLEEP,
        "the answer cannot arrive before the worker has finished producing it \
         ({completion_seen:?} < {WORKER_SLEEP:?})"
    );
    assert!(
        delivery_seen + WORKER_SLEEP / 2 < completion_seen,
        "delivery and completion must not land together: delivery at \
         {delivery_seen:?}, completion at {completion_seen:?}"
    );
    assert!(
        delegate_returned < WORKER_SLEEP,
        "delegate confirms delivery, it does not wait for the answer \
         ({delegate_returned:?})"
    );
    println!(
        "delegate returned at {delegate_returned:?}; delivery_confirmed seen at \
         {delivery_seen:?}; execution_succeeded seen at {completion_seen:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// A registry whose one worker takes the brief and then dies.
fn failing_registry(root: &std::path::Path) {
    worker_registry(
        root,
        "failing-worker",
        "#!/bin/sh\necho 'something went wrong' >&2\nexit 3\n",
    );
}

/// A registry whose one worker sleeps for `runtime` before answering.
fn slow_registry(root: &std::path::Path, runtime: std::time::Duration) {
    worker_registry(
        root,
        "slow-worker",
        &format!(
            "#!/bin/sh\nsleep {}\necho 'done thinking'\nexit 0\n",
            runtime.as_secs_f32()
        ),
    );
}

/// Register `key` as the only worker, backed by `script`.
fn worker_registry(root: &std::path::Path, key: &str, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("create bin");
    let path = bin.join(format!("{key}.sh"));
    std::fs::write(&path, script).expect("write worker");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut registry = orc_core::bench::HarnessRegistry::default();
    for config in registry.harnesses.values_mut() {
        config.roles.retain(|role| role == "brain");
    }
    registry.harnesses.insert(
        key.to_owned(),
        orc_core::bench::HarnessConfig {
            command: "/bin/sh".to_owned(),
            args: vec![path.to_string_lossy().into_owned()],
            resume_args: Vec::new(),
            roles: vec!["worker".to_owned()],
            adapter: key.to_owned(),
            dispatch_args: vec!["--oneshot".to_owned()],
            dispatch_uses_stdin: false,
            dispatch_timeout_sec: 30,
            extra: Default::default(),
        },
    );
    orc_core::bench::write_harness_registry(&registry).expect("write registry");
}

#[test]
fn a_worker_that_dies_after_taking_the_brief_comes_back_failed() {
    // The other half of the completion event, and the branch of
    // `append_execution` no end-to-end test reached: the worker really ran, so
    // `delivery_failed` is not the story — it took the brief and then died.
    // The board has to say that, and STAGE has to fly it home as a failure
    // rather than leaving the task looking permanently in progress.
    let _guard = lock();
    let root = temp_session("failing-delegation");
    failing_registry(&root);

    let mut session = orc_core::bench::read_session("failing-delegation").expect("read session");
    session.panes.push(orc_core::bench::SessionPaneRecord {
        id: "failing-delegation-worker-1".to_owned(),
        harness: "failing-worker".to_owned(),
        role: "worker".to_owned(),
        state: "running".to_owned(),
        pid: None,
        down_at: None,
        extra: Default::default(),
    });
    write_session(&session).expect("seat the worker");

    let outcome = orc_core::orch::delegate(orc_core::orch::DelegateRequest {
        session: "failing-delegation".to_owned(),
        harness: "failing-worker".to_owned(),
        task: None,
        title: Some("this will fail".to_owned()),
        description: None,
        depends_on: Vec::new(),
        isolate: false,
        contract: None,
        prompt: Some("try".to_owned()),
        pane: None,
        run: None,
        timeout_sec: None,
        actor: orc_core::orch::OrchActor::Brain,
    })
    .expect("delegate");

    let task_id = outcome.tasks[0].id.clone();
    let failed = poll_task("failing-delegation", &task_id, |task| {
        task.history
            .iter()
            .any(|entry| entry.action == "execution_failed")
    });
    let words = trace(&failed)
        .into_iter()
        .map(|(action, _)| action)
        .collect::<Vec<_>>();
    assert!(
        words.contains(&"delivery_confirmed".to_owned()),
        "the brief was taken before it failed: {words:?}"
    );
    assert!(
        !words.contains(&"delivery_failed".to_owned()),
        "delivery did not fail; execution did: {words:?}"
    );
    assert_eq!(
        classify(&failed)
            .into_iter()
            .flatten()
            .last()
            .expect("something animates"),
        (Direction::Inbound, Outcome::Failed),
        "and it comes home as a failure: {words:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// --- Issue #51 defect 3: the reviewer's wire ---------------------------------

/// A registry with two capable workers of *different* adapter families, which
/// is what `report::choose_reviewer` requires before it will call a review
/// independent.
fn reviewed_registry(root: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("create bin");
    let mut registry = orc_core::bench::HarnessRegistry::default();
    for config in registry.harnesses.values_mut() {
        config.roles.retain(|role| role == "brain");
    }
    for (key, family) in [
        ("exec-worker", "exec-family"),
        ("review-worker", "review-family"),
    ] {
        let script = bin.join(format!("{key}.sh"));
        // The reviewer's brief asks for JSON verdicts; the executor's does not.
        // Both exit 0, which is what makes the dispatch `succeeded`.
        std::fs::write(
            &script,
            "#!/bin/sh\ncase \"$*\" in\n  *\"Acceptance checks:\"*)\n    echo '{\"verdicts\":[{\"check\":\"it exists\",\"verdict\":\"pass\",\"evidence\":\"fixture\"}]}'\n    ;;\n  *)\n    echo 'worker done'\n    ;;\nesac\nexit 0\n",
        )
        .expect("write worker");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        registry.harnesses.insert(
            key.to_owned(),
            orc_core::bench::HarnessConfig {
                command: "/bin/sh".to_owned(),
                args: vec![script.to_string_lossy().into_owned()],
                resume_args: Vec::new(),
                roles: vec!["worker".to_owned()],
                adapter: family.to_owned(),
                dispatch_args: vec!["--oneshot".to_owned()],
                dispatch_uses_stdin: false,
                dispatch_timeout_sec: 30,
                extra: Default::default(),
            },
        );
    }
    registry.default_workers = vec!["exec-worker".to_owned()];
    orc_core::bench::write_harness_registry(&registry).expect("write registry");
}

#[test]
fn a_real_review_links_the_reviewers_pane_and_leaves_the_executors_alone() {
    // Issue #51 defect 3, acceptance check 7, driven through the real
    // `orch::review` the CLI and the MCP server both call — not a hand-built
    // board.
    //
    // `record_review_delivery` used to take a bare `confirmed: bool` and throw
    // the reviewer's link away, on the reasoning that writing it would clobber
    // the executor's. It would have: there was only one linkage field. So the
    // task's single run link stayed the executor's pane, `note_task_events`
    // aimed every classified entry at it, and a reviewer seated in a different
    // pane had its brief and its verdict drawn crossing the executor's
    // connector.
    //
    // The board now records both, and this is what proves the two are really
    // different panes on a real dispatch rather than by construction in a test
    // fixture.
    let _guard = lock();
    let root = temp_session("reviewed-delegation");

    // A git repo for the session cwd: a contracted task isolates itself, and
    // `render_review_brief` refuses to build a brief without a worktree.
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "review@example.invalid"]);
    git(&["config", "user.name", "Review Test"]);
    std::fs::write(repo.join("README.md"), "fixture\n").expect("seed repo");
    git(&["add", "README.md"]);
    git(&["commit", "-m", "fixture"]);

    reviewed_registry(&root);
    let mut session = orc_core::bench::create_session(
        "claude",
        &["exec-worker".to_owned(), "review-worker".to_owned()],
        &repo,
    )
    .expect("create session");
    let session_id = session.id.clone();
    let executor_pane = format!("{session_id}-worker-1");
    let reviewer_pane = format!("{session_id}-worker-2");
    for (id, harness) in [
        (executor_pane.clone(), "exec-worker"),
        (reviewer_pane.clone(), "review-worker"),
    ] {
        session.panes.push(orc_core::bench::SessionPaneRecord {
            id,
            harness: harness.to_owned(),
            role: "worker".to_owned(),
            state: "running".to_owned(),
            pid: None,
            down_at: None,
            extra: Default::default(),
        });
    }
    write_session(&session).expect("seat both panes");

    let delegated = orc_core::orch::delegate(orc_core::orch::DelegateRequest {
        session: session_id.clone(),
        harness: "exec-worker".to_owned(),
        task: None,
        title: Some("do the work".to_owned()),
        description: None,
        depends_on: Vec::new(),
        isolate: true,
        contract: Some(orc_core::contract::TaskContract {
            objective: "The thing exists.".to_owned(),
            allowed_paths: vec!["README.md".to_owned()],
            forbidden: vec!["no new dependencies".to_owned()],
            acceptance_checks: vec!["it exists".to_owned()],
            // Pin the reviewer so "it picked a different pane" is a claim
            // about linkage, not about `choose_reviewer`'s preference order.
            reviewer: Some("review-worker".to_owned()),
            ..Default::default()
        }),
        prompt: Some("do the work".to_owned()),
        pane: None,
        run: None,
        timeout_sec: None,
        actor: orc_core::orch::OrchActor::Brain,
    })
    .expect("delegate a contracted task");
    let task_id = delegated.tasks[0].id.clone();

    // Wait on the executor's **dispatch record**, not on the board.
    //
    // `orch::review` gates on the record being terminal and `succeeded`, and
    // `append_execution` deliberately writes the board *before* `write_dispatch`
    // (#50, so that "the dispatch is terminal" implies "the board has been
    // told"). That ordering means the converse does not hold: there is a real
    // window where the board already says `execution_succeeded` and the record
    // is not terminal yet. Polling the board for it and then calling `review`
    // raced that window and failed 1 run in 10 with "executor has not completed
    // successfully" — a defect in this test, not in the code under it, and a
    // neat demonstration of the ordering guarantee's exact shape.
    //
    // `orch::await_delegation` is what a real conductor calls here, and it
    // waits on the thing `review` actually reads.
    let awaited = orc_core::orch::await_delegation(orc_core::orch::AwaitRequest {
        session: session_id.clone(),
        task: task_id.clone(),
        timeout_sec: Some(30),
        poll_interval_ms: Some(20),
    })
    .expect("await the executor");
    assert_eq!(
        awaited.dispatches[0].execution_status.as_deref(),
        Some("succeeded"),
        "the executor must really have finished before a review is dispatched: {:?}",
        awaited.dispatches[0]
    );

    let reviewed = orc_core::orch::review(orc_core::orch::TaskRef {
        session: session_id.clone(),
        task: task_id.clone(),
        actor: orc_core::orch::OrchActor::Brain,
    })
    .expect("review through the real verb");
    assert_eq!(
        reviewed.dispatches[0].pane_id.as_deref(),
        Some(reviewer_pane.as_str()),
        "the review dispatch selected the seated reviewer pane, unasked"
    );

    let task = poll_task(&session_id, &task_id, |task| {
        task.history
            .iter()
            .any(|entry| entry.action == "review_delivery_confirmed")
    });

    // 1. Two links, two panes.
    assert_eq!(
        task.assignee_run.as_deref(),
        Some(executor_pane.as_str()),
        "the executor's link is untouched by the review: {:?}",
        trace(&task)
    );
    assert_eq!(
        task.reviewer_run.as_deref(),
        Some(reviewer_pane.as_str()),
        "and the reviewer has one of its own: {:?}",
        trace(&task)
    );
    assert_ne!(
        task.assignee_run, task.reviewer_run,
        "a review that lands on the executor's pane is the whole defect"
    );

    // 2. Every review word is in the reviewer's lane, and nothing else is —
    //    which is what `note_task_events` reads to pick between the two links.
    for entry in &task.history {
        let expected = if entry.action.starts_with("review_") {
            Lane::Reviewer
        } else {
            Lane::Executor
        };
        assert_eq!(
            lane_for(&entry.action),
            expected,
            "action {:?} belongs to the {:?} lane",
            entry.action,
            expected
        );
    }
    assert!(
        task.history
            .iter()
            .any(|entry| lane_for(&entry.action) == Lane::Reviewer
                && message_for(&entry.action, entry.to.as_deref()).is_some()),
        "a real review really does produce animated reviewer traffic: {:?}",
        trace(&task)
    );

    let _ = std::fs::remove_dir_all(&root);
}
