use super::*;
use serde_json::json;

fn user(content: Value) -> Message {
    Message {
        role: Role::User,
        content: crate::bridge::claude_api::wire::blocks(&content, "m").unwrap(),
    }
}

fn assistant(content: Value) -> Message {
    Message {
        role: Role::Assistant,
        content: crate::bridge::claude_api::wire::blocks(&content, "m").unwrap(),
    }
}

#[test]
fn a_fingerprint_survives_how_clients_send_an_answer_back() {
    // What the server answered…
    let answered = assistant(json!([
        { "type": "thinking", "thinking": "hmm", "signature": "sig" },
        { "type": "text", "text": "Hi there" },
        { "type": "tool_use", "id": "toolu_1", "name": "lookup", "input": { "q": 1 } },
    ]));
    // …and what an SDK sends back: no thinking, extra nulls, cache marks.
    let echoed = assistant(json!([
        { "type": "text", "text": "Hi there", "citations": null },
        {
            "type": "tool_use", "id": "toolu_1", "name": "lookup", "input": { "q": 1 },
            "cache_control": { "type": "ephemeral" },
        },
    ]));
    assert_eq!(fingerprint(&answered), fingerprint(&echoed));
    assert_ne!(
        fingerprint(&answered),
        fingerprint(&assistant(json!("Hi there!")))
    );
    // The role counts.
    assert_ne!(
        fingerprint(&user(json!("Hi there"))),
        fingerprint(&assistant(json!("Hi there")))
    );
}

#[test]
fn a_string_and_its_text_block_are_the_same_message() {
    assert_eq!(
        fingerprint(&user(json!("Hello"))),
        fingerprint(&user(json!([{ "type": "text", "text": "Hello" }])))
    );
    let a = user(json!([{ "type": "tool_result", "tool_use_id": "t", "content": "42" }]));
    let b = user(json!([{
        "type": "tool_result", "tool_use_id": "t", "is_error": false,
        "content": [{ "type": "text", "text": "42" }],
    }]));
    assert_eq!(fingerprint(&a), fingerprint(&b));
}

#[test]
fn images_count_by_their_bytes() {
    let img = |data: &str| {
        user(
            json!([{ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": data } }]),
        )
    };
    assert_eq!(fingerprint(&img("AAAA")), fingerprint(&img("AAAA")));
    assert_ne!(fingerprint(&img("AAAA")), fingerprint(&img("BBBB")));
}

#[test]
fn tool_results_are_split_from_the_rest() {
    let blocks = vec![
        json!({ "type": "tool_result", "tool_use_id": "a", "content": "ok" }),
        json!({ "type": "tool_result", "tool_use_id": "b", "is_error": true,
                "content": [{ "type": "text", "text": "boom", "cache_control": { "type": "ephemeral" } }] }),
        json!({ "type": "text", "text": "and then?" }),
    ];
    let (results, rest) = split_tool_results(&blocks);
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].content,
        vec![json!({ "type": "text", "text": "ok" })]
    );
    assert!(results[1].is_error);
    assert_eq!(
        results[1].content,
        vec![json!({ "type": "text", "text": "boom" })]
    );
    assert_eq!(rest, vec![json!({ "type": "text", "text": "and then?" })]);
}

#[test]
fn claude_code_gets_blocks_without_cache_marks_and_no_file_ids() {
    let out = for_claude_code(
        &[json!({ "type": "text", "text": "x", "cache_control": { "type": "ephemeral" } })],
        "messages.0.content",
    )
    .unwrap();
    assert_eq!(out, vec![json!({ "type": "text", "text": "x" })]);
    let e = for_claude_code(
        &[json!({ "type": "document", "source": { "type": "file", "file_id": "file_1" } })],
        "messages.0.content",
    )
    .unwrap_err();
    assert!(
        e.message.starts_with("messages.0.content.0.source"),
        "{}",
        e.message
    );
}

#[test]
fn a_history_never_seen_is_written_into_the_first_message() {
    let history = vec![
        user(json!("What is 2+2?")),
        assistant(json!([
            { "type": "thinking", "thinking": "easy", "signature": "s" },
            { "type": "text", "text": "4." },
            { "type": "tool_use", "id": "t1", "name": "calc", "input": { "x": 2 } },
        ])),
        user(json!([
            { "type": "tool_result", "tool_use_id": "t1", "content": "4" },
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" } },
        ])),
    ];
    let out =
        seeded_first_message(&history, &[json!({ "type": "text", "text": "And 3+3?" })]).unwrap();
    // Text runs are joined; the image stays where it was.
    let kinds: Vec<&str> = out.iter().map(|b| b["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["text", "image", "text"]);
    let first = out[0]["text"].as_str().unwrap();
    assert!(first.starts_with("<conversation_so_far>"));
    assert!(first.contains("User:What is 2+2?"));
    assert!(first.contains("Assistant:4."));
    assert!(!first.contains("easy"), "thinking is left out");
    assert!(first.contains("[called the tool `calc` with {\"x\":2}]"));
    assert!(first.contains("[result of the tool call t1:]4"));
    let last = out[2]["text"].as_str().unwrap();
    assert!(
        last.ends_with("</conversation_so_far>\n\nThe user now says:\nAnd 3+3?"),
        "{last}"
    );
}

#[test]
fn records_leave_the_bytes_out_and_come_back_as_notes() {
    let m = user(json!([
        { "type": "text", "text": "look" },
        { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAABBBB" } },
    ]));
    let kept = for_record(&m);
    assert_eq!(kept["role"], "user");
    let data = &kept["content"][1]["source"]["data"];
    assert_eq!(data["omitted"], true);
    assert_eq!(data["bytes"], 6);
    assert_eq!(data["sha256"].as_str().unwrap().len(), 64);

    let back = from_record(&kept).unwrap();
    assert_eq!(back.role, Role::User);
    assert_eq!(back.content[0], json!({ "type": "text", "text": "look" }));
    assert_eq!(
        back.content[1],
        json!({ "type": "text", "text": "[image not kept]" })
    );
    assert!(from_record(&json!({ "role": "system", "content": [] })).is_none());
}

#[test]
fn late_tool_results_are_written_out() {
    let out = describe_tool_results(
        &[
            json!({ "type": "tool_result", "tool_use_id": "t9", "is_error": true, "content": "nope" }),
            json!({ "type": "text", "text": " Try again." }),
        ],
        "messages.2.content",
    )
    .unwrap();
    assert_eq!(
        out,
        vec![json!({ "type": "text", "text": "[error of the tool call t9:]\nnope Try again." })]
    );
}
