use super::*;

fn parse(body: Value) -> Result<MessagesRequest, ApiError> {
    MessagesRequest::parse(&body, false)
}

fn minimal() -> Value {
    json!({
        "model": "claude-sonnet-5-5",
        "max_tokens": 1024,
        "messages": [{ "role": "user", "content": "Hello, Claude" }],
    })
}

#[test]
fn a_minimal_request_is_read() {
    let r = parse(minimal()).unwrap();
    assert_eq!(r.model, "claude-sonnet-5-5");
    assert_eq!(r.max_tokens, 1024);
    assert_eq!(r.messages.len(), 1);
    assert_eq!(r.messages[0].role, Role::User);
    assert_eq!(
        r.messages[0].content,
        vec![json!({ "type": "text", "text": "Hello, Claude" })]
    );
    assert!(!r.stream);
    assert!(r.system.is_empty());
    assert!(r.ignored.is_empty());
    assert_eq!(r.first_text().as_deref(), Some("Hello, Claude"));
}

#[test]
fn required_fields_are_named_as_the_api_names_them() {
    let mut b = minimal();
    b.as_object_mut().unwrap().remove("model");
    let e = parse(b).unwrap_err();
    assert_eq!(e.status, StatusCode::BAD_REQUEST);
    assert_eq!(e.kind, "invalid_request_error");
    assert_eq!(e.message, "model: Field required");

    let mut b = minimal();
    b.as_object_mut().unwrap().remove("max_tokens");
    assert_eq!(
        parse(b.clone()).unwrap_err().message,
        "max_tokens: Field required"
    );
    // count_tokens takes none.
    assert_eq!(MessagesRequest::parse(&b, true).unwrap().max_tokens, 0);

    let mut b = minimal();
    b.as_object_mut().unwrap().remove("messages");
    assert_eq!(parse(b).unwrap_err().message, "messages: Field required");

    assert!(parse(json!([1, 2])).is_err());
}

#[test]
fn system_prompts_join_and_system_messages_join_them() {
    let mut b = minimal();
    b["system"] = json!([{ "type": "text", "text": "One." }, { "type": "text", "text": "Two." }]);
    b["messages"] = json!([
        { "role": "system", "content": "Three." },
        { "role": "user", "content": "Hi" },
    ]);
    let r = parse(b).unwrap();
    assert_eq!(r.system, "One.\n\nTwo.\n\nThree.");
    assert_eq!(r.messages.len(), 1);

    let mut b = minimal();
    b["system"] = json!([{ "type": "image" }]);
    assert!(parse(b).is_err());
}

#[test]
fn consecutive_messages_of_a_role_are_one_turn() {
    let mut b = minimal();
    b["messages"] = json!([
        { "role": "user", "content": "a" },
        { "role": "user", "content": [{ "type": "text", "text": "b" }] },
        { "role": "assistant", "content": "c" },
        { "role": "user", "content": "d" },
    ]);
    let r = parse(b).unwrap();
    assert_eq!(r.messages.len(), 3);
    assert_eq!(r.messages[0].content.len(), 2);

    let mut b = minimal();
    b["messages"] = json!([{ "role": "assistant", "content": "x" }]);
    assert!(parse(b).unwrap_err().message.contains("first message"));

    let mut b = minimal();
    b["messages"] = json!([{ "role": "tool", "content": "x" }]);
    assert!(parse(b).is_err());

    let mut b = minimal();
    b["messages"] = json!([{ "role": "user", "content": [{ "text": "no type" }] }]);
    assert_eq!(
        parse(b).unwrap_err().message,
        "messages.0.content.0.type: Field required"
    );
}

#[test]
fn stop_sequences_and_sampling_are_checked() {
    let mut b = minimal();
    b["stop_sequences"] = json!(["END", "\n\nHuman:"]);
    assert_eq!(parse(b).unwrap().stop_sequences.len(), 2);

    let mut b = minimal();
    b["stop_sequences"] = json!([""]);
    assert!(parse(b).is_err());

    let mut b = minimal();
    b["stop_sequences"] = json!((0..33).map(|i| format!("s{i}")).collect::<Vec<_>>());
    assert!(parse(b).is_err());

    let mut b = minimal();
    b["temperature"] = json!(1.5);
    assert!(parse(b).is_err());

    let mut b = minimal();
    b["temperature"] = json!(0.2);
    b["top_k"] = json!(5);
    b["service_tier"] = json!("auto");
    let r = parse(b).unwrap();
    assert_eq!(r.ignored, vec!["temperature", "top_k", "service_tier"]);
    let (name, value) = ignored_header(&r.ignored).unwrap();
    assert_eq!(name.as_str(), "x-mother-claude-ignored");
    assert_eq!(value.to_str().unwrap(), "temperature, top_k, service_tier");
    assert!(ignored_header(&[]).is_none());
}

#[test]
fn metadata_names_the_user_and_the_conversation() {
    let mut b = minimal();
    b["metadata"] = json!({ "user_id": "u-1", "conversation_id": "c-1" });
    let r = parse(b).unwrap();
    assert_eq!(r.user_id.as_deref(), Some("u-1"));
    assert_eq!(r.conversation.as_deref(), Some("c-1"));

    let mut b = minimal();
    b["conversation_id"] = json!("c-2");
    assert_eq!(parse(b).unwrap().conversation.as_deref(), Some("c-2"));

    let mut b = minimal();
    b["metadata"] = json!({ "user_id": "x".repeat(513) });
    assert!(parse(b).is_err());
}

#[test]
fn thinking_is_checked_as_the_api_checks_it() {
    let mut b = minimal();
    b["thinking"] = json!({ "type": "enabled", "budget_tokens": 512 });
    assert!(parse(b).unwrap_err().message.contains("1024"));

    let mut b = minimal();
    b["thinking"] = json!({ "type": "enabled", "budget_tokens": 2048 });
    assert!(parse(b)
        .unwrap_err()
        .message
        .contains("max_tokens must be greater"));

    let mut b = minimal();
    b["max_tokens"] = json!(4096);
    b["thinking"] = json!({ "type": "enabled", "budget_tokens": 2048, "display": "summarized" });
    assert_eq!(
        parse(b).unwrap().thinking,
        Some(json!({ "type": "enabled", "budget_tokens": 2048, "display": "summarized" }))
    );

    let mut b = minimal();
    b["thinking"] = json!({ "type": "adaptive", "display": "summarized" });
    assert_eq!(
        parse(b).unwrap().thinking,
        Some(json!({ "type": "adaptive", "display": "summarized" }))
    );

    let mut b = minimal();
    b["thinking"] = json!({ "type": "between_tools" });
    assert_eq!(
        parse(b).unwrap().thinking,
        Some(json!({ "type": "disabled" }))
    );

    let mut b = minimal();
    b["thinking"] = json!({ "type": "sometimes" });
    assert!(parse(b).is_err());
}

#[test]
fn custom_tools_are_taken_and_server_tools_refused() {
    let mut b = minimal();
    b["tools"] = json!([{
        "name": "get_weather",
        "description": "Weather in a city",
        "input_schema": { "type": "object", "properties": { "city": { "type": "string" } } },
    }]);
    b["tool_choice"] = json!({ "type": "tool", "name": "get_weather" });
    let r = parse(b).unwrap();
    assert_eq!(r.tools.len(), 1);
    assert_eq!(r.tool_choice.as_deref(), Some("tool"));
    assert_eq!(r.tool_name.as_deref(), Some("get_weather"));
    // Claude Code cannot force the call: named in the header.
    assert!(r.ignored.contains(&"tool_choice"));

    let mut b = minimal();
    b["tools"] = json!([{ "type": "web_search_20250305", "name": "web_search" }]);
    let e = parse(b).unwrap_err();
    assert!(e.message.contains("Anthropic's servers"), "{}", e.message);

    let mut b = minimal();
    b["tools"] = json!([{ "name": "bad name", "input_schema": { "type": "object" } }]);
    assert!(parse(b).unwrap_err().message.contains("pattern"));

    let mut b = minimal();
    b["tools"] = json!([
        { "name": "t", "input_schema": { "type": "object" } },
        { "name": "t", "input_schema": { "type": "object" } },
    ]);
    assert!(parse(b).unwrap_err().message.contains("unique"));

    let mut b = minimal();
    b["tools"] = json!([{ "name": "t" }]);
    assert!(parse(b).unwrap_err().message.contains("input_schema"));

    let mut b = minimal();
    b["tools"] = json!([{ "name": "t", "input_schema": { "type": "object" } }]);
    b["tool_choice"] = json!({ "type": "tool", "name": "other" });
    assert!(parse(b).is_err());

    let mut b = minimal();
    b["tools"] = json!([{ "name": "t", "input_schema": { "type": "object" } }]);
    b["tool_choice"] = json!({ "type": "auto", "disable_parallel_tool_use": true });
    let r = parse(b).unwrap();
    assert!(r.ignored.contains(&"tool_choice.disable_parallel_tool_use"));
    assert!(!r.ignored.contains(&"tool_choice"));
}

#[test]
fn output_config_and_refused_fields() {
    let mut b = minimal();
    b["output_config"] = json!({
        "effort": "high",
        "format": { "type": "json_schema", "schema": { "type": "object" } },
    });
    let r = parse(b).unwrap();
    assert_eq!(r.effort.as_deref(), Some("high"));
    assert!(r.output_format.is_some());

    let mut b = minimal();
    b["output_config"] = json!({ "effort": "huge" });
    assert!(parse(b).is_err());

    let mut b = minimal();
    b["output_config"] = json!({ "format": { "type": "text" } });
    assert!(parse(b).is_err());

    for key in ["container", "mcp_servers"] {
        let mut b = minimal();
        b[key] = json!("x");
        let e = parse(b).unwrap_err();
        assert!(e.message.starts_with(key), "{}", e.message);
    }
}

#[test]
fn errors_come_in_the_apis_envelope() {
    let e = ApiError::not_found("model: nope");
    assert_eq!(
        e.body("req_1"),
        json!({
            "type": "error",
            "error": { "type": "not_found_error", "message": "model: nope" },
            "request_id": "req_1",
        })
    );
    let resp = e.respond("req_1");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(resp.headers()["request-id"], "req_1");
    assert_eq!(ApiError::overloaded("x").status.as_u16(), 529);
    assert!(request_id().starts_with("req_mc"));
    assert_eq!(request_id().len(), 28);

    let from: ApiError = BridgeError::unprocessable("bad", "Bad input.").into();
    assert_eq!(from.status, StatusCode::BAD_REQUEST);
    assert_eq!(from.kind, "invalid_request_error");
    assert_eq!(from.message, "Bad input.");
}

#[test]
fn a_title_is_the_first_text_cut_short() {
    let mut b = minimal();
    b["messages"] = json!([{ "role": "user", "content": [
        { "type": "image", "source": { "type": "url", "url": "https://x/y.png" } },
        { "type": "text", "text": "é".repeat(100) },
    ]}]);
    let r = parse(b).unwrap();
    assert_eq!(r.first_text().unwrap().chars().count(), 80);
}

#[test]
fn left_out_thinking_is_what_the_api_does_for_the_model() {
    for model in [
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
        "claude-opus-5",
        "claude-sonnet-5",
        "claude-fable-5-2",
        "Claude-Mythos-5-2",
        "opus",
        "haiku",
        "sonnet[1m]",
    ] {
        assert!(thinks_by_default(model), "{model}");
    }
    for model in [
        "claude-opus-4-8",
        "claude-opus-4-1-20250805",
        "claude-sonnet-4-6",
        "claude-sonnet-4-5-20250929[1m]",
        "claude-haiku-4-5",
        "claude-3-7-sonnet-20250219",
    ] {
        assert!(!thinks_by_default(model), "{model}");
    }

    let mut b = minimal();
    b["model"] = json!("claude-haiku-5-5");
    assert_eq!(
        parse(b).unwrap().effective_thinking(),
        json!({ "type": "adaptive" })
    );
    let mut b = minimal();
    b["model"] = json!("claude-opus-4-8");
    assert_eq!(
        parse(b).unwrap().effective_thinking(),
        json!({ "type": "disabled" })
    );
    let mut b = minimal();
    b["model"] = json!("claude-haiku-5-5");
    b["thinking"] = json!({ "type": "disabled" });
    assert_eq!(
        parse(b).unwrap().effective_thinking(),
        json!({ "type": "disabled" })
    );
}
