//! The user's own launch settings — model / effort — as persisted by the
//! Claude Code pickers (VS Code and CLI both write `~/.claude/settings.json`),
//! plus the account's cached extra model options from `~/.claude.json`.
//!
//! Tolerant reads: these are undocumented research-preview internals, so every
//! field is optional and parse failures degrade to defaults.

use serde::Serialize;
use serde_json::Value;

use super::home::ClaudeHome;

/// One selectable model for the launch forms.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What the spawn/continue forms should offer and pre-select.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchDefaults {
    /// The user's current model selection (settings.json `model`), if any.
    pub model: Option<String>,
    /// The user's current effort selection (settings.json `effortLevel`).
    pub effort: Option<String>,
    /// Models to offer: built-in aliases + account extras + the current pick.
    pub models: Vec<ModelOption>,
}

/// settings.json → (model, effortLevel), both optional.
fn parse_settings(text: &str) -> (Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return (None, None);
    };
    let model = v.get("model").and_then(|m| m.as_str()).map(String::from);
    let effort = v
        .get("effortLevel")
        .and_then(|e| e.as_str())
        .map(String::from);
    (model, effort)
}

/// ~/.claude.json `additionalModelOptionsCache` (and `modelAccessCache`) →
/// extra account-specific model options, shaped `{value, label, description}`.
fn parse_model_cache(text: &str) -> Vec<ModelOption> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in ["additionalModelOptionsCache", "modelAccessCache"] {
        let Some(entries) = v.get(key).and_then(|c| c.as_array()) else {
            continue;
        };
        for entry in entries {
            let Some(value) = entry.get("value").and_then(|s| s.as_str()) else {
                continue;
            };
            out.push(ModelOption {
                value: value.to_string(),
                label: entry
                    .get("label")
                    .and_then(|s| s.as_str())
                    .unwrap_or(value)
                    .to_string(),
                description: entry
                    .get("description")
                    .and_then(|s| s.as_str())
                    .map(String::from),
            });
        }
    }
    out
}

fn builtin_models() -> Vec<ModelOption> {
    let opt = |value: &str, label: &str, description: &str| ModelOption {
        value: value.into(),
        label: label.into(),
        description: Some(description.into()),
    };
    vec![
        opt(
            "fable",
            "Fable",
            "Most capable — hardest, longest-running tasks",
        ),
        opt("opus", "Opus", "Highly capable general model"),
        opt("sonnet", "Sonnet", "Balanced speed and capability"),
        opt("haiku", "Haiku", "Fastest and lightest"),
    ]
}

/// Merge built-ins + account cache + the current selection, deduped by value
/// (first occurrence wins, so account labels override nothing built-in).
fn merge_models(current: &Option<String>, cached: Vec<ModelOption>) -> Vec<ModelOption> {
    let mut out = builtin_models();
    for m in cached {
        if !out.iter().any(|o| o.value == m.value) {
            out.push(m);
        }
    }
    if let Some(cur) = current {
        if !out.iter().any(|o| o.value == *cur) {
            out.push(ModelOption {
                value: cur.clone(),
                label: cur.clone(),
                description: Some("Current selection".into()),
            });
        }
    }
    out
}

/// Read the launch defaults, tolerantly, from the user's Claude config.
pub fn read_launch_defaults(home: &ClaudeHome) -> LaunchDefaults {
    let (model, effort) = std::fs::read_to_string(home.user_settings())
        .map(|t| parse_settings(&t))
        .unwrap_or((None, None));
    let cached = std::fs::read_to_string(home.claude_json())
        .map(|t| parse_model_cache(&t))
        .unwrap_or_default();
    let models = merge_models(&model, cached);
    LaunchDefaults {
        model,
        effort,
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_and_effort_from_settings() {
        let (m, e) = parse_settings(r#"{"model":"claude-fable-5[1m]","effortLevel":"xhigh"}"#);
        assert_eq!(m.as_deref(), Some("claude-fable-5[1m]"));
        assert_eq!(e.as_deref(), Some("xhigh"));
        assert_eq!(parse_settings("{}"), (None, None));
        assert_eq!(parse_settings("not json"), (None, None));
    }

    #[test]
    fn parses_account_model_cache() {
        let cached = parse_model_cache(
            r#"{"additionalModelOptionsCache":[{"value":"claude-fable-5[1m]","label":"Fable","description":"1M context"}],"modelAccessCache":[]}"#,
        );
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].value, "claude-fable-5[1m]");
        assert_eq!(cached[0].description.as_deref(), Some("1M context"));
        assert!(parse_model_cache("[]").is_empty());
    }

    #[test]
    fn merges_builtins_cache_and_current_selection() {
        let cached = vec![ModelOption {
            value: "claude-fable-5[1m]".into(),
            label: "Fable · 1M".into(),
            description: None,
        }];
        let models = merge_models(&Some("my-custom-model".into()), cached);
        let values: Vec<&str> = models.iter().map(|m| m.value.as_str()).collect();
        assert!(values.contains(&"fable"));
        assert!(values.contains(&"claude-fable-5[1m]"));
        assert!(values.contains(&"my-custom-model"));
        // no duplicate when current is already offered
        let models = merge_models(&Some("opus".into()), Vec::new());
        assert_eq!(models.iter().filter(|m| m.value == "opus").count(), 1);
    }
}
