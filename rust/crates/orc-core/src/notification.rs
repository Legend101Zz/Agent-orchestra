use std::fs;
use std::process::{Command, Stdio};

use crate::model::RunMeta;
use crate::quota::load_config;
use crate::registry::{atomic_write_json, home, list_runs};

fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn session_key(session: &str) -> String {
    session
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn send(title: &str, body: &str) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript(body),
        escape_applescript(title)
    );
    let _ = Command::new("osascript")
        .args(["-e", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn run_finished(meta: &RunMeta) {
    let mode = load_config().notifications;
    if mode == "off" {
        return;
    }
    if meta.status == "failed" || meta.attention.as_deref() == Some("handoff_needed") {
        let reason = meta.failure_kind.as_deref().unwrap_or("worker failed");
        send("orc · attention needed", &format!("{} · {reason}", meta.id));
        return;
    }
    if mode == "all" && meta.status == "done" {
        send("orc · worker complete", &meta.id);
    }
    let Some(session) = &meta.session else {
        return;
    };
    let Ok(runs) = list_runs(false) else {
        return;
    };
    let members = runs
        .iter()
        .filter(|run| run.session.as_deref() == Some(session))
        .collect::<Vec<_>>();
    if members.is_empty() || members.iter().any(|run| !run.is_terminal()) {
        return;
    }
    let marker = home()
        .join("sessions")
        .join(session_key(session))
        .join("completion.json");
    if marker.exists() {
        return;
    }
    if let Some(parent) = marker.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return;
    }
    if atomic_write_json(
        &marker,
        &serde_json::json!({"session": session, "notified": true}),
    )
    .is_ok()
    {
        send("orc · session complete", session);
    }
}
