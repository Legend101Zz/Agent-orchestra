#![allow(unsafe_code)]

//! Durable slot-leasing integration tests for quota guard v2 (issue #7).
//!
//! These need an isolated `ORC_HOME`, so they live here (an integration test
//! crate can `#![allow(unsafe_code)]`) rather than as library unit tests, where
//! the workspace denies `unsafe`. They prove the per-harness concurrency cap is
//! enforced by durable, cross-session leases: N slots admit N holders and queue
//! the next, a released slot frees exactly one, each harness has its own pool,
//! and an abandoned lease is pruned so it never wedges the cap forever.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use orc_core::spawn_guard::{DEFAULT_LEASE_TTL, acquire_slot, active_slots};

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
    let dir = std::env::temp_dir().join(format!("orc-slot-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&dir).expect("create home");
    // SAFETY: ORC_HOME mutation is serialized through `lock()`.
    unsafe { std::env::set_var("ORC_HOME", &dir) };
    dir
}

#[test]
fn cap_of_n_admits_n_then_refuses_the_next_until_one_releases() {
    let _guard = lock();
    let home = fresh_home("cap");

    // Cap of 2: two acquisitions succeed, the third is refused (queue it).
    let first = acquire_slot("hermes", 2, DEFAULT_LEASE_TTL, None)
        .expect("acquire 1")
        .expect("slot 1 free");
    let second = acquire_slot("hermes", 2, DEFAULT_LEASE_TTL, None)
        .expect("acquire 2")
        .expect("slot 2 free");
    assert_eq!(active_slots("hermes").expect("count"), 2);
    assert!(
        acquire_slot("hermes", 2, DEFAULT_LEASE_TTL, None)
            .expect("acquire 3")
            .is_none(),
        "the third acquisition must be refused at cap 2"
    );

    // Releasing one frees exactly one slot.
    drop(first);
    assert_eq!(active_slots("hermes").expect("count after release"), 1);
    let third = acquire_slot("hermes", 2, DEFAULT_LEASE_TTL, None)
        .expect("acquire 3 retry")
        .expect("slot free after release");
    assert_eq!(active_slots("hermes").expect("count refilled"), 2);

    drop(second);
    drop(third);
    assert_eq!(active_slots("hermes").expect("count drained"), 0);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn each_harness_has_an_independent_pool() {
    let _guard = lock();
    let home = fresh_home("independent");
    let _pi = acquire_slot("pi-m3", 1, DEFAULT_LEASE_TTL, None)
        .expect("acquire pi")
        .expect("pi slot free");
    // pi-m3 is full at cap 1, but hermes is untouched — pools are per-harness.
    assert!(
        acquire_slot("pi-m3", 1, DEFAULT_LEASE_TTL, None)
            .expect("second pi")
            .is_none()
    );
    let _hermes = acquire_slot("hermes", 1, DEFAULT_LEASE_TTL, None)
        .expect("acquire hermes")
        .expect("hermes slot free");
    let _ = fs::remove_dir_all(home);
}

#[test]
fn an_abandoned_lease_is_pruned_and_does_not_wedge_the_cap() {
    let _guard = lock();
    let home = fresh_home("expired");
    // A 1ms TTL lease is abandoned the instant time advances past it.
    let lease = acquire_slot("codex", 1, Duration::from_millis(1), None)
        .expect("acquire codex")
        .expect("codex slot free");
    // Forget the guard so its file is NOT removed on drop — simulate a
    // dispatcher that died still holding the lease.
    std::mem::forget(lease);
    std::thread::sleep(Duration::from_millis(5));
    // The stale lease is pruned, so a fresh slot is available.
    assert_eq!(
        active_slots("codex").expect("count prunes stale"),
        0,
        "an abandoned lease must not count toward the cap forever"
    );
    let refreshed = acquire_slot("codex", 1, DEFAULT_LEASE_TTL, None)
        .expect("acquire after prune")
        .expect("slot free after prune");
    drop(refreshed);
    let _ = fs::remove_dir_all(home);
}
