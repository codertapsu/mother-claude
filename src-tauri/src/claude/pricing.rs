//! Cost and context-window accounting (design §3.3).
//!
//! Owner: **B2 (data layer)** — STUB with naive helpers. What B2 implements:
//! the per-MTok price table, the cost formula (1h/5m cache writes, `us`
//! inference geo ×1.1, web search $0.01, prefer `cost-state.totalCostUSD`),
//! and the context window per model (1e6 for opus-4-7+/opus-5/5-5/sonnet-5/
//! fable/mythos, 200k for older/haiku unless `[1m]`).
//!
//! Keep the signatures of [`model_label`], [`context_window`] and
//! [`cost_usd`]; the summary builder calls them.

use super::model::Usage;

/// Display label for a model id: `claude-opus-4-8` → `Opus 4.8`,
/// `claude-sonnet-4-5-20250929` → `Sonnet 4.5`, `claude-haiku-4-5[1m]` →
/// `Haiku 4.5 (1M)`. Unknown shapes are returned unchanged.
pub fn model_label(id: &str) -> String {
    let one_m = id.contains("[1m]");
    let core = id.replace("[1m]", "");
    let core = core.trim().trim_start_matches("claude-");
    let mut parts: Vec<&str> = core.split('-').filter(|p| !p.is_empty()).collect();
    // Drop a trailing date stamp (8 digits).
    if parts
        .last()
        .is_some_and(|p| p.len() == 8 && p.chars().all(|c| c.is_ascii_digit()))
    {
        parts.pop();
    }
    let Some((family, version)) = parts.split_first() else {
        return id.to_string();
    };
    if family.is_empty() || !family.chars().all(|c| c.is_ascii_alphabetic()) {
        return id.to_string();
    }
    let mut label = String::new();
    let mut chars = family.chars();
    if let Some(f) = chars.next() {
        label.extend(f.to_uppercase());
        label.push_str(chars.as_str());
    }
    if !version.is_empty()
        && version
            .iter()
            .all(|v| v.chars().all(|c| c.is_ascii_digit()))
    {
        label.push(' ');
        label.push_str(&version.join("."));
    } else if !version.is_empty() {
        return id.to_string();
    }
    if one_m {
        label.push_str(" (1M)");
    }
    label
}

/// Context window in tokens for a model id. STUB: 200k unless the id carries
/// `[1m]` (B2 implements the full table).
pub fn context_window(id: &str) -> u64 {
    if id.contains("[1m]") {
        1_000_000
    } else {
        200_000
    }
}

/// Estimated cost in USD. STUB: unknown (`None` renders as "—").
pub fn cost_usd(_model_id: &str, _usage: &Usage) -> Option<f64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(model_label("claude-opus-4-8"), "Opus 4.8");
        assert_eq!(model_label("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
        assert_eq!(model_label("claude-haiku-4-5[1m]"), "Haiku 4.5 (1M)");
        assert_eq!(model_label("claude-opus-5"), "Opus 5");
        assert_eq!(model_label("<synthetic>"), "<synthetic>");
        assert_eq!(model_label("gpt-x-y"), "gpt-x-y");
        assert_eq!(context_window("claude-opus-4-8[1m]"), 1_000_000);
    }
}
