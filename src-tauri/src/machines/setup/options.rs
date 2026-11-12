//! The code options (D10; docs/PLAN-MACHINES.md §5.10 "Where each choice is
//! made", §6): the setup chooses the defaults for every move
//! (`settings.code` — machine to machine or through your git remote, the
//! project folder or always a worktree, keeping hidden refs on the remote —
//! offered to the other computers like the relay); a project may change
//! them (`projects`, this computer's own), and each move may change them
//! again. A project may also carry a recipe: what to run after a move, per
//! system (it joins the move note as a request to Claude, never run by
//! Mother Claude — Phase 3a decision 8), and the files git ignores that
//! travel with it. **Setting files to carry is a dangerous act**: they are
//! the ones that hold secrets (`.env.local`), so adding one needs the
//! dangerous right; removing one does not.
//!
//! A project's settings are kept under `rt_…/<its path in the root>` when a
//! root holds it on this computer — so they follow the root when its folder
//! changes — else under its folder. A move asks [`effective`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::folders;
use crate::claude::paths::{self, PathForm};
use crate::machines::roots::{self, Root};
use crate::machines::store::{
    is_project_key, AfterMove, CodeApply, CodeOverride, CodeSettings, CodeTransfer, MachinesFile,
    ProjectSettings, Recipe,
};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Commands per system, and characters per command, at most.
pub const MAX_COMMANDS: usize = 20;
pub const MAX_COMMAND: usize = 300;
/// Files a project carries at most.
pub const MAX_CARRIED: usize = 50;
/// Projects with settings, at most.
pub const MAX_PROJECTS: usize = 500;

// ─── what a move uses ──────────────────────────────────────────────────────────

/// The options in force for a project.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effective {
    pub code: CodeSettings,
    /// Its recipe, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe: Option<Recipe>,
    /// The computers its sessions keep a copy on (§5.9).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<String>,
    /// The key its own settings are kept under, when it has some.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// The key a project's settings are kept under: `rt_…/<path in the root>`
/// for the deepest live root holding it on this computer, else its folder.
pub fn key_for(roots_now: &[Root], here: &str, project: &str) -> String {
    roots_now
        .iter()
        .filter(|r| r.live())
        .filter_map(|r| {
            let base = r.path_of(here)?;
            let rel = folders::rel_inside(project, base)?;
            Some((paths::depth(base), format!("{}/{rel}", r.id)))
        })
        .max_by_key(|(d, _)| *d)
        .map(|(_, k)| k)
        .unwrap_or_else(|| project.to_string())
}

/// `rel` (`/`-separated) inside `base`, written the way `base`'s system
/// writes paths.
fn join(base: &str, rel: &str) -> String {
    let win = paths::is_windows_absolute(base);
    let sep = if win { '\\' } else { '/' };
    let mut out = paths::trim_end_separators(base).to_string();
    if !out.ends_with(sep) {
        out.push(sep);
    }
    if win {
        out.push_str(&rel.replace('/', "\\"));
    } else {
        out.push_str(rel);
    }
    out
}

/// The folder on this computer a key names (`None`: its root is gone, or
/// names no folder here).
pub fn folder_of(roots_now: &[Root], here: &str, key: &str) -> Option<String> {
    match key.split_once('/') {
        Some((id, rel)) if roots::is_root_id(id) => {
            let base = roots_now
                .iter()
                .find(|r| r.live() && r.id == id)?
                .path_of(here)?;
            Some(join(base, rel))
        }
        _ => paths::is_absolute_anywhere(key).then(|| key.to_string()),
    }
}

/// A project's own settings and the key they are under: by its root, else
/// by its folder (kept that way before a root held it).
pub fn lookup(file: &MachinesFile, here: &str, project: &str) -> Option<(String, ProjectSettings)> {
    let key = key_for(&file.roots, here, project);
    if let Some(p) = file.projects.get(&key) {
        return Some((key, p.clone()));
    }
    file.projects
        .iter()
        .find(|(k, _)| paths::is_absolute_anywhere(k) && paths::same_path(k, project))
        .map(|(k, p)| (k.clone(), p.clone()))
}

/// What a move of a session in `project` (a folder of this computer) uses:
/// the project's own choices over the settings', and its recipe. Each move
/// may still change them.
pub fn effective(state: &AppState, project: &str) -> Effective {
    let here = roots::here_id(state).unwrap_or_default();
    state
        .machines
        .store_read(|f| match lookup(f, &here, project) {
            Some((key, p)) => Effective {
                code: p.code.over(&f.settings.code),
                recipe: p.recipe.filter(|r| !r.is_empty()),
                copies: p
                    .copies
                    .into_iter()
                    .filter(|m| f.peer(m).is_some())
                    .collect(),
                key: Some(key),
            },
            None => Effective {
                code: f.settings.code.clone(),
                recipe: None,
                copies: Vec::new(),
                key: None,
            },
        })
}

// ─── changing them ─────────────────────────────────────────────────────────────

/// `code` in `PUT /api/machines/settings`: the defaults, each field
/// optional (a field left out keeps its value).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeInput {
    #[serde(default)]
    pub transfer: Option<CodeTransfer>,
    #[serde(default)]
    pub apply: Option<CodeApply>,
    #[serde(default)]
    pub keep_remote_refs: Option<bool>,
}

impl CodeInput {
    /// `current` with this input over it (fields a newer build keeps stay).
    pub fn over(&self, current: &CodeSettings) -> CodeSettings {
        CodeSettings {
            transfer: self.transfer.unwrap_or(current.transfer),
            apply: self.apply.unwrap_or(current.apply),
            keep_remote_refs: self.keep_remote_refs.unwrap_or(current.keep_remote_refs),
            extra: current.extra.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AfterMoveInput {
    #[serde(default)]
    pub macos: Vec<String>,
    #[serde(default)]
    pub windows: Vec<String>,
    #[serde(default)]
    pub linux: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeInput {
    #[serde(default)]
    pub after_move: AfterMoveInput,
    #[serde(default)]
    pub carry_ignored: Vec<String>,
}

/// One project's settings, as asked for: the whole of them (a field left
/// out follows the defaults; no recipe, none).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectInput {
    /// The project's folder on this computer.
    pub project: String,
    #[serde(default)]
    pub code: CodeInput,
    #[serde(default)]
    pub recipe: Option<RecipeInput>,
    /// The computers its sessions keep a copy on (§5.9); left out, it stays
    /// as it was.
    #[serde(default)]
    pub copies: Option<Vec<String>>,
}

/// `projects` in `PUT /api/machines/settings`: projects to set and to
/// clear (by folder, or by the key they are listed under), all or nothing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectsChange {
    #[serde(default)]
    pub upsert: Vec<ProjectInput>,
    #[serde(default)]
    pub remove: Vec<String>,
}

/// A command to run after a move: one line, 1 to 300 characters.
pub fn clean_command(raw: &str) -> Result<String, String> {
    let c = raw.trim();
    if c.is_empty() {
        return Err("A command is empty.".into());
    }
    if c.chars().count() > MAX_COMMAND {
        return Err(format!("A command has at most {MAX_COMMAND} characters."));
    }
    if c.chars().any(char::is_control) {
        return Err("A command is one line.".into());
    }
    Ok(c.to_string())
}

/// A file a move carries although git ignores it: a path inside the
/// project, `/`-separated, never absolute, `..`, inside `.git` or a
/// pattern.
pub fn clean_carried(raw: &str) -> Result<String, String> {
    let t = raw.trim().replace('\\', "/");
    let t = t.trim_start_matches("./");
    if t.is_empty() {
        return Err("A file to carry is empty.".into());
    }
    if t.chars().count() > 260 || t.chars().any(char::is_control) {
        return Err(format!("{t} is not a file's path."));
    }
    if t.starts_with('/') || t.starts_with('~') || paths::is_absolute_anywhere(t) || t.contains(':')
    {
        return Err(format!("{t} must be a path inside the project."));
    }
    if t.contains(['*', '?', '[', ']']) {
        return Err(format!("{t}: name the file itself, not a pattern."));
    }
    let parts: Vec<&str> = t.trim_end_matches('/').split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return Err(format!("{t} must be a path inside the project."));
    }
    if parts[0].eq_ignore_ascii_case(".git") {
        return Err("Git's own folder never travels this way.".into());
    }
    Ok(parts.join("/"))
}

fn commands(list: &[String], os: &str) -> ApiResult<Vec<String>> {
    if list.len() > MAX_COMMANDS {
        return Err(ApiError::unprocessable(
            "machines_bad_recipe",
            format!("At most {MAX_COMMANDS} commands for {os}."),
        ));
    }
    list.iter()
        .map(|c| clean_command(c).map_err(|m| ApiError::unprocessable("machines_bad_recipe", m)))
        .collect()
}

/// A recipe as asked for, checked.
pub fn clean_recipe(r: &RecipeInput) -> ApiResult<Recipe> {
    if r.carry_ignored.len() > MAX_CARRIED {
        return Err(ApiError::unprocessable(
            "machines_bad_recipe",
            format!("At most {MAX_CARRIED} files to carry."),
        ));
    }
    let mut carried: Vec<String> = Vec::new();
    for f in &r.carry_ignored {
        let c = clean_carried(f).map_err(|m| ApiError::unprocessable("machines_bad_recipe", m))?;
        if !carried.contains(&c) {
            carried.push(c);
        }
    }
    Ok(Recipe {
        after_move: AfterMove {
            macos: commands(&r.after_move.macos, "macOS")?,
            windows: commands(&r.after_move.windows, "Windows")?,
            linux: commands(&r.after_move.linux, "Linux")?,
            extra: BTreeMap::new(),
        },
        carry_ignored: carried,
        extra: BTreeMap::new(),
    })
}

/// The files `after` carries that `before` did not: setting them needs the
/// dangerous right (they are the ones that hold secrets).
pub fn newly_carried(before: Option<&Recipe>, after: Option<&Recipe>) -> Vec<String> {
    let had: &[String] = before.map_or(&[], |r| &r.carry_ignored);
    after
        .map_or(&[][..], |r| &r.carry_ignored)
        .iter()
        .filter(|f| !had.contains(f))
        .cloned()
        .collect()
}

/// What a change did, for its audit line.
#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    pub set: Vec<String>,
    pub removed: Vec<String>,
    /// Files newly carried (the dangerous part).
    pub carried: Vec<String>,
    /// Projects whose copies changed (to computers that keep them).
    pub copies: Vec<String>,
}

/// Applies `change` to `file`'s projects as this computer (`here`) at
/// `now`: all of it, or nothing and why.
pub fn apply(
    file: &mut MachinesFile,
    here: &str,
    change: &ProjectsChange,
    now: i64,
) -> ApiResult<Applied> {
    if change.upsert.len() + change.remove.len() > 100 {
        return Err(ApiError::unprocessable(
            "machines_bad_request",
            "At most 100 changes at once.",
        ));
    }
    let mut out = Applied::default();
    for key in &change.remove {
        let found: Vec<String> = file
            .projects
            .keys()
            .filter(|k| {
                *k == key
                    || (paths::is_absolute_anywhere(key)
                        && (paths::same_path(k, key)
                            || folder_of(&file.roots, here, k)
                                .is_some_and(|f| paths::same_path(&f, key))))
            })
            .cloned()
            .collect();
        for k in found {
            file.projects.remove(&k);
            out.removed.push(k);
        }
    }
    for p in &change.upsert {
        let folder = paths::clean_folder(&p.project, PathForm::here()).map_err(|why| {
            ApiError::unprocessable("machines_bad_project", format!("That project: {why}."))
                .with("project", p.project.as_str())
        })?;
        let key = key_for(&file.roots, here, &folder);
        if !is_project_key(&key) {
            return Err(ApiError::unprocessable(
                "machines_bad_project",
                "That project's folder is not one Mother Claude can keep.",
            ));
        }
        let before = lookup(file, here, &folder);
        let recipe = match &p.recipe {
            Some(r) => Some(clean_recipe(r)?).filter(|r| !r.is_empty()),
            None => None,
        };
        out.carried.extend(newly_carried(
            before.as_ref().and_then(|(_, b)| b.recipe.as_ref()),
            recipe.as_ref(),
        ));
        // The computers its sessions keep a copy on: paired ones only.
        let copies = match &p.copies {
            Some(list) => {
                let mut out: Vec<String> = Vec::new();
                for m in list {
                    if file.peer(m).is_none() {
                        return Err(ApiError::not_found(
                            "machines_unknown",
                            "Not a paired computer.",
                        )
                        .with("machine", m.as_str()));
                    }
                    if !out.contains(m) {
                        out.push(m.clone());
                    }
                }
                out
            }
            None => before
                .as_ref()
                .map(|(_, b)| b.copies.clone())
                .unwrap_or_default(),
        };
        if before.as_ref().map(|(_, b)| &b.copies) != Some(&copies) && !copies.is_empty() {
            out.copies.push(key.clone());
        }
        // Kept under its folder before a root held it: under the root now.
        if let Some((old, _)) = &before {
            file.projects.remove(old);
        }
        let next = ProjectSettings {
            code: CodeOverride {
                transfer: p.code.transfer,
                apply: p.code.apply,
                keep_remote_refs: p.code.keep_remote_refs,
            },
            recipe,
            copies,
            updated_at: now,
            extra: before.map(|(_, b)| b.extra).unwrap_or_default(),
        };
        if next.is_empty() {
            out.removed.push(key);
        } else {
            file.projects.insert(key.clone(), next);
            out.set.push(key);
        }
    }
    if file.projects.len() > MAX_PROJECTS {
        return Err(ApiError::unprocessable(
            "machines_too_many_projects",
            format!("At most {MAX_PROJECTS} projects have their own settings."),
        ));
    }
    Ok(out)
}

/// `projects` in `PUT /api/machines/settings`, by `ctx`: checked on a copy
/// first; adding a file to carry needs the dangerous right; audited.
pub fn change(state: &AppState, ctx: &ClientContext, change: &ProjectsChange) -> ApiResult<()> {
    let here = roots::here_id(state)
        .ok_or_else(|| ApiError::internal("This computer's id could not be read."))?;
    let now = crate::machines::now();
    let dry = state
        .machines
        .store_read(|f| apply(&mut f.clone(), &here, change, now))?;
    if !dry.carried.is_empty() {
        crate::server::auth::require_dangerous(ctx, "Carrying files git ignores")?;
    }
    let applied = state.machines.store_write(|f| {
        let mut next = f.clone();
        let applied = apply(&mut next, &here, change, now)?;
        f.projects = next.projects;
        Ok::<_, ApiError>(applied)
    })?;
    if applied != Applied::default() {
        state.devices.audit.record(AuditEntry {
            detail: Some(json!({
                "set": applied.set,
                "removed": applied.removed,
                "carried": applied.carried.len(),
                "copies": applied.copies,
            })),
            ..AuditEntry::from_ctx(ctx, "machines.projects")
        });
    }
    Ok(())
}

/// Every project with its own settings: its key, its folder here (when it
/// names one), its root, its choices and its recipe.
pub fn view(state: &AppState) -> Vec<Value> {
    let here = roots::here_id(state).unwrap_or_default();
    state.machines.store_read(|f| {
        f.projects
            .iter()
            .map(|(key, p)| {
                let root = key
                    .split_once('/')
                    .filter(|(id, _)| roots::is_root_id(id))
                    .and_then(|(id, _)| f.roots.iter().find(|r| r.live() && r.id == id))
                    .map(|r| json!({ "id": r.id, "name": r.name }));
                json!({
                    "key": key,
                    "project": folder_of(&f.roots, &here, key),
                    "root": root,
                    "code": p.code,
                    "recipe": p.recipe,
                    "copies": p.copies,
                    "updatedAt": p.updated_at,
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PC: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn folder(parts: &[&str]) -> String {
        if cfg!(windows) {
            format!(r"D:\dev\{}", parts.join("\\"))
        } else {
            format!("/Users/m/dev/{}", parts.join("/"))
        }
    }

    fn root_file() -> MachinesFile {
        let base = if cfg!(windows) {
            r"D:\dev"
        } else {
            "/Users/m/dev"
        };
        MachinesFile {
            roots: vec![Root {
                id: "rt_1111111111111111".into(),
                name: "dev".into(),
                paths: [
                    (HERE.to_string(), base.to_string()),
                    (PC.to_string(), "/x/dev".to_string()),
                ]
                .into_iter()
                .collect(),
                updated_at: 1,
                by: HERE.into(),
                pair: None,
                removed: None,
                extra: BTreeMap::new(),
            }],
            ..MachinesFile::default()
        }
    }

    fn input(project: &str, code: CodeInput, recipe: Option<RecipeInput>) -> ProjectsChange {
        ProjectsChange {
            upsert: vec![ProjectInput {
                project: project.into(),
                code,
                recipe,
                copies: None,
            }],
            remove: vec![],
        }
    }

    #[test]
    fn a_project_under_a_root_is_kept_by_the_root_and_follows_it() {
        let mut f = root_file();
        let app = folder(&["app"]);
        let change = input(
            &app,
            CodeInput {
                transfer: Some(CodeTransfer::Remote),
                ..CodeInput::default()
            },
            None,
        );
        let done = apply(&mut f, HERE, &change, 5).unwrap();
        assert_eq!(done.set, ["rt_1111111111111111/app"]);
        assert_eq!(
            folder_of(&f.roots, HERE, "rt_1111111111111111/app").as_deref(),
            Some(app.as_str())
        );
        let (key, p) = lookup(&f, HERE, &app).unwrap();
        assert_eq!(key, "rt_1111111111111111/app");
        assert_eq!(p.code.over(&f.settings.code).transfer, CodeTransfer::Remote);
        assert_eq!(
            p.code.over(&f.settings.code).apply,
            CodeApply::Checkout,
            "the default"
        );

        // Outside any root: by its folder.
        let lone = if cfg!(windows) {
            r"E:\elsewhere\tool"
        } else {
            "/opt/tool"
        };
        let done = apply(
            &mut f,
            HERE,
            &input(
                lone,
                CodeInput {
                    apply: Some(CodeApply::Worktree),
                    ..CodeInput::default()
                },
                None,
            ),
            6,
        )
        .unwrap();
        assert_eq!(done.set, [lone]);
        // Empty settings remove the project.
        let done = apply(&mut f, HERE, &input(&app, CodeInput::default(), None), 7).unwrap();
        assert_eq!(done.removed, ["rt_1111111111111111/app"]);
        assert!(lookup(&f, HERE, &app).is_none());
        // Removing by folder.
        let done = apply(
            &mut f,
            HERE,
            &ProjectsChange {
                upsert: vec![],
                remove: vec![lone.to_string()],
            },
            8,
        )
        .unwrap();
        assert_eq!(done.removed, [lone]);
        assert!(f.projects.is_empty());
    }

    #[test]
    fn recipes_are_checked() {
        let ok = clean_recipe(&RecipeInput {
            after_move: AfterMoveInput {
                windows: vec![" npm ci ".into()],
                macos: vec!["npm ci".into()],
                linux: vec![],
            },
            carry_ignored: vec![
                "./.env.local".into(),
                r"config\local.json".into(),
                ".env.local".into(),
            ],
        })
        .unwrap();
        assert_eq!(ok.after_move.windows, ["npm ci"]);
        assert_eq!(ok.carry_ignored, [".env.local", "config/local.json"]);
        for bad in [
            "",
            "/etc/passwd",
            "../secret",
            "a/../b",
            ".git/config",
            "~/.ssh/id",
            r"C:\x",
            "*.env",
            "a\u{0}b",
        ] {
            assert!(clean_carried(bad).is_err(), "{bad:?}");
        }
        assert!(clean_command("rm -rf /\nmore").is_err(), "one line");
        assert!(clean_command(&"x".repeat(301)).is_err());
        let too_many = RecipeInput {
            after_move: AfterMoveInput {
                macos: vec!["make".into(); 21],
                ..AfterMoveInput::default()
            },
            carry_ignored: vec![],
        };
        assert!(clean_recipe(&too_many).is_err());
    }

    #[test]
    fn only_a_new_file_to_carry_is_the_dangerous_part() {
        let mut f = root_file();
        let app = folder(&["app"]);
        let with = |files: &[&str]| {
            input(
                &app,
                CodeInput::default(),
                Some(RecipeInput {
                    after_move: AfterMoveInput {
                        macos: vec!["npm ci".into()],
                        ..AfterMoveInput::default()
                    },
                    carry_ignored: files.iter().map(|s| s.to_string()).collect(),
                }),
            )
        };
        let first = apply(&mut f, HERE, &with(&[".env.local"]), 1).unwrap();
        assert_eq!(first.carried, [".env.local"]);
        let same = apply(&mut f, HERE, &with(&[".env.local"]), 2).unwrap();
        assert!(same.carried.is_empty(), "nothing new");
        let fewer = apply(&mut f, HERE, &with(&[]), 3).unwrap();
        assert!(fewer.carried.is_empty(), "carrying less is not dangerous");
        let more = apply(&mut f, HERE, &with(&[".env.local", "certs/dev.pem"]), 4).unwrap();
        assert_eq!(
            more.carried,
            [".env.local", "certs/dev.pem"],
            "both are new again"
        );
        assert_eq!(newly_carried(None, None), Vec::<String>::new());
    }

    #[test]
    fn a_bad_project_or_recipe_is_refused_whole() {
        let f = root_file();
        let mut change = input(
            &folder(&["app"]),
            CodeInput {
                apply: Some(CodeApply::Worktree),
                ..CodeInput::default()
            },
            None,
        );
        change.upsert.push(ProjectInput {
            project: "relative/path".into(),
            ..ProjectInput::default()
        });
        let e = apply(&mut f.clone(), HERE, &change, 1).unwrap_err();
        assert_eq!(e.code, "machines_bad_project");
        let mut change = input(
            &folder(&["app"]),
            CodeInput::default(),
            Some(RecipeInput {
                after_move: AfterMoveInput::default(),
                carry_ignored: vec!["../escape".into()],
            }),
        );
        change.remove.push("nothing".into());
        let e = apply(&mut f.clone(), HERE, &change, 1).unwrap_err();
        assert_eq!(e.code, "machines_bad_recipe");
    }

    #[test]
    fn the_defaults_take_input_field_by_field() {
        let current = CodeSettings::default();
        let next = CodeInput {
            transfer: Some(CodeTransfer::Remote),
            ..CodeInput::default()
        }
        .over(&current);
        assert_eq!(next.transfer, CodeTransfer::Remote);
        assert_eq!(next.apply, CodeApply::Checkout);
        assert!(serde_json::from_value::<CodeInput>(json!({ "transfer": "ftp" })).is_err());
        assert!(serde_json::from_value::<CodeInput>(json!({ "later": 1 })).is_err());
        assert_eq!(
            serde_json::from_value::<CodeInput>(json!({ "transfer": "direct" }))
                .unwrap()
                .transfer,
            Some(CodeTransfer::Bundle)
        );
    }
}
