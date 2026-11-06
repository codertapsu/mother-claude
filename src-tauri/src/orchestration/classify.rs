//! Task classes for tasks that arrive without one (docs/ROUTING.md, plan
//! §7.1): keyword rules by default, and optionally Claude Haiku.
//!
//! **Rules** ([`classify`]): whole-word keywords in the title (counted
//! twice) and the goal; the class with the most hits wins, ties going to
//! the more specific class (a fix before an implementation). No hit:
//! `other`, which runs on the role's default.
//!
//! **Haiku** ([`classify_task`], off by default — it spends tokens): one
//! `POST /v1/messages` to `claude-haiku-5-5` through the bridge's direct
//! Messages API client (`bridge::messages`, the existing path that already
//! calls Claude), so it needs an API key (`ANTHROPIC_API_KEY` when Mother
//! Claude starts); thinking off, effort low, a 16-token answer that must be
//! one class name. Its tokens and estimated cost are counted
//! ([`ClassifierStats`]). Any failure falls back to the rules, saying why.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::policy::{class_from, class_name};
use super::router::TaskClass;
use super::scheduler::lock;
use crate::state::AppState;

/// The model the optional classifier asks.
pub const HAIKU_MODEL: &str = "claude-haiku-5-5";
/// Longest title + goal sent to it (characters).
const TEXT_MAX: usize = 4_000;
/// How long it may take.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// What classified a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClassSource {
    Rules,
    Haiku,
}

/// A class with its reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub class: TaskClass,
    pub source: ClassSource,
    pub reason: String,
}

/// The optional classifier's spend (since the app started).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifierStats {
    pub calls: u64,
    pub failures: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Estimated USD.
    pub cost_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Keywords per class, most specific classes first (their order breaks
/// ties). Multi-word phrases match as phrases.
const RULES: [(TaskClass, &[&str]); 10] = [
    (
        TaskClass::Fix,
        &[
            "fix",
            "fixes",
            "bug",
            "bugs",
            "crash",
            "crashes",
            "broken",
            "regression",
            "failing",
            "fails",
            "error",
            "errors",
            "hotfix",
            "repair",
            "flaky",
        ],
    ),
    (
        TaskClass::Test,
        &[
            "test",
            "tests",
            "testing",
            "unit test",
            "e2e",
            "coverage",
            "spec",
            "specs",
            "fixture",
            "fixtures",
            "benchmark",
            "assert",
        ],
    ),
    (
        TaskClass::Review,
        &[
            "review",
            "reviews",
            "audit",
            "critique",
            "code review",
            "proofread",
            "inspect",
            "assess",
            "evaluate",
        ],
    ),
    (
        TaskClass::Architecture,
        &[
            "architecture",
            "architect",
            "design",
            "adr",
            "schema",
            "data model",
            "interface",
            "api design",
            "system design",
            "trade-off",
            "tradeoffs",
        ],
    ),
    (
        TaskClass::Plan,
        &[
            "plan",
            "planning",
            "roadmap",
            "break down",
            "breakdown",
            "scope",
            "estimate",
            "prioritize",
            "prioritise",
            "milestone",
            "milestones",
            "outline",
            "strategy",
        ],
    ),
    (
        TaskClass::Ops,
        &[
            "deploy",
            "deployment",
            "release",
            "ci",
            "pipeline",
            "infra",
            "infrastructure",
            "docker",
            "kubernetes",
            "k8s",
            "terraform",
            "monitoring",
            "backup",
            "cron",
            "rollback",
            "provision",
            "upgrade dependencies",
        ],
    ),
    (
        TaskClass::Summarize,
        &[
            "summarize",
            "summarise",
            "summary",
            "recap",
            "digest",
            "tl;dr",
            "changelog",
            "release notes",
            "report on",
        ],
    ),
    (
        TaskClass::Classify,
        &[
            "classify",
            "categorize",
            "categorise",
            "triage",
            "label",
            "labels",
            "tag",
            "tags",
            "sort into",
        ],
    ),
    (
        TaskClass::Search,
        &[
            "find",
            "search",
            "locate",
            "look up",
            "lookup",
            "where is",
            "grep",
            "investigate",
            "explore",
            "research",
            "survey",
        ],
    ),
    (
        TaskClass::Implement,
        &[
            "implement",
            "implementation",
            "add",
            "build",
            "create",
            "feature",
            "support",
            "integrate",
            "port",
            "migrate",
            "refactor",
            "write",
            "wire",
            "extend",
        ],
    ),
];

/// Lowercase words (letters, digits, `;`, `#`, `+`, `-`) joined by single
/// spaces, padded with spaces so a phrase matches only whole words.
fn words(text: &str) -> String {
    let mut out = String::from(" ");
    let mut last_space = true;
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || matches!(c, ';' | '#' | '+' | '-') {
            out.push(c);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    if !last_space {
        out.push(' ');
    }
    out
}

fn hits(text: &str, keyword: &str) -> usize {
    text.matches(&format!(" {keyword} ")).count()
}

/// Classify by keywords in the title and the goal.
pub fn classify(title: &str, goal: &str) -> Classification {
    let (t, g) = (words(title), words(goal));
    let mut best: Option<(TaskClass, usize, Vec<&str>)> = None;
    for (class, keywords) in RULES {
        let mut score = 0;
        let mut matched = Vec::new();
        for kw in keywords {
            let n = 2 * hits(&t, kw) + hits(&g, kw);
            if n > 0 {
                score += n;
                matched.push(*kw);
            }
        }
        if score > 0 && best.as_ref().is_none_or(|(_, s, _)| score > *s) {
            best = Some((class, score, matched));
        }
    }
    match best {
        Some((class, _, matched)) => {
            let shown: Vec<String> = matched.iter().take(3).map(|k| format!("“{k}”")).collect();
            Classification {
                class,
                source: ClassSource::Rules,
                reason: format!(
                    "keyword{} {}",
                    if shown.len() == 1 { "" } else { "s" },
                    shown.join(", ")
                ),
            }
        }
        None => Classification {
            class: TaskClass::Other,
            source: ClassSource::Rules,
            reason: "no keyword matched".into(),
        },
    }
}

/// Whether the optional classifier can run: switched on, and the direct
/// Messages API path has a key.
pub fn haiku_ready(state: &AppState) -> Result<(), String> {
    if !state.scheduler.limits().haiku_classifier {
        return Err("the Haiku classifier is off".into());
    }
    if !state.bridge.messages.is_configured() {
        return Err(
            "the Haiku classifier needs an Anthropic API key (ANTHROPIC_API_KEY when Mother Claude starts)"
                .into(),
        );
    }
    Ok(())
}

/// The class Haiku named in its answer: the first class word in it.
pub fn class_in_answer(text: &str) -> Option<TaskClass> {
    words(text).split_whitespace().find_map(class_from)
}

fn prompt(title: &str, goal: &str) -> String {
    let mut text = format!("Title: {}\nGoal: {}", title.trim(), goal.trim());
    if text.chars().count() > TEXT_MAX {
        text = text.chars().take(TEXT_MAX).collect();
    }
    let names: Vec<&str> = super::policy::CLASSES
        .iter()
        .map(|c| class_name(*c))
        .collect();
    format!(
        "Classify this software or business task into exactly one class: {}.\n\
         plan = planning and breakdown; architecture = design decisions; review = \
         reviewing or auditing work; implement = building features; test = writing or \
         running tests; fix = fixing bugs; search = finding information; summarize = \
         summaries and reports; classify = sorting or labelling items; ops = deploys, \
         CI and infrastructure; other = none of these.\n\
         Answer with the class name only.\n\n{text}",
        names.join(", ")
    )
}

/// Ask Haiku (the caller checked [`haiku_ready`]). Counts the call.
async fn ask_haiku(
    state: &AppState,
    title: &str,
    goal: &str,
) -> Result<(TaskClass, String), String> {
    let body = json!({
        "model": HAIKU_MODEL,
        "max_tokens": 16,
        "thinking": { "type": "disabled" },
        "output_config": { "effort": "low" },
        "messages": [{ "role": "user", "content": prompt(title, goal) }],
    });
    let call = state.bridge.messages.create_message(&body, None);
    let result = tokio::time::timeout(TIMEOUT, call).await;
    let mut stats = lock(&state.scheduler.classifier);
    stats.calls += 1;
    let resp = match result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            stats.failures += 1;
            let msg = format!(
                "the Messages API answered an error ({}, {})",
                e.status.as_u16(),
                e.code
            );
            stats.last_error = Some(msg.clone());
            return Err(msg);
        }
        Err(_) => {
            stats.failures += 1;
            stats.last_error = Some("it did not answer within 20 s".into());
            return Err("it did not answer within 20 s".into());
        }
    };
    let input = resp["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let output = resp["usage"]["output_tokens"].as_u64().unwrap_or(0);
    stats.input_tokens += input;
    stats.output_tokens += output;
    let cost = crate::claude::pricing::message_cost(
        HAIKU_MODEL,
        &crate::claude::pricing::UsageDetail {
            input,
            output,
            standard_speed: true,
            ..Default::default()
        },
    )
    .unwrap_or(0.0);
    stats.cost_usd += cost;
    let text: String = resp["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    match class_in_answer(&text) {
        Some(class) => {
            stats.last_error = None;
            Ok((class, format!("Claude Haiku (${cost:.5}, estimated)")))
        }
        None => {
            stats.failures += 1;
            stats.last_error = Some("its answer named no class".into());
            Err("its answer named no class".into())
        }
    }
}

/// Classify a task that arrived without a class: Haiku when the user turned
/// it on (and it can run), the keyword rules otherwise or when it fails.
pub async fn classify_task(state: &AppState, title: &str, goal: &str) -> Classification {
    if let Err(why) = haiku_ready(state) {
        let mut c = classify(title, goal);
        if state.scheduler.limits().haiku_classifier {
            c.reason = format!("{} ({why})", c.reason);
        }
        return c;
    }
    match ask_haiku(state, title, goal).await {
        Ok((class, reason)) => Classification {
            class,
            source: ClassSource::Haiku,
            reason,
        },
        Err(why) => {
            let mut c = classify(title, goal);
            c.reason = format!("{}; Haiku could not classify it: {why}", c.reason);
            c
        }
    }
}

/// The classifier's spend so far.
pub fn stats(state: &AppState) -> ClassifierStats {
    lock(&state.scheduler.classifier).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(title: &str, goal: &str) -> TaskClass {
        classify(title, goal).class
    }

    #[test]
    fn keywords_pick_the_class() {
        assert_eq!(class("Fix the crash on startup", ""), TaskClass::Fix);
        assert_eq!(class("Add unit tests for the parser", ""), TaskClass::Test);
        assert_eq!(
            class("Review PR 42", "check the migration"),
            TaskClass::Review
        );
        assert_eq!(class("Design the sync schema", ""), TaskClass::Architecture);
        assert_eq!(class("Plan the Q3 roadmap", ""), TaskClass::Plan);
        assert_eq!(class("Deploy to staging", "CI pipeline"), TaskClass::Ops);
        assert_eq!(
            class("Summarise yesterday's incidents", ""),
            TaskClass::Summarize
        );
        assert_eq!(
            class("Triage the inbox", "label each issue"),
            TaskClass::Classify
        );
        assert_eq!(class("Find where tokens are logged", ""), TaskClass::Search);
        assert_eq!(
            class("Implement the export button", ""),
            TaskClass::Implement
        );
        assert_eq!(class("Lunch", "nothing to see"), TaskClass::Other);
    }

    #[test]
    fn titles_weigh_more_and_ties_go_to_the_specific_class() {
        // "fix" in the title (2) beats "add" in the goal (1).
        assert_eq!(class("Fix login", "add a retry"), TaskClass::Fix);
        // One hit each: fix comes before implement.
        assert_eq!(class("", "add bug"), TaskClass::Fix);
        // A title hit counts twice.
        assert_eq!(class("add", "bug"), TaskClass::Implement);
        // Whole words only: "prefix" is not "fix", "address" is not "add".
        assert_eq!(class("Prefix the address", ""), TaskClass::Other);
        let c = classify("Fix the crash", "the error repeats");
        assert_eq!(c.source, ClassSource::Rules);
        assert_eq!(c.reason, "keywords “fix”, “crash”, “error”");
        assert_eq!(classify("x", "").reason, "no keyword matched");
    }

    #[test]
    fn haikus_answer_is_read_tolerantly() {
        assert_eq!(class_in_answer("fix"), Some(TaskClass::Fix));
        assert_eq!(
            class_in_answer("Class: Summarise."),
            Some(TaskClass::Summarize)
        );
        assert_eq!(
            class_in_answer("I think it is a review."),
            Some(TaskClass::Review)
        );
        assert_eq!(class_in_answer("no idea"), None);
        assert!(prompt("t", "g").contains("plan, architecture, review"));
        let long = "x".repeat(10_000);
        assert!(prompt(&long, "").chars().count() < 5_000);
    }

    #[tokio::test]
    async fn the_classifier_falls_back_to_rules_unless_it_can_run() {
        let st = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir().join("mc-classify-no-home")),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        // Off by default: the rules, with no word about Haiku.
        let c = classify_task(&st, "Fix the crash", "").await;
        assert_eq!((c.class, c.source), (TaskClass::Fix, ClassSource::Rules));
        assert!(!c.reason.contains("Haiku"));
        // On, without an API key (tests never have one): the rules, saying why.
        let mut limits = st.scheduler.limits();
        limits.haiku_classifier = true;
        st.scheduler.set_limits(limits);
        if !st.bridge.messages.is_configured() {
            let c = classify_task(&st, "Fix the crash", "").await;
            assert_eq!(c.source, ClassSource::Rules);
            assert!(
                c.reason.contains("needs an Anthropic API key"),
                "{}",
                c.reason
            );
            assert_eq!(stats(&st).calls, 0, "no call was made");
        }
    }
}
