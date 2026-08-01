#![allow(unsafe_code)]

//! Durable partial output for a live dispatch — issue #49 phase 2, defect 3.
//!
//! Before this branch, `drain_to_eof` accumulated into an in-memory `Captured`
//! and `Drain::finish` ran only after the child exited, so between spawn and
//! exit the durable record said nothing at all. Measured on `origin/main` with
//! a worker emitting a line every 200 ms for six seconds: `record.stdout` was
//! `0` at every one of nine samples and the record on disk was byte-for-byte
//! identical, 777 B, for the whole run.
//!
//! Every test here names the guarantee it holds and was checked by breaking
//! that guarantee on purpose — see `docs/archive/notes/2026-07-31-issue-49-phase2-evidence.md`
//! for the mutation table.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessConfig, HarnessRegistry, create_session, write_harness_registry};
use orc_core::dispatch::{
    self, DispatchActor, DispatchRecord, DispatchRequest, PROGRESS_LOG_MAX_BYTES,
    dispatch_with_policy, progress_paths,
};
use orc_core::dispatch_progress::read_progress;
use orc_core::ratelimit::BackoffPolicy;
use orc_core::tasks::{self, NewTask, TaskActor, assign_task, start_task};

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fresh_home(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "orc-progress-{label}-{}-{nonce}",
        std::process::id()
    ))
}

/// Write an executable shell worker and return a harness that runs it.
fn worker(home: &Path, name: &str, adapter: &str, body: &str) -> HarnessConfig {
    let bin = home.join("bin");
    fs::create_dir_all(&bin).expect("bin");
    let script = bin.join(format!("{name}.sh"));
    fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("write worker");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
    HarnessConfig {
        command: "/bin/sh".to_owned(),
        args: vec![script.to_string_lossy().into_owned()],
        resume_args: Vec::new(),
        roles: vec!["worker".to_owned()],
        adapter: adapter.to_owned(),
        dispatch_args: vec!["--oneshot".to_owned()],
        dispatch_uses_stdin: false,
        dispatch_timeout_sec: 120,
        extra: Default::default(),
    }
}

struct Fixture {
    home: PathBuf,
    session: String,
    task: String,
}

fn setup(label: &str, harness: &str, config: HarnessConfig) -> Fixture {
    let home = fresh_home(label);
    fs::create_dir_all(&home).expect("home");
    // SAFETY: every test in this binary serializes through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert(harness.to_owned(), config);
    registry.default_workers = vec![harness.to_owned()];
    write_harness_registry(&registry).expect("registry");

    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &[harness.to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: format!("{label} probe"),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        harness.to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");
    Fixture {
        home,
        session,
        task: task.id,
    }
}

fn deliver(fixture: &Fixture, harness: &str) -> DispatchRecord {
    dispatch::dispatch(&DispatchRequest {
        session: fixture.session.clone(),
        task: fixture.task.clone(),
        actor: DispatchActor::Brain,
        harness: harness.to_owned(),
        pane_id: Some(format!("{}-worker-1", fixture.session)),
        run: Some("W-1".to_owned()),
        prompt: "do the thing".to_owned(),
        timeout_sec: Some(120),
    })
    .expect("dispatch")
}

fn await_terminal(session: &str, id: &str) -> DispatchRecord {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let record = dispatch::read_dispatch(session, id).expect("read");
        if record.is_terminal() {
            return record;
        }
        assert!(Instant::now() < deadline, "never terminal");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn len_of(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |meta| meta.len())
}

/// **The definition of done.** Partial output is durable BEFORE the worker
/// exits.
///
/// Mutation: move `log.append` out of `drain_to_eof`'s loop and into
/// `Drain::finish` — i.e. `origin/main`'s behaviour. The mid-flight sample
/// finds a zero-length log while the worker is provably still running.
#[test]
fn partial_output_is_durable_before_the_worker_exits() {
    let _guard = lock();
    let fixture = setup(
        "before-exit",
        "slow",
        worker(
            &fresh_home("before-exit-bin"),
            "slow",
            "slow",
            "echo 'FIRST LINE'\nsleep 1\necho 'SECOND LINE'\nsleep 3\necho 'FINAL ANSWER'",
        ),
    );
    // The worker script lives under a different home than the session; rebuild
    // it under the real one so the harness path resolves.
    let config = worker(
        &fixture.home,
        "slow",
        "slow",
        "echo 'FIRST LINE'\nsleep 1\necho 'SECOND LINE'\nsleep 3\necho 'FINAL ANSWER'",
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("slow".to_owned(), config);
    registry.default_workers = vec!["slow".to_owned()];
    write_harness_registry(&registry).expect("registry");

    let record = deliver(&fixture, "slow");
    let paths = progress_paths(&fixture.session, &record.id, 1);

    // Sample until the log has the worker's first two lines, while asserting
    // at every sample that the dispatch is still running and `stdout` empty.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut caught = None;
    loop {
        let live = dispatch::read_dispatch(&fixture.session, &record.id).expect("read");
        let bytes = fs::read(&paths.stdout_log).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if text.contains("SECOND LINE") && !live.is_terminal() {
            assert_eq!(
                live.execution_status.as_deref(),
                Some("running"),
                "the worker must still be running when its partial output is read"
            );
            assert!(
                live.ended_at.is_none(),
                "a running dispatch must have no end time"
            );
            assert!(
                live.stdout.is_empty(),
                "mid-flight output must never land in `stdout`; got {:?}",
                live.stdout
            );
            assert!(
                !text.contains("FINAL ANSWER"),
                "the answer must not be durable yet — the worker has not produced it"
            );
            caught = Some(text);
            break;
        }
        if live.is_terminal() {
            break;
        }
        assert!(Instant::now() < deadline, "worker never produced two lines");
        std::thread::sleep(Duration::from_millis(50));
    }

    let caught = caught.expect(
        "no sample found the worker's partial output on disk while it was still running — \
         this is exactly issue #49 defect 3",
    );
    assert!(caught.contains("FIRST LINE") && caught.contains("SECOND LINE"));

    let record = await_terminal(&fixture.session, &record.id);
    assert_eq!(record.execution_status.as_deref(), Some("succeeded"));
    let full = fs::read_to_string(&paths.stdout_log).expect("final log");
    assert!(
        full.contains("FINAL ANSWER"),
        "the finished log must hold the whole run"
    );
    let _ = fs::remove_dir_all(&fixture.home);
}

/// **Prefix stability** — the property `Captured::raw()` does not have, because
/// its `tail` pops from the front and slides the middle away.
///
/// Mutation: give the log head+tail semantics, or open it with
/// `.truncate(true)` per write. The test names the sample where an earlier
/// read stops being a prefix of a later one.
#[test]
fn the_progress_log_is_append_only_and_never_shrinks() {
    let _guard = lock();
    let home = fresh_home("append-only");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let config = worker(
        &home,
        "chatty",
        "chatty",
        "i=0\nwhile [ $i -lt 400 ]; do echo \"line $i \
         ........................................\"; i=$((i+1)); \
         if [ $((i % 80)) -eq 0 ]; then sleep 0.15; fi; done",
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("chatty".to_owned(), config);
    registry.default_workers = vec!["chatty".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["chatty".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "append only".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "chatty".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");

    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "chatty".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "talk".to_owned(),
        timeout_sec: Some(120),
    })
    .expect("dispatch");
    let paths = progress_paths(&session, &record.id, 1);

    let mut samples: Vec<Vec<u8>> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        samples.push(fs::read(&paths.stdout_log).unwrap_or_default());
        if dispatch::read_dispatch(&session, &record.id)
            .expect("read")
            .is_terminal()
        {
            samples.push(fs::read(&paths.stdout_log).unwrap_or_default());
            break;
        }
        assert!(Instant::now() < deadline, "worker never finished");
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(
        samples.len() >= 5,
        "need several samples to say anything about growth; got {}",
        samples.len()
    );
    for pair in samples.windows(2) {
        let (earlier, later) = (&pair[0], &pair[1]);
        assert!(
            later.len() >= earlier.len(),
            "the log shrank from {} to {} bytes — an append-only file never does",
            earlier.len(),
            later.len()
        );
        assert_eq!(
            &later[..earlier.len()],
            &earlier[..],
            "an earlier read must stay a byte-exact prefix of every later one; \
             the log was rewritten rather than appended to"
        );
    }
    assert!(
        samples.last().expect("last").len() > 4096,
        "the worker produced far more than this; the log kept {}",
        samples.last().expect("last").len()
    );
    let _ = fs::remove_dir_all(&home);
}

/// **Gate A** — a write happens only because a byte arrived.
///
/// Mutation: replace the change gate with an unconditional heartbeat on the
/// 25 ms tick. The journal's length advances during the silence and this fails.
#[test]
fn a_silent_worker_writes_nothing_at_all() {
    let _guard = lock();
    let home = fresh_home("silent");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let config = worker(
        &home,
        "quiet",
        "quiet",
        "echo 'one line then silence'\nsleep 4\necho done",
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("quiet".to_owned(), config);
    registry.default_workers = vec!["quiet".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["quiet".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "silence".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "quiet".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");
    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "quiet".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "be quiet".to_owned(),
        timeout_sec: Some(120),
    })
    .expect("dispatch");
    let paths = progress_paths(&session, &record.id, 1);

    // Wait for the one line to land, then watch the silence.
    let deadline = Instant::now() + Duration::from_secs(15);
    while len_of(&paths.stdout_log) == 0 {
        assert!(Instant::now() < deadline, "the first line never arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let the note floor elapse so a heartbeat mutation has time to fire.
    std::thread::sleep(Duration::from_millis(700));
    let settled_log = len_of(&paths.stdout_log);
    let settled_journal = len_of(&paths.journal);

    let watch_until = Instant::now() + Duration::from_millis(2500);
    let mut samples = 0;
    while Instant::now() < watch_until {
        assert_eq!(
            len_of(&paths.stdout_log),
            settled_log,
            "the log grew while the worker was silent"
        );
        assert_eq!(
            len_of(&paths.journal),
            settled_journal,
            "the journal grew while the worker was silent — a write happened that \
             no arriving byte caused, which is a clock, not real state"
        );
        samples += 1;
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        samples >= 20,
        "too few samples across the silence: {samples}"
    );

    await_terminal(&session, &record.id);
    assert!(
        len_of(&paths.stdout_log) > settled_log,
        "the log must advance again once the worker speaks"
    );
    let _ = fs::remove_dir_all(&home);
}

/// **The amplification bound**, derived from the constants rather than
/// hardcoded.
///
/// Mutation: remove the cap in `ProgressLog::append` (the log grows to
/// megabytes), or set the note floor to zero (tens of thousands of notes).
/// Either blows a bound computed here from the shipped constants.
#[test]
fn a_chatty_worker_writes_a_bounded_number_of_bytes() {
    let _guard = lock();
    let home = fresh_home("chatty-bound");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    // ~5 MB of output, far past the cap.
    let config = worker(
        &home,
        "flood",
        "flood",
        "i=0\nwhile [ $i -lt 40000 ]; do echo \"flood line $i \
         ........................................................................\"; \
         i=$((i+1)); done",
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("flood".to_owned(), config);
    registry.default_workers = vec!["flood".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["flood".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "flood".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "flood".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");
    let started = Instant::now();
    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "flood".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "flood".to_owned(),
        timeout_sec: Some(120),
    })
    .expect("dispatch");
    let record = await_terminal(&session, &record.id);
    let elapsed = started.elapsed();
    let paths = progress_paths(&session, &record.id, 1);

    let log_len = len_of(&paths.stdout_log);
    assert!(
        log_len <= PROGRESS_LOG_MAX_BYTES as u64,
        "the log kept {log_len} bytes, past the {PROGRESS_LOG_MAX_BYTES}-byte cap"
    );
    assert!(
        log_len >= PROGRESS_LOG_MAX_BYTES as u64 - 4096,
        "this worker produces megabytes; the log should have reached its cap, got {log_len}"
    );

    let view = read_progress(&paths.journal)
        .expect("journal readable")
        .expect("journal exists");
    let notes = view
        .frames
        .iter()
        .filter(|frame| frame.kind == "note")
        .count();
    // One note per floor interval at most, plus slack for the open/close pair
    // and the tick quantisation.
    let ceiling = (elapsed.as_millis() / u128::from(orc_core::orch::DEFAULT_AWAIT_POLL_MS)) + 3;
    assert!(
        notes as u128 <= ceiling,
        "{notes} notes in {elapsed:?} exceeds the derived ceiling of {ceiling}; \
         the floor is not bounding the write rate"
    );
    assert!(view.closed, "the journal must end with a close frame");
    let _ = fs::remove_dir_all(&home);
}

/// **The floor is the reader's poll interval, referenced rather than copied.**
///
/// Mutation: hardcode `500` in `PROGRESS_NOTE_MIN_INTERVAL`. This still passes
/// today but fails the moment `DEFAULT_AWAIT_POLL_MS` moves — which is the
/// drift it exists to catch. Asserted here so the coupling is a test, not a
/// comment.
#[test]
fn the_note_floor_is_the_fastest_durable_readers_poll_interval() {
    assert_eq!(
        orc_core::dispatch_progress::note_min_interval().as_millis(),
        u128::from(orc_core::orch::DEFAULT_AWAIT_POLL_MS),
        "the progress note floor must be derived from the await poll interval: \
         publishing faster than the fastest durable reader polls is amplification \
         with no observer"
    );
}

/// **The capability declaration matches the extractor that actually exists.**
///
/// Mutation: hardcode `extractable: true`, or add an adapter to
/// `has_extractor` without adding it to `extract_adapter_event`. The paired
/// assertion catches the drift in one place.
#[test]
fn the_record_declares_the_extractor_it_actually_has() {
    let event = serde_json::json!({
        "assistantMessageEvent": {"type": "text_delta", "delta": "hello"}
    });
    for adapter in ["pi", "codex", "claude", "hermes", "fake-worker"] {
        let declared = orc_core::runner::has_extractor(adapter);
        let (text, _usage) = orc_core::runner::extract_adapter_event(adapter, &event);
        assert_eq!(
            declared,
            text.is_some(),
            "`has_extractor({adapter})` says {declared} but `extract_adapter_event` \
             {} extract — the durable record would declare a capability that was \
             never probed",
            if text.is_some() { "does" } else { "does not" }
        );
    }
}

/// **The sidecars are invisible to the listing path.**
///
/// `list_dispatches` filters on `extension() == "json"` and fully parses every
/// match, and `orch await` drives it up to 600 times per await. A `.json`
/// sidecar would be read every time — and one carrying `DispatchRecord`'s
/// required fields would be listed as a second dispatch for the same task.
///
/// Mutation: rename any sidecar to `.progress.json` in `progress_paths`. The
/// extension assertion fails, and the count assertion fails if it parses.
#[test]
fn progress_sidecars_are_never_read_by_the_listing_path() {
    let paths = progress_paths("some-session", "D-x-1-y-0000", 1);
    for path in [&paths.stdout_log, &paths.stderr_log, &paths.journal] {
        let extension = path
            .extension()
            .map(|ext| ext.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert_ne!(
            extension,
            "json",
            "{} would be read and parsed by every `list_dispatches` call",
            path.display()
        );
    }
    assert_eq!(
        paths
            .journal
            .extension()
            .map(|ext| ext.to_string_lossy().into_owned()),
        Some("jsonl".to_owned())
    );
    assert!(paths.stdout_log.to_string_lossy().ends_with(".a1.out.log"));
    assert!(paths.stderr_log.to_string_lossy().ends_with(".a1.err.log"));
}

/// **Every attempt gets its own files; nothing is replaced.**
///
/// Mutation: drop `attempt` from `progress_paths`. Attempt 2 either truncates
/// attempt 1's bytes away or appends onto them, and both assertions below fail.
#[test]
fn each_attempt_has_its_own_paths_and_they_never_collide() {
    // File names only: `progress_paths` resolves against `ORC_HOME`, which the
    // dispatch-driving tests in this binary rewrite, so comparing whole paths
    // here would race them under the default parallel test runner.
    let name = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let first = progress_paths("s", "D-x", 1);
    let second = progress_paths("s", "D-x", 2);
    assert_ne!(name(&first.stdout_log), name(&second.stdout_log));
    assert_ne!(name(&first.stderr_log), name(&second.stderr_log));
    assert_ne!(name(&first.journal), name(&second.journal));
    assert!(name(&first.stdout_log).contains(".a1."));
    assert!(name(&second.stdout_log).contains(".a2."));
    assert_eq!(
        first.stdout_log.parent(),
        first.journal.parent(),
        "an attempt's three artifacts live beside each other"
    );
}

/// **A torn trailing line is tolerated, not fatal.**
///
/// A SIGKILL mid-`write_all` on an append-only file can leave one. A strict
/// reader would be a hazard the moment anything on the listing path used it:
/// `list_dispatches` drops a record whose reconcile errors, so one truncated
/// sidecar could remove a dispatch from every listing.
///
/// Mutation: make `read_progress` use `?` on a malformed line. The tolerance
/// assertions fail.
#[test]
fn a_torn_trailing_frame_does_not_lose_the_frames_before_it() {
    let dir = fresh_home("torn");
    fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("torn.progress.jsonl");
    let good = concat!(
        r#"{"v":1,"seq":0,"t":"2026-07-31T00:00:00+00:00","attempt":1,"kind":"open","adapter":"pi","extractable":true}"#,
        "\n",
        r#"{"v":1,"seq":1,"t":"2026-07-31T00:00:01+00:00","attempt":1,"kind":"note","stdout":{"bytes":10,"lines":1,"dropped":0,"kept":10},"stderr":{"bytes":0,"lines":0,"dropped":0,"kept":0}}"#,
        "\n",
        r#"{"v":1,"seq":2,"t":"2026-07-31T00:00:02+00:00","attempt":1,"kin"#,
    );
    fs::write(&path, good).expect("write torn journal");

    let view = read_progress(&path)
        .expect("a torn journal must never be an error")
        .expect("the journal exists");
    assert_eq!(
        view.frames.len(),
        2,
        "every complete frame before the tear must survive"
    );
    assert!(view.torn_tail, "the tear must be reported, not hidden");
    assert!(
        !view.closed,
        "a torn journal has not been closed, and must not claim to be"
    );

    assert!(
        read_progress(&dir.join("absent.jsonl"))
            .expect("a missing journal is not an error")
            .is_none(),
        "a missing journal is `None`, distinct from a journal with no frames"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// **An old record parses, and a record with no progress serializes without
/// a `progress` key at all.**
///
/// Mutation: drop `skip_serializing_if = "Option::is_none"`. `"progress": null`
/// appears in every record ever written.
#[test]
fn the_progress_field_is_additive_in_both_directions() {
    let before = r#"{
        "id": "D-old-1-s-0000",
        "session": "s",
        "task": "T0001",
        "actor": "brain",
        "harness": "codex",
        "command_line": "codex --oneshot",
        "prompt": "old",
        "status": "confirmed",
        "created_at": "2026-07-01T00:00:00+00:00",
        "updated_at": "2026-07-01T00:00:00+00:00"
    }"#;
    let record: DispatchRecord =
        serde_json::from_str(before).expect("a pre-phase-2 record must still parse");
    assert!(
        record.progress.is_none(),
        "an old record has no progress, and absence means `not streaming`"
    );

    let rendered = serde_json::to_string(&record).expect("serialize");
    assert!(
        !rendered.contains("progress"),
        "a record with no progress must not carry a `progress` key: {rendered}"
    );

    // A record carrying an unknown future sub-key round-trips it.
    let forward = r#"{
        "id": "D-new-1-s-0000",
        "session": "s",
        "task": "T0001",
        "actor": "brain",
        "harness": "pi",
        "command_line": "pi",
        "prompt": "new",
        "status": "confirmed",
        "progress": {
            "attempt": 1,
            "attempts": 1,
            "stdout_log": "D-new-1-s-0000.a1.out.log",
            "stderr_log": "D-new-1-s-0000.a1.err.log",
            "journal": "D-new-1-s-0000.a1.progress.jsonl",
            "adapter": "pi",
            "extractable": true,
            "log_max_bytes": 262144,
            "invented_later": {"kept": true}
        },
        "created_at": "2026-07-31T00:00:00+00:00",
        "updated_at": "2026-07-31T00:00:00+00:00"
    }"#;
    let record: DispatchRecord = serde_json::from_str(forward).expect("parse");
    let progress = record.progress.as_ref().expect("progress present");
    assert!(progress.extractable);
    assert_eq!(progress.log_max_bytes, 262_144);
    let rendered = serde_json::to_string(&record).expect("serialize");
    assert!(
        rendered.contains("invented_later"),
        "an unknown future sub-key must survive a round trip: {rendered}"
    );
}

/// **The I/O shape decision itself, measured rather than asserted.**
///
/// Issue #49 phase 2's definition of done requires the I/O cost to be measured.
/// The claim under test is the one the whole design rests on: making partial
/// output durable adds **no** durable record writes and **no** task-board
/// writes, so it costs the STAGE client — which watches `~/.orchestra/tasks`
/// and pays a blocking `task_board` socket round-trip per board write —
/// exactly nothing.
///
/// Follows the precedent of `orc-app`'s
/// `six_workers_all_producing_still_repaint_inside_one_frame`: the measurement
/// is printed and recorded in `docs/archive/notes/`, and the assertion is a loose
/// ceiling rather than a tight budget, because this repo already has
/// storage-dependent wall-clock flakes and a tight bound here would be another.
///
/// Mutation: publish the counters into the dispatch record instead of the
/// journal (`write_dispatch` per note) and the record-write count explodes.
/// Append a task-history word per note and the board-write count explodes.
#[test]
fn making_progress_durable_adds_no_record_writes_and_no_board_writes() {
    let _guard = lock();
    let home = fresh_home("io-shape");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    // ~2000 lines over ~2 s: chatty enough that a per-line or per-tick record
    // writer would be obvious, slow enough to span many note intervals.
    let config = worker(
        &home,
        "measured",
        "measured",
        "i=0\nwhile [ $i -lt 2000 ]; do echo \"measured line $i \
         ................................................\"; i=$((i+1)); \
         if [ $((i % 200)) -eq 0 ]; then sleep 0.2; fi; done",
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("measured".to_owned(), config);
    registry.default_workers = vec!["measured".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["measured".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "measured".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "measured".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");

    let started = Instant::now();
    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "measured".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "measure me".to_owned(),
        timeout_sec: Some(120),
    })
    .expect("dispatch");

    // Sample the two files a progress writer must NOT be touching, by mtime.
    // Counting distinct mtimes rather than distinct `updated_at` strings is
    // deliberate: `now_iso()` has one-second granularity, so two real writes a
    // few milliseconds apart produce one indistinguishable timestamp and a
    // string-based count would understate every implementation equally.
    let record_path = dispatch::dispatch_path(&session, &record.id);
    let task_path = tasks::task_path(&session, &task.id);
    let mut record_mtimes = std::collections::BTreeSet::new();
    let mut task_mtimes = std::collections::BTreeSet::new();
    let mtime = |path: &Path| {
        fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|delta| delta.as_nanos())
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(stamp) = mtime(&record_path) {
            record_mtimes.insert(stamp);
        }
        if let Some(stamp) = mtime(&task_path) {
            task_mtimes.insert(stamp);
        }
        if dispatch::read_dispatch(&session, &record.id)
            .expect("read")
            .is_terminal()
        {
            break;
        }
        assert!(Instant::now() < deadline, "worker never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    if let Some(stamp) = mtime(&record_path) {
        record_mtimes.insert(stamp);
    }
    if let Some(stamp) = mtime(&task_path) {
        task_mtimes.insert(stamp);
    }
    let elapsed = started.elapsed();

    let paths = progress_paths(&session, &record.id, 1);
    let view = read_progress(&paths.journal)
        .expect("journal readable")
        .expect("journal exists");
    let notes = view
        .frames
        .iter()
        .filter(|frame| frame.kind == "note")
        .count();
    let final_record = dispatch::read_dispatch(&session, &record.id).expect("read");
    let lines = view
        .frames
        .iter()
        .filter_map(|frame| frame.stdout.as_ref())
        .map(|counters| counters.lines)
        .max()
        .unwrap_or(0);

    println!(
        "\nissue #49 phase 2 — I/O shape over one {:.2}s dispatch of {lines} worker lines:\n\
         \x20 dispatch record writes (distinct mtimes) : {}\n\
         \x20 task board writes      (distinct mtimes) : {}\n\
         \x20 progress journal notes                   : {notes}\n\
         \x20 progress log bytes                       : {}\n\
         \x20 journal bytes                            : {}\n\
         \x20 dispatch record bytes                    : {}",
        elapsed.as_secs_f64(),
        record_mtimes.len(),
        task_mtimes.len(),
        len_of(&paths.stdout_log),
        len_of(&paths.journal),
        len_of(&record_path),
    );

    // The record is written by `dispatch`, `execute`, `mark_started` and
    // `persist_terminal` — the same four as before this branch. A generous
    // ceiling: the claim is "not per-tick", not an exact count.
    assert!(
        record_mtimes.len() <= 6,
        "the dispatch record was written {} times across {notes} progress notes — \
         progress must not ride the record",
        record_mtimes.len()
    );
    assert!(
        task_mtimes.len() <= 6,
        "the task board was written {} times — progress must never touch the board, \
         because every board write costs a STAGE client a blocking round-trip",
        task_mtimes.len()
    );
    assert!(
        notes >= 1,
        "this worker produced {lines} lines over {elapsed:?}; the journal should have \
         noted at least once"
    );
    assert!(
        final_record.stdout.contains("measured line"),
        "the terminal record still carries the answer, unchanged by this branch"
    );
    let _ = fs::remove_dir_all(&home);
}

/// **A rate-limited attempt's bytes are neither deleted nor spliced onto the
/// next attempt's** — driven through a real retry, not asserted about a helper.
///
/// This is the guarantee the design's answer to "what happens on a rate-limit
/// retry" rests on, and the first version of this branch tested only that
/// `progress_paths(.., 1)` and `progress_paths(.., 2)` differ. That tests the
/// naming function. Hardcoding the ordinal the *supervisor* passes left the
/// whole suite green while attempt 2's `create(..).truncate(true)` destroyed
/// attempt 1's output — the exact outcome the design calls impossible. Found in
/// review; this drives the real path.
///
/// Mutation: pass a constant `1` as the attempt ordinal in
/// `invoke_with_backoff`. Attempt 1's marker vanishes from disk and this fails.
#[test]
fn a_rate_limited_retry_keeps_the_earlier_attempts_bytes() {
    let _guard = lock();
    let home = fresh_home("retry");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };

    // Attempt 1 emits a distinctive line and exits non-zero with a rate-limit
    // signal `ratelimit::detect` recognises; attempt 2 emits a different line
    // and succeeds. A marker file distinguishes the two runs.
    let marker = home.join("attempted");
    let config = worker(
        &home,
        "flaky",
        "flaky",
        &format!(
            "if [ -f {m} ]; then \
               echo 'SECOND ATTEMPT OUTPUT'; exit 0; \
             else \
               touch {m}; echo 'FIRST ATTEMPT OUTPUT'; \
               echo 'HTTP 429 Too Many Requests' 1>&2; exit 1; \
             fi",
            m = marker.display()
        ),
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("flaky".to_owned(), config);
    registry.default_workers = vec!["flaky".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["flaky".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "retry".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "flaky".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");

    // The millisecond-scale schedule, so the backoff is deterministic and the
    // test does not sit through production's 2 s minimum.
    let record = dispatch_with_policy(
        &DispatchRequest {
            session: session.clone(),
            task: task.id.clone(),
            actor: DispatchActor::Brain,
            harness: "flaky".to_owned(),
            pane_id: None,
            run: Some("W-1".to_owned()),
            prompt: "retry me".to_owned(),
            timeout_sec: Some(60),
        },
        &BackoffPolicy::fast(),
    )
    .expect("dispatch");
    let record = await_terminal(&session, &record.id);
    assert_eq!(
        record.execution_status.as_deref(),
        Some("succeeded"),
        "the second attempt must succeed, or this is testing the wrong path"
    );
    assert!(
        marker.exists(),
        "the worker must actually have run twice for this test to mean anything"
    );

    let first = progress_paths(&session, &record.id, 1);
    let second = progress_paths(&session, &record.id, 2);

    // Both attempts' artifacts exist. This is what a truncating or
    // ordinal-blind implementation destroys.
    let first_out = fs::read_to_string(&first.stdout_log)
        .expect("attempt 1's log must still exist after the retry");
    let second_out = fs::read_to_string(&second.stdout_log).expect("attempt 2's log must exist");

    assert!(
        first_out.contains("FIRST ATTEMPT OUTPUT"),
        "the rate-limited attempt's bytes must survive its own retry; got {first_out:?}"
    );
    assert!(
        second_out.contains("SECOND ATTEMPT OUTPUT"),
        "attempt 2's own output must be there; got {second_out:?}"
    );
    assert!(
        !second_out.contains("FIRST ATTEMPT OUTPUT"),
        "attempt 1's bytes must not be spliced into attempt 2's log; got {second_out:?}"
    );
    assert!(
        !first_out.contains("SECOND ATTEMPT OUTPUT"),
        "attempt 2 must not have written into attempt 1's log; got {first_out:?}"
    );

    // The record points at the newest attempt, and says which one it is.
    let progress = record.progress.as_ref().expect("progress pointer");
    assert_eq!(
        progress.attempt, 2,
        "the record must name the attempt whose files it points at"
    );
    assert!(progress.stdout_log.ends_with(".a2.out.log"));

    // Each attempt's journal is its own: own `open`, own `close`, own sequence.
    for (label, paths, reason) in [("attempt 1", &first, "eof"), ("attempt 2", &second, "eof")] {
        let view = read_progress(&paths.journal)
            .unwrap_or_else(|error| panic!("{label} journal readable: {error}"))
            .unwrap_or_else(|| panic!("{label} journal exists"));
        assert_eq!(
            view.frames.first().map(|frame| frame.kind.as_str()),
            Some("open"),
            "{label}'s journal starts its own sequence"
        );
        assert!(view.closed, "{label}'s journal is closed");
        assert_eq!(
            view.frames.last().and_then(|frame| frame.reason.as_deref()),
            Some(reason),
            "{label} closes with its own reason"
        );
    }
    let _ = fs::remove_dir_all(&home);
}

/// **The record's declared capability and cap are the ones actually in force.**
///
/// `extractable` and `log_max_bytes` are read by anything deciding what a log
/// can be turned into and whether a short log is capped or quiet. Both survived
/// being falsified in review, because nothing compared them against the code
/// that enforces them.
///
/// Mutation: hardcode `extractable: true`, or write `log_max_bytes: 0`. Each
/// fails here.
#[test]
fn the_record_states_the_capability_and_cap_that_are_actually_in_force() {
    let _guard = lock();
    let home = fresh_home("declared");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    // `fake` has no extractor; the record must say so rather than leave an
    // empty answer to be misread as "the worker said nothing".
    let config = worker(&home, "declared", "fake", "echo hello");
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("declared".to_owned(), config);
    registry.default_workers = vec!["declared".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["declared".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "declared".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "declared".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");
    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "declared".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "declare".to_owned(),
        timeout_sec: Some(60),
    })
    .expect("dispatch");
    let record = await_terminal(&session, &record.id);
    let progress = record.progress.as_ref().expect("progress");

    assert_eq!(
        progress.extractable,
        orc_core::runner::has_extractor(&progress.adapter),
        "the record must declare the capability the extractor actually has for \
         the adapter it names"
    );
    assert!(
        !progress.extractable,
        "`fake` has no extractor, and the record must say so rather than hide it"
    );
    assert_eq!(
        progress.log_max_bytes, PROGRESS_LOG_MAX_BYTES as u64,
        "the record's stated cap must be the cap the log actually enforces — a \
         reader tells `capped` from `quiet` by comparing against it"
    );
    let _ = fs::remove_dir_all(&home);
}

/// **A progress log that cannot be opened says so on the record.**
///
/// `ProgressLog::create`'s doc promises "a disabled writer and one durable
/// warning on the record". The warning was built and then dropped on the floor
/// (`let _ = progress_warnings;`), so deleting both `ORC WARNING:` strings
/// passed the whole suite — and a reader was left with `record.progress`
/// naming files that do not exist and nothing explaining why. Found in review.
///
/// The obstruction is a **directory** planted at the path attempt 2's stdout
/// log will take, which makes exactly that `open` fail with `EISDIR` while
/// leaving `write_dispatch` and the journal alone. Making the whole dispatch
/// directory read-only was the first attempt and it is the wrong instrument:
/// it also breaks `atomic_write_json`, so the dispatch never reaches a terminal
/// state and the test hangs on a different failure. The backoff delay is what
/// makes the plant possible — attempt 1's `open` happens microseconds after
/// spawn and cannot be raced.
///
/// Mutation: drop the `warnings.borrow_mut().extend(progress_warnings)` line.
/// The artifacts are missing and nothing on the record says so.
#[test]
fn a_progress_log_that_cannot_be_opened_is_reported_on_the_record() {
    let _guard = lock();
    let home = fresh_home("unwritable");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let marker = home.join("attempted");
    let config = worker(
        &home,
        "obstructed",
        "fake",
        &format!(
            "if [ -f {m} ]; then \
               echo 'SECOND ATTEMPT'; exit 0; \
             else \
               touch {m}; echo 'FIRST ATTEMPT'; \
               echo 'HTTP 429 Too Many Requests' 1>&2; exit 1; \
             fi",
            m = marker.display()
        ),
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("obstructed".to_owned(), config);
    registry.default_workers = vec!["obstructed".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["obstructed".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "obstructed".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "obstructed".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");

    // A backoff long enough to plant the obstruction between the two attempts.
    let policy = BackoffPolicy {
        min_delay: Duration::from_millis(2_000),
        max_delay: Duration::from_millis(2_000),
        factor: 1.0,
        max_retries: 1,
        jitter: false,
    };
    let record = dispatch_with_policy(
        &DispatchRequest {
            session: session.clone(),
            task: task.id.clone(),
            actor: DispatchActor::Brain,
            harness: "obstructed".to_owned(),
            pane_id: None,
            run: Some("W-1".to_owned()),
            prompt: "obstruct me".to_owned(),
            timeout_sec: Some(60),
        },
        &policy,
    )
    .expect("dispatch");

    // Wait until attempt 1 has run, then obstruct attempt 2's stdout log.
    let deadline = Instant::now() + Duration::from_secs(20);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "attempt 1 never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
    let second = progress_paths(&session, &record.id, 2);
    fs::create_dir_all(&second.stdout_log).expect("plant the obstruction");

    let record = await_terminal(&session, &record.id);
    assert!(
        marker.exists(),
        "the worker must really have retried for this to mean anything"
    );
    assert!(
        second.stdout_log.is_dir(),
        "the obstruction must still be the thing that blocked the open"
    );

    assert!(
        record
            .warnings
            .iter()
            .any(|warning| { warning.contains("ORC WARNING") && warning.contains("progress") }),
        "a progress artifact that could not be opened must leave a durable \
         warning on the record, or the record names files that do not exist \
         and nothing explains why; warnings were {:?}",
        record.warnings
    );
    // The dispatch itself is unharmed: losing a progress log never fails a
    // delivery.
    assert_eq!(record.execution_status.as_deref(), Some("succeeded"));
    let _ = fs::remove_dir_all(&home);
}

/// **The log never ends mid-line, and never fabricates a byte.**
///
/// The first implementation clipped a line at the cap. That ends the log on a
/// byte boundary — a truncated JSON object no reader can parse, or a split
/// UTF-8 code point — and makes the log's last line something the worker never
/// emitted, which is the one claim the byte log exists to make. Found in
/// review, with a 262144-byte log ending mid-word.
///
/// Mutation: restore the clip (`&line[..room.min(line.len())]`). The final-byte
/// assertion fails.
#[test]
fn the_log_never_ends_mid_line() {
    let _guard = lock();
    let home = fresh_home("wholeline");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    // Long lines that do not divide the cap evenly, so a clipping
    // implementation is guaranteed to land mid-line.
    let config = worker(
        &home,
        "longlines",
        "fake",
        "i=0\nwhile [ $i -lt 4000 ]; do \
         echo \"$i-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"; \
         i=$((i+1)); done",
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("longlines".to_owned(), config);
    registry.default_workers = vec!["longlines".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["longlines".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "wholeline".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "longlines".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");
    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "longlines".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "long lines".to_owned(),
        timeout_sec: Some(60),
    })
    .expect("dispatch");
    let record = await_terminal(&session, &record.id);
    let paths = progress_paths(&session, &record.id, 1);
    let bytes = fs::read(&paths.stdout_log).expect("log");

    assert!(
        bytes.len() <= PROGRESS_LOG_MAX_BYTES,
        "still bounded: {} bytes",
        bytes.len()
    );
    assert!(
        bytes.len() > PROGRESS_LOG_MAX_BYTES - 200,
        "this worker far exceeds the cap, so the log should be near it; got {}",
        bytes.len()
    );
    assert_eq!(
        bytes.last(),
        Some(&b'\n'),
        "the log must end on a line terminator — a clipped last line is bytes \
         the worker never wrote as a line, and for a JSON transport it is an \
         object nothing can parse"
    );
    // Nothing fabricated: every line in the log is one the worker emitted.
    let text = String::from_utf8(bytes).expect("the log is never cut mid-code-point");
    for line in text.lines() {
        assert!(
            line.ends_with('a') && line.contains('-'),
            "every line must be verbatim worker output; found {line:?}"
        );
    }
    let _ = fs::remove_dir_all(&home);
}

/// **`lines` counts complete lines, not reads.**
///
/// `read_until` returns the unterminated remainder at EOF too, so counting per
/// read reports `alpha\nbeta` as two lines when only one is complete. That
/// matters precisely because a block-buffered worker sitting mid-line is the
/// case the journal must not overstate. Found in review.
///
/// Mutation: count unconditionally in `Captured::push`. This reports 2.
#[test]
fn an_unterminated_final_chunk_is_not_counted_as_a_line() {
    let _guard = lock();
    let home = fresh_home("halfline");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    // `printf` with no trailing newline: one complete line, then a fragment.
    let config = worker(&home, "halfline", "fake", "printf 'alpha\\nbeta'");
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("halfline".to_owned(), config);
    registry.default_workers = vec!["halfline".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["halfline".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "halfline".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "halfline".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");
    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "halfline".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "half a line".to_owned(),
        timeout_sec: Some(60),
    })
    .expect("dispatch");
    let record = await_terminal(&session, &record.id);
    let paths = progress_paths(&session, &record.id, 1);
    let view = read_progress(&paths.journal)
        .expect("journal readable")
        .expect("journal exists");
    let close = view
        .frames
        .iter()
        .find(|frame| frame.kind == "close")
        .expect("a close frame");
    let stdout = close.stdout.expect("close carries stdout counters");

    assert_eq!(
        stdout.bytes, 10,
        "all ten bytes of `alpha\\nbeta` were read"
    );
    assert_eq!(
        stdout.lines, 1,
        "only `alpha\\n` is a complete line; the trailing `beta` has no \
         terminator and must not be counted, or a worker stalled mid-line \
         reads as having produced one more line than it has"
    );
    let _ = fs::remove_dir_all(&home);
}

/// **A `note` frame reports both streams, not just stdout.**
///
/// `stderr` on a mid-flight frame was built and serialised and asserted by
/// nothing: setting every note frame's `stderr` to `None` passed the whole
/// suite. It is the counter that tells a reader a worker is producing
/// *diagnostics* while its stdout stays empty — which for a harness that logs
/// its thinking to stderr is the whole of what is observable before exit.
///
/// The worker writes to both streams for long enough to cross the note floor
/// (`PROGRESS_NOTE_MIN_INTERVAL`, 500 ms) several times.
///
/// Mutation: `stderr: None` in `ProgressJournal::note`, or dropping the stderr
/// half of the counters the drain threads hand it.
#[test]
fn a_note_frame_carries_the_stderr_counters_too() {
    let _guard = lock();
    let home = fresh_home("bothstreams");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    // ~3 s of steady output on BOTH streams: comfortably more than the 500 ms
    // note floor, so several `note` frames are written rather than just
    // `open` and `close`.
    let config = worker(
        &home,
        "bothstreams",
        "fake",
        "i=0\nwhile [ $i -lt 30 ]; do echo \"OUT-$i\"; echo \"ERR-$i\" 1>&2; \
         sleep 0.1; i=$((i+1)); done",
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("bothstreams".to_owned(), config);
    registry.default_workers = vec!["bothstreams".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["bothstreams".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "bothstreams".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "bothstreams".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");
    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "bothstreams".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "both streams".to_owned(),
        timeout_sec: Some(60),
    })
    .expect("dispatch");
    let record = await_terminal(&session, &record.id);
    let paths = progress_paths(&session, &record.id, 1);
    let view = read_progress(&paths.journal)
        .expect("journal readable")
        .expect("journal exists");

    let notes = view
        .frames
        .iter()
        .filter(|frame| frame.kind == "note")
        .collect::<Vec<_>>();
    // Without this the loop below is vacuously true on an empty set — the
    // failure mode that would make the whole test unable to fail.
    assert!(
        !notes.is_empty(),
        "a ~3s worker must cross the 500ms note floor and produce note frames; \
         frames were {:?}",
        view.frames.iter().map(|f| &f.kind).collect::<Vec<_>>()
    );
    for frame in &notes {
        assert!(
            frame.stderr.is_some(),
            "every note frame must carry stderr counters, not just stdout; \
             seq {} did not",
            frame.seq
        );
    }
    // The counters must track a stream that really moved, or `Some(default)`
    // would satisfy the loop above while saying nothing.
    let stderr_seen = notes
        .iter()
        .filter_map(|frame| frame.stderr)
        .map(|counters| counters.bytes)
        .max()
        .expect("stderr counters on a note frame");
    assert!(
        stderr_seen > 0,
        "the worker wrote 30 lines to stderr, so a note frame must have \
         observed some of them; saw {stderr_seen} bytes"
    );
    // And they must be the *worker's* stderr, not a copy of stdout's counters.
    let bytes_on_disk = len_of(&paths.stderr_log);
    assert!(
        bytes_on_disk > 0,
        "the stderr byte log must hold the worker's diagnostics"
    );
    let close = view
        .frames
        .iter()
        .find(|frame| frame.kind == "close")
        .expect("a close frame");
    let closing = close.stderr.expect("close carries stderr counters");
    assert_eq!(
        closing.kept, bytes_on_disk,
        "the close frame's stderr `kept` must equal the stderr log's length"
    );
    assert!(
        closing.bytes >= stderr_seen,
        "cumulative counters are monotone: close ({}) cannot be behind a note \
         ({stderr_seen})",
        closing.bytes
    );
    let _ = fs::remove_dir_all(&home);
}

/// **The log is a contiguous prefix of the stream, including at the cap.**
///
/// `ProgressLog::append` declines a line that does not fit *and latches
/// closed*. The latch is the whole of the guarantee: without it a long line is
/// declined, a later **shorter** one still fits, and the log gains a **hole** —
/// which is the one thing "byte N is byte N forever" forbids. With
/// variable-length lines that is not an exotic case, it is the ordinary
/// behaviour at the cap.
///
/// The worker alternates a 4000-byte line with a ~9-byte one, so the cap is
/// reached mid-alternation by construction: the long line at that point does
/// not fit, and the short one that follows it does.
///
/// Mutation: drop `self.capped` from `ProgressLog::append` — both the read in
/// the guard and the assignment. The log then reads `…SHORT-64\nSHORT-65\n`
/// where the worker wrote 4000 `L`s between them, and `starts_with` is false.
///
/// Found in review of #49 phase 2 (the latch was new code in `881fb37`, so no
/// earlier mutation battery covered it) and landed here because phase 3 is the
/// first code that *reads* this log and therefore the first a hole would lie to.
#[test]
fn the_log_is_a_contiguous_prefix_even_at_the_cap() {
    /// Bytes of `L` in the long line, before its terminator.
    const LONG: usize = 4000;
    /// Enough alternations to overrun the cap several times over.
    const ITERATIONS: usize = 100;

    let _guard = lock();
    let home = fresh_home("prefix");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    // 40 -> 200 -> 1000 -> 4000 by concatenation, so the script needs neither
    // `seq` nor a bash-only `printf` repeat and stays POSIX `sh`.
    let body = format!(
        "long={forty}\n\
         long=$long$long$long$long$long\n\
         long=$long$long$long$long$long\n\
         long=$long$long$long$long\n\
         i=0\n\
         while [ $i -lt {ITERATIONS} ]; do echo \"$long\"; echo \"SHORT-$i\"; i=$((i+1)); done",
        forty = "L".repeat(40),
    );
    let config = worker(&home, "prefix", "fake", &body);
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("prefix".to_owned(), config);
    registry.default_workers = vec!["prefix".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["prefix".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "prefix".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "prefix".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");
    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "prefix".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "alternating line lengths".to_owned(),
        timeout_sec: Some(60),
    })
    .expect("dispatch");
    let record = await_terminal(&session, &record.id);
    let paths = progress_paths(&session, &record.id, 1);
    let log = fs::read_to_string(&paths.stdout_log).expect("log");

    // Rebuilt from the script's own definition, not from the log, so the
    // comparison below has an independent witness.
    let mut stream = String::new();
    for i in 0..ITERATIONS {
        stream.push_str(&"L".repeat(LONG));
        stream.push('\n');
        stream.push_str(&format!("SHORT-{i}\n"));
    }

    // Without these two the prefix assertion is vacuous: `starts_with` is true
    // of an empty log, and true of any log that never reached the cap at all —
    // and it is only *at* the cap that the latch does anything.
    assert!(
        log.len() <= PROGRESS_LOG_MAX_BYTES,
        "still bounded: {} bytes",
        log.len()
    );
    assert!(
        log.len() > PROGRESS_LOG_MAX_BYTES - (LONG + 16),
        "this worker overruns the cap {}x over, so the log must sit within one \
         long line of it — otherwise the cap was never exercised and this test \
         proves nothing about the latch; got {} of {PROGRESS_LOG_MAX_BYTES}",
        (ITERATIONS * (LONG + 10)) / PROGRESS_LOG_MAX_BYTES,
        log.len()
    );

    assert!(
        stream.starts_with(&log),
        "the log must be a contiguous prefix of what the worker wrote. Without \
         the `capped` latch a declined long line is followed by a shorter one \
         that fits, and the log gains a hole — the one thing \"byte N is byte N \
         forever\" forbids. log ends: {:?}",
        &log[log.len().saturating_sub(60)..]
    );

    // The same defect named directly, because `starts_with` reports only that
    // something diverged and this says what.
    let mut previous_was_short = false;
    for line in log.lines() {
        let short = line.starts_with("SHORT-");
        assert!(
            !(short && previous_was_short),
            "two consecutive SHORT lines means the long line between them was \
             declined and the log kept going: {line:?}"
        );
        previous_was_short = short;
    }
    let _ = fs::remove_dir_all(&home);
}

/// **The bounded tail returns whole worker lines and never a fragment.**
///
/// Issue #49 phase 3. The reveal has a few rows and the log has up to 256 KiB,
/// so it seeks rather than reads the file whole. The risk that buys is the one
/// thing the byte log exists to prevent: a window starting mid-line would put
/// bytes on screen in an order and a grouping the worker never wrote, and a
/// window splitting a multi-byte code point would put a replacement character
/// there that the worker never wrote at all.
///
/// Mutations, each at the call site of the line-boundary trim:
///   - return `bytes` instead of `from_line_start`  → the fragment assertion fails
///   - drop the `window == len` arm                 → the small-file assertion fails
///   - `String::from_utf8_lossy(&bytes)` before the trim → the UTF-8 assertion fails
#[test]
fn the_bounded_tail_never_returns_a_partial_line() {
    let _guard = lock();
    let home = fresh_home("tail");
    fs::create_dir_all(&home).expect("home");
    let log = home.join("out.log");

    // Whole file smaller than the window: everything, nothing discarded.
    fs::write(&log, "alpha\nbeta\ngamma\n").expect("write");
    assert_eq!(
        orc_core::dispatch_progress::tail_lines(&log, 4096).expect("exists"),
        vec!["alpha", "beta", "gamma"],
        "a log smaller than the window is returned whole — the first line is \
         not a fragment of anything"
    );

    // Window lands mid-line: that line is discarded, not shown in part.
    let tail = orc_core::dispatch_progress::tail_lines(&log, 9).expect("exists");
    assert_eq!(
        tail,
        vec!["gamma"],
        "a window starting inside `beta` must drop it rather than show `ta`"
    );
    for line in &tail {
        assert!(
            ["alpha", "beta", "gamma"].contains(&line.as_str()),
            "every line returned must be one the worker actually wrote: {line:?}"
        );
    }

    // Multi-byte code points: a naive byte window splits one.
    fs::write(&log, "ααααα\nβββββ\nγγγγγ\n").expect("write utf8");
    let tail = orc_core::dispatch_progress::tail_lines(&log, 12).expect("exists");
    for line in &tail {
        assert!(
            !line.contains('\u{FFFD}'),
            "the tail must not put a replacement character on screen — the \
             worker never wrote one: {line:?}"
        );
        assert!(
            ["ααααα", "βββββ", "γγγγγ"].contains(&line.as_str()),
            "verbatim worker lines only: {line:?}"
        );
    }

    // Present and empty is a fact, and it is NOT the same as absent.
    fs::write(&log, "").expect("write empty");
    assert_eq!(
        orc_core::dispatch_progress::tail_lines(&log, 4096),
        Some(Vec::new()),
        "an existing empty log is `Some(empty)`: we looked and there was \
         nothing yet"
    );
    assert_eq!(
        orc_core::dispatch_progress::tail_lines(&home.join("absent.log"), 4096),
        None,
        "an absent log is `None`: this supervisor is not streaming. Collapsing \
         the two is the lie `DispatchProgress`'s doc exists to forbid"
    );

    // A worker mid-line with no terminator yet has produced no complete line.
    fs::write(&log, "no terminator yet").expect("write partial");
    assert_eq!(
        orc_core::dispatch_progress::tail_lines(&log, 4).expect("exists"),
        Vec::<String>::new(),
        "a window inside an unterminated line yields nothing: no COMPLETE line \
         has been observed, which is the only thing the vocabulary may claim"
    );
    // Same fact at the other end: whole lines followed by a fragment. The
    // fragment is the worker's own bytes, but it is not a line yet.
    fs::write(&log, "done\nalso done\nstill typin").expect("write trailing fragment");
    assert_eq!(
        orc_core::dispatch_progress::tail_lines(&log, 4096).expect("exists"),
        vec!["done", "also done"],
        "an unterminated trailing chunk must be dropped, not shown as a line: \
         `StreamCounters::lines` counts terminators, and a reveal showing three \
         lines beside a journal saying two has started lying"
    );

    let _ = fs::remove_dir_all(&home);
}

/// **The tail is bounded by its window, not by the file.**
///
/// The reason this function exists rather than `fs::read` + `.lines().rev()`.
/// A worker that fills the cap leaves 256 KiB; a reveal repainting on that
/// would read a quarter of a megabyte to show four rows.
#[test]
fn the_bounded_tail_reads_a_window_not_the_whole_log() {
    let _guard = lock();
    let home = fresh_home("tail-bound");
    fs::create_dir_all(&home).expect("home");
    let log = home.join("big.log");

    // A full-cap log: 262144 bytes of 32-byte lines.
    let line = format!("{}\n", "y".repeat(31));
    let mut body = String::new();
    while body.len() + line.len() <= PROGRESS_LOG_MAX_BYTES {
        body.push_str(&line);
    }
    fs::write(&log, &body).expect("write big");
    assert!(
        body.len() > 250_000,
        "the fixture must actually be large: {} bytes",
        body.len()
    );

    let tail = orc_core::dispatch_progress::tail_lines(&log, 512).expect("exists");
    // 512 bytes of 32-byte lines, minus the fragment at the front.
    assert!(
        (14..=16).contains(&tail.len()),
        "a 512-byte window over 32-byte lines is ~15 lines, not the whole \
         file's {}; got {}",
        body.lines().count(),
        tail.len()
    );
    assert!(
        tail.iter().all(|shown| *shown == "y".repeat(31)),
        "and every one is a whole worker line"
    );

    let _ = fs::remove_dir_all(&home);
}

/// **A real worker that exits mid-line: the log ends unterminated, and the
/// reader agrees with the journal about how many lines exist.**
///
/// The helper test above proves `tail_lines` drops a trailing fragment from a
/// file this test suite wrote. This proves the case is real — that
/// `drain_to_eof` (`dispatch_supervisor.rs:356-378`) appends whatever
/// `read_until(b'\n')` returned at EOF, terminator or not — and that the two
/// numbers a reveal would put on screen beside each other agree.
///
/// This is the wiring, not the component: it drives a real dispatch through a
/// real supervisor process and reads what that process actually left on disk.
///
/// Mutation: drop the `rposition` trim from `tail_lines`. The reader then
/// reports three lines while the journal reports two.
#[test]
fn a_worker_that_exits_mid_line_leaves_an_unterminated_log_and_the_reader_says_two() {
    let _guard = lock();
    let home = fresh_home("midline");
    fs::create_dir_all(&home).expect("home");
    // SAFETY: serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    // Two complete lines and a fragment, exactly as a worker killed or exiting
    // mid-write leaves them.
    let config = worker(
        &home,
        "midline",
        "fake",
        "printf 'done\\nalso done\\nstill typin'",
    );
    let mut registry = HarnessRegistry::default();
    registry.harnesses.insert("midline".to_owned(), config);
    registry.default_workers = vec!["midline".to_owned()];
    write_harness_registry(&registry).expect("registry");
    let cwd = home.join("cwd");
    fs::create_dir_all(&cwd).expect("cwd");
    let session = create_session("codex", &["midline".to_owned()], &cwd)
        .expect("session")
        .id;
    let task = tasks::add_task(
        &session,
        TaskActor::Brain,
        NewTask {
            title: "midline".to_owned(),
            ..NewTask::default()
        },
    )
    .expect("task");
    assign_task(
        &session,
        &task.id,
        "midline".to_owned(),
        Some("W-1".to_owned()),
        TaskActor::Brain,
    )
    .expect("assign");
    start_task(&session, &task.id, TaskActor::Brain).expect("start");
    let record = dispatch::dispatch(&DispatchRequest {
        session: session.clone(),
        task: task.id.clone(),
        actor: DispatchActor::Brain,
        harness: "midline".to_owned(),
        pane_id: None,
        run: Some("W-1".to_owned()),
        prompt: "exit mid line".to_owned(),
        timeout_sec: Some(60),
    })
    .expect("dispatch");
    let record = await_terminal(&session, &record.id);
    let paths = progress_paths(&session, &record.id, 1);

    // 1. The premise: the log really does end without a terminator. If this
    //    ever stops being true the trim becomes dead code and should go.
    let raw = fs::read(&paths.stdout_log).expect("log");
    assert_eq!(
        raw.last(),
        Some(&b'n'),
        "the worker exited mid-line, so the log's last byte is its last \
         character and not a terminator: {:?}",
        String::from_utf8_lossy(&raw[raw.len().saturating_sub(20)..])
    );

    // 2. The journal counts terminators, so it says two.
    let view = read_progress(&paths.journal)
        .expect("journal readable")
        .expect("journal exists");
    let close = view
        .frames
        .iter()
        .find(|frame| frame.kind == "close")
        .expect("a close frame");
    let counters = close.stdout.expect("close carries stdout counters");
    assert_eq!(counters.lines, 2, "two complete lines were observed");
    assert_eq!(
        counters.bytes,
        raw.len() as u64,
        "and every byte read reached the log"
    );

    // 3. The reader must say two as well. Showing `still typin` would put a
    //    third line on screen beside a journal reporting two.
    let shown =
        orc_core::dispatch_progress::tail_lines(&paths.stdout_log, 4096).expect("the log exists");
    assert_eq!(
        shown,
        vec!["done", "also done"],
        "the reveal and the counters beside it must agree"
    );
    assert_eq!(
        shown.len() as u64,
        counters.lines,
        "the reader's line count IS the journal's line count"
    );

    let _ = fs::remove_dir_all(&home);
}

/// **"Since T" is when a stdout LINE moved, never when any counter moved.**
///
/// The journal is stamped whenever *any* counter changes — `note` compares the
/// whole `(stdout, stderr)` pair — so a worker writing only to stderr re-stamps
/// it without producing a single stdout line. A reader taking `frames.last().t`
/// would then advance "newest line at …" while stdout had been still, which is
/// a claim about the worker manufactured out of a fact about stderr.
///
/// That is the reveal's hardest honesty problem and it is invisible to any test
/// whose fixture moves both streams together, which is why this fixture moves
/// them apart.
///
/// Mutation: `facts()` assigns `newest_stdout_line_at` from every frame that
/// carries counters, or uses `>=` instead of `>`.
#[test]
fn since_t_is_when_a_stdout_line_moved_and_not_when_any_counter_moved() {
    let _guard = lock();
    let home = fresh_home("since-t");
    fs::create_dir_all(&home).expect("home");
    let journal = home.join("j.jsonl");

    // seq 1: the only frame where a stdout LINE appears.
    // seq 2 and 3: stderr moves, stdout does not. The journal is re-stamped
    //              both times because `note`'s change gate saw a counter move.
    fs::write(
        &journal,
        concat!(
            r#"{"v":1,"seq":0,"t":"2026-08-01T12:00:00+00:00","attempt":1,"kind":"open","adapter":"pi","extractable":true}"#,
            "\n",
            r#"{"v":1,"seq":1,"t":"2026-08-01T12:00:05+00:00","attempt":1,"kind":"note","stdout":{"bytes":10,"lines":1,"dropped":0,"kept":10},"stderr":{"bytes":0,"lines":0,"dropped":0,"kept":0}}"#,
            "\n",
            r#"{"v":1,"seq":2,"t":"2026-08-01T12:03:00+00:00","attempt":1,"kind":"note","stdout":{"bytes":10,"lines":1,"dropped":0,"kept":10},"stderr":{"bytes":40,"lines":2,"dropped":0,"kept":40}}"#,
            "\n",
            r#"{"v":1,"seq":3,"t":"2026-08-01T12:09:00+00:00","attempt":1,"kind":"note","stdout":{"bytes":10,"lines":1,"dropped":0,"kept":10},"stderr":{"bytes":90,"lines":5,"dropped":0,"kept":90}}"#,
            "\n",
        ),
    )
    .expect("write journal");

    let view = read_progress(&journal).expect("readable").expect("exists");
    let facts = orc_core::dispatch_progress::facts(&view);

    assert_eq!(
        facts.newest_stdout_line_at.as_deref(),
        Some("2026-08-01T12:00:05+00:00"),
        "the last time a stdout LINE appeared was 12:00:05. The journal was \
         re-stamped at 12:03 and 12:09 because stderr moved, and taking the \
         newest frame's `t` would tell the reader a line arrived at 12:09"
    );
    assert_ne!(
        facts.newest_stdout_line_at.as_deref(),
        Some("2026-08-01T12:09:00+00:00"),
        "explicitly not the newest frame"
    );
    assert_eq!(facts.stdout.lines, 1);
    assert_eq!(facts.stderr.lines, 5, "stderr really did move");
    assert_eq!(
        facts.opened_at.as_deref(),
        Some("2026-08-01T12:00:00+00:00"),
        "the open frame is the fallback when no line has ever appeared"
    );
    assert!(!facts.capped(), "kept == bytes here");

    // No stdout line has EVER appeared: `None`, so a reader falls back to
    // "since the worker started" rather than inventing a timestamp.
    fs::write(
        &journal,
        concat!(
            r#"{"v":1,"seq":0,"t":"2026-08-01T12:00:00+00:00","attempt":1,"kind":"open","adapter":"pi","extractable":true}"#,
            "\n",
            r#"{"v":1,"seq":1,"t":"2026-08-01T12:04:00+00:00","attempt":1,"kind":"note","stdout":{"bytes":300,"lines":0,"dropped":0,"kept":300},"stderr":{"bytes":0,"lines":0,"dropped":0,"kept":0}}"#,
            "\n",
        ),
    )
    .expect("rewrite journal");
    let view = read_progress(&journal).expect("readable").expect("exists");
    let facts = orc_core::dispatch_progress::facts(&view);
    assert_eq!(
        facts.newest_stdout_line_at, None,
        "300 bytes and no terminator is a worker mid-line: no complete line \
         has been observed, so there is no time to name"
    );
    assert_eq!(facts.stdout.bytes, 300);
    assert_eq!(facts.stdout.lines, 0);

    // The capped discriminator, in both directions, against a file length that
    // would answer the opposite.
    fs::write(
        &journal,
        concat!(
            r#"{"v":1,"seq":0,"t":"2026-08-01T12:00:00+00:00","attempt":1,"kind":"open","adapter":"pi","extractable":true}"#,
            "\n",
            r#"{"v":1,"seq":1,"t":"2026-08-01T12:04:00+00:00","attempt":1,"kind":"note","stdout":{"bytes":300001,"lines":1,"dropped":0,"kept":0},"stderr":{"bytes":0,"lines":0,"dropped":0,"kept":0}}"#,
            "\n",
        ),
    )
    .expect("rewrite journal");
    let view = read_progress(&journal).expect("readable").expect("exists");
    let facts = orc_core::dispatch_progress::facts(&view);
    assert!(
        facts.capped(),
        "300 KB observed and nothing mirrored is capped, however empty the file"
    );
    assert!(
        facts.capped_before_first_line(),
        "and it is the specific case the log's own hole produces: `kept == 0` \
         with `bytes > 0`, which `kept` against `log_max_bytes` calls quiet"
    );

    let _ = fs::remove_dir_all(&home);
}
