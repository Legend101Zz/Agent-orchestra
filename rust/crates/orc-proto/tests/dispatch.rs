//! Wire-shape tests for the Phase 4A dispatch protocol.
//!
//! These tests only exercise the additive JSON round-trip for the new
//! dispatch messages and never start a child process or touch the filesystem.

use orc_proto::{
    ClientRequest, DispatchCommand, DispatchSummary, PROTOCOL_VERSION, ServerResponse,
};

#[test]
fn dispatch_request_round_trips_with_optional_linkage() {
    let request = ClientRequest::Dispatch {
        command: DispatchCommand {
            session_id: "session-1".to_owned(),
            task_id: "T0001".to_owned(),
            actor: "brain".to_owned(),
            harness: "hermes".to_owned(),
            pane_id: Some("session-1-worker-1".to_owned()),
            run: Some("W-1".to_owned()),
            prompt: "summarize diff".to_owned(),
            timeout_sec: Some(30),
        },
    };
    let encoded = serde_json::to_vec(&request).expect("encode dispatch request");
    let decoded: ClientRequest = serde_json::from_slice(&encoded).expect("decode dispatch request");
    assert_eq!(decoded, request);

    let additive: ClientRequest = serde_json::from_str(
        r#"{
            "type": "dispatch",
            "command": {
                "session_id": "session-1",
                "task_id": "T0001",
                "actor": "brain",
                "harness": "hermes",
                "pane_id": "session-1-worker-1",
                "run": "W-1",
                "prompt": "summarize diff",
                "timeout_sec": 30,
                "future_capability": true
            }
        }"#,
    )
    .expect("decode additive dispatch");
    assert_eq!(additive, request);
}

#[test]
fn dispatched_response_round_trips_with_linkage_and_failure() {
    let summary = DispatchSummary {
        id: "D-hermes-1-aaa".to_owned(),
        session_id: "session-1".to_owned(),
        task_id: "T0001".to_owned(),
        actor: "brain".to_owned(),
        harness: "hermes".to_owned(),
        pane_id: Some("session-1-worker-1".to_owned()),
        run: Some("W-1".to_owned()),
        command_line: "hermes -z 'summarize diff'".to_owned(),
        status: "failed".to_owned(),
        exit_code: Some(127),
        failure_kind: Some("missing_executable".to_owned()),
        error: Some("MISSING EXECUTABLE: hermes command not found on PATH".to_owned()),
        updated_at: "2026-07-11T20:21:00+00:00".to_owned(),
    };
    let response = ServerResponse::Dispatched { record: summary };
    let encoded = serde_json::to_vec(&response).expect("encode dispatched");
    let decoded: ServerResponse = serde_json::from_slice(&encoded).expect("decode dispatched");
    assert_eq!(decoded, response);
}

#[test]
fn dispatch_board_response_omits_empty_optional_fields() {
    let summary = DispatchSummary {
        id: "D-hermes-1-aaa".to_owned(),
        session_id: "session-1".to_owned(),
        task_id: "T0001".to_owned(),
        actor: "brain".to_owned(),
        harness: "hermes".to_owned(),
        pane_id: None,
        run: None,
        command_line: "hermes -z 'summarize diff'".to_owned(),
        status: "confirmed".to_owned(),
        exit_code: Some(0),
        failure_kind: None,
        error: None,
        updated_at: "2026-07-11T20:21:00+00:00".to_owned(),
    };
    let response = ServerResponse::DispatchBoard {
        session_id: "session-1".to_owned(),
        records: vec![summary],
    };
    let encoded = serde_json::to_vec(&response).expect("encode board");
    let text = std::str::from_utf8(&encoded).expect("utf8 encoded board");
    assert!(text.contains("\"type\":\"dispatch_board\""));
    assert!(text.contains("\"status\":\"confirmed\""));
    assert!(!text.contains("\"pane_id\""));
    assert!(!text.contains("\"run\""));
    assert!(!text.contains("\"failure_kind\""));
    assert!(!text.contains("\"error\""));
    let decoded: ServerResponse = serde_json::from_slice(&encoded).expect("decode board");
    assert_eq!(decoded, response);
}

#[test]
fn dispatch_board_request_round_trips() {
    let request = ClientRequest::DispatchBoard {
        session_id: "session-1".to_owned(),
    };
    let encoded = serde_json::to_vec(&request).expect("encode board request");
    let decoded: ClientRequest = serde_json::from_slice(&encoded).expect("decode board request");
    assert_eq!(decoded, request);
}

#[test]
fn hello_request_still_round_trips() {
    let request = ClientRequest::Hello {
        version: PROTOCOL_VERSION,
    };
    let encoded = serde_json::to_vec(&request).expect("encode hello");
    let decoded: ClientRequest = serde_json::from_slice(&encoded).expect("decode hello");
    assert_eq!(decoded, request);
}
