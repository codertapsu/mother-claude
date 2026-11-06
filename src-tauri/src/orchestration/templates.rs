//! Workflow templates (docs/TEAMS.md "Templates"; plan §6.1, §6.6): the
//! roles a team runs and how their work comes together. A template is data —
//! the orchestrator never knows what "frontend" means.
//!
//! Built-in templates live in code and cannot be changed (duplicate one to
//! edit it). The user's own are JSON files in `<app data dir>/workflows/`
//! (0600, written atomically through [`crate::persist`]); never anything in
//! a project or an account's config directory.
//!
//! Parsing is tolerant: unknown fields are kept as they were, missing ones
//! take their defaults, and [`validate`] reports every problem at once
//! (errors block saving and starting, warnings do not).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claude::owned::{EFFORT_LEVELS, PERMISSION_MODES, SAFE_MODES};

/// Longest template id.
pub const TEMPLATE_ID_MAX: usize = 64;
/// Longest role id (it names the role's worktree and is part of its session
/// name).
pub const ROLE_ID_MAX: usize = 32;
/// Roles in one template.
pub const MAX_ROLES: usize = 12;
/// Longest charter.
pub const CHARTER_MAX: usize = 20_000;
/// The budget ceiling a role may name.
pub const BUDGET_MAX_USD: f64 = 1000.0;
/// Idle minutes before a role's session stops, when the template says
/// nothing.
pub const DEFAULT_IDLE_STOP_MINUTES: u32 = 30;

/// The task classes the router knows (`router::TaskClass`, camelCase).
pub const TASK_CLASSES: [&str; 11] = [
    "plan",
    "architecture",
    "review",
    "implement",
    "test",
    "fix",
    "search",
    "summarize",
    "classify",
    "ops",
    "other",
];

/// A template's or a role's id: lower case letters, digits and inner `-`.
pub fn is_id(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// One role of a template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RoleDef {
    pub id: String,
    pub title: String,
    /// The lead (the "CEO"): plans, writes the packets, reviews and
    /// integrates. Exactly one role is the lead.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub lead: bool,
    /// One line: what the role is for (its agent definition's description).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The role's standing instructions: appended to Claude Code's system
    /// prompt in its sessions, and the prompt of its agent definition.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub charter: String,
    /// A Claude Code model alias (`opus`, `sonnet`, `haiku`, `fable`) or id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Always passed explicitly (`default` when absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Its sessions run in a git worktree of their own
    /// (`<project>/.claude/worktrees/<role>`, made by Claude Code).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub worktree: bool,
    /// The files it may change: globs relative to the project root (or to
    /// its worktree). Empty: no limit.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// It changes nothing: every write is denied.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// `--max-budget-usd` of each of its sessions, on accounts that pay per
    /// token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    /// Started with the team (the lead's usual setting), else only when it
    /// has work.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub autostart: bool,
    /// Extra folders its sessions may use (`--add-dir`), relative to the
    /// project root.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub add_dirs: Vec<String>,
    /// The tools of its agent definition (when another session runs it as a
    /// subagent). Empty: every tool.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// The task classes it usually takes (shown to the lead).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
    /// Fields a newer version wrote; kept as they were.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl RoleDef {
    /// The permission mode its sessions start in.
    pub fn mode(&self) -> &str {
        self.permission_mode.as_deref().unwrap_or("default")
    }
}

/// How the roles' work comes together.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Integration {
    /// The branch the lead integrates into.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The role that merges (the lead when absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_by: Option<String>,
    /// The command that must pass before a merge (`npm test && cargo test`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    /// A worker's finished task waits for the lead's review (`review`)
    /// before it is done. Default: on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A workflow template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Template {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub roles: Vec<RoleDef>,
    pub integration: Integration,
    /// Minutes a role's session may sit idle with nothing to do before it is
    /// stopped (0: never). Default 30.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_stop_minutes: Option<u32>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Template {
    /// The lead role (validation guarantees one).
    pub fn lead(&self) -> Option<&RoleDef> {
        self.roles.iter().find(|r| r.lead)
    }

    pub fn role(&self, id: &str) -> Option<&RoleDef> {
        self.roles.iter().find(|r| r.id == id)
    }

    pub fn idle_stop_minutes(&self) -> u32 {
        self.idle_stop_minutes.unwrap_or(DEFAULT_IDLE_STOP_MINUTES)
    }

    /// Workers' finished tasks wait for review.
    pub fn reviews(&self) -> bool {
        self.integration.review.unwrap_or(true)
    }

    /// Whether starting it needs the dangerous right: a role that runs
    /// without permission prompts.
    pub fn needs_dangerous(&self) -> bool {
        self.roles
            .iter()
            .any(|r| crate::claude::owned::is_dangerous_mode(Some(r.mode())))
    }
}

/// One problem in a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    /// Where (`roles[2].paths[0]`), empty for the whole template.
    pub path: String,
    pub message: String,
}

/// What [`validate`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    fn error(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(Issue {
            path: path.into(),
            message: message.into(),
        });
    }

    fn warn(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(Issue {
            path: path.into(),
            message: message.into(),
        });
    }

    /// The first error, for a one-line refusal.
    pub fn first_error(&self) -> Option<String> {
        self.errors.first().map(|i| {
            if i.path.is_empty() {
                i.message.clone()
            } else {
                format!("{}: {}", i.path, i.message)
            }
        })
    }
}

/// A relative path or glob that stays inside the folder it is relative to.
fn is_inside_relative(p: &str) -> bool {
    let p = p.trim();
    !p.is_empty()
        && !crate::claude::paths::is_absolute_anywhere(p)
        && !p.starts_with('~')
        && !p.split(['/', '\\']).any(|c| c == "..")
}

/// Every problem of a template.
pub fn validate(t: &Template) -> Report {
    let mut r = Report::default();
    if !is_id(&t.id, TEMPLATE_ID_MAX) {
        r.error(
            "id",
            format!("Use lower case letters, digits and - (at most {TEMPLATE_ID_MAX})."),
        );
    }
    if t.name.trim().is_empty() {
        r.error("name", "Give the template a name.");
    } else if t.name.chars().count() > 80 {
        r.error("name", "At most 80 characters.");
    }
    if t.description.chars().count() > 2000 {
        r.error("description", "At most 2000 characters.");
    }
    if t.roles.is_empty() {
        r.error("roles", "A team needs at least one role.");
    }
    if t.roles.len() > MAX_ROLES {
        r.error("roles", format!("At most {MAX_ROLES} roles."));
    }
    let leads = t.roles.iter().filter(|x| x.lead).count();
    if !t.roles.is_empty() && leads == 0 {
        r.error("roles", "Make one role the lead (\"lead\": true).");
    }
    if leads > 1 {
        r.error("roles", "Only one role can be the lead.");
    }
    let mut seen: Vec<&str> = Vec::new();
    for (i, role) in t.roles.iter().enumerate() {
        let at = |f: &str| format!("roles[{i}].{f}");
        if !is_id(&role.id, ROLE_ID_MAX) {
            r.error(
                at("id"),
                format!("Use lower case letters, digits and - (at most {ROLE_ID_MAX})."),
            );
        } else if seen.contains(&role.id.as_str()) {
            r.error(at("id"), format!("Two roles are called {:?}.", role.id));
        }
        seen.push(&role.id);
        if role.title.trim().is_empty() {
            r.error(at("title"), "Give the role a title.");
        } else if role.title.chars().count() > 60 {
            r.error(at("title"), "At most 60 characters.");
        }
        if role.description.chars().count() > 300 {
            r.error(at("description"), "At most 300 characters.");
        }
        if role.charter.chars().count() > CHARTER_MAX {
            r.error(at("charter"), format!("At most {CHARTER_MAX} characters."));
        }
        if let Some(m) = &role.model {
            if crate::claude::owned::validate_model(m).is_err() {
                r.error(at("model"), format!("{m:?} is not a model name."));
            }
        }
        if let Some(e) = &role.effort {
            if !EFFORT_LEVELS.contains(&e.as_str()) {
                r.error(
                    at("effort"),
                    format!("Use one of {}.", EFFORT_LEVELS.join(", ")),
                );
            }
        }
        if let Some(m) = &role.permission_mode {
            if !PERMISSION_MODES.contains(&m.as_str()) {
                r.error(
                    at("permissionMode"),
                    format!("Use one of {}.", PERMISSION_MODES.join(", ")),
                );
            } else if !SAFE_MODES.contains(&m.as_str()) {
                r.warn(
                    at("permissionMode"),
                    "Runs without permission prompts: starting this team needs the dangerous \
                     right (this computer, or a device allowed dangerous actions).",
                );
            }
        }
        for (j, p) in role.paths.iter().enumerate() {
            if !is_inside_relative(p) {
                r.error(
                    format!("roles[{i}].paths[{j}]"),
                    "A path glob is relative to the project (no leading /, ~ or ..).",
                );
            }
        }
        if role.paths.len() > 100 {
            r.error(at("paths"), "At most 100 globs.");
        }
        if role.read_only && !role.paths.is_empty() {
            r.warn(
                at("paths"),
                "A read-only role changes nothing: its paths are unused.",
            );
        }
        if role.lead && role.worktree {
            r.warn(
                at("worktree"),
                "The lead usually integrates in the main checkout, not a worktree.",
            );
        }
        if role.read_only && role.worktree {
            r.warn(at("worktree"), "A read-only role needs no worktree.");
        }
        if let Some(b) = role.budget_usd {
            if !(b > 0.0 && b <= BUDGET_MAX_USD) {
                r.error(
                    at("budgetUsd"),
                    format!("A budget above $0 and at most ${BUDGET_MAX_USD:.0}."),
                );
            }
        }
        for (j, d) in role.add_dirs.iter().enumerate() {
            if !is_inside_relative(d) || d.contains(['*', '?', '[']) {
                r.error(
                    format!("roles[{i}].addDirs[{j}]"),
                    "A folder relative to the project (no leading /, ~ or .., no globs).",
                );
            }
        }
        if role.tools.len() > 60 {
            r.error(at("tools"), "At most 60 tools.");
        }
        for (j, tool) in role.tools.iter().enumerate() {
            let ok = !tool.trim().is_empty()
                && tool.len() <= 200
                && !tool.starts_with('-')
                && !tool.chars().any(|c| c.is_control());
            if !ok {
                r.error(format!("roles[{i}].tools[{j}]"), "Not a tool name.");
            }
        }
        for (j, c) in role.classes.iter().enumerate() {
            if !TASK_CLASSES.contains(&c.as_str()) {
                r.warn(
                    format!("roles[{i}].classes[{j}]"),
                    format!(
                        "{c:?} is not a task class ({}); it is ignored.",
                        TASK_CLASSES.join(", ")
                    ),
                );
            }
        }
    }
    if let Some(m) = &t.integration.merge_by {
        if !t.roles.iter().any(|x| &x.id == m) {
            r.error("integration.mergeBy", format!("No role is called {m:?}."));
        }
    }
    if let Some(b) = &t.integration.branch {
        let ok = !b.trim().is_empty()
            && b.len() <= 200
            && !b.starts_with('-')
            && !b.chars().any(|c| c.is_whitespace() || c.is_control());
        if !ok {
            r.error("integration.branch", "Not a branch name.");
        }
    }
    if t.integration
        .gate
        .as_deref()
        .is_some_and(|g| g.len() > 2000)
    {
        r.error("integration.gate", "At most 2000 characters.");
    }
    if t.idle_stop_minutes.is_some_and(|m| m > 24 * 60) {
        r.error("idleStopMinutes", "At most 1440 (a day); 0 never stops.");
    }
    r
}

/// Parse a template from JSON text: the template, or why it is not one.
pub fn parse(text: &str) -> Result<Template, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("Not JSON: {e}"))?;
    if !v.is_object() {
        return Err("A template is a JSON object.".into());
    }
    serde_json::from_value(v).map_err(|e| format!("Not a template: {e}"))
}

// ─────────────────────────────── built-ins ───────────────────────────────

/// What every coding worker's charter ends with.
const WORKER_HABITS: &str = "Work in small, reviewable steps. Run the tests that cover what you \
change before you report. When the packet is unclear or you need a file another role owns, \
ask the lead instead of guessing.";

#[allow(clippy::too_many_arguments)]
fn role(
    id: &str,
    title: &str,
    description: &str,
    charter: &str,
    model: &str,
    effort: &str,
    mode: &str,
    paths: &[&str],
) -> RoleDef {
    RoleDef {
        id: id.into(),
        title: title.into(),
        description: description.into(),
        charter: charter.into(),
        model: Some(model.into()),
        effort: Some(effort.into()),
        permission_mode: Some(mode.into()),
        paths: paths.iter().map(|p| p.to_string()).collect(),
        ..Default::default()
    }
}

fn classes(list: &[&str]) -> Vec<String> {
    list.iter().map(|c| c.to_string()).collect()
}

fn software_delivery() -> Template {
    let mut lead = role(
        "lead",
        "Lead",
        "Plans the work, writes task packets, reviews and integrates.",
        "You run the delivery of features on this project. Break each request into tasks one \
         role can finish alone, write a packet for each, review what comes back and merge it.",
        "opus",
        "high",
        "default",
        &["docs/**", "*.md"],
    );
    lead.lead = true;
    lead.autostart = true;
    lead.classes = classes(&["plan", "architecture", "review"]);
    let mut frontend = role(
        "frontend",
        "Frontend",
        "Builds the user interface.",
        &format!("You build the user interface: components, pages, styles. {WORKER_HABITS}"),
        "sonnet",
        "medium",
        "acceptEdits",
        &[
            "src/app/**",
            "src/components/**",
            "src/pages/**",
            "src/styles/**",
            "frontend/**",
            "web/**",
            "public/**",
        ],
    );
    frontend.worktree = true;
    frontend.budget_usd = Some(5.0);
    frontend.classes = classes(&["implement", "test", "fix"]);
    let mut backend = role(
        "backend",
        "Backend & API",
        "Builds the server side and the API.",
        &format!("You build the server side: the API, data and services. {WORKER_HABITS}"),
        "sonnet",
        "medium",
        "acceptEdits",
        &[
            "src/server/**",
            "src/api/**",
            "server/**",
            "backend/**",
            "api/**",
            "src-tauri/**",
            "migrations/**",
        ],
    );
    backend.worktree = true;
    backend.budget_usd = Some(5.0);
    backend.classes = classes(&["implement", "test", "fix"]);
    let mut scout = role(
        "scout",
        "Scout",
        "Finds things in the code and reports; changes nothing.",
        "You answer questions about the code base: where things are, how they work, what a \
         change would touch. You change nothing; report findings in task_done.",
        "haiku",
        "low",
        "plan",
        &[],
    );
    scout.read_only = true;
    scout.classes = classes(&["search", "summarize", "classify"]);
    Template {
        id: "software-delivery".into(),
        name: "Software delivery".into(),
        description: "A lead with frontend and backend workers in their own worktrees, and a \
                      read-only scout. Adjust the paths to your project's layout (duplicate it \
                      first)."
            .into(),
        roles: vec![lead, frontend, backend, scout],
        integration: Integration {
            branch: Some("main".into()),
            merge_by: Some("lead".into()),
            gate: Some("npm test".into()),
            review: Some(true),
            ..Default::default()
        },
        idle_stop_minutes: Some(30),
        ..Default::default()
    }
}

fn content_production() -> Template {
    let mut planner = role(
        "planner",
        "Planner",
        "Plans the piece, assigns the parts and accepts the result.",
        "You plan a piece of content from the brief: its outline, the parts each role makes \
         and the order. Review every part against the brief before you accept it.",
        "opus",
        "high",
        "default",
        &["briefs/**", "*.md"],
    );
    planner.lead = true;
    planner.autostart = true;
    planner.classes = classes(&["plan", "review"]);
    let mut writer = role(
        "scriptwriter",
        "Scriptwriter",
        "Writes scripts and copy.",
        "You write scripts and copy that follow the brief's voice and length.",
        "sonnet",
        "medium",
        "acceptEdits",
        &["scripts/**", "copy/**", "drafts/**"],
    );
    writer.classes = classes(&["implement"]);
    let mut designer = role(
        "designer",
        "Designer",
        "Makes the visual assets.",
        "You make the visual assets the plan asks for (images, layouts, thumbnails) with the \
         tools you have, and describe each one in task_done.",
        "sonnet",
        "medium",
        "acceptEdits",
        &["assets/**", "design/**"],
    );
    designer.classes = classes(&["implement"]);
    let mut producer = role(
        "producer",
        "Producer",
        "Assembles and renders the final media.",
        "You assemble the parts into the final media (for example with ffmpeg or Remotion) \
         and check that it plays.",
        "sonnet",
        "medium",
        "acceptEdits",
        &["build/**", "renders/**", "media/**"],
    );
    producer.classes = classes(&["implement", "ops"]);
    let mut qa = role(
        "qa",
        "QA",
        "Checks every part against a rubric; changes nothing.",
        "You check each part against the brief and a rubric (accuracy, tone, length, \
         technical quality) and report pass or fail with reasons. You change nothing.",
        "opus",
        "medium",
        "plan",
        &[],
    );
    qa.read_only = true;
    qa.classes = classes(&["review"]);
    let mut publisher = role(
        "publisher",
        "Publisher",
        "Prepares the release: titles, descriptions, upload drafts.",
        "You prepare the release: titles, descriptions, tags and upload drafts. Never publish \
         anything without the planner's go.",
        "sonnet",
        "medium",
        "default",
        &["publish/**"],
    );
    publisher.classes = classes(&["ops"]);
    Template {
        id: "content-production".into(),
        name: "Content production".into(),
        description: "From a brief to a finished piece: plan, script, design, produce, check, \
                      publish."
            .into(),
        roles: vec![planner, writer, designer, producer, qa, publisher],
        integration: Integration {
            merge_by: Some("planner".into()),
            review: Some(true),
            ..Default::default()
        },
        idle_stop_minutes: Some(20),
        ..Default::default()
    }
}

fn research() -> Template {
    let mut lead = role(
        "lead",
        "Research lead",
        "Frames the questions and decides what is known.",
        "You frame the research questions, split them into tasks for the researcher, and \
         decide what is established. Every claim in the report needs a source.",
        "opus",
        "high",
        "default",
        &["reports/**", "*.md"],
    );
    lead.lead = true;
    lead.autostart = true;
    lead.classes = classes(&["plan", "review"]);
    let mut researcher = role(
        "researcher",
        "Researcher",
        "Finds and reads sources, takes notes.",
        "You find and read sources for one question at a time and write notes with a link and \
         the exact passage for every claim.",
        "sonnet",
        "medium",
        "acceptEdits",
        &["notes/**", "sources/**"],
    );
    researcher.classes = classes(&["search", "summarize"]);
    let mut synthesiser = role(
        "synthesiser",
        "Synthesiser",
        "Turns the notes into the report.",
        "You turn the notes into a clear report: findings first, then the evidence, then what \
         is still open.",
        "opus",
        "medium",
        "acceptEdits",
        &["reports/**"],
    );
    synthesiser.classes = classes(&["summarize", "implement"]);
    let mut checker = role(
        "fact-checker",
        "Fact-checker",
        "Checks every claim against its source; changes nothing.",
        "You check each claim of the report against its source and list what does not hold. \
         You change nothing.",
        "sonnet",
        "medium",
        "plan",
        &[],
    );
    checker.read_only = true;
    checker.classes = classes(&["review"]);
    Template {
        id: "research".into(),
        name: "Research & analysis".into(),
        description: "A lead, a researcher, a synthesiser and a fact-checker.".into(),
        roles: vec![lead, researcher, synthesiser, checker],
        integration: Integration {
            merge_by: Some("lead".into()),
            review: Some(true),
            ..Default::default()
        },
        idle_stop_minutes: Some(20),
        ..Default::default()
    }
}

fn browser_operations() -> Template {
    let mut operator = role(
        "operator",
        "Operator",
        "Carries out web tasks in the browser on allow-listed sites.",
        "You carry out web tasks in the user's browser on the sites the task names, one step \
         at a time, and stop before anything irreversible (payment, sending, deleting) for the \
         user's approval. Record what you did in runs/.",
        "sonnet",
        "medium",
        "default",
        &["runs/**", "*.md"],
    );
    operator.lead = true;
    operator.autostart = true;
    operator.classes = classes(&["ops", "plan"]);
    let mut verifier = role(
        "verifier",
        "Verifier",
        "Reads the pages again and confirms the result; changes nothing.",
        "You confirm that each web task really happened by reading the pages again. You change \
         nothing and never act on a page.",
        "haiku",
        "low",
        "plan",
        &[],
    );
    verifier.read_only = true;
    verifier.classes = classes(&["review"]);
    Template {
        id: "browser-operations".into(),
        name: "Browser operations".into(),
        description: "An operator that acts in the browser with approvals, and a read-only \
                      verifier."
            .into(),
        roles: vec![operator, verifier],
        integration: Integration {
            review: Some(true),
            ..Default::default()
        },
        idle_stop_minutes: Some(15),
        ..Default::default()
    }
}

fn documentation() -> Template {
    let mut editor = role(
        "editor",
        "Editor",
        "Plans the documentation and has the last word.",
        "You plan the documentation: what is missing, what is wrong, who writes what. Edit \
         for accuracy and plain language before you accept a page.",
        "opus",
        "high",
        "default",
        &["docs/**", "*.md"],
    );
    editor.lead = true;
    editor.autostart = true;
    editor.classes = classes(&["plan", "review"]);
    let mut librarian = role(
        "librarian",
        "Librarian",
        "Indexes what exists and finds what is undocumented; changes nothing.",
        "You index what the code and the docs already say and list what is undocumented or \
         out of date, with file references. You change nothing.",
        "haiku",
        "low",
        "plan",
        &[],
    );
    librarian.read_only = true;
    librarian.classes = classes(&["search", "summarize"]);
    let mut writer = role(
        "writer",
        "Writer",
        "Writes and updates pages.",
        "You write and update documentation pages from the packet and the code, with working \
         examples.",
        "sonnet",
        "medium",
        "acceptEdits",
        &["docs/**", "*.md"],
    );
    writer.worktree = true;
    writer.classes = classes(&["implement"]);
    Template {
        id: "documentation".into(),
        name: "Documentation".into(),
        description: "An editor, a read-only librarian and a writer in its own worktree.".into(),
        roles: vec![editor, librarian, writer],
        integration: Integration {
            branch: Some("main".into()),
            merge_by: Some("editor".into()),
            review: Some(true),
            ..Default::default()
        },
        idle_stop_minutes: Some(20),
        ..Default::default()
    }
}

/// The built-in templates, in the order the UI lists them.
pub fn builtins() -> Vec<Template> {
    vec![
        software_delivery(),
        content_production(),
        research(),
        browser_operations(),
        documentation(),
    ]
}

/// Whether an id is a built-in template's.
pub fn is_builtin(id: &str) -> bool {
    builtins().iter().any(|t| t.id == id)
}

// ─────────────────────────────── the store ───────────────────────────────

/// A template as listed: built in or the user's.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    #[serde(flatten)]
    pub template: Template,
    pub builtin: bool,
    /// What validation says now (a file written by an older or newer version
    /// may not pass).
    pub report: Report,
}

/// Why a template could not be saved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("{0}")]
    Invalid(String),
    #[error("The built-in template {0:?} cannot be changed; duplicate it first.")]
    Builtin(String),
    #[error("No template {0:?}.")]
    NotFound(String),
    #[error("A template called {0:?} exists already.")]
    Exists(String),
    #[error("Could not save the template: {0}")]
    Io(String),
}

impl TemplateError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_template",
            Self::Builtin(_) => "builtin_template",
            Self::NotFound(_) => "template_not_found",
            Self::Exists(_) => "template_exists",
            Self::Io(_) => "template_not_saved",
        }
    }
}

/// The user's templates (`<app data dir>/workflows/<id>.json`) and the
/// built-ins.
#[derive(Debug, Default)]
pub struct TemplateStore {
    dir: Option<PathBuf>,
    user: RwLock<BTreeMap<String, Template>>,
}

impl TemplateStore {
    /// Read the user's templates (an unreadable file is skipped and logged).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let dir = data_dir.map(|d| d.join("workflows"));
        let mut user = BTreeMap::new();
        if let Some(dir) = &dir {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for e in entries.flatten() {
                    let path = e.path();
                    if path.extension().and_then(|x| x.to_str()) != Some("json") {
                        continue;
                    }
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    match parse(&text) {
                        Ok(t) if is_id(&t.id, TEMPLATE_ID_MAX) && !is_builtin(&t.id) => {
                            user.insert(t.id.clone(), t);
                        }
                        Ok(_) => {
                            tracing::warn!(path = %path.display(), "template with a bad or built-in id ignored")
                        }
                        Err(e) => {
                            tracing::warn!(path = %path.display(), error = %e, "unreadable template ignored")
                        }
                    }
                }
            }
        }
        Self {
            dir,
            user: RwLock::new(user),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, Template>> {
        self.user.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<String, Template>> {
        self.user.write().unwrap_or_else(|p| p.into_inner())
    }

    /// Every template: the built-ins first, then the user's by name.
    pub fn all(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = builtins()
            .into_iter()
            .map(|t| Entry {
                report: validate(&t),
                template: t,
                builtin: true,
            })
            .collect();
        let mut user: Vec<Template> = self.read().values().cloned().collect();
        user.sort_by_key(|t| t.name.to_lowercase());
        out.extend(user.into_iter().map(|t| Entry {
            report: validate(&t),
            template: t,
            builtin: false,
        }));
        out
    }

    /// One template and whether it is built in.
    pub fn get(&self, id: &str) -> Option<(Template, bool)> {
        if let Some(t) = builtins().into_iter().find(|t| t.id == id) {
            return Some((t, true));
        }
        self.read().get(id).cloned().map(|t| (t, false))
    }

    /// Create (`create: true`, the id must be free) or replace one of the
    /// user's templates. Validation errors refuse it.
    pub fn save(&self, t: Template, create: bool) -> Result<Template, TemplateError> {
        if is_builtin(&t.id) {
            return Err(TemplateError::Builtin(t.id));
        }
        let report = validate(&t);
        if let Some(e) = report.first_error() {
            return Err(TemplateError::Invalid(e));
        }
        let mut user = self.write();
        if create && user.contains_key(&t.id) {
            return Err(TemplateError::Exists(t.id));
        }
        if !create && !user.contains_key(&t.id) {
            return Err(TemplateError::NotFound(t.id));
        }
        if let Some(dir) = &self.dir {
            crate::persist::write_json_private(&dir.join(format!("{}.json", t.id)), &t)
                .map_err(|e| TemplateError::Io(e.to_string()))?;
        }
        user.insert(t.id.clone(), t.clone());
        Ok(t)
    }

    /// Remove one of the user's templates.
    pub fn remove(&self, id: &str) -> Result<(), TemplateError> {
        if is_builtin(id) {
            return Err(TemplateError::Builtin(id.to_string()));
        }
        let mut user = self.write();
        if user.remove(id).is_none() {
            return Err(TemplateError::NotFound(id.to_string()));
        }
        if let Some(dir) = &self.dir {
            let path = dir.join(format!("{id}.json"));
            if let Err(e) = std::fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(TemplateError::Io(e.to_string()));
                }
            }
        }
        Ok(())
    }

    /// A copy of a template under a new id (`<id>-copy`, `-copy-2`, …).
    pub fn duplicate(&self, id: &str, name: Option<&str>) -> Result<Template, TemplateError> {
        let (mut t, _) = self
            .get(id)
            .ok_or_else(|| TemplateError::NotFound(id.to_string()))?;
        let base: String = format!("{}-copy", t.id)
            .chars()
            .take(TEMPLATE_ID_MAX - 3)
            .collect();
        let taken = |c: &str| is_builtin(c) || self.read().contains_key(c);
        let mut candidate = base.clone();
        let mut n = 2;
        while taken(&candidate) {
            candidate = format!("{base}-{n}");
            n += 1;
        }
        t.id = candidate;
        t.name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.chars().take(80).collect(),
            None => format!("{} (copy)", t.name).chars().take(80).collect(),
        };
        self.save(t, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_builtin_is_valid_and_safe() {
        let all = builtins();
        assert_eq!(all.len(), 5);
        for t in &all {
            let r = validate(t);
            assert!(r.ok(), "{}: {:?}", t.id, r.errors);
            assert!(r.warnings.is_empty(), "{}: {:?}", t.id, r.warnings);
            assert!(t.lead().is_some(), "{}", t.id);
            assert!(!t.needs_dangerous(), "{}: built-ins never bypass", t.id);
            for role in &t.roles {
                assert!(!role.charter.is_empty(), "{}/{}", t.id, role.id);
                assert!(role.model.is_some());
            }
        }
        let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "software-delivery",
                "content-production",
                "research",
                "browser-operations",
                "documentation"
            ]
        );
    }

    #[test]
    fn parsing_is_tolerant_and_keeps_unknown_fields() {
        let t = parse(
            &json!({
                "id": "mine",
                "name": "Mine",
                "roles": [
                    {"id": "boss", "title": "Boss", "lead": true, "future": {"x": 1}},
                    {"id": "dev", "title": "Dev", "paths": ["src/**"], "worktree": true}
                ],
                "integration": {"gate": "make test", "mergeQueue": true},
                "colour": "teal"
            })
            .to_string(),
        )
        .unwrap();
        assert!(validate(&t).ok());
        assert_eq!(t.roles[0].extra["future"]["x"], 1);
        assert_eq!(t.integration.extra["mergeQueue"], true);
        assert_eq!(t.extra["colour"], "teal");
        assert_eq!(t.roles[1].mode(), "default");
        assert_eq!(t.idle_stop_minutes(), DEFAULT_IDLE_STOP_MINUTES);
        // Round trip keeps them.
        let back = serde_json::to_value(&t).unwrap();
        assert_eq!(back["colour"], "teal");
        assert_eq!(back["roles"][0]["future"]["x"], 1);
        assert!(
            back["roles"][1].get("readOnly").is_none(),
            "defaults stay out"
        );
        assert!(parse("[]").is_err());
        assert!(parse("{nope").is_err());
        assert!(parse(r#"{"roles": "x"}"#).is_err());
    }

    #[test]
    fn validation_reports_every_problem() {
        let t = Template {
            id: "Bad Id".into(),
            name: " ".into(),
            roles: vec![
                RoleDef {
                    id: "a".into(),
                    title: "A".into(),
                    lead: true,
                    model: Some("--danger".into()),
                    effort: Some("turbo".into()),
                    permission_mode: Some("yolo".into()),
                    paths: vec!["/etc/**".into(), "../x/**".into(), "ok/**".into()],
                    budget_usd: Some(0.0),
                    add_dirs: vec!["~/x".into()],
                    ..Default::default()
                },
                RoleDef {
                    id: "a".into(),
                    title: String::new(),
                    lead: true,
                    permission_mode: Some("bypassPermissions".into()),
                    classes: vec!["implement".into(), "dance".into()],
                    ..Default::default()
                },
            ],
            integration: Integration {
                merge_by: Some("nobody".into()),
                ..Default::default()
            },
            idle_stop_minutes: Some(5000),
            ..Default::default()
        };
        let r = validate(&t);
        let paths: Vec<&str> = r.errors.iter().map(|i| i.path.as_str()).collect();
        for want in [
            "id",
            "name",
            "roles",
            "roles[0].model",
            "roles[0].effort",
            "roles[0].permissionMode",
            "roles[0].paths[0]",
            "roles[0].paths[1]",
            "roles[0].budgetUsd",
            "roles[0].addDirs[0]",
            "roles[1].id",
            "roles[1].title",
            "integration.mergeBy",
            "idleStopMinutes",
        ] {
            assert!(paths.contains(&want), "missing {want} in {paths:?}");
        }
        assert!(!paths.contains(&"roles[0].paths[2]"));
        let warned: Vec<&str> = r.warnings.iter().map(|i| i.path.as_str()).collect();
        assert!(warned.contains(&"roles[1].permissionMode"));
        assert!(warned.contains(&"roles[1].classes[1]"));
        assert!(t.needs_dangerous());
        assert!(r.first_error().is_some());
    }

    #[test]
    fn the_store_keeps_user_templates_private_and_builtins_fixed() {
        let dir = tempfile::tempdir().unwrap();
        let store = TemplateStore::load(Some(dir.path()));
        assert_eq!(store.all().len(), 5);
        assert_eq!(
            store.save(software_delivery(), true).unwrap_err().code(),
            "builtin_template"
        );
        let copy = store.duplicate("software-delivery", None).unwrap();
        assert_eq!(copy.id, "software-delivery-copy");
        assert_eq!(copy.name, "Software delivery (copy)");
        let again = store.duplicate("software-delivery", Some("Ours")).unwrap();
        assert_eq!(again.id, "software-delivery-copy-2");
        let file = dir.path().join("workflows/software-delivery-copy.json");
        assert!(file.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // An invalid edit is refused; a valid one is saved.
        let mut bad = copy.clone();
        bad.roles.clear();
        assert_eq!(
            store.save(bad, false).unwrap_err().code(),
            "invalid_template"
        );
        let mut good = copy.clone();
        good.name = "Renamed".into();
        store.save(good, false).unwrap();
        // A fresh store reads them back; an unreadable file is skipped.
        std::fs::write(dir.path().join("workflows/broken.json"), "{nope").unwrap();
        let reread = TemplateStore::load(Some(dir.path()));
        assert_eq!(
            reread.get("software-delivery-copy").unwrap().0.name,
            "Renamed"
        );
        assert_eq!(reread.all().len(), 7);
        reread.remove("software-delivery-copy").unwrap();
        assert!(!file.exists());
        assert_eq!(
            reread.remove("software-delivery").unwrap_err().code(),
            "builtin_template"
        );
        assert_eq!(
            reread.remove("missing").unwrap_err().code(),
            "template_not_found"
        );
        // Without a data dir nothing touches the disk.
        let mem = TemplateStore::load(None);
        mem.duplicate("research", None).unwrap();
        assert_eq!(mem.all().len(), 6);
    }
}
