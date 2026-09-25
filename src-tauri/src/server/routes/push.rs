//! Web Push routes (design §4.5 "Push", §6, review amendments).
//!
//! Owner: **B5 (push)**. State and logic live in `server/push.rs`
//! (`state.push`). Subscriptions and prefs are per paired device
//! (`ctx.device_id()`); the desktop has no device id — it shows alerts in the
//! app — so it can read the key and hints but not subscribe.
//!
//! * `GET /api/push/vapid` → `{publicKey, available, secureContextHint}`
//! * `POST /api/push/subscribe {endpoint, keys:{p256dh, auth},
//!   applicationServerKey?}` → `{ok, subscriptions}`
//! * `DELETE /api/push/subscribe {endpoint}` → `{ok, removed}`
//! * `POST /api/push/test` → `{ok}` (only ok/failed — never the push
//!   service's status, so it cannot be used as a probe; 5 s spacing)
//! * `GET /api/push/prefs` → `{permission, question, plan, finished, failed,
//!   claude, idle, showDetails, sessions:{id:'follow'|'mute'},
//!   turnFinished:[id], subscriptions, available}`
//! * `PUT /api/push/prefs` → same, after merging the partial body (booleans;
//!   `sessions:{id:'follow'|'mute'|'default'}`; `turnFinished:{id:bool}` = arm
//!   or disarm "notify me when this turn finishes").

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::Body;
use crate::server::auth::Authed;
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::server::push::{self, PrefsPatch};
use crate::state::AppState;

/// `POST /api/push/subscribe` body (`PushSubscription.toJSON()`, plus the
/// optional key the browser subscribed with).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubscribeBody {
    pub endpoint: String,
    pub keys: SubscribeKeys,
    /// base64url of the `applicationServerKey` used; a stale one gets 409.
    pub application_server_key: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SubscribeKeys {
    pub p256dh: String,
    pub auth: String,
}

/// `DELETE /api/push/subscribe` body.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UnsubscribeBody {
    pub endpoint: String,
}

fn device_of(ctx: &ClientContext) -> ApiResult<&str> {
    ctx.device_id().ok_or_else(|| {
        ApiError::forbidden(
            "push_device_only",
            "Push notifications are for paired devices; this Mac shows alerts in the app.",
        )
    })
}

/// `GET /api/push/vapid` → `{publicKey, available, secureContextHint}`.
pub async fn vapid(Authed(ctx): Authed, State(state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "publicKey": state.push.public_key(),
        "available": ctx.device_id().is_some(),
        "secureContextHint": push::secure_context_hint(&state, &ctx),
    })))
}

/// `POST /api/push/subscribe {endpoint, keys:{p256dh, auth}}`.
pub async fn subscribe(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Body(body): Body<SubscribeBody>,
) -> ApiResult<Json<Value>> {
    let device = device_of(&ctx)?;
    let out = push::subscribe(
        &state,
        device,
        &body.endpoint,
        &body.keys.p256dh,
        &body.keys.auth,
        body.application_server_key.as_deref(),
    )
    .await?;
    Ok(Json(json!({ "ok": true, "subscriptions": out.count })))
}

/// `DELETE /api/push/subscribe {endpoint}`.
pub async fn unsubscribe(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Body(body): Body<UnsubscribeBody>,
) -> ApiResult<Json<Value>> {
    let device = device_of(&ctx)?;
    if body.endpoint.trim().is_empty() {
        return Err(ApiError::unprocessable(
            "invalid_push_endpoint",
            "endpoint is required.",
        ));
    }
    let removed = state.push.unsubscribe(device, &body.endpoint);
    Ok(Json(json!({ "ok": true, "removed": removed })))
}

/// `POST /api/push/test` → `{ok}`.
pub async fn test(Authed(ctx): Authed, State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let device = device_of(&ctx)?;
    let ok = push::send_test(&state, device).await?;
    Ok(Json(json!({ "ok": ok })))
}

/// `GET /api/push/prefs`.
pub async fn get_prefs(
    Authed(ctx): Authed,
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    let view = match ctx.device_id() {
        Some(device) => state.push.prefs(device, true),
        // The desktop: defaults, flagged unavailable.
        None => state.push.prefs("", false),
    };
    Ok(Json(serde_json::to_value(view).unwrap_or_default()))
}

/// `PUT /api/push/prefs`.
pub async fn put_prefs(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Body(patch): Body<PrefsPatch>,
) -> ApiResult<Json<Value>> {
    let device = device_of(&ctx)?;
    let view = state.push.update_prefs(device, patch)?;
    tracing::info!(device, "push prefs updated");
    Ok(Json(serde_json::to_value(view).unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::context::Principal;

    fn state() -> AppState {
        crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        )
    }

    fn device(id: &str) -> Authed {
        let mut ctx = ClientContext::unknown();
        ctx.principal = Principal::Device {
            id: id.into(),
            allow_dangerous: false,
        };
        Authed(ctx)
    }

    fn desktop() -> Authed {
        let mut ctx = ClientContext::unknown();
        ctx.ingress = crate::server::context::Ingress::Loopback;
        ctx.local = true;
        ctx.principal = Principal::Desktop;
        Authed(ctx)
    }

    fn sub_body(endpoint: &str) -> SubscribeBody {
        let (_, s) = crate::server::push::payload_test_subscription();
        SubscribeBody {
            endpoint: endpoint.into(),
            keys: SubscribeKeys {
                p256dh: s.p256dh,
                auth: s.auth,
            },
            application_server_key: None,
        }
    }

    #[tokio::test]
    async fn vapid_and_prefs_for_desktop_and_devices() {
        let st = state();
        let Json(v) = vapid(desktop(), State(st.clone())).await.unwrap();
        assert_eq!(v["publicKey"], st.push.public_key());
        assert_eq!(v["available"], false);
        assert_eq!(v["secureContextHint"]["code"], "desktop");
        assert!(v["secureContextHint"]["iosHomeScreen"]
            .as_str()
            .unwrap()
            .contains("Home Screen"));
        let Json(v) = vapid(device("d1"), State(st.clone())).await.unwrap();
        assert_eq!(v["available"], true);
        assert_eq!(v["secureContextHint"]["code"], "lan_certificate");

        let Json(p) = get_prefs(desktop(), State(st.clone())).await.unwrap();
        assert_eq!(p["available"], false);
        assert_eq!(p["permission"], true);
        let err = put_prefs(desktop(), State(st.clone()), Body(PrefsPatch::default()))
            .await
            .unwrap_err();
        assert_eq!(err.code, "push_device_only");

        let patch: PrefsPatch = serde_json::from_value(json!({
            "finished": false,
            "sessions": {"s1": "mute"},
            "turnFinished": {"s2": true}
        }))
        .unwrap();
        let Json(p) = put_prefs(device("d1"), State(st.clone()), Body(patch))
            .await
            .unwrap();
        assert_eq!(p["finished"], false);
        assert_eq!(p["sessions"]["s1"], "mute");
        assert_eq!(p["turnFinished"][0], "s2");
        let bad: PrefsPatch = serde_json::from_value(json!({"sessions": {"s1": "shout"}})).unwrap();
        let err = put_prefs(device("d1"), State(st.clone()), Body(bad))
            .await
            .unwrap_err();
        assert_eq!(err.code, "invalid_push_prefs");
        // Another device's prefs are its own.
        let Json(p) = get_prefs(device("d2"), State(st.clone())).await.unwrap();
        assert_eq!(p["finished"], true);
    }

    #[tokio::test]
    async fn subscribe_validates_everything() {
        let st = state();
        st.push.test_transport_for_routes();
        // Desktop cannot subscribe.
        let err = subscribe(
            desktop(),
            State(st.clone()),
            Body(sub_body("https://fcm.googleapis.com/fcm/send/a")),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "push_device_only");
        // SSRF attempts are refused before anything is stored or fetched.
        for bad in [
            "http://127.0.0.1:5612/v1/threads",
            "https://127.0.0.1/x",
            "https://192.168.1.1/admin",
            "https://[::1]/x",
            "https://169.254.169.254/latest",
            "https://attacker.example/collect",
            "https://fcm.googleapis.com:8080/x",
        ] {
            let err = subscribe(device("d1"), State(st.clone()), Body(sub_body(bad)))
                .await
                .unwrap_err();
            assert_eq!(err.code, "invalid_push_endpoint", "{bad}");
        }
        // Bad keys.
        let mut b = sub_body("https://fcm.googleapis.com/fcm/send/a");
        b.keys.auth = "AAAA".into();
        let err = subscribe(device("d1"), State(st.clone()), Body(b))
            .await
            .unwrap_err();
        assert_eq!(err.code, "invalid_push_keys");
        // Stale applicationServerKey.
        let mut b = sub_body("https://fcm.googleapis.com/fcm/send/a");
        b.application_server_key = Some("BAAA".into());
        let err = subscribe(device("d1"), State(st.clone()), Body(b))
            .await
            .unwrap_err();
        assert_eq!(err.code, "vapid_key_mismatch");
        // A good one (the current key may be echoed back).
        let mut b = sub_body("https://fcm.googleapis.com/fcm/send/a");
        b.application_server_key = Some(st.push.public_key().to_string());
        let Json(v) = subscribe(device("d1"), State(st.clone()), Body(b))
            .await
            .unwrap();
        assert_eq!(v, json!({"ok": true, "subscriptions": 1}));
        // Capped at 5 per device.
        for i in 0..7 {
            let _ = subscribe(
                device("d1"),
                State(st.clone()),
                Body(sub_body(&format!("https://web.push.apple.com/t{i}"))),
            )
            .await
            .unwrap();
        }
        assert_eq!(st.push.subscription_count("d1"), 5);
        // Unsubscribe only touches the caller's own subscriptions.
        let Json(v) = unsubscribe(
            device("d2"),
            State(st.clone()),
            Body(UnsubscribeBody {
                endpoint: "https://web.push.apple.com/t6".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(v["removed"], false);
        let Json(v) = unsubscribe(
            device("d1"),
            State(st.clone()),
            Body(UnsubscribeBody {
                endpoint: "https://web.push.apple.com/t6".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(v["removed"], true);
        assert_eq!(st.push.subscription_count("d1"), 4);
        // Revocation drops the rest.
        push::drop_device(&st, "d1");
        assert_eq!(st.push.subscription_count("d1"), 0);
    }

    #[tokio::test]
    async fn private_dns_answers_are_refused() {
        let st = state();
        st.push.test_transport_resolving_to("10.0.0.7");
        let err = subscribe(
            device("d1"),
            State(st.clone()),
            Body(sub_body("https://fcm.googleapis.com/fcm/send/a")),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "invalid_push_endpoint");
        assert_eq!(st.push.subscription_count("d1"), 0);
    }

    #[tokio::test]
    async fn test_route_needs_a_subscription_and_is_rate_limited() {
        let st = state();
        let err = test(device("d1"), State(st.clone())).await.unwrap_err();
        assert_eq!(err.code, "push_not_subscribed");
        let err = test(device("d1"), State(st.clone())).await.unwrap_err();
        assert_eq!(err.status.as_u16(), 429);
        let err = test(desktop(), State(st.clone())).await.unwrap_err();
        assert_eq!(err.code, "push_device_only");
    }
}
