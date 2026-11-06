//! `mc_session`: the calling session as Mother Claude sees it (read-only).

use serde_json::{json, Value};

use super::{Registry, ToolAnnotations, ToolCtx, ToolDef, ToolError, ToolOutput};
use crate::claude::accounts::{Account, DEFAULT_ID};

pub(super) fn register(r: &mut Registry) {
    r.add(
        ToolDef {
            name: "mc_session".into(),
            title: Some("This session in Mother Claude".into()),
            description: "Describe the calling Claude Code session as the Mother Claude desktop \
                          app sees it: its session id, project folder, Claude account, whether \
                          Mother Claude launched it (owned), its model and permission mode. \
                          Read-only."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(true),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
                ..Default::default()
            }),
        },
        |ctx: ToolCtx, _args: Value| async move { describe(&ctx).await.map(ToolOutput::json) },
    );
}

fn account_json(id: &str, name: &str, kind: impl serde::Serialize) -> Value {
    json!({ "id": id, "name": name, "kind": kind })
}

fn from_registry(a: &Account) -> Value {
    account_json(&a.id, &a.name, a.kind)
}

/// The JSON `mc_session` returns, or why the caller is unknown.
pub async fn describe(ctx: &ToolCtx) -> Result<Value, ToolError> {
    let Some(id) = ctx.caller.session_id.clone() else {
        return Err(ToolError::new(
            "unknown_session",
            "Mother Claude could not tell which session is calling. It knows sessions by the \
             Claude Code process that runs them; if this session started moments ago, try \
             again.",
        ));
    };
    let state = &ctx.state;
    let summary = state.find_session(&id).await;
    let live = state.control.state(&id);
    let account = state
        .control
        .account_of(&id)
        .and_then(|a| state.accounts.get(&a))
        .map(|a| from_registry(&a))
        .or_else(|| {
            summary
                .as_ref()
                .and_then(|s| s.account.as_ref())
                .map(|a| account_json(&a.id, &a.name, a.kind))
        })
        .or_else(|| {
            // With the Default alone, sessions name no account: it is that one.
            state
                .accounts
                .is_default_only()
                .then(|| state.accounts.get(DEFAULT_ID))
                .flatten()
                .map(|a| from_registry(&a))
        });
    let model = summary
        .as_ref()
        .and_then(|s| s.model.as_ref())
        .map(|m| json!({ "id": m.id, "label": m.label }))
        .or_else(|| {
            live.as_ref()
                .and_then(|l| l.model.clone())
                .map(|m| json!({ "id": m, "label": m }))
        });
    let permission_mode = summary
        .as_ref()
        .and_then(|s| s.permission_mode.clone())
        .or_else(|| live.as_ref().and_then(|l| l.permission_mode.clone()));
    let cwd = ctx.caller.cwd.clone().or_else(|| {
        summary
            .as_ref()
            .map(|s| s.cwd.clone())
            .filter(|c| !c.is_empty())
    });
    Ok(json!({
        "session": id,
        "owned": ctx.caller.owned,
        "pid": ctx.caller.pid,
        "cwd": cwd,
        "project": summary.as_ref().map(|s| s.project_name.clone()).filter(|p| !p.is_empty()),
        "title": summary.as_ref().and_then(|s| s.title.clone()),
        "surface": summary.as_ref().map(|s| s.surface),
        "status": summary.as_ref().map(|s| s.status),
        "account": account,
        "model": model,
        "permissionMode": permission_mode,
        "effort": summary.as_ref().and_then(|s| s.effort.clone()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::Caller;

    fn state() -> crate::state::AppState {
        crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        )
    }

    #[tokio::test]
    async fn an_unknown_caller_is_told_so() {
        let ctx = ToolCtx {
            state: state(),
            caller: Caller::default(),
        };
        let err = describe(&ctx).await.unwrap_err();
        assert_eq!(err.code, "unknown_session");
    }

    #[tokio::test]
    async fn a_known_session_is_described() {
        let st = state();
        let id = "0f1e2d3c-4b5a-4968-8776-655443322110";
        st.mark_owned(id, "/tmp/proj", 1).await;
        let ctx = ToolCtx {
            state: st,
            caller: Caller {
                session_id: Some(id.into()),
                pid: Some(4242),
                cwd: Some("/tmp/proj".into()),
                owned: true,
            },
        };
        let v = describe(&ctx).await.unwrap();
        assert_eq!(v["session"], id);
        assert_eq!(v["owned"], true);
        assert_eq!(v["pid"], 4242);
        assert_eq!(v["cwd"], "/tmp/proj");
        assert_eq!(v["account"]["id"], DEFAULT_ID);
        assert_eq!(v["surface"], "mother-claude");
    }
}
