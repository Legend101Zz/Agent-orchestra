//! Per-harness spawn concurrency caps and durable slot leasing (issue #7).
//!
//! Quota guard v1 ([`crate::quota`]) protects the MiniMax coding-plan token
//! budget. This is the second half of "never hurt the user's subscriptions":
//! a bound on how many workers of one harness run **at the same time**, so
//! pi-orchestra never opens more concurrent sessions than a provider's
//! rate/seat limits tolerate.
//!
//! ## The cap
//!
//! The effective cap for a harness is, in order:
//!
//! 1. an explicit user override in `harnesses.json` (`concurrency.<key>`), else
//! 2. a conservative per-*adapter* default ([`default_concurrency_for`]).
//!
//! It is always at least 1 — a configured worker can always run one at a time —
//! and is independent of the global session worker-pool bound
//! ([`crate::bench::HarnessRegistry::max_parallel_workers`]), which limits how
//! many workers a *session* opens, not how many the provider will tolerate.
//!
//! The per-adapter defaults are deliberately conservative, not probed at
//! runtime: no harness advertises its account's concurrency limit in `--help`,
//! so — exactly like the rate-limit signal strings (see [`crate::ratelimit`]) —
//! these are a maintained table the user overrides, never a guess dressed up as
//! a measurement.
//!
//! ## The leases
//!
//! A slot is a durable file under `~/.orchestra/slots/<harness>/`. Because the
//! directory is keyed by **harness, not session**, the cap is honored across
//! every session and every `pio` process on the machine (acceptance check 3).
//! [`acquire_slot`] takes a short per-harness lock, prunes leases whose owning
//! process is gone or whose TTL elapsed, counts what remains, and — only if
//! under the cap — writes a new lease and hands back a [`SlotLease`] RAII guard
//! that deletes its file on drop. A caller who cannot get a slot gets
//! `Ok(None)` and is expected to queue the work, never to spawn anyway.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bench::HarnessRegistry;
use crate::registry::{atomic_write_json, home, pid_alive};

/// Default TTL for a slot lease when a caller does not supply one.
///
/// A backstop against a crashed dispatcher whose pid was later recycled: a
/// lease older than this is considered abandoned even if some live process now
/// happens to own that pid.
pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(1800);

/// A `.slots.lock` older than this whose holder cannot be proven alive is
/// treated as abandoned and reclaimed. The lock is only ever held for a few
/// microseconds (prune + count + one write), so any lock this old is a crash
/// artifact, not real contention.
const STALE_LOCK: Duration = Duration::from_secs(30);

const LOCK_ATTEMPTS: usize = 200;
const LOCK_WAIT: Duration = Duration::from_millis(5);

static LEASE_NONCE: AtomicU64 = AtomicU64::new(0);

/// Conservative default concurrent-worker cap for one adapter.
///
/// These mirror the safest reading of each provider's per-account limits; the
/// user raises or lowers them per harness in `harnesses.json`. Unknown adapters
/// get a single slot so a brand-new harness never floods a provider by default.
#[must_use]
pub fn default_concurrency_for(adapter: &str) -> usize {
    match adapter {
        // Subscription seats that tolerate a small fan-out of headless runs.
        "pi" | "hermes" => 3,
        // Frontier coding subscriptions with tighter per-account rate limits.
        "claude" | "codex" | "opencode" => 2,
        // Anything not yet characterized: one at a time until the user opts in.
        _ => 1,
    }
}

/// The effective concurrent-worker cap for one registry harness key.
///
/// User override (`registry.concurrency[key]`) wins; otherwise the adapter
/// default. Never returns 0 — a configured worker can always run singly.
#[must_use]
pub fn effective_cap(registry: &HarnessRegistry, harness_key: &str) -> usize {
    if let Some(&override_cap) = registry.concurrency.get(harness_key) {
        return override_cap.max(1);
    }
    let adapter = registry
        .harnesses
        .get(harness_key)
        .map_or(harness_key, |config| config.adapter.as_str());
    default_concurrency_for(adapter).max(1)
}

/// Durable record of one held concurrency slot.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct LeaseRecord {
    /// Harness registry key this slot belongs to.
    harness: String,
    /// Process id of the dispatcher holding the slot.
    pid: u32,
    /// Epoch seconds when the slot was acquired.
    acquired_at: f64,
    /// Seconds after which the lease is considered abandoned.
    ttl_sec: f64,
    /// Dispatch id linked to the slot, when one is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dispatch: Option<String>,
    /// Unknown future fields, preserved on round-trip.
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

/// RAII handle to a held concurrency slot; releases the slot when dropped.
#[derive(Debug)]
pub struct SlotLease {
    path: PathBuf,
    harness: String,
    released: bool,
}

impl SlotLease {
    /// Harness key this lease was acquired for.
    #[must_use]
    pub fn harness(&self) -> &str {
        &self.harness
    }

    /// Explicitly release the slot now instead of at drop.
    pub fn release(mut self) {
        self.remove();
    }

    fn remove(&mut self) {
        if !self.released {
            let _ = fs::remove_file(&self.path);
            self.released = true;
        }
    }
}

impl Drop for SlotLease {
    fn drop(&mut self) {
        self.remove();
    }
}

fn epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// Filesystem-safe encoding of a harness key for a directory name.
fn harness_key_dir(harness: &str) -> String {
    harness
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

fn slots_dir(harness: &str) -> PathBuf {
    home().join("slots").join(harness_key_dir(harness))
}

struct SlotLock {
    path: PathBuf,
    _file: File,
}

impl Drop for SlotLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn lock_slots(dir: &Path) -> Result<SlotLock> {
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(".slots.lock");
    for _ in 0..LOCK_ATTEMPTS {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                // Record the holder pid so a crashed holder's lock is reclaimable.
                let _ = writeln!(file, "{}", std::process::id());
                return Ok(SlotLock { path, _file: file });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                // Self-heal a lock abandoned by a crashed dispatcher (parity with
                // the dead-pid/TTL lease prune) so a SIGKILL mid-hold never wedges
                // this harness's cap forever (reviewer Fix 2).
                if !reclaim_if_stale(&path) {
                    thread::sleep(LOCK_WAIT);
                }
            }
            Err(error) => return Err(error).with_context(|| format!("lock {}", path.display())),
        }
    }
    anyhow::bail!(
        "slot guard for {} is busy; retry the command",
        dir.display()
    )
}

/// Reclaim an abandoned `.slots.lock` if its holder is dead or it is older than
/// [`STALE_LOCK`]. Returns whether the caller should retry immediately.
///
/// The steal is atomic: only one racer can `rename` the lockfile away, so a
/// concurrent reclaim can never delete a lock a different process just recreated
/// — losers see the source gone and simply retry `create_new`.
fn reclaim_if_stale(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        // Already gone (a racer reclaimed it) — retry the create immediately.
        return true;
    };
    let aged = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|elapsed| elapsed >= STALE_LOCK);
    let holder = fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok());
    // Dead recorded holder -> reclaim now. Unknown holder (empty/old-binary
    // lock) or a live-but-recycled pid -> only reclaim once it is clearly aged,
    // so a genuinely held lock is never stolen from a live holder.
    let reclaimable = match holder {
        Some(pid) => !pid_alive(Some(pid)) || aged,
        None => aged,
    };
    if !reclaimable {
        return false;
    }
    let stolen = path.with_extension(format!("stale.{}", std::process::id()));
    if fs::rename(path, &stolen).is_ok() {
        let _ = fs::remove_file(&stolen);
    }
    // Whether we won the steal or another racer did, the stale lock is going or
    // gone; retry the create.
    true
}

/// Whether a lease is still live: its owning process is alive and the TTL has
/// not elapsed. A missing/corrupt lease file counts as not live (and is pruned).
fn lease_is_live(record: &LeaseRecord, now: f64) -> bool {
    let within_ttl = now - record.acquired_at < record.ttl_sec;
    within_ttl && pid_alive(Some(record.pid))
}

/// Prune dead leases in an already-locked directory and return the live count.
fn prune_and_count(dir: &PathBuf) -> usize {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    let now = epoch_seconds();
    let mut live = 0;
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let record = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LeaseRecord>(&bytes).ok());
        match record {
            Some(record) if lease_is_live(&record, now) => live += 1,
            _ => {
                let _ = fs::remove_file(&path);
            }
        }
    }
    live
}

/// Count the live (non-abandoned) slots currently held for one harness.
///
/// Prunes abandoned leases as a side effect so the count is honest.
pub fn active_slots(harness: &str) -> Result<usize> {
    let dir = slots_dir(harness);
    if !dir.exists() {
        return Ok(0);
    }
    let _lock = lock_slots(&dir)?;
    Ok(prune_and_count(&dir))
}

/// Try to acquire one concurrency slot for `harness`, bounded by `cap`.
///
/// Returns `Ok(Some(lease))` when a slot was free (the returned guard holds it
/// until dropped), or `Ok(None)` when the harness is already at `cap` live
/// slots — the caller must then queue the work rather than spawn it. The check
/// and the write happen under a per-harness lock, so two concurrent dispatchers
/// can never both take the last slot.
pub fn acquire_slot(
    harness: &str,
    cap: usize,
    ttl: Duration,
    dispatch: Option<&str>,
) -> Result<Option<SlotLease>> {
    let cap = cap.max(1);
    let dir = slots_dir(harness);
    let _lock = lock_slots(&dir)?;
    if prune_and_count(&dir) >= cap {
        return Ok(None);
    }
    let nonce = LEASE_NONCE.fetch_add(1, Ordering::Relaxed);
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let file_name = format!("{epoch_ms}-{}-{nonce:06x}.json", std::process::id());
    let path = dir.join(file_name);
    let record = LeaseRecord {
        harness: harness.to_owned(),
        pid: std::process::id(),
        acquired_at: epoch_seconds(),
        ttl_sec: ttl.as_secs_f64(),
        dispatch: dispatch.map(ToOwned::to_owned),
        extra: BTreeMap::new(),
    };
    atomic_write_json(&path, &record)?;
    Ok(Some(SlotLease {
        path,
        harness: harness.to_owned(),
        released: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::HarnessRegistry;

    // Slot-leasing behavior is exercised in `tests/spawn_guard.rs`, which can
    // `#![allow(unsafe_code)]` to isolate `ORC_HOME`; the workspace denies
    // `unsafe` in library code, so only the pure cap logic is a unit test here.

    #[test]
    fn effective_cap_prefers_override_then_adapter_default_then_one() {
        let mut registry = HarnessRegistry::default();
        // hermes adapter default is 3.
        assert_eq!(effective_cap(&registry, "hermes"), 3);
        // claude adapter default is 2.
        assert_eq!(effective_cap(&registry, "claude"), 2);
        // A user override wins over the adapter default.
        registry.concurrency.insert("hermes".to_owned(), 1);
        assert_eq!(effective_cap(&registry, "hermes"), 1);
        // A zero override is floored to one.
        registry.concurrency.insert("claude".to_owned(), 0);
        assert_eq!(effective_cap(&registry, "claude"), 1);
        // An unknown key with no config gets the safe single slot.
        assert_eq!(effective_cap(&registry, "mystery"), 1);
    }

    #[test]
    fn default_concurrency_is_conservative_for_unknown_adapters() {
        assert_eq!(default_concurrency_for("pi"), 3);
        assert_eq!(default_concurrency_for("claude"), 2);
        assert_eq!(default_concurrency_for("brand-new-harness"), 1);
    }
}
