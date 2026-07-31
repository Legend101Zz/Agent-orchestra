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
//! that guarantee on purpose — see `docs/notes/2026-07-31-issue-49-phase2-evidence.md`
//! for the mutation table.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use orc_core::bench::{HarnessConfig, HarnessRegistry, create_session, write_harness_registry};
use orc_core::dispatch::{
    self, DispatchActor, DispatchRecord, DispatchRequest, PROGRESS_LOG_MAX_BYTES, progress_paths,
};
use orc_core::dispatch_progress::read_progress;
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
/// is printed and recorded in `docs/notes/`, and the assertion is a loose
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
