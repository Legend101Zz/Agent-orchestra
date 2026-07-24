#![allow(unsafe_code)]

//! Quota guard v2 integration tests (issue #7).
//!
//! Exercises the whole dispatch path with an isolated `ORC_HOME` and shell
//! fixtures (never a real provider):
//!
//! * **AC1** — with a harness at its concurrency cap, an extra dispatch is
//!   *queued* (visible state) and no worker is spawned; draining after a slot
//!   frees runs the same record to `confirmed`.
//! * **AC2** — a fixture that emits rate-limit output triggers exponential
//!   backoff and a user-visible ORC WARNING; a persistently limited fixture
//!   fails `rate_limited` after the backoff budget.
//! * **AC3** — the per-harness cap is read from `harnesses.json` and enforced
//!   across independent sessions.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use orc_core::bench::{
    BenchSession, HarnessConfig, HarnessRegistry, create_session, write_harness_registry,
};
use orc_core::dispatch::{
    DispatchActor, DispatchRequest, dispatch_with_policy, drain_queued_with_policy, list_dispatches,
};
use orc_core::ratelimit::BackoffPolicy;
use orc_core::spawn_guard::{DEFAULT_LEASE_TTL, acquire_slot};
use orc_core::tasks::{NewTask, TaskActor, TaskStatus, add_task, assign_task, start_task};

fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn fresh_home(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("orc-quota2-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&dir).expect("create home");
    // SAFETY: ORC_HOME mutation is serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &dir) };
    dir
}

/// Write an executable `/bin/sh` fixture into `home` and return its path.
fn write_script(home: &Path, name: &str, body: &str) -> String {
    let path = home.join(name);
    fs::write(&path, body).expect("write fixture script");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
    path.to_string_lossy().into_owned()
}

/// A `/bin/sh <script> --oneshot <prompt>` worker (explicit-override dispatch).
fn worker(script: &str, adapter: &str) -> HarnessConfig {
    HarnessConfig {
        command: "/bin/sh".to_owned(),
        args: vec![script.to_owned()],
        resume_args: Vec::new(),
        roles: vec!["worker".to_owned()],
        adapter: adapter.to_owned(),
        dispatch_args: vec!["--oneshot".to_owned()],
        dispatch_uses_stdin: false,
        dispatch_timeout_sec: 30,
        extra: BTreeMap::new(),
    }
}

fn running_task(session: &str, title: &str, assignee: &str) -> String {
    let task = add_task(
        session,
        TaskActor::Brain,
        NewTask {
            title: title.to_owned(),
            ..NewTask::default()
        },
    )
    .expect("add task");
    assign_task(
        session,
        &task.id,
        assignee.to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign task");
    let running = start_task(session, &task.id, TaskActor::Brain).expect("start task");
    assert_eq!(running.status, TaskStatus::Running.as_str());
    running.id
}

fn session_in(home: &Path, label: &str, workers: &[String]) -> BenchSession {
    let cwd = home.join(format!("cwd-{label}"));
    fs::create_dir_all(&cwd).expect("create cwd");
    create_session("codex", workers, &cwd).expect("create session")
}

fn request(session: &str, task: &str, harness: &str, prompt: &str) -> DispatchRequest {
    DispatchRequest {
        session: session.to_owned(),
        task: task.to_owned(),
        actor: DispatchActor::Brain,
        harness: harness.to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: prompt.to_owned(),
        timeout_sec: Some(30),
    }
}

// ---------------------------------------------------------------------------
// AC1 — the extra worker is queued, never spawned.
// ---------------------------------------------------------------------------

#[test]
fn dispatch_beyond_the_cap_is_queued_and_no_worker_is_spawned() {
    let _guard = lock();
    let home = fresh_home("queue");
    let sentinel = home.join("worker-ran.marker");
    let script = write_script(
        &home,
        "sentinel-worker.sh",
        &format!(
            "#!/bin/sh\necho ran > '{}'\necho 'sentinel receipt: done'\nexit 0\n",
            sentinel.display()
        ),
    );
    let mut registry = HarnessRegistry::default();
    registry
        .harnesses
        .insert("capped".to_owned(), worker(&script, "hermes"));
    // AC3 seed: the cap is configured in the registry, not the adapter default.
    registry.concurrency.insert("capped".to_owned(), 1);
    write_harness_registry(&registry).expect("write registry");

    let session = session_in(&home, "queue", &["capped".to_owned()]);
    let task = running_task(&session.id, "queued dispatch", "capped");

    // One worker is already "in flight", holding the single slot.
    let held = acquire_slot("capped", 1, DEFAULT_LEASE_TTL, None)
        .expect("acquire held slot")
        .expect("the only slot is free to hold");

    // The N+1th request must be QUEUED — recorded, visible, but not spawned.
    let queued = dispatch_with_policy(
        &request(&session.id, &task, "capped", "summarize the diff"),
        &BackoffPolicy::fast(),
    )
    .expect("dispatch persists a queued record");
    assert!(queued.is_queued(), "expected queued, got {}", queued.status);
    assert!(
        !sentinel.exists(),
        "no worker may be spawned while the cap is full"
    );
    assert!(
        queued
            .warnings
            .iter()
            .any(|line| line.contains("concurrency cap")),
        "queued record carries a cap warning: {:?}",
        queued.warnings
    );
    // The queue is visible via the normal listing path.
    let listed = list_dispatches(&session.id).expect("list dispatches");
    assert!(
        listed
            .iter()
            .any(|record| record.id == queued.id && record.is_queued())
    );

    // Freeing the slot and draining runs the SAME record to confirmed.
    drop(held);
    let drained =
        drain_queued_with_policy(&session.id, &BackoffPolicy::fast()).expect("drain queue");
    assert_eq!(drained.len(), 1, "exactly one record drained");
    assert!(drained[0].is_confirmed(), "drained record is confirmed");
    assert_eq!(drained[0].id, queued.id, "queued record became confirmed");
    assert!(sentinel.exists(), "the worker ran once the slot freed");
    let _ = fs::remove_dir_all(home);
}

// ---------------------------------------------------------------------------
// AC2 — rate-limit output triggers backoff + a user-visible warning.
// ---------------------------------------------------------------------------

#[test]
fn a_rate_limited_worker_backs_off_then_confirms_with_a_warning() {
    let _guard = lock();
    let home = fresh_home("ratelimit-recover");
    let counter = home.join("attempts.count");
    // Rate-limits (429 on stderr, exit 1) until the 3rd attempt, then succeeds.
    let script = write_script(
        &home,
        "flaky-worker.sh",
        &format!(
            r#"#!/bin/sh
count_file='{counter}'
n=0
[ -f "$count_file" ] && n=$(cat "$count_file")
n=$((n + 1))
echo "$n" > "$count_file"
if [ "$n" -lt 3 ]; then
  echo "HTTP 429 Too Many Requests: retry-after: 1" 1>&2
  exit 1
fi
echo "flaky worker receipt: done on attempt $n"
exit 0
"#,
            counter = counter.display()
        ),
    );
    let mut registry = HarnessRegistry::default();
    registry
        .harnesses
        .insert("flaky".to_owned(), worker(&script, "hermes"));
    write_harness_registry(&registry).expect("write registry");
    let session = session_in(&home, "recover", &["flaky".to_owned()]);
    let task = running_task(&session.id, "rate limited then ok", "flaky");

    let record = dispatch_with_policy(
        &request(&session.id, &task, "flaky", "do the work"),
        &BackoffPolicy::fast(),
    )
    .expect("dispatch resolves");

    assert!(
        record.is_confirmed(),
        "must recover after backoff: status={} error={:?}",
        record.status,
        record.error
    );
    let attempts: u32 = fs::read_to_string(&counter)
        .expect("counter file")
        .trim()
        .parse()
        .expect("counter is numeric");
    assert_eq!(attempts, 3, "two rate-limited attempts then a success");
    assert_eq!(
        record.warnings.len(),
        2,
        "one ORC WARNING per backoff sleep: {:?}",
        record.warnings
    );
    assert!(
        record
            .warnings
            .iter()
            .all(|line| line.contains("ORC WARNING") && line.contains("rate-limited")),
        "warnings use the ORC WARNING channel: {:?}",
        record.warnings
    );
    // The parsed retry-after hint is surfaced to the operator.
    assert!(
        record
            .warnings
            .iter()
            .any(|line| line.contains("asked for")),
        "retry-after hint surfaced: {:?}",
        record.warnings
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_persistently_rate_limited_worker_fails_rate_limited_after_the_budget() {
    let _guard = lock();
    let home = fresh_home("ratelimit-exhaust");
    let counter = home.join("attempts.count");
    let script = write_script(
        &home,
        "always-limited.sh",
        &format!(
            r#"#!/bin/sh
count_file='{counter}'
n=0
[ -f "$count_file" ] && n=$(cat "$count_file")
n=$((n + 1))
echo "$n" > "$count_file"
echo "error: rate limit exceeded (429)" 1>&2
exit 1
"#,
            counter = counter.display()
        ),
    );
    let mut registry = HarnessRegistry::default();
    registry
        .harnesses
        .insert("stuck".to_owned(), worker(&script, "hermes"));
    write_harness_registry(&registry).expect("write registry");
    let session = session_in(&home, "exhaust", &["stuck".to_owned()]);
    let task = running_task(&session.id, "always limited", "stuck");

    let record = dispatch_with_policy(
        &request(&session.id, &task, "stuck", "keep trying"),
        &BackoffPolicy::fast(),
    )
    .expect("dispatch persists a failed record");

    assert_eq!(record.status, "failed");
    assert_eq!(
        record.failure_kind.as_deref(),
        Some("rate_limited"),
        "a persistently limited worker fails as rate_limited"
    );
    assert!(
        !record.warnings.is_empty(),
        "backoff still warned before giving up: {:?}",
        record.warnings
    );
    let attempts: u32 = fs::read_to_string(&counter)
        .expect("counter file")
        .trim()
        .parse()
        .expect("counter is numeric");
    // fast() allows 3 retries → 4 total attempts.
    assert_eq!(attempts, 4, "the whole backoff budget was spent");
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_successful_worker_whose_output_mentions_a_rate_limit_is_confirmed_once() {
    // Reviewer Fix 1 regression: detection must fire only on a *non-success*
    // exit. A coding worker that finishes cleanly (exit 0) but whose output
    // merely mentions a signal substring — here this PR's own diff summary —
    // must be confirmed on the FIRST attempt, never retried or mislabeled.
    let _guard = lock();
    let home = fresh_home("success-mentions-limit");
    let counter = home.join("attempts.count");
    let script = write_script(
        &home,
        "mentions-limit.sh",
        &format!(
            r#"#!/bin/sh
count_file='{counter}'
n=0
[ -f "$count_file" ] && n=$(cat "$count_file")
n=$((n + 1))
echo "$n" > "$count_file"
echo "summary: added quota guard v2 with rate limit backoff and 429 handling"
exit 0
"#,
            counter = counter.display()
        ),
    );
    let mut registry = HarnessRegistry::default();
    registry
        .harnesses
        .insert("clean".to_owned(), worker(&script, "hermes"));
    write_harness_registry(&registry).expect("write registry");
    let session = session_in(&home, "clean", &["clean".to_owned()]);
    let task = running_task(&session.id, "summarize the diff", "clean");

    let record = dispatch_with_policy(
        &request(&session.id, &task, "clean", "summarize"),
        &BackoffPolicy::fast(),
    )
    .expect("dispatch persists a record");

    assert_eq!(
        record.status, "confirmed",
        "a clean exit-0 run is confirmed even when its output mentions a limit"
    );
    assert!(
        record.failure_kind.is_none(),
        "not a failure: {:?}",
        record.failure_kind
    );
    assert!(
        record.warnings.is_empty(),
        "no backoff warning on a successful run: {:?}",
        record.warnings
    );
    let attempts: u32 = fs::read_to_string(&counter)
        .expect("counter file")
        .trim()
        .parse()
        .expect("counter is numeric");
    assert_eq!(attempts, 1, "confirmed on the first attempt — no retries");
    let _ = fs::remove_dir_all(home);
}

// ---------------------------------------------------------------------------
// AC3 — the cap is configured in the registry and spans sessions.
// ---------------------------------------------------------------------------

#[test]
fn the_cap_is_configured_in_the_registry_and_shared_across_sessions() {
    let _guard = lock();
    let home = fresh_home("cross-session");
    let sentinel = home.join("b-ran.marker");
    let script = write_script(
        &home,
        "shared-worker.sh",
        &format!(
            "#!/bin/sh\necho ran > '{}'\necho 'shared receipt: done'\nexit 0\n",
            sentinel.display()
        ),
    );
    let mut registry = HarnessRegistry::default();
    registry
        .harnesses
        .insert("shared".to_owned(), worker(&script, "hermes"));
    registry.concurrency.insert("shared".to_owned(), 1);
    write_harness_registry(&registry).expect("write registry");

    // Two independent sessions that both use the "shared" harness.
    let _session_a = session_in(&home, "a", &["shared".to_owned()]);
    let session_b = session_in(&home, "b", &["shared".to_owned()]);
    let task_b = running_task(&session_b.id, "cross-session dispatch", "shared");

    // A worker in flight for session A holds the single machine-wide slot.
    let held = acquire_slot("shared", 1, DEFAULT_LEASE_TTL, Some("session-a-worker"))
        .expect("acquire held slot")
        .expect("slot free to hold");

    // A dispatch in session B is queued — the cap spans sessions (AC3).
    let queued = dispatch_with_policy(
        &request(&session_b.id, &task_b, "shared", "work for B"),
        &BackoffPolicy::fast(),
    )
    .expect("dispatch persists");
    assert!(
        queued.is_queued(),
        "a slot held for another session must queue B: {}",
        queued.status
    );
    assert!(
        !sentinel.exists(),
        "B's worker was not spawned while capped"
    );

    // When A's worker finishes, B drains and runs.
    drop(held);
    let drained = drain_queued_with_policy(&session_b.id, &BackoffPolicy::fast()).expect("drain B");
    assert_eq!(drained.len(), 1);
    assert!(drained[0].is_confirmed());
    assert!(sentinel.exists(), "B ran after A's slot freed");
    let _ = fs::remove_dir_all(home);
}
