//! Runtime callbacks: the human-in-the-loop surface.
//!
//! When Claude wants to run a tool that is not auto-approved, or asks the
//! operator a question, the conversation blocks and the request appears here.
//! The same request simultaneously appears as a prompt card in the Mother
//! Claude dashboard — whichever surface answers first wins, and the other is
//! dropped. That is deliberate: a phone and a script are both legitimate ways
//! to unblock a turn.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, State};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ok, validate_id, Body};
use crate::bridge::error::{BridgeError, BridgeResult};
use crate::server::auth;
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

/// Permission modes that remove the human from the loop entirely.
const UNGATED_MODES: [&str; 2] = ["bypassPermissions", "dontAsk"];

/// Whether a `PermissionUpdate` list grants a *durable* or *blanket* privilege.
///
/// This is the teeth behind the dangerous gate. Approving one benign `Read` is
/// not dangerous — but attaching `{"type":"setMode","mode":"bypassPermissions",
/// "destination":"userSettings"}` to that approval silently disarms every future
/// prompt, and writes it to the user's settings file. The severity of an answer
/// is not the severity of the question it answers.
fn grants_standing_privilege(updates: &Value) -> bool {
    let Some(entries) = updates.as_array() else {
        // Anything that is not a list of updates is not something we can reason
        // about, so treat it as privileged.
        return !updates.is_null();
    };
    entries.iter().any(|entry| {
        let kind = entry
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let destination = entry
            .get("destination")
            .and_then(Value::as_str)
            .unwrap_or("session");
        // Anything written outside this session outlives the request it rode in on.
        if destination != "session" && destination != "cliArg" {
            return true;
        }
        match kind {
            "setMode" => entry
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|m| UNGATED_MODES.contains(&m)),
            // Widening the filesystem the agent may touch.
            "addDirectories" => true,
            _ => false,
        }
    })
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(request_id): Path<String>,
    Body(body): Body<RespondBody>,
) -> BridgeResult<Response> {
    validate_id("request_id", &request_id)?;

    let pending = state
        .bridge
        .host
        .pending_request(&request_id)
        .ok_or_else(|| BridgeError::request_not_found(&request_id))?;

    // Approving a dangerous action obeys the same rule as the dashboard: local
    // desktop only, unless remote-dangerous is explicitly enabled. An answer
    // that also grants a standing privilege is dangerous regardless of how
    // harmless the question was.
    let dangerous = pending
        .get("dangerous")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || body.is_escalation();
    if body.is_approval()
        && auth::dangerous_blocked(
            dangerous,
            auth::is_loopback(&peer),
            state.auth.allow_remote_dangerous,
        )
    {
        return Err(BridgeError::forbidden(
            "dangerous_blocked",
            "Approving a dangerous action — or attaching a standing permission \
             change to an approval — is restricted to local clients.",
        )
        .with("request_id", request_id));
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
        // Session-scoped allow rules are ordinary "always allow for now".
        let ordinary = parse(json!({
            "decision": "allow",
            "updated_permissions": [{ "type": "addRules", "behavior": "allow",
                                      "rules": [{ "toolName": "Read" }],
                                      "destination": "session" }],
        }));
        assert!(!ordinary.is_escalation());

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
