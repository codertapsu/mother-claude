//! Where a moved session works on this machine (decision 4) — in one place,
//! so roots (§5.12, `machines/roots.rs`) can take over from folder pairs
//! here alone.

use std::path::Path;

use crate::claude::paths::{self, LocalPair, PathForm};
use crate::state::AppState;

/// The folder a session goes on in here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// The folder, when one was found (it exists, as this machine writes
    /// paths).
    pub path: Option<String>,
    /// `chosen` | `pair` | `hint` | `same`.
    pub how: &'static str,
    /// Folders here that may be the project, to choose from when none was.
    pub candidates: Vec<String>,
}

/// Why a chosen folder is no folder here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadChoice(pub String);

fn usable(p: &str) -> bool {
    PathForm::of_path(p) == Some(PathForm::here()) && Path::new(p).is_dir()
}

/// Where a session that worked in `folder` on machine `from` goes on here:
/// the folder the user chose (`chosen`); else — until roots take this over —
/// the folder a folder pair between the two machines maps it to; else where
/// the source knows it goes (`hint`: it came from here); else the same path
/// when it is a folder here; else none, with candidates (folders here of
/// sessions and trusted projects that look like it).
pub fn folder_there(
    state: &AppState,
    from: &str,
    folder: &str,
    chosen: Option<&str>,
    hint: Option<&str>,
) -> Result<Found, BadChoice> {
    if let Some(c) = chosen.map(str::trim).filter(|c| !c.is_empty()) {
        let c = paths::trim_end_separators(c).to_string();
        if !usable(&c) {
            return Err(BadChoice(format!(
                "{c} is not a folder on this {}.",
                paths::machine_noun()
            )));
        }
        return Ok(Found {
            path: Some(c),
            how: "chosen",
            candidates: Vec::new(),
        });
    }
    if let Some(m) = map_by_pairs(state, from, folder).filter(|m| usable(m)) {
        return Ok(Found {
            path: Some(m),
            how: "pair",
            candidates: Vec::new(),
        });
    }
    if let Some(h) = hint.map(paths::trim_end_separators).filter(|h| usable(h)) {
        return Ok(Found {
            path: Some(h.to_string()),
            how: "hint",
            candidates: Vec::new(),
        });
    }
    if usable(folder) {
        return Ok(Found {
            path: Some(paths::trim_end_separators(folder).to_string()),
            how: "same",
            candidates: Vec::new(),
        });
    }
    Ok(Found {
        path: None,
        how: "",
        candidates: candidates(state, folder),
    })
}

/// `folder` (machine `from`'s) mapped here by the folder pairs this machine
/// knows between the two (the longest match wins).
fn map_by_pairs(state: &AppState, from: &str, folder: &str) -> Option<String> {
    let view = crate::claude::backup::ops::pairs_view(state);
    let me = view.machine.clone()?;
    let local: Vec<LocalPair> = view
        .pairs
        .iter()
        .filter_map(|p| {
            let here = p.folders.iter().find(|s| s.machine == me)?;
            let there = p.folders.iter().find(|s| s.machine == from)?;
            Some(LocalPair {
                here: here.folder.clone(),
                there: there.folder.clone(),
            })
        })
        .collect();
    paths::map_here(folder, &local)
}

/// Folders here that may be `folder` (another machine's).
fn candidates(state: &AppState, folder: &str) -> Vec<String> {
    let mut known: Vec<String> = Vec::new();
    if let Ok(snapshot) = state.snapshot.try_read() {
        known.extend(snapshot.sessions.iter().map(|s| s.cwd.clone()));
    }
    known.extend(crate::claude::settings::trusted_projects(&state.home));
    paths::folder_candidates(folder, crate::claude::user_home_dir().as_deref(), &known)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder the user chose wins (when it is one here); then a pair
    /// (none here), then where the source says it came from, then the same
    /// path; else none, with candidates.
    #[tokio::test]
    async fn a_folder_is_chosen_then_hinted_then_the_same_path() {
        let tmp = tempfile::tempdir().unwrap();
        let st = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(tmp.path().join("home")),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        let here = dunce::canonicalize(tmp.path())
            .unwrap()
            .display()
            .to_string();
        let other = "0123456789abcdef0123456789abcdef";
        let away = "/nowhere/at/all/app";
        let f = folder_there(&st, other, away, Some(&here), None).unwrap();
        assert_eq!((f.path.as_deref(), f.how), (Some(here.as_str()), "chosen"));
        let missing = format!("{here}-missing");
        assert!(folder_there(&st, other, away, Some(&missing), None).is_err());
        let f = folder_there(&st, other, away, None, Some(&here)).unwrap();
        assert_eq!((f.path.as_deref(), f.how), (Some(here.as_str()), "hint"));
        let f = folder_there(&st, other, &here, None, Some(&missing)).unwrap();
        assert_eq!((f.path.as_deref(), f.how), (Some(here.as_str()), "same"));
        let f = folder_there(&st, other, away, None, None).unwrap();
        assert!(f.path.is_none());
    }
}
