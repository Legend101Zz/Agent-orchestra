use std::collections::BTreeMap;

use orc_core::model::Config;
use orc_core::quota::{QuotaSample, level_for, parse_remains};
use serde_json::json;

fn config() -> Config {
    Config {
        warn_pct: 25.0,
        block_pct: 10.0,
        cache_ttl_sec: 60,
        max_parallel_workers: 3,
        idle_timeout_sec: 300.0,
        theme: "ember".to_owned(),
        notifications: "actionable".to_owned(),
        advisory_budget_usd: None,
        extra: BTreeMap::new(),
    }
}

#[test]
fn parse_selects_general_and_converts_reset_time() {
    let raw = json!({"model_remains": [
        {"model_name": "video", "current_interval_remaining_percent": 1},
        {"model_name": "general", "current_interval_remaining_percent": 83,
         "current_weekly_remaining_percent": 49, "remains_time": 1_920_000}
    ]});
    let sample = parse_remains(&raw, 12.0).unwrap();
    assert_eq!(sample.five_hour_pct, 83.0);
    assert_eq!(sample.weekly_pct, 49.0);
    assert_eq!(sample.window_resets_in_min, 32);
}

#[test]
fn thresholds_use_the_lower_window() {
    let mut sample = QuotaSample {
        five_hour_pct: 90.0,
        weekly_pct: 90.0,
        window_resets_in_min: 1,
        fetched_at: 1.0,
    };
    assert_eq!(level_for(&sample, &config()), "ok");
    sample.weekly_pct = 20.0;
    assert_eq!(level_for(&sample, &config()), "warn");
    sample.weekly_pct = 10.0;
    assert_eq!(level_for(&sample, &config()), "block");
}
