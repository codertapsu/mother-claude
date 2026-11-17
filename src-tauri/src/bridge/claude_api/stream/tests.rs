use super::*;
use serde_json::json;

fn se(event: Value) -> Value {
    json!({ "type": "stream_event", "event": event, "parent_tool_use_id": null })
}

fn start() -> Value {
    se(json!({ "type": "message_start", "message": {
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-5-5",
        "content": [], "stop_reason": null, "stop_sequence": null,
        "usage": { "input_tokens": 12, "output_tokens": 1 },
    }}))
}

fn text_start(index: u64) -> Value {
    se(
        json!({ "type": "content_block_start", "index": index, "content_block": { "type": "text", "text": "" } }),
    )
}

fn text_delta(index: u64, t: &str) -> Value {
    se(
        json!({ "type": "content_block_delta", "index": index, "delta": { "type": "text_delta", "text": t } }),
    )
}

fn stop(index: u64) -> Value {
    se(json!({ "type": "content_block_stop", "index": index }))
}

fn end(reason: &str, out: u64) -> Vec<Value> {
    vec![
        se(
            json!({ "type": "message_delta", "delta": { "stop_reason": reason, "stop_sequence": null },
                   "usage": { "output_tokens": out } }),
        ),
        se(json!({ "type": "message_stop" })),
    ]
}

fn run(tr: &mut Translator, msgs: &[Value]) -> Vec<Event> {
    msgs.iter().flat_map(|m| tr.push(m)).collect()
}

fn texts(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|e| e.data.pointer("/delta/text").and_then(Value::as_str))
        .collect()
}

#[test]
fn the_apis_events_pass_through_and_build_the_message() {
    let mut tr = Translator::new(Vec::new());
    let mut msgs = vec![
        start(),
        text_start(0),
        text_delta(0, "Hel"),
        text_delta(0, "lo"),
        stop(0),
    ];
    msgs.extend(end("end_turn", 7));
    let events = run(&mut tr, &msgs);
    let names: Vec<&str> = events.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
        ]
    );
    assert!(tr.started() && tr.ended());
    let m = tr.message();
    assert_eq!(m["id"], "msg_1");
    assert_eq!(m["content"], json!([{ "type": "text", "text": "Hello" }]));
    assert_eq!(m["stop_reason"], "end_turn");
    // message_delta's usage is merged into message_start's.
    assert_eq!(
        m["usage"],
        json!({ "input_tokens": 12, "output_tokens": 7 })
    );
    assert_eq!(tr.stop_reason().as_deref(), Some("end_turn"));
    // After the end, nothing more.
    assert!(tr.push(&text_delta(0, "late")).is_empty());
}

#[test]
fn only_the_main_conversations_stream_counts() {
    let mut tr = Translator::new(Vec::new());
    assert!(tr
        .push(&json!({ "type": "assistant", "message": {} }))
        .is_empty());
    let mut sub = start();
    sub["parent_tool_use_id"] = json!("toolu_9");
    assert!(tr.push(&sub).is_empty());
    assert!(!tr.started());
    tr.push(&start());
    assert!(tr.push(&se(json!({ "type": "ping" }))).is_empty());
}

#[test]
fn client_tools_lose_their_prefix_and_their_input_is_whole() {
    let mut tr = Translator::new(Vec::new());
    let mut msgs = vec![
        start(),
        se(
            json!({ "type": "content_block_start", "index": 0, "content_block": {
                "type": "tool_use", "id": "toolu_1", "name": "mcp__client__get_weather", "input": {},
            }}),
        ),
        se(json!({ "type": "content_block_delta", "index": 0,
                   "delta": { "type": "input_json_delta", "partial_json": "{\"city\": \"Pa" } })),
        se(json!({ "type": "content_block_delta", "index": 0,
                   "delta": { "type": "input_json_delta", "partial_json": "ris\"}" } })),
        stop(0),
    ];
    msgs.extend(end("tool_use", 20));
    let events = run(&mut tr, &msgs);
    assert_eq!(events[1].data["content_block"]["name"], "get_weather");
    assert_eq!(
        tr.tool_calls(),
        vec![ToolCall {
            id: "toolu_1".into(),
            name: "get_weather".into(),
            input: json!({ "city": "Paris" }),
        }]
    );
    assert_eq!(
        tr.message()["content"][0]["input"],
        json!({ "city": "Paris" })
    );
}

#[test]
fn a_stop_sequence_ends_the_message_where_it_begins() {
    let mut tr = Translator::new(vec!["STOP".into()]);
    let msgs = vec![
        start(),
        text_start(0),
        text_delta(0, "Hel"),
        text_delta(0, "lo ST"),
        text_delta(0, "OP world"),
        text_delta(0, "never sent"),
    ];
    let events = run(&mut tr, &msgs);
    assert_eq!(texts(&events), "Hello ");
    let names: Vec<&str> = events.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        &names[names.len() - 3..],
        &["content_block_stop", "message_delta", "message_stop"]
    );
    let delta = &events[events.len() - 2].data;
    assert_eq!(delta["delta"]["stop_reason"], "stop_sequence");
    assert_eq!(delta["delta"]["stop_sequence"], "STOP");
    assert!(tr.ended());
    assert_eq!(tr.stopped_at(), Some("STOP"));
    assert_eq!(tr.message()["content"][0]["text"], "Hello ");
    assert_eq!(tr.message()["stop_reason"], "stop_sequence");
}

#[test]
fn text_that_only_began_a_stop_sequence_goes_out_at_the_blocks_end() {
    let mut tr = Translator::new(vec!["STOP".into()]);
    let mut msgs = vec![start(), text_start(0), text_delta(0, "a ST"), stop(0)];
    msgs.extend(end("end_turn", 3));
    let events = run(&mut tr, &msgs);
    assert_eq!(texts(&events), "a ST");
    assert_eq!(tr.message()["content"][0]["text"], "a ST");
    assert_eq!(tr.stopped_at(), None);
}

#[test]
fn a_message_the_turn_left_open_is_closed() {
    let mut tr = Translator::new(Vec::new());
    run(&mut tr, &[start(), text_start(0), text_delta(0, "partial")]);
    let events = tr.finish();
    let names: Vec<&str> = events.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["content_block_stop", "message_delta", "message_stop"]
    );
    assert!(tr.ended());
    assert_eq!(tr.message()["stop_reason"], "end_turn");
    assert!(tr.finish().is_empty());
    // Nothing started: nothing to close.
    assert!(Translator::new(Vec::new()).finish().is_empty());
}

#[test]
fn a_retried_call_starts_the_message_over() {
    let mut tr = Translator::new(Vec::new());
    run(
        &mut tr,
        &[start(), text_start(0), text_delta(0, "first try")],
    );
    let mut msgs = vec![start(), text_start(0), text_delta(0, "second"), stop(0)];
    msgs.extend(end("end_turn", 2));
    run(&mut tr, &msgs);
    assert_eq!(
        tr.message()["content"],
        json!([{ "type": "text", "text": "second" }])
    );
}

#[test]
fn frames_are_server_sent_events() {
    let e = Event::of(json!({ "type": "message_stop" }));
    assert_eq!(
        sse_frame(&e),
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
    );
    assert!(PING_FRAME.starts_with("event: ping\n"));
}
