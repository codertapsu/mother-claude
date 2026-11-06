//! What the desktop knows about each tab of a lease: its URL and title, the
//! `ref`s of its last snapshot (role, name, whether the field is secret) and
//! an open JavaScript dialog. Learned from results (`snapshot`, `find`,
//! `click` deltas, `tabs.*`, `navigate`) and events (`tab.updated`,
//! `dialog.opened`); used by the danger classifier (a form with password or
//! card fields; typing into a secret field) and to check a page's site mode
//! before a command is sent. A ref the desktop never saw is unknown, never
//! assumed harmless for typing: the extension's own `refuseSecret` check
//! covers it (docs/BROWSER-PROTOCOL.md).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::redact::{REDACTED, SECRET_NAME};

/// The key of the lease's active tab while its id is unknown.
pub const ACTIVE: i64 = -1;

/// Refs kept per tab (a snapshot is capped by `maxNodes` anyway).
const MAX_REFS: usize = 8000;

static LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*-\s+(?P<role>[A-Za-z][A-Za-z0-9_-]*)(?:\s+"(?P<name>(?:[^"\\]|\\.)*)")?(?P<rest>.*)$"#)
        .unwrap_or_else(|e| panic!("bad snapshot line pattern: {e}"))
});

static REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[ref=(?P<ref>[A-Za-z0-9_-]{1,32})\]")
        .unwrap_or_else(|e| panic!("bad ref pattern: {e}"))
});

/// One element a model can name by `ref`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefInfo {
    pub role: String,
    pub name: String,
    /// A password, card, one-time-code or other secret field.
    pub secret: bool,
}

impl RefInfo {
    /// Clicking it may submit a form (a button, or anything not a link).
    pub fn may_submit(&self) -> bool {
        !matches!(
            self.role.as_str(),
            "link"
                | "tab"
                | "menuitem"
                | "option"
                | "checkbox"
                | "radio"
                | "switch"
                | "textbox"
                | "searchbox"
                | "combobox"
                | "heading"
                | "img"
                | "listitem"
        )
    }
}

/// A JavaScript dialog blocking a tab.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dialog {
    pub kind: String,
    pub message: String,
}

/// One tab.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TabInfo {
    pub url: Option<String>,
    pub title: Option<String>,
    pub refs: HashMap<String, RefInfo>,
    /// The last snapshot showed a secret field (with or without a ref).
    pub secret_fields: bool,
    pub dialog: Option<Dialog>,
}

fn secret_line(name: &str, rest: &str) -> bool {
    rest.contains(&format!("value={REDACTED}")) || SECRET_NAME.is_match(name)
}

impl TabInfo {
    /// A new page: everything learned about the old one goes.
    pub fn navigated(&mut self, url: Option<String>, title: Option<String>) {
        if url.is_some() && url != self.url {
            self.refs.clear();
            self.secret_fields = false;
            self.dialog = None;
        }
        if url.is_some() {
            self.url = url;
        }
        if title.is_some() {
            self.title = title;
        }
    }

    /// Learn a snapshot (`full`: it replaces what the last one said; a
    /// delta after a click adds to it).
    pub fn learn_snapshot(&mut self, tree: &str, full: bool) {
        if full {
            self.refs.clear();
            self.secret_fields = false;
        }
        for line in tree.lines() {
            let Some(c) = LINE.captures(line) else {
                continue;
            };
            let role = c.name("role").map_or("", |m| m.as_str());
            let name = c.name("name").map_or("", |m| m.as_str());
            let rest = c.name("rest").map_or("", |m| m.as_str());
            let secret = secret_line(name, rest) && line.contains("value=");
            self.secret_fields |= secret;
            if let Some(r) = REF.captures(rest) {
                if self.refs.len() < MAX_REFS {
                    self.refs.insert(
                        r["ref"].to_string(),
                        RefInfo {
                            role: role.to_string(),
                            name: name.chars().take(120).collect(),
                            secret,
                        },
                    );
                }
            }
        }
    }

    /// Learn `find` matches (`[{ref, role, name, text}]`).
    pub fn learn_matches(&mut self, matches: &[Value]) {
        for m in matches {
            let Some(r) = m.get("ref").and_then(Value::as_str) else {
                continue;
            };
            if r.is_empty() || r.len() > 32 || self.refs.len() >= MAX_REFS {
                continue;
            }
            let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
            let name = m.get("name").and_then(Value::as_str).unwrap_or_default();
            let known = self.refs.get(r).is_some_and(|i| i.secret);
            let secret = known || (is_field(role) && SECRET_NAME.is_match(name));
            self.secret_fields |= secret;
            self.refs.insert(
                r.to_string(),
                RefInfo {
                    role: role.to_string(),
                    name: name.chars().take(120).collect(),
                    secret,
                },
            );
        }
    }

    pub fn get(&self, r: &str) -> Option<&RefInfo> {
        self.refs.get(r)
    }
}

/// Roles that hold a typed value.
fn is_field(role: &str) -> bool {
    matches!(role, "textbox" | "searchbox" | "combobox" | "spinbutton")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = r#"- banner
  - link "Mother Claude" [ref=e1]
- main
  - heading "Sign in" [level=1]
  - textbox "Username" [ref=e3] value=""
  - textbox "Password" [ref=e4] value=«redacted»
  - button "Sign in" [ref=e5]
  - iframe "Captcha" (cross-origin, not read)"#;

    #[test]
    fn snapshots_name_refs_and_secret_fields() {
        let mut t = TabInfo::default();
        t.learn_snapshot(TREE, true);
        assert!(t.secret_fields);
        assert_eq!(t.refs.len(), 4);
        assert_eq!(
            t.get("e1"),
            Some(&RefInfo {
                role: "link".into(),
                name: "Mother Claude".into(),
                secret: false
            })
        );
        assert!(t.get("e4").unwrap().secret);
        assert!(!t.get("e3").unwrap().secret);
        assert!(t.get("e5").unwrap().may_submit());
        assert!(!t.get("e1").unwrap().may_submit());
        // A delta adds; a full snapshot replaces.
        t.learn_snapshot("  - button \"Next\" [ref=e9]", false);
        assert_eq!(t.refs.len(), 5);
        t.learn_snapshot("- main\n  - link \"Home\" [ref=e1]", true);
        assert_eq!(t.refs.len(), 1);
        assert!(!t.secret_fields);
    }

    #[test]
    fn secret_names_count_even_without_the_marker() {
        let mut t = TabInfo::default();
        t.learn_snapshot("  - textbox \"Card number\" [ref=e7] value=\"\"", true);
        assert!(t.get("e7").unwrap().secret);
        // A button named like a secret is not a field.
        t.learn_snapshot("  - button \"Forgot password?\" [ref=e8]", true);
        assert!(!t.get("e8").unwrap().secret);
        assert!(!t.secret_fields);
    }

    #[test]
    fn find_matches_and_navigation() {
        let mut t = TabInfo::default();
        t.navigated(Some("https://x.test/login".into()), Some("Login".into()));
        t.learn_matches(&[
            serde_json::json!({"ref": "e2", "role": "textbox", "name": "One-time code"}),
            serde_json::json!({"ref": "e3", "role": "button", "name": "Verify"}),
            serde_json::json!({"role": "button"}),
        ]);
        assert!(t.get("e2").unwrap().secret);
        assert!(t.secret_fields);
        assert_eq!(t.refs.len(), 2);
        // Same URL: kept. Another URL: refs go stale.
        t.navigated(Some("https://x.test/login".into()), None);
        assert_eq!(t.refs.len(), 2);
        t.dialog = Some(Dialog::default());
        t.navigated(Some("https://x.test/home".into()), Some("Home".into()));
        assert!(t.refs.is_empty());
        assert!(t.dialog.is_none());
        assert_eq!(t.title.as_deref(), Some("Home"));
    }
}
