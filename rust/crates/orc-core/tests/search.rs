use std::fs;

use orc_core::model::RunMeta;
use orc_core::search::search_runs;
use serde_json::json;

#[test]
fn searches_only_the_bounded_tail_of_large_logs() {
    let run_dir = std::env::temp_dir().join(format!("orc-search-{}", std::process::id()));
    fs::create_dir_all(&run_dir).unwrap();
    let mut contents = "old-only\n".to_owned();
    contents.push_str(&"padding\n".repeat(300_000));
    contents.push_str("tail-only\n");
    fs::write(run_dir.join("output.log"), contents).unwrap();

    let mut run: RunMeta = serde_json::from_value(json!({
        "id": "large-run",
        "task": "unrelated",
        "brain": "codex",
        "cwd": "/tmp",
        "provider": "minimax",
        "model": "MiniMax-M3",
        "status": "done",
        "started_at": "2026-07-11T00:00:00Z",
        "tokens": {"estimated_total": 1}
    }))
    .unwrap();
    run.run_dir = Some(run_dir.clone());

    assert!(search_runs(&[run.clone()], "old-only", 10).is_empty());
    assert_eq!(search_runs(&[run], "tail-only", 10).len(), 1);
    fs::remove_dir_all(run_dir).unwrap();
}
