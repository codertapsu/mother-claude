//! Server-side redaction of what the extension reads, before it reaches a
//! model (docs/BROWSER-PROTOCOL.md "Limits and rules": the extension redacts,
//! the desktop redacts again).
//!
//! * Snapshot lines: the `value=` of a field whose role or accessible name
//!   says secret (password, card number, security code, one-time code, SSN,
//!   API key, secret) becomes `«redacted»`, whatever the extension sent.
//! * Every text (snapshots, page text, markdown, HTML, console lines, `find`
//!   texts, `eval` values): private keys, well-known token formats (Anthropic,
//!   OpenAI, GitHub, Slack, AWS, Google, GitLab, npm, Mother Claude's own),
//!   JWTs, bearer credentials and `password=…`-style pairs.
//! * HTML: the `value` attribute of password inputs and of inputs named like
//!   a secret.
//! * JSON values: strings as above; a value under a key named like a secret
//!   (`password`, `token`, `apiKey`, `cookie`, `authorization`, …); an object
//!   marked `"redacted": true` keeps no `value`.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::Value;

/// What replaces a secret.
pub const REDACTED: &str = "«redacted»";

fn re(pattern: &str) -> Regex {
    // The patterns are constants, tested below.
    Regex::new(pattern).unwrap_or_else(|e| panic!("bad redaction pattern {pattern}: {e}"))
}

/// An accessible name or attribute that says "secret field".
pub static SECRET_NAME: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)(passw|passcode|passphrase|\bpwd\b|\bpin\b|\bcvc\b|\bcvv\b|\bcsc\b|card ?number|cc-number|cc-csc|security code|one[- ]?time|\botp\b|verification code|\bssn\b|social security|api[ _-]?key|secret)",
    )
});

static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    re(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?(-----END [A-Z0-9 ]*PRIVATE KEY-----|$)")
});

static TOKENS: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"\b(?:sk-ant-[A-Za-z0-9_\-]{16,}",
        r"|sk-(?:proj-)?[A-Za-z0-9_\-]{20,}",
        r"|gh[pousr]_[A-Za-z0-9]{20,}",
        r"|github_pat_[A-Za-z0-9_]{20,}",
        r"|xox[abprs]-[A-Za-z0-9\-]{10,}",
        r"|AKIA[0-9A-Z]{16}",
        r"|AIza[0-9A-Za-z_\-]{35}",
        r"|glpat-[A-Za-z0-9_\-]{20,}",
        r"|npm_[A-Za-z0-9]{36}",
        r"|mc[dxhs]_[A-Za-z0-9_\-]{20,}",
        r"|eyJ[A-Za-z0-9_\-]{8,}\.eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,})"
    ))
});

static BEARER: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._\-~+/]{16,}=*"));

static PAIRS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"(?i)\b(password|passwd|pwd|passcode|secret|api[_-]?key|access[_-]?token|auth[_-]?token|refresh[_-]?token|client[_-]?secret)(\s*["']?\s*[:=]\s*)("[^"\s]{3,}"|'[^'\s]{3,}'|[^\s"'&,;<>]{3,})"#,
    )
});

static SNAPSHOT_VALUE: LazyLock<Regex> = LazyLock::new(|| re(r#"value=(?:"(?:[^"\\]|\\.)*"|\S+)"#));

static HTML_INPUT: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<input\b[^>]*>"));

static HTML_VALUE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?is)\bvalue\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)"#));

static HTML_SECRET_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"(?is)\b(type\s*=\s*["']?password|(name|id|autocomplete|aria-label|placeholder)\s*=\s*["'][^"']*(passw|passcode|cc-number|cc-csc|card.?number|cvc|cvv|one-time-code|otp|ssn|api.?key|secret|token))"#,
    )
});

/// JSON keys whose string values are secrets.
fn secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase().replace(['_', '-'], "");
    [
        "password",
        "passwd",
        "passcode",
        "secret",
        "clientsecret",
        "token",
        "accesstoken",
        "refreshtoken",
        "authtoken",
        "idtoken",
        "apikey",
        "authorization",
        "cookie",
        "cookies",
        "setcookie",
        "sessionid",
        "ssn",
        "cvc",
        "cvv",
        "cardnumber",
        "ccnumber",
    ]
    .contains(&k.as_str())
}

/// Secrets in free text: private keys, tokens, JWTs, bearer credentials and
/// `password=…` pairs.
pub fn text(input: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(input);
    if PRIVATE_KEY.is_match(&out) {
        out = Cow::Owned(PRIVATE_KEY.replace_all(&out, REDACTED).into_owned());
    }
    if TOKENS.is_match(&out) {
        out = Cow::Owned(TOKENS.replace_all(&out, REDACTED).into_owned());
    }
    if BEARER.is_match(&out) {
        out = Cow::Owned(
            BEARER
                .replace_all(&out, |c: &Captures| format!("{} {REDACTED}", &c[1]))
                .into_owned(),
        );
    }
    if PAIRS.is_match(&out) {
        out = Cow::Owned(
            PAIRS
                .replace_all(&out, |c: &Captures| format!("{}{}{REDACTED}", &c[1], &c[2]))
                .into_owned(),
        );
    }
    out
}

/// A snapshot tree: secret fields' values, then [`text`].
pub fn snapshot(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for (i, line) in input.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.contains("value=") && SECRET_NAME.is_match(line) {
            out.push_str(&SNAPSHOT_VALUE.replace_all(line, format!("value={REDACTED}")));
        } else {
            out.push_str(line);
        }
    }
    text(&out).into_owned()
}

/// HTML: values of password and secret-named inputs, then [`text`].
pub fn html(input: &str) -> String {
    let inputs = HTML_INPUT.replace_all(input, |c: &Captures| {
        let tag = &c[0];
        if HTML_SECRET_ATTR.is_match(tag) {
            HTML_VALUE
                .replace_all(tag, format!("value=\"{REDACTED}\""))
                .into_owned()
        } else {
            tag.to_string()
        }
    });
    text(&inputs).into_owned()
}

/// An HTTP header's value as a model may see it: the whole value of a header
/// that carries credentials (`Authorization`, `Cookie`, `Set-Cookie`, API
/// keys, CSRF and session tokens, anything named like a token, key, secret
/// or password); any other through [`text`].
pub fn header(name: &str, value: &str) -> String {
    let n = name.to_ascii_lowercase();
    let credential = matches!(
        n.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "x-api-key"
    ) || [
        "token", "secret", "password", "api-key", "apikey", "session", "csrf", "xsrf", "auth",
    ]
    .iter()
    .any(|w| n.contains(w));
    if credential && !value.is_empty() {
        REDACTED.to_string()
    } else {
        text(value).into_owned()
    }
}

/// A JSON value from the page: every string through [`text`], secret-named
/// keys' values and `redacted: true` objects' values replaced.
pub fn value(v: &mut Value) {
    match v {
        Value::String(s) => {
            if let Cow::Owned(clean) = text(s) {
                *s = clean;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(value),
        Value::Object(map) => {
            let marked = map.get("redacted").and_then(Value::as_bool) == Some(true);
            for (k, item) in map.iter_mut() {
                let secret = (marked && k == "value") || (secret_key(k) && !item.is_null());
                if secret && (item.is_string() || item.is_number() || marked) {
                    *item = Value::from(REDACTED);
                    continue;
                }
                value(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_values_of_secret_fields_are_redacted() {
        let tree = "- main\n  - textbox \"Title\" [ref=e3] value=\"Fix the bug\"\n  - textbox \
                    \"Password\" [ref=e4] value=\"hunter2hunter2\"\n  - textbox \"Card number\" \
                    [ref=e5] value=\"4242 4242 4242 4242\"\n  - textbox \"One-time code\" [ref=e6] \
                    value=123456\n  - textbox \"Email\" [ref=e7] value=«redacted»";
        let out = snapshot(tree);
        assert!(out.contains("\"Title\" [ref=e3] value=\"Fix the bug\""));
        assert!(!out.contains("hunter2"));
        assert!(!out.contains("4242"));
        assert!(!out.contains("123456"));
        assert!(out.contains("\"Password\" [ref=e4] value=«redacted»"));
        assert!(
            out.contains("[ref=e7] value=«redacted»"),
            "kept as the extension sent it"
        );
        assert_eq!(out.lines().count(), tree.lines().count());
    }

    #[test]
    fn tokens_keys_and_pairs_are_redacted_in_text() {
        let cases = [
            "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "AKIAABCDEFGHIJKLMNOP",
            "token mcx_abcdefghijklmnopqrstuvwxyz012345",
            "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijk",
            "Authorization: Bearer abcdefghijklmnopqrstuvwxyz123456",
            "password=hunter2&user=me",
            "{\"api_key\": \"abc123def456\"}",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----",
        ];
        for case in cases {
            let out = text(case);
            assert!(out.contains(REDACTED), "{case} → {out}");
        }
        assert!(!text("password=hunter2&user=me").contains("hunter2"));
        assert!(text("password=hunter2&user=me").contains("user=me"));
        // Ordinary text is untouched (and not copied).
        let plain = "The issue was opened 3 days ago by marcus.";
        assert!(matches!(text(plain), Cow::Borrowed(_)));
    }

    #[test]
    fn credential_headers_are_redacted_whole() {
        for name in [
            "Authorization",
            "Cookie",
            "set-cookie",
            "X-API-Key",
            "X-CSRF-Token",
            "x-amz-security-token",
            "X-Session-Id",
        ] {
            assert_eq!(header(name, "anything at all"), REDACTED, "{name}");
        }
        assert_eq!(
            header("Content-Type", "application/json"),
            "application/json"
        );
        assert_eq!(header("Cookie", ""), "", "nothing to hide");
        // Any other header still loses a token inside it.
        assert!(header(
            "Link",
            "<https://x/?t=ghp_abcdefghijklmnopqrstuvwxyz0123456789>"
        )
        .contains(REDACTED));
    }

    #[test]
    fn html_inputs_lose_secret_values() {
        let page = r#"<form><input type="text" name="user" value="marcus">
            <input type="password" name="pw" value="hunter2">
            <input name="cc-number" autocomplete="cc-number" value='4242424242424242'>
            <input name="q" value="rust"></form>"#;
        let out = html(page);
        assert!(out.contains(r#"value="marcus""#));
        assert!(out.contains(r#"value="rust""#));
        assert!(!out.contains("hunter2"));
        assert!(!out.contains("4242424242424242"));
    }

    #[test]
    fn json_values_lose_secrets() {
        let mut v = serde_json::json!({
            "user": "marcus",
            "password": "hunter2",
            "nested": {"apiKey": "abc", "list": ["ghp_abcdefghijklmnopqrstuvwxyz0123456789"]},
            "field": {"redacted": true, "value": "x"},
            "cookie": "sid=1",
            "token": null,
        });
        value(&mut v);
        assert_eq!(v["user"], "marcus");
        assert_eq!(v["password"], REDACTED);
        assert_eq!(v["nested"]["apiKey"], REDACTED);
        assert_eq!(v["nested"]["list"][0], REDACTED);
        assert_eq!(v["field"]["value"], REDACTED);
        assert_eq!(v["cookie"], REDACTED);
        assert!(v["token"].is_null());
    }
}
