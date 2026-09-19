//! The OpenAPI 3.1 document, composed at request time.
//!
//! Generated rather than checked in, for the reason the Codex bridge gives:
//! the served document has to describe *this* listener. A bridge running
//! tokenless on loopback and the same bridge mounted on the LAN port have
//! different security requirements, and a static file cannot say so.

use serde_json::{json, Map, Value};

/// Build the document.
///
/// `require_token` and `messages_api` describe the live configuration, so the
/// schema a client reads is the one it must satisfy. `mount` is the path prefix
/// this document was served under — listing a guessed `/` would send every
/// "Try it out" under the `/v1` mount to the dashboard's SPA fallback.
pub fn document(require_token: bool, messages_api: bool, mount: &str) -> Value {
    let mut paths = Map::new();
    let mut add = |path: &str, method: &str, op: Op| {
        let entry = paths.entry(path.to_string()).or_insert_with(|| json!({}));
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(method.to_string(), op.0);
        }
    };

    // --- discovery ---------------------------------------------------------
    add(
        "/health",
        "get",
        operation(
            "Discovery",
            "health",
            "Bridge lifecycle state",
            "Reports whether the runtime host is built and running. Does not call \
             the model or refresh any login.",
            &[],
            None,
            json!({ "$ref": "#/components/schemas/Health" }),
        ),
    );
    add(
        "/capabilities",
        "get",
        operation(
            "Discovery",
            "capabilities",
            "Limits, features and auth policy",
            "The machine-readable contract: every cap, which backends are usable, \
             and whether this listener requires a token.",
            &[],
            None,
            json!({ "type": "object" }),
        ),
    );
    add(
        "/metadata",
        "get",
        operation(
            "Discovery",
            "metadata",
            "Versions of everything in the chain",
            "",
            &[],
            None,
            json!({ "type": "object" }),
        ),
    );
    add(
        "/models",
        "get",
        operation(
            "Discovery",
            "models",
            "Models this account can select",
            "Rich model info when a conversation is live, otherwise the user's own \
             saved Claude settings.",
            &[],
            None,
            json!({ "type": "object" }),
        ),
    );

    add(
        "/auth",
        "get",
        operation(
            "Discovery",
            "auth",
            "Which Anthropic account Claude is signed in as",
            "Checked once when the app starts and cached. `?refresh=true` re-runs \
             the check, which spawns the CLI — not something to poll on.",
            &[query(
                "refresh",
                "boolean",
                "Re-check instead of reading the cache.",
            )],
            None,
            json!({ "$ref": "#/components/schemas/Auth" }),
        ),
    );
    add(
        "/auth/login",
        "post",
        operation(
            "Discovery",
            "authLogin",
            "Start the Claude sign-in flow",
            "Opens a browser on the host machine, so it is restricted to local \
             clients. Returns as soon as the flow is launched; poll \
             `GET /auth?refresh=true` until `authenticated` is true.",
            &[],
            Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "console": { "type": "boolean",
                                 "description": "Use an Anthropic Console account (API billing) \
                                                 instead of a Claude subscription." },
                },
            })),
            json!({ "type": "object" }),
        ),
    );

    // --- conversations -----------------------------------------------------
    add(
        "/threads",
        "post",
        operation(
            "Conversations",
            "createThread",
            "Start a conversation",
            "Starts a Claude Code conversation and returns its id immediately. No \
             model work happens until the first turn.",
            &[],
            Some(json!({ "$ref": "#/components/schemas/CreateThread" })),
            json!({ "$ref": "#/components/schemas/ThreadCreated" }),
        )
        .merge_status(201),
    );
    add(
        "/threads",
        "get",
        operation(
            "Conversations",
            "listThreads",
            "List conversations",
            "Live conversations plus what is on disk. `?live_only=true` skips the \
             session store.",
            &[
                query(
                    "live_only",
                    "boolean",
                    "Only conversations this bridge is running.",
                ),
                query("limit", "integer", "Stored conversations to return."),
                query("offset", "integer", "Stored conversations to skip."),
            ],
            None,
            json!({ "type": "object" }),
        ),
    );
    add(
        "/threads/{thread_id}",
        "get",
        operation(
            "Conversations",
            "readThread",
            "Read one conversation",
            "",
            &[query(
                "include_messages",
                "boolean",
                "Inline the stored transcript.",
            )],
            None,
            json!({ "type": "object" }),
        ),
    );
    add(
        "/threads/{thread_id}",
        "delete",
        operation(
            "Conversations",
            "closeThread",
            "End a conversation",
            "Stops the runtime. The transcript stays on disk unless `?purge=true`, \
             which is restricted to local clients.",
            &[query(
                "purge",
                "boolean",
                "Also delete the stored transcript.",
            )],
            None,
            json!({ "type": "object" }),
        ),
    );
    add(
        "/threads/{thread_id}/messages",
        "get",
        operation(
            "Conversations",
            "threadMessages",
            "Stored transcript",
            "",
            &[
                query("limit", "integer", "Messages to return."),
                query("offset", "integer", "Messages to skip."),
                query(
                    "include_system_messages",
                    "boolean",
                    "Include system messages.",
                ),
            ],
            None,
            json!({ "type": "object" }),
        ),
    );
    add(
        "/threads/{thread_id}/context",
        "get",
        operation(
            "Conversations",
            "threadContext",
            "Context-window breakdown",
            "",
            &[],
            None,
            json!({ "type": "object" }),
        ),
    );
    add(
        "/threads/{thread_id}/name",
        "post",
        operation(
            "Conversations",
            "renameThread",
            "Set the stored title",
            "",
            &[],
            Some(json!({
                "type": "object",
                "required": ["name"],
                "additionalProperties": false,
                "properties": { "name": { "type": "string", "minLength": 1 } },
            })),
            json!({ "type": "object" }),
        ),
    );
    add(
        "/threads/{thread_id}/fork",
        "post",
        operation(
            "Conversations",
            "forkThread",
            "Branch a conversation",
            "Copies the transcript into a new conversation id, optionally truncated \
             at a message.",
            &[],
            Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "up_to_message_id": { "type": "string" },
                    "title": { "type": "string" },
                },
            })),
            json!({ "type": "object" }),
        )
        .merge_status(201),
    );
    add(
        "/threads/{thread_id}/model",
        "post",
        operation(
            "Conversations",
            "setThreadModel",
            "Switch model mid-conversation",
            "",
            &[],
            Some(json!({
                "type": "object",
                "required": ["model"],
                "additionalProperties": false,
                "properties": {
                    "model": { "type": ["string", "null"],
                               "description": "null restores the account default." },
                },
            })),
            json!({ "type": "object" }),
        ),
    );
    add(
        "/threads/{thread_id}/permission-mode",
        "post",
        operation(
            "Conversations",
            "setThreadPermissionMode",
            "Change approval behaviour live",
            "`bypassPermissions` is restricted to local clients.",
            &[],
            Some(json!({
                "type": "object",
                "required": ["permission_mode"],
                "additionalProperties": false,
                "properties": {
                    "permission_mode": { "$ref": "#/components/schemas/PermissionMode" },
                },
            })),
            json!({ "type": "object" }),
        ),
    );

    // --- turns -------------------------------------------------------------
    add(
        "/threads/{thread_id}/turns",
        "post",
        operation(
            "Turns",
            "startTurn",
            "Start a turn",
            "Returns 202 with an operation id; the turn runs on. Stream \
             `/operations/{id}/events` or poll `/operations/{id}`.",
            &[],
            Some(json!({ "$ref": "#/components/schemas/TurnRequest" })),
            json!({ "$ref": "#/components/schemas/Operation" }),
        )
        .merge_status(202),
    );
    add(
        "/threads/{thread_id}/run",
        "post",
        operation(
            "Turns",
            "runTurn",
            "Start a turn and wait",
            "Same as starting a turn, with the wait built in. 502 carries the turn's \
             error; 504 means it is still running.",
            &[],
            Some(json!({ "$ref": "#/components/schemas/TurnRequest" })),
            json!({ "$ref": "#/components/schemas/TurnResult" }),
        ),
    );
    add(
        "/threads/{thread_id}/turns/{turn_id}/steer",
        "post",
        operation(
            "Turns",
            "steerTurn",
            "Add input to a running turn",
            "Redirects work already in flight. Defaults to `now` priority.",
            &[],
            Some(json!({ "$ref": "#/components/schemas/TurnRequest" })),
            json!({ "$ref": "#/components/schemas/Operation" }),
        ),
    );
    add(
        "/threads/{thread_id}/turns/{turn_id}/interrupt",
        "post",
        operation(
            "Turns",
            "interruptTurn",
            "Stop a running turn",
            "The conversation survives; only the turn ends.",
            &[],
            Some(json!({ "type": "object", "additionalProperties": false })),
            json!({ "$ref": "#/components/schemas/Operation" }),
        ),
    );
    add(
        "/chat",
        "post",
        operation(
            "Turns",
            "chat",
            "One message in, one answer out",
            "Runs one turn and waits. With no `thread_id`, the message joins the \
             current conversation — the one nominated when the API was started, \
             or the one an earlier message created. `create_new_chat: true` \
             branches off a new one and makes it current.",
            &[],
            Some(json!({ "$ref": "#/components/schemas/ChatRequest" })),
            json!({ "$ref": "#/components/schemas/ChatResponse" }),
        ),
    );

    // --- operations and events --------------------------------------------
    add(
        "/operations/{operation_id}",
        "get",
        operation(
            "Operations",
            "readOperation",
            "Poll one operation",
            "",
            &[],
            None,
            json!({ "$ref": "#/components/schemas/Operation" }),
        ),
    );
    add(
        "/operations/{operation_id}/events",
        "get",
        sse_operation(
            "readOperationEvents",
            "Stream one turn",
            "Replayable Server-Sent Events. Replays from the beginning by default; \
             resume with `?after=` or `Last-Event-ID`. 410 means the events were \
             evicted — read the operation result instead.",
        ),
    );
    add(
        "/events",
        "get",
        sse_operation(
            "readEvents",
            "Stream everything",
            "Every event across every conversation. A fresh subscriber starts at the \
             oldest retained event.",
        ),
    );

    // --- human-in-the-loop -------------------------------------------------
    add(
        "/requests",
        "get",
        operation(
            "Requests",
            "listRequests",
            "Tool approvals and questions awaiting a human",
            "A blocked conversation appears here and simultaneously as a prompt card \
             in the Mother Claude dashboard. Either surface may answer it.",
            &[],
            None,
            json!({ "type": "object" }),
        ),
    );
    add(
        "/requests/{request_id}/respond",
        "post",
        operation(
            "Requests",
            "respondToRequest",
            "Unblock a conversation",
            "Send one of `result` (raw), `decision` (allow|deny), or `answer`. \
             Approving a dangerous action is restricted to local clients.",
            &[],
            Some(json!({ "$ref": "#/components/schemas/RespondRequest" })),
            json!({ "type": "object" }),
        ),
    );

    // --- direct Messages API ----------------------------------------------
    let messages_note = if messages_api {
        "Enabled on this server."
    } else {
        "Disabled: set ANTHROPIC_API_KEY and restart. Returns 503 until then."
    };
    add(
        "/messages",
        "post",
        operation(
            "Messages API",
            "createMessage",
            "One Messages API call",
            &format!(
                "A faithful pass-through to POST /v1/messages, plus `localImage` / \
                 `localDocument` input sugar, `betas` as a JSON field, and model \
                 defaults. Tools, programmatic tool calling, server tools, \
                 `output_config` and `transformations` are forwarded untouched. {messages_note}"
            ),
            &[],
            Some(json!({ "$ref": "#/components/schemas/MessagesRequest" })),
            json!({ "type": "object" }),
        ),
    );
    add(
        "/files",
        "post",
        multipart_operation(
            "uploadFile",
            "Upload a file",
            &format!("Multipart upload to the Files API. {messages_note}"),
        ),
    );
    add(
        "/files",
        "get",
        operation(
            "Messages API",
            "listFiles",
            "List uploaded files",
            messages_note,
            &[
                query("limit", "integer", "Files to return (max 1000)."),
                query("page", "string", "Pagination cursor."),
            ],
            None,
            json!({ "type": "object" }),
        ),
    );
    add(
        "/files/{file_id}/content",
        "get",
        operation(
            "Messages API",
            "downloadFile",
            "Download a generated file",
            "Only files the API generated are downloadable; uploads are not.",
            &[],
            None,
            json!({ "type": "string", "format": "binary" }),
        ),
    );
    add(
        "/files/{file_id}",
        "delete",
        operation(
            "Messages API",
            "deleteFile",
            "Delete an uploaded file",
            messages_note,
            &[],
            None,
            json!({ "type": "object" }),
        ),
    );

    // Path parameters are derived from the templates rather than repeated.
    for (path, item) in paths.iter_mut() {
        let params = path_params(path);
        if params.is_empty() {
            continue;
        }
        if let Some(methods) = item.as_object_mut() {
            for op in methods.values_mut() {
                if let Some(existing) = op.get_mut("parameters").and_then(Value::as_array_mut) {
                    for p in params.iter().rev() {
                        existing.insert(0, p.clone());
                    }
                }
            }
        }
    }

    let security = if require_token {
        json!([{ "BridgeBearer": [] }])
    } else {
        json!([])
    };

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Mother Claude — Claude HTTP bridge",
            "version": env!("CARGO_PKG_VERSION"),
            "description": description(require_token, messages_api),
        },
        "servers": [
            { "url": if mount.is_empty() { "/" } else { mount }, "description": "This mount" },
        ],
        "security": security,
        "x-bridge-require-token": require_token,
        "x-bridge-messages-api": messages_api,
        "tags": [
            { "name": "Discovery", "description": "Health, capabilities, models." },
            { "name": "Conversations", "description": "Claude Code sessions." },
            { "name": "Turns", "description": "Running, steering and interrupting work." },
            { "name": "Operations", "description": "Background work and its event streams." },
            { "name": "Requests", "description": "Tool approvals and questions." },
            { "name": "Messages API", "description": "Direct api.anthropic.com access." },
        ],
        "paths": Value::Object(paths),
        "components": {
            "securitySchemes": {
                "BridgeBearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "The Mother Claude API token. Also accepted as ?token= \
                                    (for EventSource) or an mc_token cookie.",
                },
            },
            "schemas": schemas(),
        },
    })
}

fn description(require_token: bool, messages_api: bool) -> String {
    let auth = if require_token {
        "This listener requires the Mother Claude API token on every API route. \
         Documentation, the OpenAPI document, the JS client and the example page are public."
    } else {
        "Bearer checks are disabled on this listener, which is loopback-only. \
         Any local process can drive Claude through it."
    };
    let api = if messages_api {
        "The direct Messages API backend is enabled."
    } else {
        "The direct Messages API backend is disabled (no ANTHROPIC_API_KEY); \
         /messages and /files answer 503."
    };
    format!(
        "Drive Claude on this machine over HTTP: conversations, turns, live event \
         streams, steering, interruption, tool-approval callbacks, image and \
         document input, and a direct Messages API path.\n\n{auth}\n\n{api}\n\n\
         The API must be started from the Mother Claude app before any of these \
         routes will run: until then they answer 503 `bridge_not_started`, while \
         discovery and this document stay available.\n\n\
         Conversations and operations are process-local and do not survive a restart. \
         Requests are never retried; a disconnected client does not interrupt a \
         running turn — use the interrupt endpoint."
    )
}

/// Extract `{name}` segments from a path template.
fn path_params(path: &str) -> Vec<Value> {
    path.split('/')
        .filter_map(|segment| {
            let name = segment.strip_prefix('{')?.strip_suffix('}')?;
            Some(json!({
                "name": name,
                "in": "path",
                "required": true,
                "schema": { "type": "string", "minLength": 1, "maxLength": 200 },
            }))
        })
        .collect()
}

fn query(name: &str, ty: &str, description: &str) -> Value {
    json!({
        "name": name,
        "in": "query",
        "required": false,
        "description": description,
        "schema": { "type": ty },
    })
}

/// A small builder so the success status can be overridden after the fact.
struct Op(Value);

impl Op {
    /// Re-file the success response under the status this endpoint really
    /// returns (201 for a creation, 202 for a started turn).
    fn merge_status(mut self, status: u16) -> Self {
        if let Some(responses) = self.0.get_mut("responses").and_then(Value::as_object_mut) {
            if let Some(ok) = responses.remove("200") {
                responses.insert(status.to_string(), ok);
            }
        }
        self
    }
}

fn operation(
    tag: &str,
    id: &str,
    summary: &str,
    description: &str,
    query_params: &[Value],
    body: Option<Value>,
    result: Value,
) -> Op {
    let mut op = json!({
        "tags": [tag],
        "operationId": id,
        "summary": summary,
        "parameters": query_params,
        "responses": {
            "200": {
                "description": "Success",
                "content": { "application/json": { "schema": result } },
            },
            "default": {
                "description": "Error",
                "content": {
                    "application/json": {
                        "schema": { "$ref": "#/components/schemas/Error" },
                    },
                },
            },
        },
    });
    if !description.is_empty() {
        op["description"] = json!(description);
    }
    if let Some(schema) = body {
        op["requestBody"] = json!({
            "required": true,
            "content": { "application/json": { "schema": schema } },
        });
    }
    Op(op)
}

fn sse_operation(id: &str, summary: &str, description: &str) -> Op {
    Op(json!({
        "tags": ["Operations"],
        "operationId": id,
        "summary": summary,
        "description": description,
        "parameters": [
            query("after", "integer", "Resume after this event id."),
        ],
        "responses": {
            "200": {
                "description": "An SSE stream. Each frame carries `id:` (the event \
                                sequence) and a JSON `data:` payload.",
                "content": { "text/event-stream": { "schema": { "type": "string" } } },
            },
            "410": {
                "description": "The requested events are no longer retained.",
                "content": {
                    "application/json": {
                        "schema": { "$ref": "#/components/schemas/Error" },
                    },
                },
            },
            "default": {
                "description": "Error",
                "content": {
                    "application/json": {
                        "schema": { "$ref": "#/components/schemas/Error" },
                    },
                },
            },
        },
    }))
}

fn multipart_operation(id: &str, summary: &str, description: &str) -> Op {
    Op(json!({
        "tags": ["Messages API"],
        "operationId": id,
        "summary": summary,
        "description": description,
        "parameters": [],
        "requestBody": {
            "required": true,
            "content": {
                "multipart/form-data": {
                    "schema": {
                        "type": "object",
                        "required": ["file"],
                        "properties": {
                            "file": { "type": "string", "format": "binary" },
                            "filename": { "type": "string" },
                            "expires_in_seconds": { "type": "integer" },
                        },
                    },
                },
            },
        },
        "responses": {
            "200": {
                "description": "The uploaded file object.",
                "content": { "application/json": { "schema": { "type": "object" } } },
            },
            "default": {
                "description": "Error",
                "content": {
                    "application/json": {
                        "schema": { "$ref": "#/components/schemas/Error" },
                    },
                },
            },
        },
    }))
}

fn schemas() -> Value {
    json!({
        "Error": {
            "type": "object",
            "required": ["error"],
            "properties": {
                "error": {
                    "type": "object",
                    "required": ["code", "message"],
                    "description": "Context fields (thread_id, turn_id, request_id, field, \
                                    operation_id, upstream) are flattened alongside code and \
                                    message when they apply.",
                    "properties": {
                        "code": { "type": "string" },
                        "message": { "type": "string" },
                    },
                },
            },
        },
        "Auth": {
            "type": "object",
            "description": "Claude's own Anthropic sign-in — not the Mother Claude API token.",
            "properties": {
                "authenticated": { "type": "boolean" },
                "summary": { "type": "string" },
                "method": { "type": ["string", "null"] },
                "api_provider": { "type": ["string", "null"] },
                "email": { "type": ["string", "null"] },
                "organization": { "type": ["string", "null"] },
                "subscription": { "type": ["string", "null"] },
                "config_directory": { "type": ["string", "null"] },
                "error": { "type": ["string", "null"],
                           "description": "Set when the check itself failed, which is \
                                           different from being signed out." },
            },
        },
        "Health": {
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["ready", "unavailable"] },
                "backend": { "type": "string" },
                "version": { "type": "string" },
                "host_running": { "type": "boolean" },
                "host_built": { "type": "boolean" },
                "messages_api": { "type": "boolean" },
                "active_operations": { "type": "integer" },
                "claude_authenticated": { "type": "boolean" },
            },
        },
        "PermissionMode": {
            "type": "string",
            "enum": ["default", "acceptEdits", "bypassPermissions", "plan", "dontAsk", "auto"],
        },
        "Effort": {
            "type": "string",
            "enum": ["low", "medium", "high", "xhigh", "max"],
        },
        "InputItem": {
            "description": "A string, one item, or an array of items. Anthropic content \
                            blocks pass through untouched; `localImage` and `localDocument` \
                            are bridge conveniences that read a file on this machine.",
            "oneOf": [
                { "type": "object", "required": ["type", "text"], "properties": {
                    "type": { "const": "text" }, "text": { "type": "string" } } },
                { "type": "object", "required": ["type"], "properties": {
                    "type": { "const": "image" },
                    "url": { "type": "string", "description": "data: or http(s) URL." },
                    "source": { "type": "object" } } },
                { "type": "object", "required": ["type", "path"], "properties": {
                    "type": { "const": "localImage" },
                    "path": { "type": "string", "description": "Absolute path on this machine." } } },
                { "type": "object", "required": ["type"], "properties": {
                    "type": { "const": "document" }, "source": { "type": "object" } } },
                { "type": "object", "required": ["type", "path"], "properties": {
                    "type": { "const": "localDocument" }, "path": { "type": "string" } } },
            ],
        },
        "Input": {
            "oneOf": [
                { "type": "string" },
                { "$ref": "#/components/schemas/InputItem" },
                { "type": "array", "items": { "$ref": "#/components/schemas/InputItem" } },
            ],
        },
        "CreateThread": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "thread_id": { "type": "string", "format": "uuid",
                               "description": "Pre-assign the conversation id." },
                "cwd": { "type": "string", "description": "Working directory on this machine." },
                "model": { "type": "string" },
                "effort": { "$ref": "#/components/schemas/Effort" },
                "thinking": { "type": "string", "enum": ["on", "off"] },
                "permission_mode": { "$ref": "#/components/schemas/PermissionMode" },
                "resume": { "type": "string", "description": "Continue an existing conversation." },
                "fork": { "type": "boolean", "description": "With `resume`, branch to a new id." },
                "title": { "type": "string" },
                "system_prompt_append": { "type": "string" },
                "allowed_tools": { "type": "array", "items": { "type": "string" } },
                "disallowed_tools": { "type": "array", "items": { "type": "string" } },
                "additional_directories": { "type": "array", "items": { "type": "string" } },
                "setting_sources": { "type": "array",
                    "items": { "type": "string", "enum": ["user", "project", "local"] } },
                "mcp_servers": { "type": "object" },
                "output_schema": { "type": "object",
                    "description": "JSON Schema; the result arrives as structured_output." },
                "max_turns": { "type": "integer" },
                "max_budget_usd": { "type": "number" },
                "skills": {},
                "agents": { "type": "object" },
                "include_partial_messages": { "type": "boolean", "default": true },
            },
        },
        "ThreadCreated": {
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "status": { "const": "created" },
                "cwd": { "type": "string" },
                "model": { "type": ["string", "null"] },
                "effort": { "type": ["string", "null"] },
                "permission_mode": { "type": "string" },
            },
        },
        "TurnRequest": {
            "type": "object",
            "required": ["input"],
            "additionalProperties": false,
            "properties": {
                "input": { "$ref": "#/components/schemas/Input" },
                "priority": { "type": "string", "enum": ["now", "next", "later"] },
            },
        },
        "Operation": {
            "type": "object",
            "properties": {
                "operation_id": { "type": "string" },
                "thread_id": { "type": ["string", "null"] },
                "turn_id": { "type": ["string", "null"] },
                "status": { "type": "string",
                            "enum": ["running", "completed", "failed", "interrupted"] },
                "result": {},
                "error": {},
            },
        },
        "TurnResult": {
            "type": "object",
            "properties": {
                "operation_id": { "type": "string" },
                "thread_id": { "type": "string" },
                "turn_id": { "type": "string" },
                "status": { "type": "string" },
                "result": {
                    "type": "object",
                    "properties": {
                        "result": { "type": ["string", "null"] },
                        "structured_output": {},
                        "usage": { "type": ["object", "null"] },
                        "total_cost_usd": { "type": ["number", "null"] },
                        "stop_reason": { "type": ["string", "null"] },
                        "num_turns": { "type": ["integer", "null"] },
                        "permission_denials": { "type": "array", "items": { "type": "object" } },
                    },
                },
            },
        },
        "ChatRequest": {
            "type": "object",
            "description": "Every CreateThread property is also accepted here and applies \
                            when a new conversation is created.",
            "properties": {
                "message": { "type": "string" },
                "input": { "$ref": "#/components/schemas/Input" },
                "thread_id": {
                    "type": "string",
                    "description": "Send to this conversation specifically. One that is not \
                                    currently running is resumed from disk, so any past \
                                    conversation can be picked up. Does not change which \
                                    conversation later unaddressed messages join.",
                },
                "create_new_chat": {
                    "type": "boolean",
                    "default": false,
                    "description": "Start a fresh conversation instead of joining the current \
                                    one, and make it current. Also accepted as `createNewChat`. \
                                    False by default so a client sending only `{\"message\": …}` \
                                    has one continuous conversation rather than a new one per \
                                    message.",
                },
            },
        },
        "ChatResponse": {
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "turn_id": { "type": "string" },
                "operation_id": { "type": "string" },
                "status": { "type": "string" },
                "response": { "type": ["string", "null"] },
                "structured_output": {},
                "usage": { "type": ["object", "null"] },
                "total_cost_usd": { "type": ["number", "null"] },
            },
        },
        "RespondRequest": {
            "type": "object",
            "additionalProperties": false,
            "description": "Use exactly one of `result`, `decision` or `answer`.",
            "properties": {
                "result": { "type": "object", "description": "Raw callback result." },
                "decision": { "type": "string", "enum": ["allow", "deny"] },
                "answer": { "type": "string" },
                "message": { "type": "string", "description": "Reason shown when denying." },
                "updated_permissions": { "type": "array", "items": { "type": "object" },
                    "description": "Persist a rule alongside an approval (\"always allow\")." },
                "updated_input": { "type": "object",
                    "description": "Approve a modified tool call." },
            },
        },
        "MessagesRequest": {
            "type": "object",
            "description": "The Messages API request body. `model` and `max_tokens` default \
                            when omitted; `betas` is sent as the anthropic-beta header; \
                            `localImage` / `localDocument` blocks inside messages[].content \
                            are expanded. Everything else is forwarded untouched.",
            "properties": {
                "model": { "type": "string" },
                "max_tokens": { "type": "integer" },
                "messages": { "type": "array", "items": { "type": "object" } },
                "system": {},
                "tools": { "type": "array", "items": { "type": "object" } },
                "tool_choice": { "type": "object" },
                "thinking": { "type": "object" },
                "output_config": { "type": "object" },
                "container": { "type": "string" },
                "stream": { "type": "boolean" },
                "betas": { "oneOf": [
                    { "type": "string" },
                    { "type": "array", "items": { "type": "string" } },
                ] },
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_is_openapi_31_with_every_family_present() {
        let doc = document(true, true, "");
        assert_eq!(doc["openapi"], "3.1.0");
        let paths = doc["paths"].as_object().unwrap();
        for expected in [
            "/health",
            "/capabilities",
            "/models",
            "/threads",
            "/threads/{thread_id}",
            "/threads/{thread_id}/turns",
            "/threads/{thread_id}/run",
            "/threads/{thread_id}/turns/{turn_id}/steer",
            "/threads/{thread_id}/turns/{turn_id}/interrupt",
            "/chat",
            "/operations/{operation_id}",
            "/operations/{operation_id}/events",
            "/events",
            "/requests",
            "/requests/{request_id}/respond",
            "/messages",
            "/files",
        ] {
            assert!(paths.contains_key(expected), "missing {expected}");
        }
    }

    #[test]
    fn security_reflects_the_running_listener() {
        assert_eq!(
            document(true, false, "")["security"],
            json!([{ "BridgeBearer": [] }])
        );
        assert_eq!(document(false, false, "")["security"], json!([]));
        assert_eq!(document(false, false, "")["x-bridge-require-token"], false);
        assert_eq!(document(true, true, "")["x-bridge-messages-api"], true);

        // The server list names the mount the document came from.
        assert_eq!(document(true, true, "")["servers"][0]["url"], "/");
        assert_eq!(document(true, true, "/v1")["servers"][0]["url"], "/v1");
    }

    #[test]
    fn path_parameters_are_derived_from_the_template() {
        let params = path_params("/threads/{thread_id}/turns/{turn_id}/steer");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0]["name"], "thread_id");
        assert_eq!(params[1]["name"], "turn_id");
        assert_eq!(params[0]["in"], "path");
        assert!(path_params("/health").is_empty());

        let doc = document(true, true, "");
        let op = &doc["paths"]["/threads/{thread_id}/turns/{turn_id}/steer"]["post"];
        let listed = op["parameters"].as_array().unwrap();
        assert_eq!(listed[0]["name"], "thread_id");
        assert_eq!(listed[1]["name"], "turn_id");
    }

    #[test]
    fn creation_endpoints_document_their_real_status_codes() {
        let doc = document(true, true, "");
        assert!(doc["paths"]["/threads"]["post"]["responses"]["201"].is_object());
        assert!(doc["paths"]["/threads"]["post"]["responses"]["200"].is_null());
        assert!(doc["paths"]["/threads/{thread_id}/turns"]["post"]["responses"]["202"].is_object());
        assert!(doc["paths"]["/threads/{thread_id}/run"]["post"]["responses"]["200"].is_object());
    }

    #[test]
    fn every_operation_documents_the_error_envelope() {
        let doc = document(true, true, "");
        for (path, item) in doc["paths"].as_object().unwrap() {
            for (method, op) in item.as_object().unwrap() {
                assert!(
                    op["responses"]["default"]["content"]["application/json"]["schema"]["$ref"]
                        == "#/components/schemas/Error",
                    "{method} {path} has no error schema"
                );
                assert!(op["operationId"].is_string(), "{method} {path} has no id");
            }
        }
    }

    #[test]
    fn the_description_tells_the_truth_about_this_listener() {
        assert!(document(false, false, "")["info"]["description"]
            .as_str()
            .unwrap()
            .contains("Bearer checks are disabled"));
        assert!(document(true, true, "")["info"]["description"]
            .as_str()
            .unwrap()
            .contains("Messages API backend is enabled"));
    }
}
