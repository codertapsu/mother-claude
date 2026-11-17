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

fn tool_start(index: u64, id: &str, name: &str) -> Value {
    se(json!({ "type": "content_block_start", "index": index,
               "content_block": { "type": "tool_use", "id": id, "name": name, "input": {} } }))
}

fn json_delta(index: u64, part: &str) -> Value {
    se(json!({ "type": "content_block_delta", "index": index,
               "delta": { "type": "input_json_delta", "partial_json": part } }))
}

fn tool_result(id: &str, error: bool) -> Value {
    json!({ "type": "user", "parent_tool_use_id": null, "message": { "role": "user", "content": [{
        "type": "tool_result", "tool_use_id": id, "is_error": error,
        "content": if error { "Output does not match required schema" } else { "Structured output provided successfully" },
    }]}})
}

fn names(events: &[Event]) -> Vec<&str> {
    events.iter().map(|e| e.name.as_str()).collect()
}

fn structured() -> Translator {
    Translator::new(Vec::new()).structured(true)
}

/// As Claude Code 2.1.326 sends it: the call's blocks, its tool result,
/// then the message's end.
#[test]
fn a_structured_answer_is_the_apis_text_of_json() {
    let mut tr = structured();
    let mut msgs = vec![
        start(),
        se(json!({ "type": "content_block_start", "index": 0,
                   "content_block": { "type": "thinking", "thinking": "", "signature": "" } })),
        se(json!({ "type": "content_block_delta", "index": 0,
                   "delta": { "type": "signature_delta", "signature": "sig" } })),
        stop(0),
        text_start(1),
        text_delta(1, "Here is the JSON."),
        stop(1),
        tool_start(2, "toolu_so", STRUCTURED_TOOL),
        json_delta(2, "{\"lang\": \"fr\", "),
        json_delta(2, "\"greeting\": \"Bonjour\"}"),
        stop(2),
        tool_result("toolu_so", false),
    ];
    msgs.extend(end("tool_use", 7));
    let events = run(&mut tr, &msgs);
    assert_eq!(
        names(&events),
        [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
        ]
    );
    assert_eq!(events[4].data["index"], 1);
    assert_eq!(events[4].data["content_block"]["type"], "text");
    assert_eq!(
        texts(&events),
        "{\"lang\": \"fr\", \"greeting\": \"Bonjour\"}"
    );
    assert_eq!(events[7].data["delta"]["stop_reason"], "end_turn");
    assert!(tr.ended());
    assert_eq!(tr.stop_reason().as_deref(), Some("end_turn"));
    assert!(tr.tool_calls().is_empty());
    let m = tr.message();
    assert_eq!(m["content"].as_array().unwrap().len(), 2);
    assert_eq!(m["content"][0]["type"], "thinking");
    let answer: Value = serde_json::from_str(m["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(answer, json!({ "lang": "fr", "greeting": "Bonjour" }));
}

#[test]
fn an_output_the_schema_refused_is_tried_again_in_the_same_message() {
    let mut tr = structured();
    let mut msgs = vec![
        start(),
        tool_start(0, "toolu_1", STRUCTURED_TOOL),
        json_delta(0, "{\"lang\": 1}"),
        stop(0),
        tool_result("toolu_1", true),
    ];
    msgs.extend(end("tool_use", 5));
    msgs.extend([
        start(),
        tool_start(0, "toolu_2", STRUCTURED_TOOL),
        json_delta(0, "{\"lang\": \"fr\"}"),
        stop(0),
        tool_result("toolu_2", false),
    ]);
    msgs.extend(end("tool_use", 6));
    let events = run(&mut tr, &msgs);
    // One message for the client: an SDK refuses a second message_start.
    assert_eq!(
        names(&events)
            .iter()
            .filter(|n| **n == "message_start")
            .count(),
        1
    );
    assert_eq!(
        names(&events)
            .iter()
            .filter(|n| **n == "message_stop")
            .count(),
        1
    );
    assert_eq!(texts(&events), "{\"lang\": \"fr\"}");
    assert_eq!(events[1].data["index"], 0);
    assert!(tr.ended());
    assert_eq!(tr.message()["content"].as_array().unwrap().len(), 1);
}

#[test]
fn the_end_waits_for_claude_codes_verdict() {
    let mut tr = structured();
    let mut msgs = vec![
        start(),
        tool_start(0, "toolu_so", STRUCTURED_TOOL),
        json_delta(0, "{}"),
        stop(0),
    ];
    msgs.extend(end("tool_use", 3));
    assert_eq!(names(&run(&mut tr, &msgs)), ["message_start"]);
    assert!(!tr.ended());
    let events = run(&mut tr, &[tool_result("toolu_so", false)]);
    assert_eq!(
        names(&events),
        [
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ]
    );
    assert_eq!(events[3].data["delta"]["stop_reason"], "end_turn");
    assert!(tr.ended());
}

#[test]
fn without_a_structured_output_the_text_answers() {
    let mut tr = structured();
    let mut msgs = vec![start(), text_start(0), text_delta(0, "I can't."), stop(0)];
    msgs.extend(end("end_turn", 4));
    let events = run(&mut tr, &msgs);
    assert_eq!(texts(&events), "I can't.");
    assert_eq!(tr.stop_reason().as_deref(), Some("end_turn"));
    assert!(tr.ended());
}

#[test]
fn a_structured_answer_cut_short_goes_out_as_it_is() {
    let mut tr = structured();
    let mut msgs = vec![
        start(),
        tool_start(0, "toolu_so", STRUCTURED_TOOL),
        json_delta(0, "{\"lang\": \"f"),
        stop(0),
    ];
    msgs.extend(end("max_tokens", 9));
    let events = run(&mut tr, &msgs);
    assert_eq!(texts(&events), "{\"lang\": \"f");
    assert_eq!(tr.stop_reason().as_deref(), Some("max_tokens"));
    assert!(tr.ended());
}

#[test]
fn a_turn_that_ends_holding_a_structured_answer_closes_it() {
    let mut tr = structured();
    run(
        &mut tr,
        &[
            start(),
            tool_start(0, "toolu_so", STRUCTURED_TOOL),
            json_delta(0, "{\"a\": 1}"),
            stop(0),
        ],
    );
    let events = tr.finish();
    assert_eq!(texts(&events), "{\"a\": 1}");
    assert_eq!(names(&events).last(), Some(&"message_stop"));
    assert_eq!(tr.stop_reason().as_deref(), Some("end_turn"));
}

#[test]
fn without_output_config_format_nothing_changes() {
    let mut tr = Translator::new(Vec::new());
    let mut msgs = vec![
        start(),
        tool_start(0, "toolu_so", STRUCTURED_TOOL),
        json_delta(0, "{}"),
        stop(0),
        tool_result("toolu_so", false),
    ];
    msgs.extend(end("tool_use", 3));
    let events = run(&mut tr, &msgs);
    assert_eq!(events[1].data["content_block"]["type"], "tool_use");
    assert_eq!(tr.stop_reason().as_deref(), Some("tool_use"));
}
