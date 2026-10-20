#!/usr/bin/env python3
"""A fake `claude` for Mother Claude's owned-session tests (selected with
MOTHER_CLAUDE_CLI). It speaks the stream-json control protocol the way CLI
2.1.282 does and records everything it receives.

* argv and a few environment variables are appended to `<cwd>/fake-claude.log`
  as the first JSON line, then every line read from stdin as `{"in": …}`.
* `stop|rm|respawn <short>` print a line and exit 0; `logs <short>` prints a
  raw TUI screen (escape sequences, no line feeds).
* Otherwise it answers control requests and runs one scripted turn per user
  message, chosen by the message text (see `run_turn`).
"""
import json
import os
import queue
import sys
import threading
import time
import uuid as uuidlib

ARGS = sys.argv[1:]
LOG = os.path.join(os.getcwd(), "fake-claude.log")
ENV_KEYS = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CONFIG_DIR",
    "MOTHER_CLAUDE_TOKEN",
    "MOTHER_CLAUDE_OWNED",
    "MAX_THINKING_TOKENS",
]


def log(obj):
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(json.dumps(obj) + "\n")


def out(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def arg(name):
    if name in ARGS:
        i = ARGS.index(name)
        if i + 1 < len(ARGS):
            return ARGS[i + 1]
    return None


# Job subcommands run in the server's own cwd: answer them without logging.
if ARGS and ARGS[0] in ("stop", "rm", "respawn"):
    print(f"{ARGS[0]} {ARGS[1] if len(ARGS) > 1 else ''}".strip())
    sys.exit(0)
if ARGS and ARGS[0] == "logs":
    sys.stdout.write("\x1b7\x1b8\x1b[2J\x1b[H\x1b[1mfake\x1b[0m screen\x1b[3;1H❯ ok")
    sys.stdout.flush()
    sys.exit(0)

log({"argv": ARGS, "env": {k: os.environ.get(k) for k in ENV_KEYS}})

SESSION = arg("--session-id") or arg("--resume") or str(uuidlib.uuid4())
MODE = arg("--permission-mode") or "default"
MODEL = arg("--model") or "claude-fake-1"
STATE = {"mode": MODE, "started": False, "interrupted": False, "busy": None,
         # What `get_settings` reports (CLI 2.1.284 shapes).
         "model": MODEL, "effort": arg("--effort"), "ultracode": False,
         "thinking": os.environ.get("MAX_THINKING_TOKENS")}


def model_caps(model):
    """(resolved id, has effort, can run ultracode, has max) like the CLI's
    catalog."""
    m = (model or "").lower()
    if "haiku" in m:
        return "claude-haiku-4-5-20251001", False, False, False
    if "opus" in m:
        return "claude-opus-5-5", True, True, True
    if "sonnet" in m:
        return "claude-sonnet-5", True, False, False
    return model, True, False, True
HELD = []  # user messages received while a turn runs
TASKS = {}
REPLIES = {}  # request_id -> host control_response

inbox = queue.Queue()


def reader():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except ValueError:
            log({"bad": line})
            continue
        log({"in": obj})
        inbox.put(obj)
    inbox.put(None)


threading.Thread(target=reader, daemon=True).start()


def sys_msg(subtype, **fields):
    m = {"type": "system", "subtype": subtype, "uuid": str(uuidlib.uuid4()), "session_id": SESSION}
    m.update(fields)
    out(m)


def ok(rid, resp):
    out({"type": "control_response", "response": {"subtype": "success", "request_id": rid, "response": resp}})


def err(rid, msg, code=None):
    r = {"subtype": "error", "request_id": rid, "error": msg}
    if code:
        r["error_code"] = code
    out({"type": "control_response", "response": r})


def handle_host_request(obj):
    rid = obj.get("request_id")
    req = obj.get("request") or {}
    sub = req.get("subtype")
    if sub == "initialize":
        ok(rid, {
            "commands": [
                {"name": "compact", "description": "Compact the conversation", "argumentHint": "[instructions]"},
                {"name": "review", "description": "Review changes", "argumentHint": ""},
            ],
            "models": [
                {"value": "haiku", "resolvedModel": "claude-haiku-4-5-20251001", "displayName": "Haiku"},
                {"value": "opus", "resolvedModel": "claude-opus-5-5", "displayName": "Opus",
                 "supportsEffort": True, "supportsAdaptiveThinking": True},
            ],
            "agents": [{"name": "Explore", "description": "Search"}],
            "current_permission_mode": STATE["mode"],
            "fast_mode_state": "off",
        })
    elif sub == "get_settings":
        resolved, has_effort, can_ultra, has_max = model_caps(STATE["model"])
        effort = STATE["effort"] or "high"
        if effort in ("max", "xhigh") and not has_max:
            effort = "high"  # the CLI runs it as high on such a model
        ok(rid, {"effective": {}, "sources": [], "applied": {
            "model": resolved,
            "effort": effort if has_effort else None,
            "advisor": None,
            "ultracode": STATE["ultracode"] and can_ultra,
            "ultracodeRequested": STATE["ultracode"],
            "ultracodeAvailable": can_ultra,
        }})
    elif sub == "set_max_thinking_tokens":
        STATE["thinking"] = req.get("max_thinking_tokens")
        ok(rid, None)
    elif sub == "get_usage":
        ok(rid, {"session": {}, "subscription_type": "max", "rate_limits_available": True,
                 "behaviors": None, "rate_limits": {
                     "five_hour": {"utilization": 42, "resets_at": "2099-01-01T00:00:00.000000+00:00"},
                     "seven_day": {"utilization": 7, "resets_at": "2099-01-05T00:00:00+00:00"},
                     "amber_cistern": {"utilization": 99}, "spend": {"used": 1}}})
    elif sub == "set_permission_mode":
        if req.get("mode") == "auto":
            err(rid, "Auto mode is not available for this model.", "auto_mode_model")
        else:
            STATE["mode"] = req.get("mode")
            ok(rid, {})
            sys_msg("status", status=None, permissionMode=STATE["mode"])
    elif sub == "set_model":
        STATE["model"] = req.get("model")
        ok(rid, {})
    elif sub == "rename_session":
        ok(rid, {})
    elif sub == "apply_flag_settings":
        settings = req.get("settings") or {}
        if "effortLevel" in settings:
            # Like the CLI: a level that moves, sent without the ultracode
            # key, turns ultracode off; the key alone leaves the level.
            if settings["effortLevel"] != STATE["effort"] and "ultracode" not in settings:
                STATE["ultracode"] = False
            STATE["effort"] = settings["effortLevel"]
        if "ultracode" in settings:
            STATE["ultracode"] = bool(settings["ultracode"])
        ok(rid, None)
    elif sub == "interrupt":
        ok(rid, {"still_queued": [m.get("uuid") for m in HELD]})
        if STATE["busy"]:
            STATE["interrupted"] = True
    elif sub == "stop_task":
        tid = req.get("task_id")
        if tid in TASKS:
            del TASKS[tid]
            ok(rid, {})
            sys_msg("task_notification", task_id=tid, status="stopped", output_file="", summary="stopped")
        else:
            err(rid, f"No task {tid}")
    elif sub == "get_context_usage":
        ok(rid, {"totalTokens": 1234, "maxTokens": 200000})
    elif sub == "get_task_output":
        tid = req.get("task_id")
        if tid:
            ok(rid, {"output": f"[{tid}] ready on :3000\n", "total_bytes": 9000, "truncated": True})
        else:
            err(rid, "task_id required")
    elif sub == "mcp_status":
        ok(rid, {"mcpServers": [{"name": "gh", "status": "connected", "scope": "user",
                                 "config": {"env": {"TOKEN": "secret"}}}]})
    elif sub == "cancel_async_message":
        target = req.get("message_uuid")
        found = any(m.get("uuid") == target for m in HELD)
        HELD[:] = [m for m in HELD if m.get("uuid") != target]
        ok(rid, {"cancelled": found})
    else:
        err(rid, f"Unsupported control request: {sub}")


def process(obj, want=None):
    """Handle one inbound line. Returns True when it is the awaited reply."""
    if obj is None:
        sys.exit(0)
    t = obj.get("type")
    if t == "control_request":
        handle_host_request(obj)
    elif t == "control_response":
        rid = obj.get("response", {}).get("request_id")
        REPLIES[rid] = obj
        return want is not None and rid in want
    elif t == "user":
        HELD.append(obj)
    return False


def pump(deadline, want=None):
    """Process input until `deadline`, an interrupt, or all `want` replies."""
    while True:
        if want is not None and all(w in REPLIES for w in want):
            return True
        if STATE["interrupted"]:
            return False
        left = deadline - time.time()
        if left <= 0:
            return False
        try:
            obj = inbox.get(timeout=left)
        except queue.Empty:
            return False
        process(obj, want)


def ask(request, timeout=30.0):
    rid = str(uuidlib.uuid4())
    out({"type": "control_request", "request_id": rid, "request": request})
    pump(time.time() + timeout, want={rid})
    reply = REPLIES.get(rid, {}).get("response", {})
    return rid, reply.get("response") if reply.get("subtype") == "success" else {"error": reply.get("error")}


def text_of(msg):
    content = msg.get("message", {}).get("content")
    if isinstance(content, str):
        return content
    return "".join(b.get("text", "") for b in content or [] if b.get("type") == "text")


def stream_text(text, answered):
    mid = "msg_" + uuidlib.uuid4().hex[:12]
    first = {"type": "stream_event", "event": {"type": "message_start", "message": {"id": mid}},
             "session_id": SESSION, "uuid": str(uuidlib.uuid4()), "parent_tool_use_id": None}
    if answered:
        first["user_message_uuid"] = answered[-1]
        first["user_message_uuids"] = answered
    out(first)

    def ev(e):
        out({"type": "stream_event", "event": e, "session_id": SESSION,
             "uuid": str(uuidlib.uuid4()), "parent_tool_use_id": None})

    ev({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})
    for part in (text[: len(text) // 2], text[len(text) // 2:]):
        ev({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": part}})
    ev({"type": "content_block_stop", "index": 0})
    ev({"type": "message_stop"})
    out({"type": "assistant", "message": {"id": mid, "model": MODEL, "role": "assistant",
                                          "content": [{"type": "text", "text": text}]},
         "session_id": SESSION, "uuid": str(uuidlib.uuid4()), "parent_tool_use_id": None})


def result(text, answered, subtype="success", is_error=False):
    r = {"type": "result", "subtype": subtype, "is_error": is_error, "result": text,
         "duration_ms": 1200, "session_id": SESSION, "uuid": str(uuidlib.uuid4())}
    if answered:
        r["user_message_uuid"] = answered[-1]
        r["user_message_uuids"] = answered
    out(r)


BASH_SUGGESTIONS = [
    {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "touch x1 *"}],
     "behavior": "allow", "destination": "localSettings"},
    {"type": "setMode", "mode": "acceptEdits", "destination": "session"},
]


def can_use_tool(tool, tool_input, suggestions=None):
    return {"subtype": "can_use_tool", "tool_name": tool, "display_name": tool, "input": tool_input,
            "description": tool_input.get("description", ""),
            "permission_suggestions": suggestions or [], "tool_use_id": "toolu_" + uuidlib.uuid4().hex[:10]}


def run_turn(msg):
    answered = [msg["uuid"]] if msg.get("uuid") else []
    text = text_of(msg).strip()
    images = [b for b in msg.get("message", {}).get("content") or [] if isinstance(b, dict) and b.get("type") == "image"]
    if not STATE["started"]:
        STATE["started"] = True
        sys_msg("init", cwd=os.getcwd(), model=MODEL,
                permissionMode="bypassPermissions" if MODEL == "fake-bypass" else STATE["mode"],
                claude_code_version="9.9.9-fake", capabilities=["interrupt_receipt_v1"], tools=[])
    STATE["interrupted"] = False

    if text == "hello" or (not text and images):
        stream_text(f"Hello there ({len(images)} images)", answered)
        result("hello", answered)
    elif text == "slow":
        STATE["busy"] = True
        stream_text("Working slowly", answered)
        pump(time.time() + 1.5)
        STATE["busy"] = None
        if STATE["interrupted"]:
            result("[Request interrupted by user]", answered, "error_during_execution", True)
        else:
            result("slow done", answered)
    elif text == "bash":
        _, resp = ask(can_use_tool("Bash", {"command": "touch x1", "description": "Create file x1"},
                                   BASH_SUGGESTIONS))
        log({"decision": resp})
        if resp.get("behavior") == "allow":
            open(os.path.join(os.getcwd(), "x1"), "w").close()
            result("allowed", answered)
        else:
            result("denied: " + str(resp.get("message")), answered)
    elif text == "danger":
        _, resp = ask(can_use_tool("Write", {"file_path": ".claude/settings.local.json", "content": "{}"}))
        log({"decision": resp})
        result("danger " + str(resp.get("behavior")), answered)
    elif text == "ask":
        _, resp = ask(can_use_tool("AskUserQuestion", {"questions": [
            {"question": "Favourite colour?", "header": "Colour", "multiSelect": False,
             "options": [{"label": "Blue", "description": "calm"}, {"label": "Red"}]}]}))
        log({"decision": resp})
        result("answers: " + json.dumps((resp.get("updatedInput") or {}).get("answers")), answered)
    elif text == "plan":
        _, resp = ask(can_use_tool("ExitPlanMode", {"plan": "1. Create the file\n2. Done", "planFilePath": "/tmp/p.md"}))
        log({"decision": resp})
        for p in resp.get("updatedPermissions") or []:
            if p.get("type") == "setMode":
                STATE["mode"] = p.get("mode")
                sys_msg("status", status=None, permissionMode=STATE["mode"])
        result("plan " + str(resp.get("behavior")), answered)
    elif text == "cancel":
        rid = str(uuidlib.uuid4())
        out({"type": "control_request", "request_id": rid,
             "request": can_use_tool("Bash", {"command": "sleep 1", "description": "Nap"})})
        pump(time.time() + 0.5)
        out({"type": "control_cancel_request", "request_id": rid})
        pump(time.time() + 0.3)
        log({"cancel_reply": REPLIES.get(rid)})
        result("cancelled", answered)
    elif text == "cancel-fast":
        # The request and its cancel in ONE write: both lines sit in the
        # host's read buffer together (a hook won the race at once).
        rid = str(uuidlib.uuid4())
        req = {"type": "control_request", "request_id": rid,
               "request": can_use_tool("Bash", {"command": "sleep 1", "description": "Nap"})}
        sys.stdout.write(json.dumps(req) + "\n" +
                         json.dumps({"type": "control_cancel_request", "request_id": rid}) + "\n")
        sys.stdout.flush()
        pump(time.time() + 0.5)
        log({"cancel_fast_reply": REPLIES.get(rid)})
        result("cancelled fast", answered)
    elif text == "elicit":
        _, resp = ask({"subtype": "elicitation", "mcp_server_name": "gh", "message": "Which repo?",
                       "requested_schema": {"type": "object", "properties": {"repo": {"type": "string"}}}})
        log({"decision": resp})
        result("elicited " + json.dumps(resp), answered)
    elif text == "dialog":
        rid1 = str(uuidlib.uuid4())
        rid2 = str(uuidlib.uuid4())
        out({"type": "control_request", "request_id": rid1,
             "request": {"subtype": "request_user_dialog", "dialog_kind": "mystery", "payload": {}}})
        out({"type": "control_request", "request_id": rid2, "request": {"subtype": "frobnicate"}})
        pump(time.time() + 10, want={rid1, rid2})
        log({"dialog": REPLIES.get(rid1), "unknown": REPLIES.get(rid2)})
        result("dialog", answered)
    elif text == "task":
        TASKS["t1"] = True
        sys_msg("task_started", task_id="t1", tool_use_id="toolu_task", description="Explore the repo",
                task_type="local_agent", is_backgrounded=True)
        sys_msg("task_progress", task_id="t1", description="Explore the repo", subagent_type="Explore",
                last_tool_name="Grep", usage={"total_tokens": 900, "tool_uses": 2, "duration_ms": 50})
        result("task started", answered)
    elif text == "fail":
        result("API Error: overloaded", answered, "error_during_execution_api", True)
    elif text == "limits":
        # What the CLI emits when a window's rounded percentage moves.
        out({"type": "rate_limit_event", "session_id": SESSION, "uuid": str(uuidlib.uuid4()),
             "rate_limit_info": {"status": "allowed_warning", "rateLimitType": "five_hour",
                                 "resetsAt": 4070908800, "isUsingOverage": False,
                                 "unifiedWindows": {
                                     "five_hour": {"utilization": 0.424, "resetsAt": 4070908800},
                                     "seven_day": {"utilization": 0.07, "resetsAt": 4071254400}}}})
        stream_text("limits", answered)
        result("limits", answered)
    else:
        stream_text("ok", answered)
        result("ok", answered)


while True:
    if HELD:
        run_turn(HELD.pop(0))
        continue
    obj = inbox.get()
    if obj is None:
        sys.exit(0)
    if obj.get("type") == "user":
        run_turn(obj)
    else:
        process(obj)
