use std::collections::BTreeMap;

use orc_core::metrics::{delegated_value, worker_stats};
use orc_core::model::{RunMeta, Tokens};

fn run(brain: &str, exact: bool) -> RunMeta {
    RunMeta {
        id: format!("{brain}-{exact}"),
        task: "task".to_owned(),
        brain: brain.to_owned(),
        cwd: "/tmp".to_owned(),
        provider: "minimax".to_owned(),
        model: "MiniMax-M3".to_owned(),
        pid: None,
        status: "done".to_owned(),
        started_at: "2026-07-10T00:00:00+00:00".to_owned(),
        created_ts: 1.0,
        ended_at: None,
        exit_code: Some(0),
        tokens: if exact {
            Tokens {
                estimated_total: 1_100,
                input: Some(1_000),
                output: Some(100),
                cache_read: Some(0),
                total: Some(1_100),
                cost_usd: Some(0.00042),
                extra: BTreeMap::new(),
            }
        } else {
            Tokens {
                estimated_total: 2_000,
                ..Tokens::default()
            }
        },
        session: Some("s".to_owned()),
        name: None,
        mode: None,
        retry_of: None,
        handoff_from: None,
        attention: None,
        failure_kind: None,
        brain_model: None,
        extra: BTreeMap::new(),
        run_dir: None,
    }
}

#[test]
fn worker_and_value_stats_keep_exactness_visible() {
    let runs = vec![run("codex", true), run("human", false)];
    let stats = worker_stats(&runs);
    assert_eq!(stats.runs, 2);
    assert_eq!(stats.exact.runs, 1);
    assert_eq!(stats.estimated.runs, 1);
    assert_eq!(stats.by_session["s"].runs, 2);
    let value = delegated_value(&runs);
    assert!(value.saved_usd > 0.0);
    assert!(value.exact_share > 0.0 && value.exact_share < 1.0);
}
