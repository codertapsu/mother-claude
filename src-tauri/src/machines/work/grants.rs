//! Commands, as permissions read them (docs/PERMISSIONS.md): the first
//! words of a command **at a word boundary** — `cargo test` covers `cargo
//! test --lib`, never `cargo test-foo` — and nothing for a command holding
//! a shell metacharacter: anything that could chain, substitute or redirect
//! (`;`, `&`, `|`, a backtick, `$`, parentheses, braces, `<`, `>`, a line
//! break) makes every pattern not apply, so a prefix never smuggles a
//! second command in. `pending::rules` uses these for `Bash` and
//! `PowerShell` calls and for runs on another computer.
//!
//! Until v0.13 runs had standing grants of their own, in
//! `<app data>/machines/run-grants.json`; [`legacy`] reads them once for
//! `pending::rules` to take over.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
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

/// Whether `word` names a program that runs whatever it is given (a shell,
/// `env`, `sudo`, `xargs`…): no pattern may start with one.
pub fn is_shell(word: &str) -> bool {
    SHELLS.contains(&base_name(word).as_str())
}

/// Whether `word` names an interpreter (`python`, `node`, `npx`…): a
/// pattern names what it runs, never the interpreter alone.
pub fn is_interpreter(word: &str) -> bool {
    INTERPRETERS.contains(&base_name(word).as_str())
}

/// The standing grants for runs v0.11–v0.13 kept in
/// `<app data>/machines/run-grants.json`, read entry by entry for
/// `pending::rules` to take over (one this build cannot read is skipped).
pub fn legacy(data_dir: &Path) -> Vec<RunGrant> {
    let path = data_dir.join("machines").join(FILE);
    let shape: FileShape = crate::persist::read_json(&path).unwrap_or_default();
    shape
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
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn the_old_grants_are_read_entry_by_entry() {
        let dir = tempfile::tempdir().unwrap();
        let m = "0123456789abcdef0123456789abcdef";
        std::fs::create_dir_all(dir.path().join("machines")).unwrap();
        std::fs::write(
            dir.path().join("machines").join(FILE),
            json!({
                "version": 1,
                "grants": [
                    {"id": "rg_0123456789ab", "project": "/Users/me/app", "projectName": "app",
                     "machine": m, "prefix": "cargo test", "createdAt": 1, "createdBy": "Mac",
                     "uses": 3},
                    {"id": "bad", "prefix": "x"},
                    {"id": "rg_0123456789ac", "project": "/Users/me/app", "machine": m,
                     "prefix": "cargo; rm", "createdAt": 1}
                ]
            })
            .to_string(),
        )
        .unwrap();
        let l = legacy(dir.path());
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].prefix, "cargo test");
        assert_eq!(l[0].uses, 3);
        assert!(legacy(&dir.path().join("none")).is_empty());
    }

    #[test]
    fn shells_and_interpreters_are_known_by_their_base_name() {
        assert!(is_shell("bash") && is_shell("/bin/zsh") && is_shell("PowerShell.exe"));
        assert!(is_shell("sudo") && is_shell("xargs"));
        assert!(!is_shell("cargo"));
        assert!(is_interpreter("python3") && is_interpreter("C:\\Python\\python.exe"));
        assert!(!is_interpreter("cargo"));
    }
}
