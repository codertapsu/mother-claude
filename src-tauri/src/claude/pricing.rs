//! Cost and context-window accounting (design §3.3, amended §10 B2).
//!
//! * The price table is the CLI's baked catalog (per MTok: input, output,
//!   5-minute cache write, 1-hour cache write, cache read).
//! * Model ids are normalised before lookup: `[1m]` and a trailing `-YYYYMMDD`
//!   are stripped, as is the `claude-` prefix.
//! * `<synthetic>` records (the CLI's own placeholder messages) are never
//!   priced and never count as a model.
//! * The formula matches the CLI (2.1.282 `Uqe`/`yA`/`SA`):
//!   `base = (in·Pin + out·Pout + cr·Pcr + w1h·Pw1h + (cc − w1h)·Pw5m) / 1e6`
//!   with `w1h = min(ephemeral_1h, cc)`, `× 1.1` when `inference_geo == "us"`,
//!   plus `$0.01` per web search request.
//!
//! The session-level rule ("last `cost-state` total plus priced usage after
//! it") lives in the index/summaries; this module prices one message.

use super::model::Usage;

/// Per-MTok prices in USD.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub write_5m: f64,
    pub write_1h: f64,
    pub cache_read: f64,
}

const fn p(input: f64, output: f64, write_5m: f64, write_1h: f64, cache_read: f64) -> Price {
    Price {
        input,
        output,
        write_5m,
        write_1h,
        cache_read,
    }
}

/// Token usage of one API message, as the transcript records it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UsageDetail {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    /// `cache_creation_input_tokens` (5 m + 1 h writes together).
    pub cache_creation: u64,
    /// `cache_creation.ephemeral_1h_input_tokens`.
    pub cache_1h: u64,
    /// `server_tool_use.web_search_requests`.
    pub web_search: u64,
    /// `inference_geo == "us"`.
    pub us_geo: bool,
    /// `speed` absent or `"standard"`; other tiers are priced differently.
    pub standard_speed: bool,
}

/// True for placeholder ids that must never be priced or displayed.
pub fn is_synthetic(model: &str) -> bool {
    let m = model.trim();
    m.is_empty() || m.starts_with('<')
}

/// Normalise a model id for price/window lookup: lowercase, no `[1m]`, no
/// `claude-` prefix, no trailing 8-digit date stamp.
pub fn normalize(model: &str) -> String {
    let mut m = model.trim().to_ascii_lowercase().replace("[1m]", "");
    if let Some(rest) = m.strip_prefix("claude-") {
        m = rest.to_string();
    }
    // Strip `-YYYYMMDD`.
    if let Some((head, tail)) = m.rsplit_once('-') {
        if tail.len() == 8 && tail.bytes().all(|c| c.is_ascii_digit()) {
            m = head.to_string();
        }
    }
    // Some ids carry `-latest` / `-v1` style suffixes.
    m.trim_end_matches("-latest").to_string()
}

fn has(m: &str, prefix: &str) -> bool {
    m == prefix || m.starts_with(&format!("{prefix}-"))
}

/// Whether a model thinks adaptively when thinking is on (the CLI then sends
/// `{type:"adaptive"}` and ignores any budget). Mirrors CLI 2.1.284's
/// `initialize` catalog: Opus/Sonnet 4.6 and later, and every Fable, do;
/// Haiku and the 3.x models do not. `None` when unknown (older 4.x ids,
/// third-party names).
pub fn supports_adaptive_thinking(model: &str) -> Option<bool> {
    if is_synthetic(model) {
        return None;
    }
    let m = normalize(model);
    let m = m.as_str();
    if has(m, "fable") || has(m, "mythos") {
        return Some(true);
    }
    for family in ["opus", "sonnet"] {
        if let Some(rest) = m.strip_prefix(family).and_then(|r| r.strip_prefix('-')) {
            let mut parts = rest.split('-').map(|p| p.parse::<u32>().ok());
            let major = parts.next().flatten();
            let minor = parts.next().flatten().filter(|n| *n < 100);
            return match (major, minor) {
                (Some(major), _) if major >= 5 => Some(true),
                (Some(4), Some(minor)) if minor >= 6 => Some(true),
                _ => None,
            };
        }
    }
    if m.contains("haiku") || m.starts_with("3-") {
        return Some(false);
    }
    None
}

/// Price for a model id, if known.
pub fn price(model: &str) -> Option<Price> {
    if is_synthetic(model) {
        return None;
    }
    let m = normalize(model);
    let m = m.as_str();
    // Order matters: more specific ids first.
    let price = if has(m, "opus-5-5") {
        p(4.0, 20.0, 5.0, 8.0, 0.2)
    } else if has(m, "opus-5")
        || has(m, "opus-4-5")
        || has(m, "opus-4-6")
        || has(m, "opus-4-7")
        || has(m, "opus-4-8")
    {
        p(5.0, 25.0, 6.25, 10.0, 0.5)
    } else if has(m, "opus-4-0") || has(m, "opus-4-1") || m == "opus-4" || has(m, "3-opus") {
        p(15.0, 75.0, 18.75, 30.0, 1.5)
    } else if has(m, "sonnet-5") {
        p(2.0, 10.0, 2.5, 4.0, 0.2)
    } else if has(m, "sonnet-4") || has(m, "3-7-sonnet") || has(m, "3-5-sonnet") {
        p(3.0, 15.0, 3.75, 6.0, 0.3)
    } else if has(m, "fable-5-1") || has(m, "mythos-5-1") {
        p(10.0, 50.0, 12.5, 20.0, 0.25)
    } else if has(m, "fable-5") || has(m, "mythos-5") {
        p(10.0, 50.0, 12.5, 20.0, 1.0)
    } else if has(m, "haiku-4-5") {
        p(1.0, 5.0, 1.25, 2.0, 0.1)
    } else if has(m, "3-5-haiku") {
        p(0.8, 4.0, 1.0, 1.6, 0.08)
    } else {
        return None;
    };
    Some(price)
}

/// Price one message. `None` when the model has no known price.
pub fn message_cost(model: &str, u: &UsageDetail) -> Option<f64> {
    let pr = price(model)?;
    let w1h = u.cache_1h.min(u.cache_creation);
    let w5m = u.cache_creation - w1h;
    let base = (u.input as f64 * pr.input
        + u.output as f64 * pr.output
        + u.cache_read as f64 * pr.cache_read
        + w1h as f64 * pr.write_1h
        + w5m as f64 * pr.write_5m)
        / 1_000_000.0;
    let geo = if u.us_geo { 1.1 } else { 1.0 };
    Some(base * geo + u.web_search as f64 * 0.01)
}

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

/// Context window in tokens for a model id (the CLI catalog's
/// `context.window`): 1M for Opus 4.7+, Opus 5/5.5, Sonnet 5, Fable, Mythos;
/// 200k for older models and Haiku unless the id carries `[1m]`.
pub fn context_window(id: &str) -> u64 {
    if id.contains("[1m]") {
        return 1_000_000;
    }
    let m = normalize(id);
    let m = m.as_str();
    let one_m = has(m, "opus-4-7")
        || has(m, "opus-4-8")
        || has(m, "opus-5")
        || has(m, "sonnet-5")
        || has(m, "fable-5")
        || has(m, "mythos-5");
    if one_m {
        1_000_000
    } else {
        200_000
    }
}

/// Estimated cost of aggregated usage (all cache writes priced as 5-minute
/// writes). Kept for callers that only have the contract [`Usage`]; the index
/// prices each message exactly with [`message_cost`].
pub fn cost_usd(model_id: &str, usage: &Usage) -> Option<f64> {
    message_cost(
        model_id,
        &UsageDetail {
            input: usage.input,
            output: usage.output,
            cache_read: usage.cache_read,
            cache_creation: usage.cache_write,
            standard_speed: true,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn labels() {
        assert_eq!(model_label("claude-opus-4-8"), "Opus 4.8");
        assert_eq!(model_label("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
        assert_eq!(model_label("claude-haiku-4-5[1m]"), "Haiku 4.5 (1M)");
        assert_eq!(model_label("claude-opus-5"), "Opus 5");
        assert_eq!(model_label("<synthetic>"), "<synthetic>");
        assert_eq!(model_label("gpt-x-y"), "gpt-x-y");
    }

    #[test]
    fn adaptive_thinking_follows_the_cli_catalog() {
        for yes in [
            "claude-opus-5-5",
            "claude-opus-5[1m]",
            "claude-sonnet-5-5",
            "claude-sonnet-5",
            "claude-fable-5-1",
            "claude-fable-5",
            "claude-opus-4-8",
            "claude-opus-4-6",
            "claude-sonnet-4-6",
        ] {
            assert_eq!(supports_adaptive_thinking(yes), Some(true), "{yes}");
        }
        for no in ["claude-haiku-4-5-20251001", "claude-3-5-haiku-20241022"] {
            assert_eq!(supports_adaptive_thinking(no), Some(false), "{no}");
        }
        for unknown in [
            "claude-sonnet-4-5-20250929",
            "claude-opus-4-1",
            "gpt-x",
            "<synthetic>",
            "",
        ] {
            assert_eq!(supports_adaptive_thinking(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn normalisation_strips_suffixes() {
        assert_eq!(normalize("claude-haiku-4-5-20251001"), "haiku-4-5");
        assert_eq!(normalize("claude-opus-5[1m]"), "opus-5");
        assert_eq!(normalize("Claude-Opus-5-5"), "opus-5-5");
        assert!(is_synthetic("<synthetic>"));
        assert!(price("<synthetic>").is_none());
    }

    #[test]
    fn price_tiers() {
        assert_eq!(price("claude-opus-5-5").unwrap().input, 4.0);
        assert_eq!(price("claude-opus-5[1m]").unwrap().input, 5.0);
        assert_eq!(price("claude-opus-4-7").unwrap().output, 25.0);
        assert_eq!(price("claude-opus-4-1-20250805").unwrap().input, 15.0);
        assert_eq!(price("claude-sonnet-5").unwrap().cache_read, 0.2);
        assert_eq!(price("claude-sonnet-4-5-20250929").unwrap().input, 3.0);
        assert_eq!(price("claude-fable-5-1").unwrap().cache_read, 0.25);
        assert_eq!(price("claude-mythos-5").unwrap().cache_read, 1.0);
        assert_eq!(price("claude-haiku-4-5-20251001").unwrap().input, 1.0);
        assert!(price("gpt-5").is_none());
    }

    #[test]
    fn cost_formula_matches_the_cli() {
        // haiku-4-5: 1/5/1.25/2/0.1 per MTok.
        let u = UsageDetail {
            input: 1_000,
            output: 2_000,
            cache_read: 10_000,
            cache_creation: 4_000,
            cache_1h: 1_000,
            web_search: 0,
            us_geo: false,
            standard_speed: true,
        };
        // (1000*1 + 2000*5 + 10000*0.1 + 1000*2 + 3000*1.25) / 1e6
        let expect = (1000.0 + 10_000.0 + 1_000.0 + 2_000.0 + 3_750.0) / 1e6;
        assert!(close(
            message_cost("claude-haiku-4-5-20251001", &u).unwrap(),
            expect
        ));
        // US geo ×1.1, web search +$0.01 each.
        let us = UsageDetail {
            us_geo: true,
            web_search: 2,
            ..u
        };
        assert!(close(
            message_cost("claude-haiku-4-5", &us).unwrap(),
            expect * 1.1 + 0.02
        ));
        // 1h larger than total creation is clamped.
        let clamp = UsageDetail {
            cache_creation: 100,
            cache_1h: 500,
            ..Default::default()
        };
        assert!(close(
            message_cost("claude-haiku-4-5", &clamp).unwrap(),
            100.0 * 2.0 / 1e6
        ));
        assert!(message_cost("<synthetic>", &u).is_none());
    }

    #[test]
    fn context_windows() {
        assert_eq!(context_window("claude-opus-4-8[1m]"), 1_000_000);
        assert_eq!(context_window("claude-opus-5"), 1_000_000);
        assert_eq!(context_window("claude-opus-5-5"), 1_000_000);
        assert_eq!(context_window("claude-sonnet-5"), 1_000_000);
        assert_eq!(context_window("claude-fable-5-1"), 1_000_000);
        assert_eq!(context_window("claude-sonnet-4-5"), 200_000);
        assert_eq!(context_window("claude-opus-4-6"), 200_000);
        assert_eq!(context_window("claude-haiku-4-5-20251001"), 200_000);
        assert_eq!(context_window("claude-haiku-4-5[1m]"), 1_000_000);
    }
}
