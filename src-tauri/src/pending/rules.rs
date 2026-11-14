//! Permissions: what Claude may do without asking (docs/PERMISSIONS.md).
//!
//! A rule names a kind of action — a session's tool call (a tool family:
//! `Bash`, `PowerShell`, `Edit` (Edit, Write, MultiEdit, NotebookEdit),
//! `Read` (Read, Glob, Grep, LS), `WebFetch`, `WebSearch`, or one exact
//! tool), a command a session runs on another computer, a task it hands to
//! one — and where it applies: one session, every session in a project
//! folder, or the whole computer (for runs and tasks: the computer they
//! run on). Patterns narrow it: the first words of a command (at a word
//! boundary, as the standing grants for runs had it: `cargo test` covers
//! `cargo test --lib`, never `cargo test-foo`, and no pattern covers a
//! command holding a shell metacharacter), files as globs relative to the
//! session's folder, or sites.
//!
//! [`super::raise`] asks [`Rules::covering`] before it shows a card, and a
//! rule's answer is an *allow* as if the user had given it (the resolution
//! names the rule, `ResolverKind::Permission`). A rule never covers a
//! dangerous item — the classifier's word is final, so those always ask —
//! nor anything while the rules are paused. Answering a card with
//! `remember` makes one: for one session by anyone who may answer it, for a
//! project or the computer only with the dangerous right (a standing
//! grant, [`super::is_dangerous`]).
//!
//! Kept in `<app data>/permissions.json` (0600, written whole and
//! atomically), read entry by entry: one this build cannot read is skipped.
//! The standing grants for runs of v0.11–v0.13
//! (`machines/run-grants.json`) are taken over once, as rules for their
//! project.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::claude::model::{
    GrantOffer, PatternKind, PendingItem, PendingKind, PendingSource, RuleKind, RuleScope,
};
use crate::machines::work::grants as cmd;

/// The file, in the app data dir.
pub const FILE: &str = "permissions.json";
/// Its format.
pub const VERSION: u32 = 1;
/// Rules kept at most (the least recently used goes first).
pub const MAX_RULES: usize = 500;
/// Patterns of one rule at most.
pub const MAX_PATTERNS: usize = 20;
/// The longest pattern.
pub const MAX_PATTERN: usize = 200;
/// A rule for one session that nothing used for this long is dropped:
/// transcripts are pruned after 30 days.
pub const SESSION_RULE_MS: i64 = 30 * 24 * 3600 * 1000;

/// One rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// `pr_` and 12 hex characters.
    pub id: String,
    pub kind: RuleKind,
    /// `kind: tool`: the tool family ([`family_of`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Empty: any.
    #[serde(default)]
    pub patterns: Vec<String>,
    pub scope: RuleScope,
    /// The session it was made in (what `scope: session` names).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_name: String,
    /// Its project folder (what `scope: project` names).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project_name: String,
    /// `kind: run | task`: the computer they run on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub machine_name: String,
    /// Epoch ms.
    pub created_at: i64,
    /// Who saved it (a name).
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub used_at: Option<i64>,
    #[serde(default)]
    pub uses: u64,
}

/// What a card asks, as a rule reads it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ask<'a> {
    pub kind: RuleKind,
    /// `kind: tool`: the tool's exact name.
    pub tool: Option<&'a str>,
    pub session: &'a str,
    /// The session's folder: what path patterns are relative to.
    pub folder: Option<&'a str>,
    /// Its project folder ([`project_of`] for a tool call, the project's top
    /// for a run or a task): what `scope: project` compares.
    pub project: Option<&'a str>,
    /// `kind: run | task`: the computer.
    pub machine: Option<&'a str>,
    /// What patterns read: the command, the file, the site.
    pub subject: Option<&'a str>,
}

/// Why a rule could not be made or changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    /// A pattern, a scope or the request does not fit (422).
    Invalid(String),
    NotFound,
}

impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleError::Invalid(m) => f.write_str(m),
            RuleError::NotFound => f.write_str("No such permission."),
        }
    }
}

pub fn is_rule_id(s: &str) -> bool {
    s.strip_prefix("pr_")
        .is_some_and(|h| h.len() == 12 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn new_rule_id() -> String {
    let hex: String = crate::server::auth::random_bytes(6)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("pr_{hex}")
}

fn now() -> i64 {
    crate::claude::registry::now_ms()
}

// ─────────────────────────────── what a rule names ───────────────────────────────

/// The family a tool's calls belong to — what a rule for it names: the
/// file-changing tools are one, the file-reading ones another.
pub fn family_of(tool: &str) -> &str {
    match tool {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "Edit",
        "Read" | "Glob" | "Grep" | "LS" | "NotebookRead" => "Read",
        other => other,
    }
}

/// What a pattern names for a rule of `kind` (`tool`: its family); `None`:
/// it takes none.
pub fn pattern_kind(kind: RuleKind, tool: Option<&str>) -> Option<PatternKind> {
    match kind {
        RuleKind::Run => Some(PatternKind::Command),
        RuleKind::Task => None,
        RuleKind::Tool => match tool? {
            "Bash" | "PowerShell" => Some(PatternKind::Command),
            "Edit" | "Read" => Some(PatternKind::Path),
            "WebFetch" => Some(PatternKind::Domain),
            _ => None,
        },
    }
}

/// The host of a URL, lower case, without a trailing dot.
fn host_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url.trim()).ok()?;
    let h = u.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    (!h.is_empty()).then_some(h)
}

/// What a tool call's patterns read: its command, its file, its site.
pub fn subject_of(tool: &str, input: &Value) -> Option<String> {
    let s = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .map(str::to_string)
    };
    match family_of(tool) {
        "Bash" | "PowerShell" => s("command"),
        "Edit" => s("file_path").or_else(|| s("notebook_path")),
        "Read" => s("file_path")
            .or_else(|| s("notebook_path"))
            .or_else(|| s("path")),
        "WebFetch" => s("url").and_then(|u| host_of(&u)),
        _ => None,
    }
}

/// `pattern` as rules keep it, or why it cannot be one.
pub fn clean_pattern(kind: PatternKind, raw: &str) -> Result<String, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Err("A pattern can't be empty.".into());
    }
    if t.chars().count() > MAX_PATTERN {
        return Err(format!("A pattern has at most {MAX_PATTERN} characters."));
    }
    match kind {
        PatternKind::Command => {
            // `cargo test *` and `cargo test*` mean what `cargo test` does.
            let t = t.strip_suffix('*').map(str::trim_end).unwrap_or(t);
            if cmd::has_metachar(t) {
                return Err(
                    "A command pattern can't hold ; & | ` $ ( ) { } < > or a line break.".into(),
                );
            }
            let p = cmd::clean_prefix(t)
                .ok_or_else(|| "Name the command's first words, e.g. cargo test.".to_string())?;
            let first = p.split(' ').next().unwrap_or_default();
            if cmd::is_shell(first) {
                return Err(format!(
                    "{first} runs whatever it is given: allow every command instead, or name the \
                     program it runs."
                ));
            }
            if cmd::is_interpreter(first) && !p.contains(' ') {
                return Err(format!(
                    "{first} alone runs any code: add what it runs, e.g. {first} -m pytest or \
                     {first} scripts/build.js."
                ));
            }
            Ok(p)
        }
        PatternKind::Path => {
            let t = t.replace('\\', "/");
            let t = t.strip_prefix("./").unwrap_or(&t).to_string();
            if !crate::orchestration::templates::is_inside_relative(&t) {
                return Err(
                    "A file pattern is relative to the session's folder (no leading /, ~ or ..), \
                     e.g. src/** or *.md."
                        .into(),
                );
            }
            if t.matches("**").count() > 4 {
                return Err("A file pattern has at most four **.".into());
            }
            Ok(t)
        }
        PatternKind::Domain => {
            let t = t.to_ascii_lowercase();
            // A pasted address: its host.
            let t = if t.contains("://") {
                host_of(&t).ok_or_else(|| "Name a site, e.g. docs.rs.".to_string())?
            } else {
                t.trim_end_matches('/').trim_end_matches('.').to_string()
            };
            let host = t.strip_prefix("*.").unwrap_or(&t);
            let ok = !host.is_empty()
                && host.len() <= 253
                && (host.contains('.') || host == "localhost")
                && host.split('.').all(|l| {
                    !l.is_empty()
                        && !l.starts_with('-')
                        && !l.ends_with('-')
                        && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                });
            if !ok {
                return Err("Name a site, e.g. docs.rs or *.github.com.".into());
            }
            Ok(t)
        }
    }
}

/// The patterns as a rule keeps them (each cleaned, each once), or why not.
pub fn clean_patterns(kind: Option<PatternKind>, raw: &[String]) -> Result<Vec<String>, String> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let Some(kind) = kind else {
        return Err("This kind of permission takes no patterns.".into());
    };
    if raw.len() > MAX_PATTERNS {
        return Err(format!("At most {MAX_PATTERNS} patterns."));
    }
    let mut out: Vec<String> = Vec::new();
    for p in raw {
        let c = clean_pattern(kind, p)?;
        if !out.contains(&c) {
            out.push(c);
        }
    }
    Ok(out)
}

/// Whether `file` (absolute, or relative to `base`) is one a path pattern
/// names: inside `base` (the session's folder), the glob naming it or a
/// folder it is in.
fn path_matches(pattern: &str, file: &str, base: &str) -> bool {
    use crate::orchestration::guard::{glob_matches, resolve, Parts};
    let Some(root) = Parts::absolute(base) else {
        return false;
    };
    let Some(p) = resolve(file, Some(&root), None) else {
        return false;
    };
    let Some(rel) = p.below(&root) else {
        return false;
    };
    glob_matches(
        pattern,
        &rel,
        crate::claude::paths::is_windows_absolute(base),
    )
}

/// `docs.rs` names that site; `*.github.com` its subdomains.
fn domain_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(suffix) => host
            .strip_suffix(suffix)
            .is_some_and(|rest| rest.ends_with('.') && rest.len() > 1),
        None => host == pattern,
    }
}

fn pattern_matches(kind: PatternKind, pattern: &str, ask: &Ask) -> bool {
    let Some(subject) = ask.subject else {
        return false;
    };
    match kind {
        PatternKind::Command => cmd::prefix_matches(pattern, subject),
        PatternKind::Path => ask
            .folder
            .is_some_and(|base| path_matches(pattern, subject, base)),
        PatternKind::Domain => domain_matches(pattern, subject),
    }
}

/// How narrow a scope is: the narrowest rule that allows something is the
/// one credited with it.
fn rank(s: RuleScope) -> u8 {
    match s {
        RuleScope::Session => 3,
        RuleScope::Project => 2,
        RuleScope::Computer => 1,
    }
}

/// Whether `r` allows what `a` asks.
pub fn applies(r: &Rule, a: &Ask) -> bool {
    if r.kind != a.kind {
        return false;
    }
    match r.kind {
        RuleKind::Tool => {
            if r.tool.as_deref() != a.tool.map(family_of) || r.machine.is_some() {
                return false;
            }
        }
        RuleKind::Run | RuleKind::Task => {
            if r.machine.is_none() || r.machine.as_deref() != a.machine {
                return false;
            }
        }
    }
    let here = match r.scope {
        RuleScope::Session => r.session.as_deref() == Some(a.session),
        RuleScope::Project => match (r.project.as_deref(), a.project) {
            (Some(p), Some(q)) => crate::claude::paths::same_path(p, q),
            _ => false,
        },
        RuleScope::Computer => true,
    };
    if !here {
        return false;
    }
    if r.patterns.is_empty() {
        return true;
    }
    match pattern_kind(r.kind, r.tool.as_deref()) {
        Some(kind) => r.patterns.iter().any(|p| pattern_matches(kind, p, a)),
        None => false,
    }
}

// ─────────────────────────────── words ───────────────────────────────

/// What a rule's kind allows, in words ("commands", "file edits",
/// "commands on MacBook").
pub fn what(kind: RuleKind, tool: Option<&str>, machine_name: &str) -> String {
    match kind {
        RuleKind::Run => format!("commands on {machine_name}"),
        RuleKind::Task => format!("tasks on {machine_name}"),
        RuleKind::Tool => match tool.unwrap_or_default() {
            "Bash" => "commands".into(),
            "PowerShell" => "PowerShell commands".into(),
            "Edit" => "file edits".into(),
            "Read" => "file reads".into(),
            "WebFetch" => "web pages".into(),
            "WebSearch" => "web searches".into(),
            other => format!("{other} calls"),
        },
    }
}

/// One line for a rule: what it allows, like which patterns, where
/// ("commands like cargo test in shop").
pub fn describe(r: &Rule) -> String {
    let what = what(r.kind, r.tool.as_deref(), &r.machine_name);
    let like = if r.patterns.is_empty() {
        String::new()
    } else {
        format!(" like {}", r.patterns.join(", "))
    };
    let place = match r.scope {
        RuleScope::Session => "in this session".to_string(),
        RuleScope::Project if !r.project_name.is_empty() => format!("in {}", r.project_name),
        RuleScope::Project => "in this project".to_string(),
        RuleScope::Computer => match r.kind {
            RuleKind::Tool => format!(
                "everywhere on this {}",
                crate::claude::paths::machine_noun()
            ),
            RuleKind::Run | RuleKind::Task => "from any project".to_string(),
        },
    };
    format!("{what}{like} {place}")
}

// ─────────────────────────────── offers ───────────────────────────────

/// The project folder a session's folder belongs to: the repository's top
/// above it (a worktree under `.claude/worktrees/` belongs to the project
/// it is in), else the folder itself.
pub fn project_of(folder: &str) -> String {
    let p = Path::new(folder);
    // `<project>/.claude/worktrees/<name>`: the project.
    let comps: Vec<_> = p.components().collect();
    for i in 0..comps.len().saturating_sub(2) {
        let a = comps[i].as_os_str().to_string_lossy();
        let b = comps[i + 1].as_os_str().to_string_lossy();
        if a == ".claude" && b == "worktrees" && i > 0 {
            let top: PathBuf = comps[..i].iter().collect();
            return top.display().to_string();
        }
    }
    let mut cur = Some(p);
    while let Some(dir) = cur {
        if dir.join(".git").exists() {
            return dir.display().to_string();
        }
        cur = dir.parent();
    }
    folder.to_string()
}

/// The patterns a card offers to start from: the command's first words,
/// the file's folder, the site.
pub fn suggest(
    kind: Option<PatternKind>,
    subject: Option<&str>,
    folder: Option<&str>,
) -> Vec<String> {
    let (Some(kind), Some(subject)) = (kind, subject) else {
        return Vec::new();
    };
    match kind {
        PatternKind::Command => cmd::suggested_prefix(subject).into_iter().collect(),
        PatternKind::Domain => vec![subject.to_string()],
        PatternKind::Path => {
            use crate::orchestration::guard::{resolve, Parts};
            let Some(root) = folder.and_then(Parts::absolute) else {
                return Vec::new();
            };
            let Some(rel) = resolve(subject, Some(&root), None).and_then(|p| p.below(&root)) else {
                return Vec::new();
            };
            let pattern = match rel.split_last() {
                Some((_, dirs)) if !dirs.is_empty() => format!("{}/**", dirs.join("/")),
                Some((name, _)) => name.clone(),
                None => return Vec::new(),
            };
            clean_pattern(PatternKind::Path, &pattern)
                .ok()
                .into_iter()
                .collect()
        }
    }
}

/// The sources whose tool calls a rule may answer (runs and tasks bring
/// their own offer; the browser's are always dangerous and have grants of
/// their own; an observed prompt cannot be answered).
fn tool_source(s: PendingSource) -> bool {
    matches!(
        s,
        PendingSource::Owned | PendingSource::Hook | PendingSource::Bridge | PendingSource::Sidecar
    )
}

/// What a session's tool call asks, as rules read it (`folder`: the
/// session's, `project`: [`project_of`] it, `subject`: [`subject_of`]).
pub fn ask_of<'a>(
    item: &'a PendingItem,
    folder: Option<&'a str>,
    project: Option<&'a str>,
    subject: Option<&'a str>,
) -> Option<Ask<'a>> {
    if item.kind != PendingKind::Permission || !tool_source(item.source) {
        return None;
    }
    let tool = item.tool_name.as_deref().filter(|t| !t.is_empty())?;
    Some(Ask {
        kind: RuleKind::Tool,
        tool: Some(tool),
        session: &item.session_id,
        folder,
        project,
        machine: None,
        subject,
    })
}

/// The offer on a session's tool-call card: its tool family, the patterns
/// to start from, the session and its project; blocked when it is
/// dangerous.
pub fn tool_offer(
    item: &PendingItem,
    folder: Option<&str>,
    session_name: Option<String>,
) -> Option<GrantOffer> {
    if item.kind != PendingKind::Permission || !tool_source(item.source) {
        return None;
    }
    let tool = item.tool_name.as_deref().filter(|t| !t.is_empty())?;
    let family = family_of(tool).to_string();
    let kind = pattern_kind(RuleKind::Tool, Some(&family));
    let subject = item.input.as_ref().and_then(|i| subject_of(tool, i));
    let project = folder.map(project_of);
    Some(GrantOffer {
        kind: RuleKind::Tool,
        suggested: suggest(kind, subject.as_deref(), folder),
        patterns: kind,
        tool: Some(family),
        session: item.session_id.clone(),
        session_name,
        project_name: project
            .as_deref()
            .map(crate::machines::setup::folders::name_of),
        project,
        machine: None,
        machine_name: None,
        blocked: item
            .dangerous
            .then(|| "Dangerous actions always ask: this one can't be remembered.".to_string()),
    })
}

/// A rule from a card's offer, the scope and the patterns its answer chose
/// (`by`: who saved it).
pub fn rule_from_offer(
    offer: &GrantOffer,
    scope: RuleScope,
    patterns: &[String],
    by: &str,
) -> Result<Rule, RuleError> {
    if let Some(why) = &offer.blocked {
        return Err(RuleError::Invalid(why.clone()));
    }
    if scope == RuleScope::Project && offer.project.is_none() {
        return Err(RuleError::Invalid(
            "This session has no project folder: choose this session or every project.".into(),
        ));
    }
    if matches!(offer.kind, RuleKind::Run | RuleKind::Task) && offer.machine.is_none() {
        return Err(RuleError::Invalid(
            "The card names no computer to allow.".into(),
        ));
    }
    let patterns = clean_patterns(pattern_kind(offer.kind, offer.tool.as_deref()), patterns)
        .map_err(RuleError::Invalid)?;
    Ok(Rule {
        id: new_rule_id(),
        kind: offer.kind,
        tool: offer.tool.clone().filter(|_| offer.kind == RuleKind::Tool),
        patterns,
        scope,
        session: Some(offer.session.clone()),
        session_name: offer.session_name.clone().unwrap_or_default(),
        project: offer.project.clone(),
        project_name: offer.project_name.clone().unwrap_or_default(),
        machine: offer.machine.clone(),
        machine_name: offer.machine_name.clone().unwrap_or_default(),
        created_at: now(),
        created_by: crate::server::devices::sanitize_label(by, 64),
        used_at: None,
        uses: 0,
    })
}

/// `r` with new patterns and scope, and whether it then allows more than
/// before — a wider scope, patterns removed or added (which needs the
/// dangerous right).
pub fn changed(
    r: &Rule,
    patterns: Option<&[String]>,
    scope: Option<RuleScope>,
) -> Result<(Rule, bool), RuleError> {
    let mut next = r.clone();
    if let Some(p) = patterns {
        next.patterns = clean_patterns(pattern_kind(next.kind, next.tool.as_deref()), p)
            .map_err(RuleError::Invalid)?;
    }
    if let Some(s) = scope {
        let anchored = match s {
            RuleScope::Session => next.session.is_some(),
            RuleScope::Project => next.project.is_some(),
            RuleScope::Computer => true,
        };
        if !anchored {
            return Err(RuleError::Invalid(match s {
                RuleScope::Session => {
                    "This permission was not made in a session: it can't be narrowed to one.".into()
                }
                _ => "This permission has no project folder.".into(),
            }));
        }
        next.scope = s;
    }
    let wider_place = rank(next.scope) < rank(r.scope);
    // No patterns allow anything: from there every change narrows it.
    let wider_patterns = !r.patterns.is_empty()
        && (next.patterns.is_empty() || next.patterns.iter().any(|p| !r.patterns.contains(p)));
    Ok((next, wider_place || wider_patterns))
}
// ─────────────────────────────── the store ───────────────────────────────

#[derive(Default, Serialize, Deserialize)]
struct FileShape {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    paused: bool,
    #[serde(default)]
    rules: Vec<Value>,
}

#[derive(Default)]
struct Kept {
    rules: Vec<Rule>,
    paused: bool,
}

/// The rules (`AppState::rules`).
pub struct Rules {
    path: Option<PathBuf>,
    kept: Mutex<Kept>,
}

impl std::fmt::Debug for Rules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rules").finish_non_exhaustive()
    }
}

/// Whether a rule read from the file holds together.
fn readable(r: &Rule) -> bool {
    let anchored = match r.scope {
        RuleScope::Session => r.session.as_deref().is_some_and(|s| !s.is_empty()),
        RuleScope::Project => r.project.as_deref().is_some_and(|p| !p.is_empty()),
        RuleScope::Computer => true,
    };
    let kind_ok = match r.kind {
        RuleKind::Tool => {
            r.machine.is_none()
                && r.tool
                    .as_deref()
                    .is_some_and(|t| !t.is_empty() && t.len() <= 200)
        }
        RuleKind::Run | RuleKind::Task => r
            .machine
            .as_deref()
            .is_some_and(crate::machines::ids::is_machine_id),
    };
    let patterns_ok = clean_patterns(pattern_kind(r.kind, r.tool.as_deref()), &r.patterns)
        .is_ok_and(|c| c == r.patterns);
    is_rule_id(&r.id) && anchored && kind_ok && patterns_ok
}

/// The standing grants for runs of v0.11–v0.13, as rules for their project.
fn import_run_grants(data_dir: &Path) -> Vec<Rule> {
    cmd::legacy(data_dir)
        .into_iter()
        .map(|g| Rule {
            id: new_rule_id(),
            kind: RuleKind::Run,
            tool: None,
            patterns: vec![g.prefix],
            scope: RuleScope::Project,
            session: None,
            session_name: String::new(),
            project: Some(g.project),
            project_name: g.project_name,
            machine: Some(g.machine),
            machine_name: String::new(),
            created_at: g.created_at,
            created_by: g.created_by,
            used_at: g.used_at,
            uses: g.uses,
        })
        .filter(readable)
        .collect()
}

impl Rules {
    /// Reads the rules (`data_dir: None`: kept in memory only, for tests).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join(FILE));
        let mut kept = Kept::default();
        let exists = path.as_deref().is_some_and(Path::exists);
        if exists {
            let shape: FileShape = path
                .as_deref()
                .and_then(crate::persist::read_json)
                .unwrap_or_default();
            kept.paused = shape.paused;
            let cutoff = now() - SESSION_RULE_MS;
            kept.rules = shape
                .rules
                .into_iter()
                .filter_map(|v| serde_json::from_value::<Rule>(v).ok())
                .filter(readable)
                .filter(|r| {
                    r.scope != RuleScope::Session || r.used_at.unwrap_or(r.created_at) >= cutoff
                })
                .take(MAX_RULES)
                .collect();
        } else if let Some(dir) = data_dir {
            kept.rules = import_run_grants(dir);
        }
        let rules = Self {
            path,
            kept: Mutex::new(kept),
        };
        if !exists && data_dir.is_some() {
            let k = rules.lock();
            if !k.rules.is_empty() {
                rules.save(&k);
                tracing::info!(
                    count = k.rules.len(),
                    "the standing grants for runs are permissions now"
                );
            }
        }
        rules
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, k: &Kept) {
        let Some(path) = &self.path else { return };
        let shape = FileShape {
            version: VERSION,
            paused: k.paused,
            rules: k
                .rules
                .iter()
                .filter_map(|r| serde_json::to_value(r).ok())
                .collect(),
        };
        if let Err(e) = crate::persist::write_json_private(path, &shape) {
            tracing::warn!(error = %e, "the permissions could not be saved");
        }
    }

    /// Every rule (the newest first) and whether they are paused.
    pub fn list(&self) -> (Vec<Rule>, bool) {
        let k = self.lock();
        let mut l = k.rules.clone();
        l.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        (l, k.paused)
    }

    pub fn get(&self, id: &str) -> Option<Rule> {
        self.lock().rules.iter().find(|r| r.id == id).cloned()
    }

    pub fn paused(&self) -> bool {
        self.lock().paused
    }

    /// Pauses (every request asks; the rules are kept) or resumes them.
    pub fn set_paused(&self, paused: bool) {
        let mut k = self.lock();
        if k.paused != paused {
            k.paused = paused;
            self.save(&k);
        }
    }

    /// The narrowest rule that allows `ask`, counted as used; `None` while
    /// paused.
    pub fn covering(&self, ask: &Ask) -> Option<Rule> {
        let mut k = self.lock();
        if k.paused {
            return None;
        }
        let at = k
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| applies(r, ask))
            .max_by_key(|(_, r)| (rank(r.scope), r.patterns.is_empty() as u8 ^ 1))
            .map(|(i, _)| i)?;
        let r = &mut k.rules[at];
        r.used_at = Some(now());
        r.uses += 1;
        let out = r.clone();
        self.save(&k);
        Some(out)
    }

    /// Whether a rule would allow `ask` (nothing counted).
    pub fn would_cover(&self, ask: &Ask) -> bool {
        let k = self.lock();
        !k.paused && k.rules.iter().any(|r| applies(r, ask))
    }

    /// Keeps `rule` — or the one there already naming the same thing, the
    /// same way. The one kept.
    pub fn add(&self, rule: Rule) -> Rule {
        let mut k = self.lock();
        let mut wanted = rule.patterns.clone();
        wanted.sort();
        if let Some(same) = k.rules.iter().find(|r| {
            let mut p = r.patterns.clone();
            p.sort();
            r.kind == rule.kind
                && r.tool == rule.tool
                && r.scope == rule.scope
                && r.machine == rule.machine
                && p == wanted
                && match rule.scope {
                    RuleScope::Session => r.session == rule.session,
                    RuleScope::Project => matches!(
                        (&r.project, &rule.project),
                        (Some(a), Some(b)) if crate::claude::paths::same_path(a, b)
                    ),
                    RuleScope::Computer => true,
                }
        }) {
            return same.clone();
        }
        if k.rules.len() >= MAX_RULES {
            // The one used (or made) longest ago makes room.
            if let Some(old) = k
                .rules
                .iter()
                .enumerate()
                .min_by_key(|(_, r)| r.used_at.unwrap_or(r.created_at))
                .map(|(i, _)| i)
            {
                k.rules.remove(old);
            }
        }
        k.rules.push(rule.clone());
        self.save(&k);
        rule
    }

    /// Changes a rule's patterns and scope ([`changed`]). The rule as it is
    /// now, and whether it allows more than before.
    pub fn update(
        &self,
        id: &str,
        patterns: Option<&[String]>,
        scope: Option<RuleScope>,
    ) -> Result<(Rule, bool), RuleError> {
        let mut k = self.lock();
        let r = k
            .rules
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or(RuleError::NotFound)?;
        let (next, widened) = changed(r, patterns, scope)?;
        *r = next.clone();
        self.save(&k);
        Ok((next, widened))
    }
    /// Removes one: it, when it was there.
    pub fn remove(&self, id: &str) -> Option<Rule> {
        let mut k = self.lock();
        let at = k.rules.iter().position(|r| r.id == id)?;
        let r = k.rules.remove(at);
        self.save(&k);
        Some(r)
    }
}

/// One rule as the Permissions page shows it (`machineName`: the paired
/// computer's name now).
pub fn view(state: &crate::state::AppState, r: &Rule) -> Value {
    let machine_name = r
        .machine
        .as_deref()
        .map(|m| crate::machines::peer_name(state, m))
        .unwrap_or_default();
    json!({
        "id": r.id,
        "kind": r.kind,
        "tool": r.tool,
        "patterns": r.patterns,
        "patternKind": pattern_kind(r.kind, r.tool.as_deref()),
        "scope": r.scope,
        "session": r.session,
        "sessionName": (!r.session_name.is_empty()).then_some(&r.session_name),
        "project": r.project,
        "projectName": (!r.project_name.is_empty()).then_some(&r.project_name),
        "machine": r.machine,
        "machineName": (!machine_name.is_empty()).then_some(machine_name),
        "createdAt": r.created_at,
        "createdBy": r.created_by,
        "usedAt": r.used_at,
        "uses": r.uses,
    })
}

/// What an audit line keeps of a rule: never a pattern's text (a command
/// can carry anything), only how many and their SHA-256.
pub fn audit_detail(r: &Rule) -> Value {
    json!({
        "rule": r.id,
        "kind": r.kind,
        "tool": r.tool,
        "scope": r.scope,
        "project": r.project_name,
        "machine": r.machine,
        "patterns": r.patterns.len(),
        "patternsSha256": r.patterns.iter().map(|p| crate::machines::work::command_sha256(p)).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests;
