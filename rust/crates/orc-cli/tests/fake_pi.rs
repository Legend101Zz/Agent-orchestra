#![allow(unsafe_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

fn root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "orc-fake-pi-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn setup(label: &str, quota: Option<(u8, u8)>) -> (PathBuf, PathBuf) {
    let root = root(label);
    let home = root.join("orchestra");
    let bin = root.join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    if let Some((five, week)) = quota {
        fs::write(
            home.join("quota.json"),
            serde_json::to_vec(&json!({
                "five_hour_pct": five,
                "weekly_pct": week,
                "window_resets_in_min": 60,
                "fetched_at": 4_102_444_800.0
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let pi = bin.join("pi");
    fs::write(
        &pi,
        r##"#!/usr/bin/env bash
set -eu
if [[ " $* " == *" --mode rpc "* ]]; then
  turn=0
  while IFS= read -r prompt; do
    turn=$((turn + 1))
    printf '%s\n' '{"type":"response","command":"prompt","success":true}'
    printf '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"rpc-turn-%s"}}\n' "$turn"
    if [[ "$turn" -eq 1 ]]; then sleep 0.4; fi
    if [[ "$turn" -eq 1 ]]; then total=10; else total=12; fi
    printf '{"type":"agent_end","messages":[{"usage":{"input":8,"output":2,"cacheRead":0,"totalTokens":%s,"cost":{"total":0.00001}}}]}\n' "$total"
  done
  exit 0
fi
task="${@: -1}"
if [[ "$task" == *IDLE* ]]; then
  printf '%s\n' '{"type":"agent_start"}'
  sleep 30
fi
if [[ "$task" == *KILL* ]]; then
  trap 'exit 143' TERM
  while true; do sleep 1; done
fi
if [[ "$task" == *CONTEXT* ]]; then
  printf '%s\n' '{"type":"agent_end","stopReason":"max_tokens","messages":[{"usage":{"input":99,"output":10,"cacheRead":0,"totalTokens":109}}]}'
  exit 0
fi
printf '%s\n' '{"type":"agent_start"}'
printf '%s\n' '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"json-ok"}}'
printf '%s\n' '{"type":"agent_end","messages":[{"usage":{"input":120,"output":30,"cacheRead":2048,"totalTokens":2198,"cost":{"total":0.000201}}}]}'
"##,
    ).unwrap();
    fs::set_permissions(&pi, fs::Permissions::from_mode(0o755)).unwrap();
    (root, home)
}

fn run(root: &Path, home: &Path, args: &[&str]) -> Output {
    let inherited = std::env::var("PATH").unwrap_or_default();
    Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .env("ORC_HOME", home)
        .env("HOME", root.join("empty-home"))
        .env(
            "PATH",
            format!("{}:{inherited}", root.join("bin").display()),
        )
        .output()
        .unwrap()
}

fn runs(home: &Path) -> Vec<Value> {
    let mut values = fs::read_dir(home.join("runs"))
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path().join("meta.json")).ok())
        .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        left["created_ts"]
            .as_f64()
            .partial_cmp(&right["created_ts"].as_f64())
            .unwrap()
    });
    values
}

fn wait_for(home: &Path, id: &str, status: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(value) = runs(home)
            .into_iter()
            .find(|value| value["id"].as_str() == Some(id))
            && value["status"] == status
        {
            return value;
        }
        assert!(Instant::now() < deadline, "run {id} did not reach {status}");
        thread::sleep(Duration::from_millis(40));
    }
}

#[test]
fn fake_pi_suite_covers_json_rpc_usage_kill_timeout_context_links_and_quota() {
    let (root, home) = setup("all", Some((90, 90)));

    let json_run = run(&root, &home, &["run", "JSON exact", "--brain", "codex"]);
    assert!(
        json_run.status.success(),
        "{}",
        String::from_utf8_lossy(&json_run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&json_run.stdout), "json-ok\n");
    let exact = runs(&home).pop().unwrap();
    assert_eq!(exact["tokens"]["total"], 2198);

    let rpc = run(
        &root,
        &home,
        &["rpc", "initial", "--bg", "--brain", "codex"],
    );
    assert!(rpc.status.success());
    let rpc_id = String::from_utf8(rpc.stdout).unwrap().trim().to_owned();
    wait_for(&home, &rpc_id, "running");
    let sent = run(&root, &home, &["send", &rpc_id, "follow-up once"]);
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );
    let done = wait_for(&home, &rpc_id, "done");
    assert_eq!(done["tokens"]["total"], 12);
    let processed = home.join("runs").join(&rpc_id).join("inbox/processed");
    assert_eq!(fs::read_dir(processed).unwrap().count(), 1);

    let idle = run(&root, &home, &["run", "IDLE", "--idle-timeout", "0.1"]);
    assert_eq!(idle.status.code(), Some(124));
    let idle_meta = runs(&home)
        .into_iter()
        .find(|value| value["task"] == "IDLE")
        .unwrap();
    assert_eq!(idle_meta["failure_kind"], "idle_timeout");

    let context = run(&root, &home, &["run", "CONTEXT"]);
    assert!(context.status.success());
    let context_meta = runs(&home)
        .into_iter()
        .find(|value| value["task"] == "CONTEXT")
        .unwrap();
    assert_eq!(context_meta["failure_kind"], "context_exhausted");

    let killed = run(&root, &home, &["run", "KILL", "--bg"]);
    let killed_id = String::from_utf8(killed.stdout).unwrap().trim().to_owned();
    wait_for(&home, &killed_id, "running");
    assert!(run(&root, &home, &["kill", &killed_id]).status.success());
    wait_for(&home, &killed_id, "killed");

    let retry = run(&root, &home, &["retry", exact["id"].as_str().unwrap()]);
    let retry_id = String::from_utf8(retry.stdout).unwrap().trim().to_owned();
    assert_eq!(wait_for(&home, &retry_id, "done")["retry_of"], exact["id"]);
    let handoff = run(
        &root,
        &home,
        &[
            "handoff",
            exact["id"].as_str().unwrap(),
            "verified remainder",
        ],
    );
    let handoff_id = String::from_utf8(handoff.stdout).unwrap().trim().to_owned();
    assert_eq!(
        wait_for(&home, &handoff_id, "done")["handoff_from"],
        exact["id"]
    );

    let (warn_root, warn_home) = setup("warn", Some((20, 90)));
    let warned = run(&warn_root, &warn_home, &["run", "warn proceeds"]);
    assert!(warned.status.success());
    assert!(String::from_utf8_lossy(&warned.stderr).contains("ORC WARNING:"));
    let (block_root, block_home) = setup("block", Some((5, 90)));
    let blocked = run(&block_root, &block_home, &["run", "blocked"]);
    assert_eq!(blocked.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("ORC BLOCKED:"));
    let (unknown_root, unknown_home) = setup("unknown", None);
    let unknown = run(&unknown_root, &unknown_home, &["run", "unknown fail open"]);
    assert!(unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("ORC NOTE:"));

    for directory in [root, warn_root, block_root, unknown_root] {
        let _ = fs::remove_dir_all(directory);
    }
}
