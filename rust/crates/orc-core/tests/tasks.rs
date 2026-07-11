#![allow(unsafe_code)]

use std::fs;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessRegistry, create_session, write_harness_registry};
use orc_core::registry::atomic_write_json;
use orc_core::tasks::{
    NewTask, TaskActor, TaskStatus, add_task, assign_task, done_task, list_tasks, move_task,
    read_task, review_task, start_task, task_path,
};
use serde_json::json;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn fresh_home() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("orc-task-core-{}-{nonce}", std::process::id()))
}

#[test]
fn tasks_are_atomic_additive_actor_attributed_and_dependency_safe() {
    let _guard = lock();
    let home = fresh_home();
    // SAFETY: this test serializes the process-wide registry root.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    write_harness_registry(&HarnessRegistry::default()).unwrap();
    let session = create_session(
        "codex",
        &["pi-m3".to_owned()],
        std::env::temp_dir().as_path(),
    )
    .unwrap();

    let prerequisite = add_task(
        &session.id,
        TaskActor::Human,
        NewTask {
            title: "prepare fixtures".to_owned(),
            ..NewTask::default()
        },
    )
    .unwrap();
    let dependent = add_task(
        &session.id,
        TaskActor::Brain,
        NewTask {
            title: "review fixtures".to_owned(),
            depends_on: vec![prerequisite.id.clone()],
            isolate: true,
            ..NewTask::default()
        },
    )
    .unwrap();
    assert_eq!(dependent.worktree.as_ref().unwrap().state, "requested");
    assert!(
        assign_task(
            &session.id,
            &dependent.id,
            "pi-m3".to_owned(),
            Some("P-7".to_owned()),
            TaskActor::Brain,
        )
        .is_ok()
    );
    assert!(start_task(&session.id, &dependent.id, TaskActor::Brain).is_err());

    let mut saved = read_task(&session.id, &prerequisite.id).unwrap();
    saved
        .extra
        .insert("future_field".to_owned(), json!({"kept": true}));
    atomic_write_json(&task_path(&session.id, &saved.id), &saved).unwrap();
    assign_task(
        &session.id,
        &prerequisite.id,
        "hermes".to_owned(),
        None,
        TaskActor::Human,
    )
    .unwrap();
    start_task(&session.id, &prerequisite.id, TaskActor::Human).unwrap();
    review_task(&session.id, &prerequisite.id, TaskActor::Human).unwrap();
    done_task(&session.id, &prerequisite.id, TaskActor::Human).unwrap();
    assert_eq!(
        read_task(&session.id, &prerequisite.id).unwrap().extra["future_field"],
        json!({"kept": true})
    );

    let running = start_task(&session.id, &dependent.id, TaskActor::Brain).unwrap();
    assert_eq!(running.status, TaskStatus::Running.as_str());
    assert_eq!(running.history.last().unwrap().actor, "brain");
    assert!(
        move_task(
            &session.id,
            &dependent.id,
            TaskStatus::Done,
            TaskActor::Human
        )
        .is_err()
    );

    let mut joins = Vec::new();
    for number in 0..12 {
        let session_id = session.id.clone();
        joins.push(thread::spawn(move || {
            add_task(
                &session_id,
                TaskActor::Human,
                NewTask {
                    title: format!("parallel {number}"),
                    ..NewTask::default()
                },
            )
            .unwrap()
            .id
        }));
    }
    let mut ids = joins
        .into_iter()
        .map(|join| join.join().unwrap())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 12);
    assert_eq!(list_tasks(&session.id).unwrap().len(), 14);

    fs::write(
        home.join("tasks").join(&session.id).join("corrupt.json"),
        "not json",
    )
    .unwrap();
    assert_eq!(list_tasks(&session.id).unwrap().len(), 14);
    assert!(
        add_task(
            &session.id,
            TaskActor::Human,
            NewTask {
                title: "must notice corruption".to_owned(),
                ..NewTask::default()
            },
        )
        .is_err()
    );
    let _ = fs::remove_dir_all(home);
}
