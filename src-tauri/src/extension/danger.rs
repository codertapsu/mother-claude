//! Which browser actions need a person's approval (docs/BROWSER.md
//! "Approvals"; plan §10). Decided here, on the desktop, for every command,
//! whatever the model asks and whatever the extension would allow:
//!
//! * navigating to a sensitive site (`navigate`, `tabs.open` with its URL);
//! * any action on a sensitive site (every `act`-class command);
//! * filling or submitting a form on a page with password, card or
//!   one-time-code fields (typing into such a field itself is refused
//!   outright, see [`secret_target`]);
//! * `eval`; `upload`;
//! * more than the per-minute limit of actions on one site (a runaway loop);
//! * any action while the user pressed Stop for another session in the same
//!   browser less than a minute ago.
//!
//! A standing grant ("always allow <kind> on <origin>") covers its kind on
//! that exact origin; the Stop rule has no grant.

use serde::Serialize;
use serde_json::Value;

use super::page::TabInfo;
use super::sites::{GrantAction, Settings, SiteMode};

/// What an action needs of the site's mode, and whether it counts as acting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// No site involved (`tabs.list`, `tabs.select`, `tabs.close`).
    Browser,
    /// Loads a page (`navigate`, `tabs.open` with a URL): the target needs `read`.
    Navigate,
    /// Moves within history (`back`, `forward`, `reload`): the page needs `read`.
    History,
    /// Observes the page: it needs `read`.
    Observe,
    /// Acts on the page: it needs `act`.
    Act,
}

/// The class of a protocol action.
pub fn class_of(action: &str) -> Class {
    match action {
        "tabs.list" | "tabs.select" | "tabs.close" | "highlight" => Class::Browser,
        "navigate" | "tabs.open" => Class::Navigate,
        "back" | "forward" | "reload" => Class::History,
        "snapshot" | "read" | "find" | "wait" | "screenshot" | "console" | "scroll" => {
            Class::Observe
        }
        _ => Class::Act,
    }
}

impl Class {
    /// The site mode this class needs.
    pub fn needs(self) -> Option<SiteMode> {
        match self {
            Class::Browser => None,
            Class::Navigate | Class::History | Class::Observe => Some(SiteMode::Read),
            Class::Act => Some(SiteMode::Act),
        }
    }

    /// Counted against the per-minute limit.
    pub fn counts(self) -> bool {
        matches!(self, Class::Navigate | Class::Act)
    }
}

/// Why an action needs approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DangerKind {
    Eval,
    Upload,
    Navigate,
    Act,
    SecretForm,
    Burst,
    AfterStop,
}

impl DangerKind {
    /// The grant that covers it.
    pub fn grant(self) -> Option<GrantAction> {
        match self {
            DangerKind::Eval => Some(GrantAction::Eval),
            DangerKind::Upload => Some(GrantAction::Upload),
            DangerKind::Navigate => Some(GrantAction::Navigate),
            DangerKind::Act => Some(GrantAction::Act),
            DangerKind::SecretForm => Some(GrantAction::SecretForm),
            DangerKind::Burst => Some(GrantAction::Burst),
            DangerKind::AfterStop => None,
        }
    }
}

/// One reason, on one origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Danger {
    pub kind: DangerKind,
    pub origin: String,
    /// A sentence for the card.
    pub text: String,
}

/// What the classifier looks at.
#[derive(Debug, Clone, Default)]
pub struct Facts<'a> {
    pub action: &'a str,
    pub params: Option<&'a Value>,
    /// The page acted on (its origin), when known.
    pub page_origin: Option<&'a str>,
    /// The page's sensitive category.
    pub page_sensitive: Option<&'static str>,
    /// What the desktop knows about the page.
    pub tab: Option<&'a TabInfo>,
    /// `navigate` / `tabs.open`: where to.
    pub target_origin: Option<&'a str>,
    pub target_sensitive: Option<&'static str>,
    /// This session already reached the per-minute limit on the page's site.
    pub over_limit: bool,
    pub limit: u32,
    /// Another session's lease in this browser was stopped < 60 s ago.
    pub after_stop: bool,
}

fn param<'a>(f: &Facts<'a>, key: &str) -> Option<&'a Value> {
    f.params.and_then(|p| p.get(key))
}

/// The refs an action types into (`type`'s `ref`, `fill`'s `fields[].ref`).
pub fn typed_refs<'a>(action: &str, params: &'a Value) -> Vec<&'a str> {
    match action {
        "type" => params
            .get("ref")
            .and_then(Value::as_str)
            .into_iter()
            .collect(),
        "fill" => params
            .get("fields")
            .and_then(Value::as_array)
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|f| f.get("ref").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A secret field the action would type into (refused outright: secrets
/// never come from a model). `Some((ref, field name))`.
pub fn secret_target(
    action: &str,
    params: &Value,
    tab: Option<&TabInfo>,
) -> Option<(String, String)> {
    let tab = tab?;
    typed_refs(action, params).into_iter().find_map(|r| {
        tab.get(r)
            .filter(|info| info.secret)
            .map(|info| (r.to_string(), info.name.clone()))
    })
}

/// Whether the action submits a form: `submit: true`, Enter, or a click on
/// something that is not a link or a field.
fn submits(f: &Facts) -> bool {
    match f.action {
        "type" | "fill" => param(f, "submit").and_then(Value::as_bool) == Some(true),
        "press" => param(f, "key")
            .and_then(Value::as_str)
            .is_some_and(|k| k.eq_ignore_ascii_case("enter")),
        "click" | "dblclick" => match param(f, "ref").and_then(Value::as_str) {
            Some(r) => f
                .tab
                .and_then(|t| t.get(r))
                .is_none_or(|info| info.may_submit()),
            // A selector: unknown target.
            None => true,
        },
        _ => false,
    }
}

/// Every reason the action needs approval (before grants).
pub fn classify(f: &Facts) -> Vec<Danger> {
    let class = class_of(f.action);
    let page = f.page_origin.unwrap_or_default().to_string();
    let mut out = Vec::new();
    let mut push = |kind, origin: &str, text: String| {
        out.push(Danger {
            kind,
            origin: origin.to_string(),
            text,
        })
    };
    if f.after_stop {
        push(
            DangerKind::AfterStop,
            &page,
            "You pressed Stop in this browser less than a minute ago.".into(),
        );
    }
    match f.action {
        "eval" => push(
            DangerKind::Eval,
            &page,
            "It runs a script in the page, which can read anything the page can.".into(),
        ),
        "upload" => push(
            DangerKind::Upload,
            &page,
            "It uploads files from this computer.".into(),
        ),
        _ => {}
    }
    if class == Class::Navigate {
        if let (Some(target), Some(category)) = (f.target_origin, f.target_sensitive) {
            push(
                DangerKind::Navigate,
                target,
                format!("{target} is a sensitive site ({category})."),
            );
        }
    }
    if class == Class::Act {
        if let Some(category) = f.page_sensitive {
            push(
                DangerKind::Act,
                &page,
                format!("{page} is a sensitive site ({category})."),
            );
        }
        let secret_form = f.tab.is_some_and(|t| t.secret_fields);
        if secret_form && (matches!(f.action, "type" | "fill") || submits(f)) {
            push(
                DangerKind::SecretForm,
                &page,
                if submits(f) {
                    "It submits a form that has password, card or one-time-code fields.".into()
                } else {
                    "It fills a form that has password, card or one-time-code fields.".into()
                },
            );
        }
    }
    if class.counts() && f.over_limit {
        let origin = if class == Class::Navigate {
            f.target_origin.unwrap_or(&page).to_string()
        } else {
            page.clone()
        };
        push(
            DangerKind::Burst,
            &origin,
            format!(
                "More than {} actions on this site in the last minute.",
                f.limit
            ),
        );
    }
    out
}

/// The reasons no standing grant covers.
pub fn uncovered(dangers: Vec<Danger>, settings: &Settings) -> Vec<Danger> {
    dangers
        .into_iter()
        .filter(|d| {
            d.kind
                .grant()
                .is_none_or(|g| d.origin.is_empty() || settings.grant_for(g, &d.origin).is_none())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::sites::Grant;
    use serde_json::json;

    fn login_tab() -> TabInfo {
        let mut t = TabInfo::default();
        t.learn_snapshot(
            "- main\n  - textbox \"Username\" [ref=e3] value=\"\"\n  - textbox \"Password\" \
             [ref=e4] value=«redacted»\n  - button \"Sign in\" [ref=e5]\n  - link \"Help\" [ref=e6]",
            true,
        );
        t
    }

    fn kinds(d: &[Danger]) -> Vec<DangerKind> {
        d.iter().map(|d| d.kind).collect()
    }

    #[test]
    fn classes_need_their_modes() {
        assert_eq!(class_of("snapshot").needs(), Some(SiteMode::Read));
        assert_eq!(class_of("scroll").needs(), Some(SiteMode::Read));
        assert_eq!(class_of("navigate").needs(), Some(SiteMode::Read));
        assert_eq!(class_of("click").needs(), Some(SiteMode::Act));
        assert_eq!(class_of("eval").needs(), Some(SiteMode::Act));
        assert_eq!(class_of("upload").needs(), Some(SiteMode::Act));
        assert_eq!(class_of("tabs.list").needs(), None);
        assert!(class_of("click").counts() && class_of("navigate").counts());
        assert!(!class_of("snapshot").counts());
    }

    #[test]
    fn eval_upload_and_harmless_actions() {
        let p = json!({"expression": "1+1"});
        let f = Facts {
            action: "eval",
            params: Some(&p),
            page_origin: Some("http://localhost:3000"),
            ..Default::default()
        };
        assert_eq!(kinds(&classify(&f)), vec![DangerKind::Eval]);
        let f = Facts {
            action: "upload",
            page_origin: Some("https://github.com"),
            ..Default::default()
        };
        assert_eq!(kinds(&classify(&f)), vec![DangerKind::Upload]);
        let p = json!({"ref": "e1"});
        let f = Facts {
            action: "click",
            params: Some(&p),
            page_origin: Some("https://github.com"),
            ..Default::default()
        };
        assert!(classify(&f).is_empty());
        let f = Facts {
            action: "snapshot",
            page_origin: Some("https://github.com"),
            ..Default::default()
        };
        assert!(classify(&f).is_empty());
    }

    #[test]
    fn sensitive_sites_need_approval_to_open_and_to_act() {
        let f = Facts {
            action: "navigate",
            target_origin: Some("https://www.paypal.com"),
            target_sensitive: Some("payment"),
            ..Default::default()
        };
        let d = classify(&f);
        assert_eq!(kinds(&d), vec![DangerKind::Navigate]);
        assert_eq!(d[0].origin, "https://www.paypal.com");
        let p = json!({"direction": "down"});
        let f = Facts {
            action: "scroll",
            params: Some(&p),
            page_origin: Some("https://www.chase.com"),
            page_sensitive: Some("banking"),
            ..Default::default()
        };
        assert!(
            classify(&f).is_empty(),
            "reading a sensitive site in read mode is fine"
        );
        let p = json!({"ref": "e1"});
        let f = Facts {
            action: "hover",
            params: Some(&p),
            page_origin: Some("https://www.chase.com"),
            page_sensitive: Some("banking"),
            ..Default::default()
        };
        assert_eq!(kinds(&classify(&f)), vec![DangerKind::Act]);
    }

    #[test]
    fn forms_with_secret_fields_need_approval_and_secret_fields_are_refused() {
        let tab = login_tab();
        let origin = Some("https://example.com");
        let check = |action: &str, p: Value| {
            let f = Facts {
                action,
                params: Some(&p),
                page_origin: origin,
                tab: Some(&tab),
                ..Default::default()
            };
            kinds(&classify(&f))
        };
        assert_eq!(
            check("type", json!({"ref": "e3", "text": "me"})),
            vec![DangerKind::SecretForm]
        );
        assert_eq!(
            check(
                "fill",
                json!({"fields": [{"ref": "e3", "value": "me"}], "submit": true})
            ),
            vec![DangerKind::SecretForm]
        );
        assert_eq!(
            check("click", json!({"ref": "e5"})),
            vec![DangerKind::SecretForm]
        );
        assert_eq!(
            check("click", json!({"selector": "#go"})),
            vec![DangerKind::SecretForm]
        );
        assert!(
            check("click", json!({"ref": "e6"})).is_empty(),
            "a link submits nothing"
        );
        assert_eq!(
            check("press", json!({"key": "Enter"})),
            vec![DangerKind::SecretForm]
        );
        assert!(check("press", json!({"key": "Tab"})).is_empty());
        // Typing into the password field itself: refused, not approvable.
        let typed = json!({"ref": "e4", "text": "hunter2"});
        assert_eq!(
            secret_target("type", &typed, Some(&tab)),
            Some(("e4".into(), "Password".into()))
        );
        let fill = json!({"fields": [{"ref": "e3", "value": "me"}, {"ref": "e4", "value": "x"}]});
        assert!(secret_target("fill", &fill, Some(&tab)).is_some());
        assert!(secret_target("type", &json!({"ref": "e3"}), Some(&tab)).is_none());
        assert!(secret_target("type", &typed, None).is_none());
    }

    #[test]
    fn bursts_and_stops() {
        let p = json!({"ref": "e1"});
        let f = Facts {
            action: "click",
            params: Some(&p),
            page_origin: Some("https://github.com"),
            over_limit: true,
            limit: 30,
            after_stop: true,
            ..Default::default()
        };
        let d = classify(&f);
        assert_eq!(kinds(&d), vec![DangerKind::AfterStop, DangerKind::Burst]);
        assert!(d[1].text.contains("30"));
        // Observing never counts.
        let f = Facts {
            action: "snapshot",
            page_origin: Some("https://github.com"),
            over_limit: true,
            ..Default::default()
        };
        assert!(classify(&f).is_empty());
    }

    #[test]
    fn grants_cover_their_kind_on_their_origin_only() {
        let settings = Settings {
            grants: vec![Grant {
                id: "g_1".into(),
                action: GrantAction::Eval,
                origin: "http://localhost:3000".into(),
                created_at: 0,
                created_by: "Mac".into(),
            }],
            ..Settings::default()
        };
        let eval_on = |origin: &'static str| {
            classify(&Facts {
                action: "eval",
                page_origin: Some(origin),
                after_stop: true,
                ..Default::default()
            })
        };
        let left = uncovered(eval_on("http://localhost:3000"), &settings);
        assert_eq!(
            kinds(&left),
            vec![DangerKind::AfterStop],
            "Stop has no grant"
        );
        let left = uncovered(eval_on("http://localhost:3001"), &settings);
        assert_eq!(kinds(&left), vec![DangerKind::AfterStop, DangerKind::Eval]);
    }
}
