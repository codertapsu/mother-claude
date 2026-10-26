//! Where conversations are on disk (docs/ACCOUNTS.md §14.2): every project
//! folder of every account's directory. Everything comes from the index —
//! which found the files by listing the directories — never from a request,
//! and no transcript is read here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::accounts::{AccountKind, DEFAULT_ID};
use super::model::SessionSummary;
use crate::state::AppState;

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The account a conversation in `dir_account`'s directory counts as. In the
/// shared folder (the Default's), while token accounts exist, the one it is
/// attributed to (docs/ACCOUNTS.md D8): a token account's conversations are
/// the shared folder's attributed to it, and one the attribution places in
/// another directory (a copy there is newer) is the Default's here.
/// Everywhere else, the directory's own account.
pub fn owner_in(state: &AppState, dir_account: &str, id: &str) -> String {
    if dir_account != DEFAULT_ID || !state.accounts.has_token_accounts() {
        return dir_account.to_string();
    }
    let acting = super::summaries::acting_account(state, id);
    match state.accounts.get(&acting) {
        Some(a) if a.kind == AccountKind::Token => acting,
        _ => DEFAULT_ID.to_string(),
    }
}

/// One project folder of one account (`GET /api/data/projects`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRow {
    /// The account whose conversations these are (a token account's are the
    /// shared folder's attributed to it).
    pub account: String,
    /// The folder the transcripts record (`projects/<enc>` decodes
    /// ambiguously, §1.4).
    pub cwd: String,
    /// Its last component.
    pub name: String,
    /// `projects/<enc>` in the account's directory, absolute.
    pub folder: String,
    /// Conversations.
    pub sessions: u32,
    /// Those of them that run now (a living pidfile, an owned session, a
    /// bridge conversation open in the host).
    pub live: u32,
    /// Their transcripts' and subagent files' sizes, as the index knows them.
    pub bytes: u64,
    pub last_activity_at: i64,
    #[serde(skip)]
    pub folder_path: PathBuf,
}

fn name_of(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(cwd)
        .to_string()
}

/// Every project folder that holds conversations, in every account's
/// directory: one row per (account, recorded folder), newest first. A
/// conversation counts where the index finds it (an id in two directories:
/// the most recently written copy). The recorded folder is the index's;
/// a conversation not indexed yet takes its summary's. Blocking.
pub fn projects(state: &AppState, summaries: &[SessionSummary]) -> Vec<ProjectRow> {
    let by_id: HashMap<&str, &SessionSummary> =
        summaries.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut rows: HashMap<(String, String), ProjectRow> = HashMap::new();
    let mut ids = state.index.session_ids();
    ids.sort();
    for id in ids {
        let Some(files) = state.index.session_files(&id) else {
            continue;
        };
        let (Some(_), Some(project_dir)) = (&files.main, &files.project_dir) else {
            continue;
        };
        let dir_account = files.account.as_deref().unwrap_or(DEFAULT_ID);
        let account = owner_in(state, dir_account, &id);
        let enc = project_dir.file_name().and_then(|n| n.to_str());
        let summary = by_id.get(id.as_str());
        let cwd = state
            .index
            .with_main(&id, |f| f.agg.project_cwd(enc).map(str::to_string))
            .flatten()
            .or_else(|| summary.map(|s| s.cwd.clone()).filter(|c| !c.is_empty()))
            .or_else(|| enc.map(super::home::best_effort_decode))
            .unwrap_or_default();
        let bytes = files.main_size + files.subagents.values().map(|a| a.size).sum::<u64>();
        let last = summary
            .and_then(|s| s.last_activity_at)
            .unwrap_or(files.main_mtime)
            .max(files.main_mtime);
        let live = super::transfer::ops::is_live(state, &id);
        let row = rows
            .entry((account.clone(), cwd.clone()))
            .or_insert_with(|| ProjectRow {
                account,
                name: name_of(&cwd),
                folder: text(project_dir),
                folder_path: project_dir.clone(),
                cwd: cwd.clone(),
                sessions: 0,
                live: 0,
                bytes: 0,
                last_activity_at: last,
            });
        // The folder the cwd names, when its conversations are in several.
        let names_cwd = |p: &Path| {
            p.file_name().and_then(|n| n.to_str()) == Some(super::encode_cwd(&cwd).as_str())
        };
        if !names_cwd(&row.folder_path) && names_cwd(project_dir) {
            row.folder = text(project_dir);
            row.folder_path = project_dir.clone();
        }
        row.sessions += 1;
        row.live += u32::from(live);
        row.bytes += bytes;
        row.last_activity_at = row.last_activity_at.max(last);
    }
    let mut out: Vec<ProjectRow> = rows.into_values().collect();
    out.sort_by(|a, b| {
        b.last_activity_at
            .cmp(&a.last_activity_at)
            .then_with(|| a.account.cmp(&b.account))
            .then_with(|| a.cwd.cmp(&b.cwd))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_named_after_its_folder() {
        assert_eq!(name_of("/Users/me/dev/app"), "app");
        assert_eq!(name_of("/"), "/");
    }
}
