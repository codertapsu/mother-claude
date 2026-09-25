//! What a notification says, and the encrypted HTTP request that carries it.
//!
//! Notification text is shown on lock screens and passes through Apple,
//! Google or Mozilla, so by default it carries only the session display name
//! (title) and the event kind plus tool name (body), e.g. "Needs approval ·
//! Bash". Command text, file paths, prompts, question or plan text and
//! messages appear only for devices that opted into `showDetails`.
//!
//! The payload never contains a URL: the service worker builds
//! `/#/session/<id>` itself from a validated `sessionId`.

use std::time::Duration;

use axum::http::{HeaderValue, Request, Uri};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use web_push_native::{Auth, WebPushBuilder};

use super::engine::{PushEvent, PushKind};
use super::store::{decode_b64, Subscription};
use super::vapid::VapidKeys;

/// Title length cap (chars).
const TITLE_MAX: usize = 80;
/// Body length cap (chars).
const BODY_MAX: usize = 240;
/// A detail snippet (chars).
const DETAIL_MAX: usize = 140;
/// JWT validity (RFC 8292: at most 24 h). The TTL header is set separately.
const JWT_VALIDITY: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Normal,
    High,
}

impl Urgency {
    pub fn as_str(self) -> &'static str {
        match self {
            Urgency::Normal => "normal",
            Urgency::High => "high",
        }
    }
}

/// A rendered notification (per device: details depend on its prefs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// `PushKind` of the first event (the SW may pick an icon from it).
    pub kind: PushKind,
    pub kinds: Vec<PushKind>,
    /// Notification tag: the pending id for a single card, else the session
    /// id — a newer notification with the same tag replaces the older one.
    pub tag: String,
    /// Alert again when replacing (attention events).
    pub renotify: bool,
    /// A single, non-dangerous permission card: a service worker may offer
    /// Allow / Deny actions (it still posts `/respond` with the device token,
    /// so the server's policy applies).
    pub actionable: bool,
    pub pending_ids: Vec<String>,
    pub session_id: Option<String>,
    pub urgency: Urgency,
    pub ttl_secs: u64,
}

/// Strip control characters, collapse whitespace, cap at `max` chars.
pub fn clean(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max * 4));
    let mut last_space = true;
    let mut n = 0;
    for c in s.chars() {
        let c = if c.is_control() || c.is_whitespace() || is_invisible(c) {
            ' '
        } else {
            c
        };
        if c == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        if n == max {
            // Replace the last char with an ellipsis.
            out.pop();
            out.push('…');
            return out.trim_end().to_string();
        }
        out.push(c);
        n += 1;
    }
    out.trim_end().to_string()
}

/// Bidi overrides and zero-width characters (spoofable text on lock screens).
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// A short machine-ish label (error kind, waitingFor) or nothing. Letters,
/// digits, spaces, `_` and `-` only, so a path or a sentence never slips
/// through.
fn safe_label(s: Option<&str>) -> Option<String> {
    let s = clean(s?, 40);
    (!s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-')))
    .then(|| s.replace('_', " "))
}

/// Tool names: `Bash`, `Edit`, `mcp__server__tool` (rendered `server · tool`).
fn tool_label(tool: Option<&str>) -> Option<String> {
    let t = tool?.trim();
    if t.is_empty() {
        return None;
    }
    let t = match t.strip_prefix("mcp__") {
        Some(rest) => rest.replacen("__", " · ", 1),
        None => t.to_string(),
    };
    let t = clean(&t, 60);
    (!t.is_empty()).then_some(t)
}

fn duration_label(ms: u64) -> Option<String> {
    let s = ms / 1000;
    if s == 0 {
        return None;
    }
    Some(if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        let m = (s % 3600) / 60;
        if m == 0 {
            format!("{} h", s / 3600)
        } else {
            format!("{} h {m} min", s / 3600)
        }
    })
}

/// The detail-free line for one event.
fn short_line(e: &PushEvent) -> String {
    match e.kind {
        PushKind::Permission => match tool_label(e.tool.as_deref()) {
            Some(t) => format!("Needs approval · {t}"),
            None => "Needs approval".into(),
        },
        PushKind::Question => "Has a question".into(),
        PushKind::Plan => "Plan ready for review".into(),
        PushKind::Waiting => match safe_label(e.label.as_deref()) {
            Some(l) => format!("Waiting at the Mac · {l}"),
            None => "Waiting at the Mac".into(),
        },
        PushKind::Finished => match e.worked_ms.and_then(duration_label) {
            Some(d) => format!("Finished after {d}"),
            None => "Finished".into(),
        },
        PushKind::Failed => match safe_label(e.label.as_deref()) {
            Some(l) => format!("Stopped on an error · {l}"),
            None => "Stopped on an error".into(),
        },
        PushKind::Claude => "Claude sent a notification".into(),
        PushKind::Idle => "Waiting for your input".into(),
        PushKind::Test => "Test notification — notifications work.".into(),
    }
}

/// One event with its detail (only for devices that opted in).
fn detailed_line(e: &PushEvent) -> String {
    let Some(detail) = e.detail.as_deref().map(|d| clean(d, DETAIL_MAX)) else {
        return short_line(e);
    };
    if detail.is_empty() {
        return short_line(e);
    }
    match e.kind {
        PushKind::Claude => detail,
        PushKind::Question => format!("Question: {detail}"),
        PushKind::Plan => format!("Plan ready: {detail}"),
        PushKind::Finished | PushKind::Failed => format!("{} · {detail}", short_line(e)),
        _ => format!("{}: {detail}", short_line(e)),
    }
}

/// Render the events of one batch for one device.
pub fn render(
    session_name: Option<&str>,
    session_id: &str,
    events: &[PushEvent],
    show_details: bool,
) -> Notification {
    let test = events.iter().all(|e| e.kind == PushKind::Test);
    let title = session_name
        .map(|n| clean(n, TITLE_MAX))
        .filter(|n| !n.is_empty() && !test)
        .unwrap_or_else(|| "Mother Claude".into());
    let body = match events {
        [] => String::new(),
        [one] if show_details => detailed_line(one),
        [one] => short_line(one),
        many => {
            let lines: Vec<String> = many.iter().map(short_line).collect();
            format!("{} updates · {}", many.len(), lines.join(", "))
        }
    };
    let pending_ids: Vec<String> = events.iter().filter_map(|e| e.pending_id.clone()).collect();
    let attention = events.iter().any(|e| e.kind.is_attention());
    let actionable = matches!(
        events,
        [e] if e.kind == PushKind::Permission && !e.dangerous && e.pending_id.is_some()
    );
    let tag = match (events, pending_ids.first()) {
        _ if test => "mother-claude-test".to_string(),
        ([_], Some(pid)) => pid.clone(),
        _ => session_id.to_string(),
    };
    let urgency = if test
        || attention
        || events
            .iter()
            .any(|e| matches!(e.kind, PushKind::Failed | PushKind::Claude))
    {
        Urgency::High
    } else {
        Urgency::Normal
    };
    let ttl_secs = if test {
        300
    } else if attention {
        3600
    } else {
        4 * 3600
    };
    Notification {
        title,
        body: clean(&body, BODY_MAX),
        kind: events.first().map(|e| e.kind).unwrap_or(PushKind::Test),
        kinds: events.iter().map(|e| e.kind).collect(),
        tag,
        renotify: attention,
        actionable,
        pending_ids,
        session_id: (!test).then(|| session_id.to_string()),
        urgency,
        ttl_secs,
    }
}

/// The JSON the service worker receives (encrypted end to end).
pub fn payload_json(n: &Notification, ts: i64) -> Value {
    let mut v = json!({
        "v": 1,
        "title": n.title,
        "body": n.body,
        "kind": n.kind,
        "kinds": n.kinds,
        "tag": n.tag,
        "renotify": n.renotify,
        "actionable": n.actionable,
        "pendingIds": n.pending_ids,
        "count": n.kinds.len(),
        "ts": ts,
    });
    if let Some(sid) = &n.session_id {
        v["sessionId"] = json!(sid);
    }
    if let [pid] = n.pending_ids.as_slice() {
        v["pendingId"] = json!(pid);
    }
    v
}

/// The RFC 8030 `Topic` for a session: 32 URL-safe chars, stable, and not the
/// session id itself (the push service sees topics in the clear). A newer
/// message with the same topic replaces an undelivered older one.
pub fn topic_for(session_id: &str) -> String {
    let digest = Sha256::digest(format!("mother-claude-topic:{session_id}").as_bytes());
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Why a request could not be built (the subscription is unusable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError(pub String);

/// Encrypt `body` for `sub` (RFC 8291 aes128gcm) and sign the VAPID JWT
/// (RFC 8292, `vapid t=…, k=…`).
pub fn build_request(
    keys: &VapidKeys,
    contact: &str,
    sub: &Subscription,
    body: Vec<u8>,
    ttl_secs: u64,
    urgency: Urgency,
    topic: Option<&str>,
) -> Result<Request<Vec<u8>>, BuildError> {
    let uri: Uri = sub
        .endpoint
        .parse()
        .map_err(|_| BuildError("endpoint is not a valid URI".into()))?;
    let point = decode_b64(&sub.p256dh).ok_or_else(|| BuildError("bad p256dh".into()))?;
    let ua_public = web_push_native::p256::PublicKey::from_sec1_bytes(&point)
        .map_err(|_| BuildError("bad p256dh".into()))?;
    let secret = decode_b64(&sub.auth).ok_or_else(|| BuildError("bad auth".into()))?;
    if secret.len() != 16 {
        // `Auth::clone_from_slice` would panic on any other length.
        return Err(BuildError("bad auth".into()));
    }
    let ua_auth = Auth::clone_from_slice(&secret);
    let mut req = WebPushBuilder::new(uri, ua_public, ua_auth)
        .with_valid_duration(JWT_VALIDITY)
        .with_vapid(keys.key_pair(), contact)
        .build(body)
        .map_err(|e| BuildError(format!("could not encrypt: {e}")))?;
    let headers = req.headers_mut();
    headers.insert("TTL", HeaderValue::from(ttl_secs));
    headers.insert("Urgency", HeaderValue::from_static(urgency.as_str()));
    if let Some(topic) = topic.and_then(|t| HeaderValue::from_str(t).ok()) {
        headers.insert("Topic", topic);
    }
    // reqwest computes Content-Length from the body.
    headers.remove(axum::http::header::CONTENT_LENGTH);
    Ok(req)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use web_push_native::jwt_simple::algorithms::ES256KeyPair;
    use web_push_native::jwt_simple::prelude::*;
    use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint as _;

    fn ev(kind: PushKind) -> PushEvent {
        PushEvent::new("s1", kind, format!("{kind:?}"))
    }

    #[test]
    fn default_text_carries_no_details() {
        let mut p = ev(PushKind::Permission);
        p.tool = Some("Bash".into());
        p.detail = Some("curl https://secret.example/token=abc".into());
        p.pending_id = Some("pid1".into());
        let n = render(Some("my-repo"), "s1", std::slice::from_ref(&p), false);
        assert_eq!(n.title, "my-repo");
        assert_eq!(n.body, "Needs approval · Bash");
        assert_eq!(n.tag, "pid1");
        assert!(n.renotify);
        assert!(n.actionable);
        let mut risky = p.clone();
        risky.dangerous = true;
        assert!(!render(None, "s1", &[risky], false).actionable);
        assert_eq!(n.urgency, Urgency::High);
        let json = payload_json(&n, 5).to_string();
        assert!(!json.contains("secret"), "{json}");
        assert!(!json.contains("http"), "no URL in the payload: {json}");
        // Opted in: the detail is included.
        let n = render(Some("my-repo"), "s1", std::slice::from_ref(&p), true);
        assert!(
            n.body.starts_with("Needs approval · Bash: curl"),
            "{}",
            n.body
        );

        let mut mcp = ev(PushKind::Permission);
        mcp.tool = Some("mcp__github__create_issue".into());
        assert_eq!(
            render(None, "s1", &[mcp], false).body,
            "Needs approval · github · create_issue"
        );

        let mut fin = ev(PushKind::Finished);
        fin.worked_ms = Some(125_000);
        fin.detail = Some("All 42 tests pass".into());
        let n = render(Some("x"), "s1", std::slice::from_ref(&fin), false);
        assert_eq!(n.body, "Finished after 2 min");
        assert_eq!(n.urgency, Urgency::Normal);
        assert_eq!(n.tag, "s1");
        assert!(!n.renotify);
        let n = render(Some("x"), "s1", &[fin], true);
        assert_eq!(n.body, "Finished after 2 min · All 42 tests pass");

        let mut failed = ev(PushKind::Failed);
        failed.label = Some("api_error".into());
        failed.detail = Some("request id req_123 overloaded".into());
        assert_eq!(
            render(None, "s1", &[failed.clone()], false).body,
            "Stopped on an error · api error"
        );
        // Labels that look like free text are dropped, not shown.
        failed.label = Some("Error: see /Users/me/secret.txt!".into());
        assert_eq!(
            render(None, "s1", &[failed.clone()], false).body,
            "Stopped on an error"
        );
        failed.label = Some("see/Users".into());
        assert_eq!(
            render(None, "s1", &[failed], false).body,
            "Stopped on an error"
        );

        let mut claude = ev(PushKind::Claude);
        claude.detail = Some("Deploy finished".into());
        assert_eq!(
            render(None, "s1", std::slice::from_ref(&claude), false).body,
            "Claude sent a notification"
        );
        assert_eq!(render(None, "s1", &[claude], true).body, "Deploy finished");
    }

    #[test]
    fn batches_and_cleaning() {
        let mut a = ev(PushKind::Permission);
        a.tool = Some("Edit".into());
        a.pending_id = Some("p1".into());
        let mut b = ev(PushKind::Question);
        b.pending_id = Some("p2".into());
        let n = render(Some("repo"), "s1", &[a, b], true);
        assert_eq!(n.body, "2 updates · Needs approval · Edit, Has a question");
        assert_eq!(n.tag, "s1");
        assert_eq!(n.pending_ids, vec!["p1", "p2"]);
        let v = payload_json(&n, 9);
        assert_eq!(v["sessionId"], "s1");
        assert!(v.get("pendingId").is_none());
        assert_eq!(v["count"], 2);

        assert_eq!(clean("a\n\tb\u{1b}[31m  c", 50), "a b [31m c");
        assert_eq!(clean("\u{202E}evil\u{200B}", 50), "evil");
        let long = clean(&"x".repeat(300), 10);
        assert_eq!(long.chars().count(), 10);
        assert!(long.ends_with('…'));
        let title = render(Some(&"n".repeat(200)), "s1", &[ev(PushKind::Idle)], false).title;
        assert!(title.chars().count() <= TITLE_MAX);

        let t = render(Some("repo"), "s1", &[ev(PushKind::Test)], false);
        assert_eq!(t.title, "Mother Claude");
        assert!(payload_json(&t, 1).get("sessionId").is_none());
        assert_eq!(topic_for("s1").len(), 32);
        assert_eq!(topic_for("s1"), topic_for("s1"));
        assert_ne!(topic_for("s1"), topic_for("s2"));
        assert!(topic_for("s1").bytes().all(|b| b.is_ascii_hexdigit()));
    }

    /// A browser-side keypair + auth secret.
    pub(crate) fn user_agent() -> (web_push_native::p256::SecretKey, Subscription) {
        let kp = ES256KeyPair::generate();
        let sk = web_push_native::p256::SecretKey::from_slice(&kp.to_bytes()).unwrap();
        let point = sk.public_key().to_encoded_point(false);
        let sub = Subscription {
            endpoint: "https://fcm.googleapis.com/fcm/send/abc".into(),
            p256dh: URL_SAFE_NO_PAD.encode(point.as_bytes()),
            auth: URL_SAFE_NO_PAD.encode([9u8; 16]),
            ..Default::default()
        };
        (sk, sub)
    }

    #[test]
    fn request_round_trip() {
        let keys = VapidKeys::generate();
        let (ua_secret, sub) = user_agent();
        let body = br#"{"v":1,"title":"t"}"#.to_vec();
        let req = build_request(
            &keys,
            "mailto:mother-claude@users.noreply.github.com",
            &sub,
            body.clone(),
            3600,
            Urgency::High,
            Some(&topic_for("s1")),
        )
        .unwrap();
        let h = req.headers();
        assert_eq!(req.method(), "POST");
        assert_eq!(h["content-encoding"], "aes128gcm");
        assert_eq!(h["content-type"], "application/octet-stream");
        assert_eq!(h["ttl"], "3600");
        assert_eq!(h["urgency"], "high");
        assert_eq!(h["topic"], topic_for("s1").as_str());
        assert!(h.get("content-length").is_none());
        // Authorization: vapid t=<jwt>, k=<our public key>.
        let auth = h["authorization"].to_str().unwrap().to_string();
        let rest = auth.strip_prefix("vapid t=").unwrap();
        let (jwt, k) = rest.split_once(", k=").unwrap();
        assert_eq!(k, keys.public_key());
        let pk = ES256PublicKey::from_bytes(&URL_SAFE_NO_PAD.decode(k).unwrap()).unwrap();
        let claims = pk.verify_token::<NoCustomClaims>(jwt, None).unwrap();
        assert_eq!(
            claims.subject.as_deref(),
            Some("mailto:mother-claude@users.noreply.github.com")
        );
        assert_eq!(
            claims.audiences.unwrap().into_string().unwrap(),
            "https://fcm.googleapis.com"
        );
        let exp = claims.expires_at.unwrap().as_secs();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(exp > now && exp <= now + 24 * 3600, "exp within 24 h");
        // The browser can decrypt it.
        let plain = web_push_native::decrypt(
            req.body().clone(),
            &ua_secret,
            &Auth::clone_from_slice(&[9u8; 16]),
        )
        .unwrap();
        assert_eq!(plain, body);
        // A different auth secret cannot.
        assert!(web_push_native::decrypt(
            req.body().clone(),
            &ua_secret,
            &Auth::clone_from_slice(&[1u8; 16]),
        )
        .is_err());
    }

    #[test]
    fn unusable_subscriptions_are_errors_not_panics() {
        let keys = VapidKeys::generate();
        let (_, good) = user_agent();
        for bad in [
            Subscription {
                auth: URL_SAFE_NO_PAD.encode([1u8; 15]),
                ..good.clone()
            },
            Subscription {
                p256dh: "AAAA".into(),
                ..good.clone()
            },
            Subscription {
                endpoint: "not a uri with spaces".into(),
                ..good.clone()
            },
        ] {
            assert!(build_request(
                &keys,
                "mailto:a@b.c",
                &bad,
                vec![1],
                60,
                Urgency::Normal,
                None
            )
            .is_err());
        }
    }
}
