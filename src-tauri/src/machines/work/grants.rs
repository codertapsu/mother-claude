//! Standing grants for runs (docs/PLAN-MACHINES.md Phase 4, decision 2):
//! *Always allow `cargo test` on DESKTOP-PC for this project*, attached
//! from a run's approval card by someone with the dangerous right, listed
//! and revoked on the Machines page. One covers a run when its project and
//! computer are the run's and its prefix starts the command **at a word
//! boundary** — `cargo test` covers `cargo test --lib`, never `cargo
//! test-foo` — and the command holds no shell metacharacter: anything that
//! could chain, substitute or redirect (`;`, `&`, `|`, a backtick, `$`,
//! parentheses, braces, `<`, `>`, a line break) makes every grant not
//! apply, so a prefix never smuggles a second command in. A command the
//! danger classifier flags is never covered (it always asks).
//!
//! Kept in `<app data>/machines/run-grants.json` (0600, written whole and
//! atomically), read entry by entry: one this build cannot read is skipped.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The file, under `<app data>/machines`.
pub const FILE: &str = "run-grants.json";
/// Its format.
pub const VERSION: u32 = 1;
/// Grants kept at most.
pub const MAX_GRANTS: usize = 200;
/// The longest prefix.
pub const MAX_PREFIX: usize = 200;

/// One standing grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunGrant {
    /// `rg_` and 12 hex characters.
    pub id: String,
    /// The project's top on this computer.
    pub project: String,
    #[serde(default)]
    pub project_name: String,
    /// The computer it may run on (its id).
    pub machine: String,
    /// The command prefix, words separated by one space.
    pub prefix: String,
    pub created_at: i64,
    /// Who attached it (a name).
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub used_at: Option<i64>,
    #[serde(default)]
    pub uses: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct FileShape {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    grants: Vec<Value>,
}

pub fn is_grant_id(s: &str) -> bool {
    s.strip_prefix("rg_")
        .is_some_and(|h| h.len() == 12 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn new_grant_id() -> String {
    let hex: String = crate::server::auth::random_bytes(6)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("rg_{hex}")
}

/// The characters that let a command line do more than run one program
/// with its arguments, in a POSIX shell or PowerShell.
const META: &[char] = &[
    ';', '&', '|', '`', '$', '(', ')', '{', '}', '<', '>', '\n', '\r', '\0',
];

/// Whether `command` holds a shell metacharacter ([`META`]): no standing
/// grant applies to it.
pub fn has_metachar(command: &str) -> bool {
    command.contains(META)
}

fn words(s: &str) -> Vec<&str> {
    s.split_whitespace().collect()
}

/// `prefix` written the one way grants keep it (words separated by one
/// space); `None` when it is empty, too long or holds a metacharacter.
pub fn clean_prefix(prefix: &str) -> Option<String> {
    let w = words(prefix);
    let joined = w.join(" ");
    (!w.is_empty() && joined.len() <= MAX_PREFIX && !has_metachar(&joined)).then_some(joined)
}

/// Whether `prefix` starts `command` at a word boundary: the command's
/// first words are the prefix's words exactly — and the command holds no
/// metacharacter.
pub fn prefix_matches(prefix: &str, command: &str) -> bool {
    if has_metachar(command) || has_metachar(prefix) {
        return false;
    }
    let p = words(prefix);
    let c = words(command);
    !p.is_empty() && c.len() >= p.len() && c[..p.len()] == p[..]
}

/// Programs that run whatever code they are given: a grant naming one
/// alone would cover anything, so none is offered for them.
const SHELLS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "csh",
    "tcsh",
    "pwsh",
    "powershell",
    "cmd",
    "env",
    "sudo",
    "doas",
    "xargs",
    "eval",
    "exec",
    "nohup",
    "time",
    "nice",
    "start-process",
    "invoke-expression",
    "iex",
    "osascript",
];

/// Interpreters: the prefix names what they run (`python -m pytest`, `node
/// scripts/build.js`), never the interpreter alone.
const INTERPRETERS: &[&str] = &[
    "python", "python3", "py", "node", "deno", "bun", "ruby", "perl", "php", "java", "dotnet",
    "npx", "pnpx", "bunx", "uvx", "uv",
];

/// Package managers whose `run` takes a script's name.
const RUNNERS: &[&str] = &["npm", "pnpm", "yarn", "bun", "uv", "poetry", "pdm"];

fn base_name(word: &str) -> String {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let base = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base);
    base.to_ascii_lowercase()
}

/// The prefix an approval card offers to always allow for `command`: the
/// program and its subcommand (`cargo test`, `npm run build`, `./gradlew
/// build`), what an interpreter runs (`python -m pytest`), never a shell
/// alone — and nothing for a command a grant could never cover.
pub fn suggested_prefix(command: &str) -> Option<String> {
    if has_metachar(command) {
        return None;
    }
    let w = words(command);
    let first = *w.first()?;
    let program = base_name(first);
    if SHELLS.contains(&program.as_str()) {
        return None;
    }
    let mut out = vec![first];
    let runs_script = RUNNERS.contains(&program.as_str())
        && w.get(1)
            .is_some_and(|s| matches!(*s, "run" | "run-script" | "exec" | "x"));
    if runs_script {
        // `npm run build`, `bun run test`: the script's name too.
        out.push(w[1]);
        let third = w.get(2).filter(|s| !s.starts_with('-'))?;
        out.push(third);
    } else if INTERPRETERS.contains(&program.as_str()) {
        // Its options, then the first thing it runs.
        for word in &w[1..] {
            out.push(word);
            if !word.starts_with('-') || out.len() >= 4 {
                break;
            }
        }
        if out.len() == 1 || out.last().is_some_and(|l| l.starts_with('-')) {
            return None;
        }
    } else if let Some(second) = w.get(1).filter(|s| !s.starts_with('-')) {
        out.push(second);
    }
    clean_prefix(&out.join(" "))
}

/// The standing grants (`MachinesState::work.grants`).
pub struct RunGrants {
    path: Option<PathBuf>,
    list: Mutex<Vec<RunGrant>>,
}

impl std::fmt::Debug for RunGrants {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunGrants").finish_non_exhaustive()
    }
}

impl RunGrants {
    pub fn load(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join("machines").join(FILE));
        let shape: FileShape = path
            .as_deref()
            .and_then(crate::persist::read_json)
            .unwrap_or_default();
        let list = shape
            .grants
            .into_iter()
            .filter_map(|v| serde_json::from_value::<RunGrant>(v).ok())
            .filter(|g| {
                is_grant_id(&g.id)
                    && crate::machines::ids::is_machine_id(&g.machine)
                    && clean_prefix(&g.prefix).as_deref() == Some(g.prefix.as_str())
                    && !g.project.is_empty()
            })
            .take(MAX_GRANTS)
            .collect();
        Self {
            path,
            list: Mutex::new(list),
        }
    }

    fn save(&self, list: &[RunGrant]) {
        let Some(path) = &self.path else { return };
        let shape = FileShape {
            version: VERSION,
            grants: list
                .iter()
                .filter_map(|g| serde_json::to_value(g).ok())
                .collect(),
        };
        if let Err(e) = crate::persist::write_json_private(path, &shape) {
            tracing::warn!(error = %e, "the standing grants for runs could not be saved");
        }
    }

    pub fn list(&self) -> Vec<RunGrant> {
        let mut l = self.list.lock().unwrap_or_else(|e| e.into_inner()).clone();
        l.sort_by_key(|g| std::cmp::Reverse(g.created_at));
        l
    }

    /// Adds one (the same project, computer and prefix: the one there).
    /// Its id.
    pub fn add(
        &self,
        project: &str,
        project_name: &str,
        machine: &str,
        prefix: &str,
        by: &str,
    ) -> Option<String> {
        let prefix = clean_prefix(prefix)?;
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(g) = list.iter().find(|g| {
            g.machine == machine
                && g.prefix == prefix
                && crate::claude::paths::same_path(&g.project, project)
        }) {
            return Some(g.id.clone());
        }
        if list.len() >= MAX_GRANTS {
            list.sort_by_key(|g| std::cmp::Reverse(g.created_at));
            list.truncate(MAX_GRANTS - 1);
        }
        let id = new_grant_id();
        list.push(RunGrant {
            id: id.clone(),
            project: project.to_string(),
            project_name: project_name.to_string(),
            machine: machine.to_string(),
            prefix,
            created_at: crate::machines::now(),
            created_by: crate::server::devices::sanitize_label(by, 64),
            used_at: None,
            uses: 0,
        });
        self.save(&list);
        Some(id)
    }

    /// Removes one; whether it was there.
    pub fn remove(&self, id: &str) -> Option<RunGrant> {
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        let at = list.iter().position(|g| g.id == id)?;
        let g = list.remove(at);
        self.save(&list);
        Some(g)
    }

    /// The grant that covers running `command` in `project` on `machine`
    /// (the longest prefix), counted as used.
    pub fn covering(&self, project: &str, machine: &str, command: &str) -> Option<RunGrant> {
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        let best = list
            .iter()
            .enumerate()
            .filter(|(_, g)| {
                g.machine == machine
                    && crate::claude::paths::same_path(&g.project, project)
                    && prefix_matches(&g.prefix, command)
            })
            .max_by_key(|(_, g)| g.prefix.len())
            .map(|(i, _)| i)?;
        let g = &mut list[best];
        g.used_at = Some(crate::machines::now());
        g.uses += 1;
        let out = g.clone();
        self.save(&list);
        Some(out)
    }
}

/// One grant as the Machines page shows it.
pub fn grant_view(state: &crate::state::AppState, g: &RunGrant) -> Value {
    json!({
        "id": g.id,
        "project": g.project,
        "projectName": g.project_name,
        "machine": g.machine,
        "machineName": crate::machines::peer_name(state, &g.machine),
        "prefix": g.prefix,
        "createdAt": g.created_at,
        "createdBy": g.created_by,
        "usedAt": g.used_at,
        "uses": g.uses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_matches_at_a_word_boundary_only() {
        assert!(prefix_matches("cargo test", "cargo test"));
        assert!(prefix_matches("cargo test", "cargo test --lib machines"));
        assert!(prefix_matches("cargo test", "  cargo   test\t--lib"));
        assert!(!prefix_matches("cargo test", "cargo test-foo"));
        assert!(!prefix_matches("cargo test", "cargo testing"));
        assert!(!prefix_matches("cargo test", "cargo"));
        assert!(!prefix_matches("cargo test", "cargo build"));
        assert!(!prefix_matches("", "cargo test"));
        assert!(!prefix_matches("Cargo test", "cargo test"), "words exactly");
    }

    #[test]
    fn any_metacharacter_defeats_a_grant() {
        for cmd in [
            "cargo test; rm -rf ~",
            "cargo test-foo; rm -rf /",
            "cargo test && curl evil",
            "cargo test || reboot",
            "cargo test | sh",
            "cargo test `rm -rf ~`",
            "cargo test $(rm -rf ~)",
            "cargo test $HOME",
            "cargo test (Remove-Item x)",
            "cargo test {x}",
            "cargo test > ~/.bashrc",
            "cargo test < input",
            "cargo test\nrm -rf ~",
            "cargo test\r\nrm",
            "cargo test & del x",
        ] {
            assert!(has_metachar(cmd), "{cmd:?}");
            assert!(!prefix_matches("cargo test", cmd), "{cmd:?}");
        }
        assert!(!has_metachar("cargo test --lib -- --nocapture"));
        assert!(!has_metachar("npm run build -- --prod=\"yes please\""));
        assert!(clean_prefix("cargo test;").is_none());
        assert_eq!(
            clean_prefix("  cargo   test ").as_deref(),
            Some("cargo test")
        );
        assert!(clean_prefix("   ").is_none());
    }

    #[test]
    fn the_offered_prefix_names_the_program_and_what_it_runs() {
        let s = |c: &str| suggested_prefix(c);
        assert_eq!(s("cargo test --lib").as_deref(), Some("cargo test"));
        assert_eq!(s("cargo build --release").as_deref(), Some("cargo build"));
        assert_eq!(s("cargo run --release").as_deref(), Some("cargo run"));
        assert_eq!(s("npm run build").as_deref(), Some("npm run build"));
        assert_eq!(s("npm test").as_deref(), Some("npm test"));
        assert_eq!(s("pnpm run lint --fix").as_deref(), Some("pnpm run lint"));
        assert_eq!(s("bun run test").as_deref(), Some("bun run test"));
        assert_eq!(s("npm run"), None, "a runner with no script");
        assert_eq!(s("npx jest --watch").as_deref(), Some("npx jest"));
        assert_eq!(s("./gradlew build").as_deref(), Some("./gradlew build"));
        assert_eq!(s("xcodebuild -scheme App").as_deref(), Some("xcodebuild"));
        assert_eq!(s("make").as_deref(), Some("make"));
        assert_eq!(
            s("python -m pytest -x").as_deref(),
            Some("python -m pytest")
        );
        assert_eq!(
            s("node scripts/build.js --prod").as_deref(),
            Some("node scripts/build.js")
        );
        assert_eq!(s("python"), None, "an interpreter alone");
        assert_eq!(s("python -c"), None);
        assert_eq!(s("bash build.sh"), None, "a shell runs anything");
        assert_eq!(s("pwsh -File build.ps1"), None);
        assert_eq!(s("sudo make install"), None);
        assert_eq!(s("cargo test; ls"), None, "never for a chained command");
        assert_eq!(s(""), None);
    }

    #[test]
    fn grants_are_kept_found_by_the_longest_prefix_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let m = "0123456789abcdef0123456789abcdef";
        let other = "fedcba9876543210fedcba9876543210";
        let g = RunGrants::load(Some(dir.path()));
        let a = g.add("/Users/me/app", "app", m, "cargo", "Mac").unwrap();
        let b = g
            .add("/Users/me/app", "app", m, "cargo  test", "Mac")
            .unwrap();
        assert_eq!(
            g.add("/Users/me/app", "app", m, "cargo test", "Mac")
                .as_deref(),
            Some(b.as_str()),
            "the same grant once"
        );
        assert!(g
            .add("/Users/me/app", "app", m, "cargo; x", "Mac")
            .is_none());
        let hit = g.covering("/Users/me/app", m, "cargo test --lib").unwrap();
        assert_eq!(hit.id, b, "the longest prefix");
        assert_eq!(hit.uses, 1);
        assert!(g.covering("/Users/me/app/", m, "cargo build").is_some());
        assert!(g.covering("/Users/me/other", m, "cargo test").is_none());
        assert!(g.covering("/Users/me/app", other, "cargo test").is_none());
        assert!(g.covering("/Users/me/app", m, "cargo test; rm x").is_none());
        // Kept across a restart; unreadable entries skipped.
        let again = RunGrants::load(Some(dir.path()));
        assert_eq!(again.list().len(), 2);
        let path = dir.path().join("machines").join(FILE);
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["grants"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "bad", "prefix": "x"}));
        std::fs::write(&path, v.to_string()).unwrap();
        assert_eq!(RunGrants::load(Some(dir.path())).list().len(), 2);
        assert_eq!(again.remove(&a).map(|g| g.prefix), Some("cargo".into()));
        assert!(again.remove(&a).is_none());
        assert_eq!(RunGrants::load(Some(dir.path())).list().len(), 1);
    }
}
