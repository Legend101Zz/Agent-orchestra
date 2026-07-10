use orc_core::runner::{extract_text, extract_usage};
use serde_json::json;

#[test]
fn extracts_delta_and_exact_usage() {
    let delta = json!({
        "type": "message_update",
        "assistantMessageEvent": {"type": "text_delta", "delta": "hello"},
        "message": {"large": "snapshot"}
    });
    assert_eq!(extract_text(&delta), Some("hello"));
    let end = json!({"type":"agent_end","messages":[
        {"role":"assistant","usage":{"input":120,"output":30,"cacheRead":2048,
         "totalTokens":2198,"cost":{"total":0.0002014}}}
    ]});
    let usage = extract_usage(&end).unwrap();
    assert_eq!(usage.input, 120);
    assert_eq!(usage.output, 30);
    assert_eq!(usage.cache_read, 2048);
    assert_eq!(usage.total, 2198);
    assert_eq!(usage.cost_usd, Some(0.000201));
}
