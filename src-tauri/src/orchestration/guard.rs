//! The ownership guard (docs/TEAMS.md "The guard"; plan §8): a role session
//! changes only the files its role owns.
//!
//! For a role session, an `Edit`/`Write`/`MultiEdit`/`NotebookEdit` whose
//! target is outside the role's path globs is denied with the reason told to
//! the model, and so is a `Bash` or `PowerShell` command whose write targets
//! are detectably outside (redirections, `cp`/`mv`/`rm`/`touch`/`mkdir`/
//! `tee`/`sed -i`/…, the common PowerShell cmdlets; a target computed by a
//! variable, a glob or a subshell is not detectable and passes). A read-only
//! role writes nothing. The OS temp folders and `/dev/*` belong to no role.
//!
//! The globs are relative to the role's base: its worktree
//! (`<project>/.claude/worktrees/<role>`) for a role that runs in one, else
//! the project root; they read like Claude Code's own path rules (gitignore
//! style): `src/app/**`, `docs/`, and a pattern without a `/` (`*.md`)
//! matches a name at any depth. Paths are compared as their own form says —
//! a Windows path (`C:\…`, `\\server\share\…`) without regard to case or to
//! which separator, a Mac's exactly — so both forms are checked the same on
//! either machine; on this machine's own paths the symlink-resolved form must
//! pass too.
//!
//! Where it runs: owned sessions register a `PreToolUse` callback in
//! `initialize` (`hooks`), which the CLI asks before every matching tool call
//! in every permission mode — `can_use_tool` alone is never asked in
//! `acceptEdits` for an edit (verified, docs/TEAMS.md "Spike") — and
//! `can_use_tool` checks again; foreign sessions that took a role go through
//! the plugin's `PreToolUse` decide hook. Every denial is audited
//! (`team.guard.deny`: session, role, tool, target — never the content or
//! the command) and told to the desktop.

use std::path::Path;

use serde_json::{json, Value};

use crate::server::hooks::HookPayload;
use crate::state::AppState;

/// The id of the `PreToolUse` callback a role session registers.
pub const HOOK_CALLBACK_ID: &str = "mc_team_guard";
/// The tools the callback is asked for.
pub const GUARDED_MATCHER: &str = "Edit|Write|MultiEdit|NotebookEdit|Bash|PowerShell";
const WRITE_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
const SHELL_TOOLS: [&str; 2] = ["Bash", "PowerShell"];

/// What the guard holds a role session to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub team: String,
    pub role: String,
    pub role_title: String,
    /// The folder its globs are relative to (its worktree or the project).
    pub base: String,
    pub globs: Vec<String>,
    pub read_only: bool,
    /// The lead's session name, for the reason.
    pub lead_name: Option<String>,
}

impl Rule {
    /// Whether it limits anything.
    pub fn limits(&self) -> bool {
        self.read_only || !self.globs.is_empty()
    }
}

// ─────────────────────────────── paths ───────────────────────────────

/// An absolute path as its own form writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parts {
    windows: bool,
    /// `/`, `C:` or `\\server\share`.
    root: String,
    comps: Vec<String>,
}

impl Parts {
    /// Parse an absolute path (Mac/Linux `/…`, Windows `C:\…`, `C:/…`,
    /// `\\server\share\…`, `\\?\…`); `None` for a relative one.
    pub fn absolute(p: &str) -> Option<Parts> {
        let p = p.trim();
        let p = if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else if let Some(rest) = p.strip_prefix(r"\\?\") {
            rest.to_string()
        } else {
            p.to_string()
        };
        if crate::claude::paths::is_windows_absolute(&p) {
            let (root, rest) = if let Some(unc) = p.strip_prefix(r"\\") {
                let mut it = unc.splitn(3, ['\\', '/']);
                let server = it.next().unwrap_or_default();
                let share = it.next().unwrap_or_default();
                (
                    format!(r"\\{server}\{share}"),
                    it.next().unwrap_or_default().to_string(),
                )
            } else {
                (p[..2].to_string(), p[2..].to_string())
            };
            let mut out = Parts {
                windows: true,
                root,
                comps: Vec::new(),
            };
            out.push_rel(&rest);
            return Some(out);
        }
        if let Some(rest) = p.strip_prefix('/') {
            let mut out = Parts {
                windows: false,
                root: "/".into(),
                comps: Vec::new(),
            };
            out.push_rel(rest);
            return Some(out);
        }
        None
    }

    /// Append a relative path, folding `.` and `..` (never above the root).
    fn push_rel(&mut self, rel: &str) {
        let seps: &[char] = if self.windows { &['/', '\\'] } else { &['/'] };
        for c in rel.split(seps) {
            match c {
                "" | "." => {}
                ".." => {
                    self.comps.pop();
                }
                other => self.comps.push(other.to_string()),
            }
        }
    }

    fn same(&self, a: &str, b: &str) -> bool {
        if self.windows {
            a.to_lowercase() == b.to_lowercase()
        } else {
            a == b
        }
    }

    /// The components of `self` below `base`, or `None` when it is not
    /// inside it.
    pub fn below(&self, base: &Parts) -> Option<Vec<String>> {
        if self.windows != base.windows || !self.same(&self.root, &base.root) {
            return None;
        }
        if base.comps.len() > self.comps.len() {
            return None;
        }
        for (a, b) in base.comps.iter().zip(&self.comps) {
            if !self.same(a, b) {
                return None;
            }
        }
        Some(self.comps[base.comps.len()..].to_vec())
    }

    fn starts_with(&self, prefix: &[&str]) -> bool {
        self.comps.len() >= prefix.len()
            && prefix.iter().zip(&self.comps).all(|(p, c)| self.same(p, c))
    }

    /// As written for a person.
    pub fn display(&self) -> String {
        if self.windows {
            let sep = if self.root.ends_with('\\') { "" } else { "\\" };
            format!("{}{sep}{}", self.root, self.comps.join("\\"))
        } else {
            format!("/{}", self.comps.join("/"))
        }
    }
}

/// `raw` (absolute, `~/…` or relative to `cwd`) as an absolute path.
pub fn resolve(raw: &str, cwd: Option<&Parts>, home: Option<&Parts>) -> Option<Parts> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(p) = Parts::absolute(raw) {
        return Some(p);
    }
    let home_rel = raw
        .strip_prefix("~/")
        .or_else(|| raw.strip_prefix(r"~\"))
        .or((raw == "~").then_some(""));
    if let (Some(rest), Some(home)) = (home_rel, home) {
        let mut out = home.clone();
        out.push_rel(rest);
        return Some(out);
    }
    if raw.starts_with('~') {
        return None; // ~user: not ours to guess
    }
    let mut out = cwd?.clone();
    out.push_rel(raw);
    Some(out)
}

/// A device rather than a file: `/dev/*`, Windows' `NUL` in any folder.
fn device(p: &Parts) -> bool {
    if p.windows {
        p.comps
            .last()
            .is_some_and(|c| c.eq_ignore_ascii_case("nul"))
    } else {
        p.starts_with(&["dev"])
    }
}

/// Folders that belong to no role: the OS temp folders.
fn exempt(p: &Parts) -> bool {
    if !p.windows {
        for prefix in [
            &["tmp"][..],
            &["private", "tmp"],
            &["var", "tmp"],
            &["var", "folders"],
            &["private", "var", "folders"],
        ] {
            if p.starts_with(prefix) {
                return true;
            }
        }
    }
    let temp = std::env::temp_dir();
    [
        temp.clone(),
        crate::claude::paths::canonical_or_given(&temp),
    ]
    .iter()
    .filter_map(|t| Parts::absolute(&t.to_string_lossy()))
    .filter(|t| !t.comps.is_empty())
    .any(|t| p.below(&t).is_some())
}

// ─────────────────────────────── globs ───────────────────────────────

/// One path component against one glob segment (`*`, `?`, `[a-z]`, `[!x]`).
fn segment_matches(pat: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star_p, mut star_n) = (None::<usize>, 0usize);
    while n < name.len() {
        if p < pat.len() {
            match pat[p] {
                '*' => {
                    star_p = Some(p);
                    star_n = n;
                    p += 1;
                    continue;
                }
                '?' => {
                    p += 1;
                    n += 1;
                    continue;
                }
                '[' => {
                    if let Some((matched, next)) = class_matches(pat, p, name[n]) {
                        if matched {
                            p = next;
                            n += 1;
                            continue;
                        }
                    } else if pat[p] == name[n] {
                        p += 1;
                        n += 1;
                        continue;
                    }
                }
                c if c == name[n] => {
                    p += 1;
                    n += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star_p {
            Some(sp) => {
                p = sp + 1;
                star_n += 1;
                n = star_n;
            }
            None => return false,
        }
    }
    while p < pat.len() && pat[p] == '*' {
        p += 1;
    }
    p == pat.len()
}

/// A `[…]` class at `pat[at]`: whether `c` is in it and where the pattern
/// goes on; `None` when the class is not closed (a literal `[`).
fn class_matches(pat: &[char], at: usize, c: char) -> Option<(bool, usize)> {
    let mut i = at + 1;
    let negate = i < pat.len() && (pat[i] == '!' || pat[i] == '^');
    if negate {
        i += 1;
    }
    let mut found = false;
    let mut first = true;
    while i < pat.len() && (pat[i] != ']' || first) {
        first = false;
        if i + 2 < pat.len() && pat[i + 1] == '-' && pat[i + 2] != ']' {
            if pat[i] <= c && c <= pat[i + 2] {
                found = true;
            }
            i += 3;
        } else {
            if pat[i] == c {
                found = true;
            }
            i += 1;
        }
    }
    if i >= pat.len() {
        return None;
    }
    Some((found != negate, i + 1))
}

fn fold(s: &str, case: bool) -> Vec<char> {
    if case {
        s.to_lowercase().chars().collect()
    } else {
        s.chars().collect()
    }
}

/// Glob segments against components, `**` matching any number of them.
fn segments_match(pat: &[String], comps: &[String], fold_case: bool) -> bool {
    match pat.split_first() {
        None => comps.is_empty(),
        Some((first, rest)) if first == "**" => {
            (0..=comps.len()).any(|skip| segments_match(rest, &comps[skip..], fold_case))
        }
        Some((first, rest)) => match comps.split_first() {
            Some((c, more)) => {
                segment_matches(&fold(first, fold_case), &fold(c, fold_case))
                    && segments_match(rest, more, fold_case)
            }
            None => false,
        },
    }
}

/// Whether a path below the base (`rel`, its components) is one a glob
/// names: the glob names it or a folder it is in. A glob without a `/`
/// (other than a trailing one) matches a name at any depth.
pub fn glob_matches(glob: &str, rel: &[String], fold_case: bool) -> bool {
    let g = glob.trim().replace('\\', "/");
    let g = g.strip_prefix("./").unwrap_or(&g);
    let anchored_root = g.starts_with('/');
    let g = g.trim_start_matches('/').trim_end_matches('/');
    if g.is_empty() || rel.is_empty() {
        return false;
    }
    if !anchored_root && !g.contains('/') {
        let pat = fold(g, fold_case);
        return rel
            .iter()
            .any(|c| segment_matches(&pat, &fold(c, fold_case)));
    }
    let pat: Vec<String> = g
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    // The glob names the path itself or one of the folders it is in.
    (1..=rel.len()).any(|n| segments_match(&pat, &rel[..n], fold_case))
}

// ─────────────────────────────── shell commands ───────────────────────────────

/// One word of a command line: its value, and whether it is literal (no
/// variable, command substitution or unquoted glob).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String, bool),
    /// `;`, `&&`, `||`, `|`, `&`, a line break.
    Sep,
    /// `(` and `)`: a subshell (its `cd` stays inside).
    Open,
    Close,
    /// A redirection writing to the next word (`>`, `>>`, `&>`, `2>`, …).
    WriteTo,
    /// A redirection that writes to no file (`2>&1`, `>&2`, `<`, `<<<`).
    Other,
    /// `<<DELIM`: a here-document follows on the next lines.
    HereDoc,
}

/// Split a command line into words and operators, shell style. PowerShell
/// (`ps`) escapes with a backtick, and a `\` is a path separator there.
fn tokenize(cmd: &str, ps: bool) -> Vec<Tok> {
    let chars: Vec<char> = cmd.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut word = String::new();
    let mut literal = true;
    let mut in_word = false;
    let flush = |out: &mut Vec<Tok>, word: &mut String, literal: &mut bool, in_word: &mut bool| {
        if *in_word {
            out.push(Tok::Word(std::mem::take(word), *literal));
        }
        *literal = true;
        *in_word = false;
    };
    let mut heredocs: Vec<(String, bool)> = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => {
                flush(&mut out, &mut word, &mut literal, &mut in_word);
                i += 1;
            }
            '\n' => {
                flush(&mut out, &mut word, &mut literal, &mut in_word);
                out.push(Tok::Sep);
                i += 1;
                // Here-documents: their bodies are not commands.
                for (delim, strip) in heredocs.drain(..) {
                    loop {
                        let start = i;
                        while i < chars.len() && chars[i] != '\n' {
                            i += 1;
                        }
                        let line: String = chars[start..i].iter().collect();
                        i = (i + 1).min(chars.len());
                        let line = if strip {
                            line.trim_start_matches('\t')
                        } else {
                            &line
                        };
                        if line == delim || i >= chars.len() {
                            break;
                        }
                    }
                }
            }
            '#' if !in_word => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\'' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    let escape = if ps { '`' } else { '\\' };
                    if chars[i] == escape && i + 1 < chars.len() {
                        i += 1;
                    } else if chars[i] == '$' || (!ps && chars[i] == '`') {
                        literal = false;
                    }
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '`' if ps => {
                in_word = true;
                if i + 1 < chars.len() {
                    word.push(chars[i + 1]);
                }
                i += 2;
            }
            '\\' if !ps => {
                in_word = true;
                if i + 1 < chars.len() {
                    if chars[i + 1] != '\n' {
                        word.push(chars[i + 1]);
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            '$' | '`' => {
                in_word = true;
                literal = false;
                if c == '$' && chars.get(i + 1) == Some(&'(') {
                    // $( … ): skip to the matching parenthesis.
                    let mut depth = 0;
                    while i < chars.len() {
                        match chars[i] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        word.push(chars[i]);
                        i += 1;
                    }
                } else if c == '`' {
                    word.push(c);
                    i += 1;
                    while i < chars.len() && chars[i] != '`' {
                        word.push(chars[i]);
                        i += 1;
                    }
                    i += 1;
                } else {
                    word.push(c);
                    i += 1;
                }
            }
            '*' | '?' | '[' => {
                in_word = true;
                literal = false;
                word.push(c);
                i += 1;
            }
            ';' => {
                flush(&mut out, &mut word, &mut literal, &mut in_word);
                out.push(Tok::Sep);
                i += 1;
            }
            '(' | ')' => {
                flush(&mut out, &mut word, &mut literal, &mut in_word);
                out.push(if c == '(' { Tok::Open } else { Tok::Close });
                i += 1;
            }
            '|' => {
                flush(&mut out, &mut word, &mut literal, &mut in_word);
                out.push(Tok::Sep);
                i += if chars.get(i + 1) == Some(&'|') { 2 } else { 1 };
            }
            '&' => {
                if chars.get(i + 1) == Some(&'&') {
                    flush(&mut out, &mut word, &mut literal, &mut in_word);
                    out.push(Tok::Sep);
                    i += 2;
                } else if chars.get(i + 1) == Some(&'>') {
                    // &> / &>>
                    flush(&mut out, &mut word, &mut literal, &mut in_word);
                    i += 2;
                    if chars.get(i) == Some(&'>') {
                        i += 1;
                    }
                    out.push(Tok::WriteTo);
                } else {
                    flush(&mut out, &mut word, &mut literal, &mut in_word);
                    out.push(Tok::Sep);
                    i += 1;
                }
            }
            '>' => {
                // A file-descriptor number right before it belongs to it.
                if in_word
                    && literal
                    && !word.is_empty()
                    && word.chars().all(|d| d.is_ascii_digit())
                {
                    word.clear();
                    in_word = false;
                } else {
                    flush(&mut out, &mut word, &mut literal, &mut in_word);
                }
                i += 1;
                if matches!(chars.get(i), Some('>') | Some('|')) {
                    i += 1;
                }
                if chars.get(i) == Some(&'&') {
                    // >&2, >&-: a descriptor, not a file.
                    i += 1;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '-') {
                        i += 1;
                    }
                    out.push(Tok::Other);
                } else {
                    out.push(Tok::WriteTo);
                }
            }
            '<' => {
                flush(&mut out, &mut word, &mut literal, &mut in_word);
                i += 1;
                if chars.get(i) == Some(&'<') {
                    i += 1;
                    if chars.get(i) == Some(&'<') {
                        i += 1;
                        out.push(Tok::Other);
                    } else {
                        let strip = chars.get(i) == Some(&'-');
                        if strip {
                            i += 1;
                        }
                        while i < chars.len() && chars[i] == ' ' {
                            i += 1;
                        }
                        let start = i;
                        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != ';' {
                            i += 1;
                        }
                        let delim: String = chars[start..i]
                            .iter()
                            .filter(|c| **c != '\'' && **c != '"')
                            .collect();
                        heredocs.push((delim, strip));
                        out.push(Tok::HereDoc);
                    }
                } else {
                    if chars.get(i) == Some(&'&') {
                        i += 1;
                    }
                    out.push(Tok::Other);
                }
            }
            _ => {
                in_word = true;
                word.push(c);
                i += 1;
            }
        }
    }
    flush(&mut out, &mut word, &mut literal, &mut in_word);
    out
}

/// A write target a command names: the word, and whether it is literal.
type Target = (String, bool);

/// Words of one simple command, after redirections were taken out.
fn command_targets(words: &[(String, bool)]) -> Vec<Target> {
    // Skip environment assignments and wrappers.
    let mut i = 0;
    while i < words.len() {
        let w = &words[i].0;
        let assignment = w.contains('=')
            && !w.starts_with('-')
            && w.split('=').next().is_some_and(|n| {
                !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        let wrapper = matches!(
            w.as_str(),
            "sudo" | "env" | "command" | "nohup" | "time" | "nice" | "exec" | "builtin"
        );
        if assignment || wrapper {
            i += 1;
        } else {
            break;
        }
    }
    let Some((cmd, _)) = words.get(i) else {
        return Vec::new();
    };
    let name = cmd.rsplit(['/', '\\']).next().unwrap_or(cmd).to_lowercase();
    let args = &words[i + 1..];
    // Not options, and not empty (`sed -i '' …` on a Mac).
    let plain: Vec<&(String, bool)> = args
        .iter()
        .filter(|(w, _)| !w.starts_with('-') && !w.is_empty())
        .collect();
    let value_of = |flags: &[&str]| -> Vec<Target> {
        let mut out = Vec::new();
        for (k, (w, lit)) in args.iter().enumerate() {
            for f in flags {
                if w == f {
                    if let Some(next) = args.get(k + 1) {
                        out.push(next.clone());
                    }
                } else if let Some(v) = w.strip_prefix(&format!("{f}=")) {
                    out.push((v.to_string(), *lit));
                }
            }
        }
        out
    };
    match name.as_str() {
        "cp" | "mv" | "install" | "rsync" => {
            let explicit = value_of(&["-t", "--target-directory"]);
            if !explicit.is_empty() {
                return explicit;
            }
            if plain.len() >= 2 {
                let last = plain[plain.len() - 1].clone();
                // host:path is not a local file (a Windows drive is).
                if last.0.contains(':') && !crate::claude::paths::is_windows_absolute(&last.0) {
                    return Vec::new();
                }
                return vec![last];
            }
            Vec::new()
        }
        "rm" | "rmdir" | "unlink" | "touch" | "mkdir" | "shred" | "tee" => {
            plain.into_iter().cloned().collect()
        }
        "truncate" => {
            let sizes = value_of(&["-s", "--size"]);
            plain
                .into_iter()
                .filter(|w| !sizes.contains(w))
                .cloned()
                .collect()
        }
        "chmod" | "chown" | "chgrp" => plain.into_iter().skip(1).cloned().collect(),
        "ln" => {
            if plain.len() >= 2 {
                vec![plain[plain.len() - 1].clone()]
            } else {
                Vec::new()
            }
        }
        "sed" => {
            let in_place = args
                .iter()
                .any(|(w, _)| w.starts_with("-i") || w.starts_with("--in-place"));
            if !in_place {
                return Vec::new();
            }
            let scripts = value_of(&["-e", "--expression", "-f", "--file"]);
            let mut files: Vec<Target> = plain
                .into_iter()
                .filter(|w| !scripts.contains(w))
                .cloned()
                .collect();
            if scripts.is_empty() && !files.is_empty() {
                files.remove(0); // the script
            }
            files
        }
        "perl" => {
            // `-i`, `-pi`, `-i.bak`; not `-Mstrict` or `-Ilib` (they take a
            // value).
            let in_place = args.iter().any(|(w, _)| {
                let Some(flags) = w.strip_prefix('-').filter(|f| !f.starts_with('-')) else {
                    return false;
                };
                !flags.starts_with(['M', 'm', 'I', 'e', 'E', 'x', 'd', 'D', 'C'])
                    && flags.contains('i')
            });
            if !in_place {
                return Vec::new();
            }
            let code = value_of(&["-e", "-E"]);
            let mut files: Vec<Target> = plain
                .into_iter()
                .filter(|w| !code.contains(w))
                .cloned()
                .collect();
            if code.is_empty() && !files.is_empty() {
                files.remove(0);
            }
            files
        }
        "dd" => args
            .iter()
            .filter_map(|(w, lit)| w.strip_prefix("of=").map(|v| (v.to_string(), *lit)))
            .collect(),
        "curl" => value_of(&["-o", "--output"]),
        "wget" => value_of(&["-O", "--output-document"]),
        // PowerShell (the PowerShell tool, or Windows' own aliases).
        "set-content" | "add-content" | "out-file" | "new-item" | "ni" | "remove-item"
        | "clear-content" | "del" | "erase" | "rd" | "ri" => {
            let named = ps_value(args, &["-path", "-literalpath", "-filepath"]);
            if !named.is_empty() {
                return named;
            }
            ps_positional(args).into_iter().take(1).collect()
        }
        "copy-item" | "move-item" | "copy" | "move" | "cpi" | "mi" => {
            let named = ps_value(args, &["-destination"]);
            if !named.is_empty() {
                return named;
            }
            ps_positional(args).into_iter().skip(1).take(1).collect()
        }
        "rename-item" | "ren" | "rni" => {
            let named = ps_value(args, &["-path", "-literalpath"]);
            if !named.is_empty() {
                return named;
            }
            ps_positional(args).into_iter().take(1).collect()
        }
        _ => Vec::new(),
    }
}

/// The values of PowerShell parameters (case-insensitive names).
fn ps_value(args: &[(String, bool)], names: &[&str]) -> Vec<Target> {
    let mut out = Vec::new();
    for (k, (w, _)) in args.iter().enumerate() {
        let lower = w.to_lowercase();
        let name = lower.split(':').next().unwrap_or(&lower);
        if names.contains(&name) {
            if let Some((_, v)) = w.split_once(':') {
                out.push((v.to_string(), true));
            } else if let Some(next) = args.get(k + 1) {
                out.push(next.clone());
            }
        }
    }
    out
}

/// PowerShell's positional arguments: words that are not parameters or the
/// value of one.
fn ps_positional(args: &[(String, bool)]) -> Vec<Target> {
    let mut out = Vec::new();
    let mut skip = false;
    for (w, lit) in args {
        if skip {
            skip = false;
            continue;
        }
        if w.starts_with('-') {
            // A switch takes no value; a parameter with one is followed by it.
            let lower = w.to_lowercase();
            skip = !matches!(
                lower.as_str(),
                "-force" | "-recurse" | "-whatif" | "-confirm" | "-nonewline" | "-append"
            ) && !w.contains(':');
            continue;
        }
        out.push((w.clone(), *lit));
    }
    out
}

/// One target resolved against the working directory it is relative to;
/// computed ones are not detectable and are left out.
fn push_target(target: &Target, cur: Option<&Parts>, home: Option<&Parts>, out: &mut Vec<Parts>) {
    if target.1 {
        if let Some(p) = resolve(&target.0, cur, home) {
            out.push(p);
        }
    }
}

/// The end of one simple command: a `cd` moves the working directory (a
/// literal folder; anything else makes it unknown, so only absolute targets
/// stay detectable), any other command adds its targets.
fn finish_command(
    words: &mut Vec<(String, bool)>,
    cur: &mut Option<Parts>,
    home: Option<&Parts>,
    out: &mut Vec<Parts>,
) {
    if words.is_empty() {
        return;
    }
    let first = words[0].0.to_lowercase();
    if first == "cd" || first == "set-location" || first == "sl" || first == "chdir" {
        *cur = match words.get(1) {
            Some((dir, true)) if dir != "-" => resolve(dir, cur.as_ref(), home),
            Some(_) => None,
            None => home.cloned(),
        };
    } else if first == "pushd" || first == "popd" || first == "push-location" {
        *cur = None;
    } else {
        for t in command_targets(words) {
            push_target(&t, cur.as_ref(), home, out);
        }
    }
    words.clear();
}

/// The files a command line writes, resolved against the working directory
/// each part of it runs in (`cd` moves it; a subshell's `cd` stays inside
/// the subshell). `ps`: a PowerShell command line.
pub fn write_targets(cmd: &str, cwd: Option<&Parts>, home: Option<&Parts>, ps: bool) -> Vec<Parts> {
    let mut out = Vec::new();
    let mut cur: Option<Parts> = cwd.cloned();
    let mut saved: Vec<Option<Parts>> = Vec::new();
    let mut words: Vec<(String, bool)> = Vec::new();
    let mut pending_write = false;
    for tok in tokenize(cmd, ps) {
        match tok {
            Tok::Word(w, lit) => {
                if pending_write {
                    push_target(&(w, lit), cur.as_ref(), home, &mut out);
                    pending_write = false;
                } else {
                    words.push((w, lit));
                }
            }
            Tok::WriteTo => pending_write = true,
            // The next word is its operand, not an argument.
            Tok::Other | Tok::HereDoc => pending_write = false,
            Tok::Sep => {
                finish_command(&mut words, &mut cur, home, &mut out);
                pending_write = false;
            }
            Tok::Open => {
                finish_command(&mut words, &mut cur, home, &mut out);
                saved.push(cur.clone());
            }
            Tok::Close => {
                finish_command(&mut words, &mut cur, home, &mut out);
                // Unbalanced: whatever it was, the cwd is not known now.
                cur = saved.pop().unwrap_or(None);
            }
        }
    }
    finish_command(&mut words, &mut cur, home, &mut out);
    out
}

// ─────────────────────────────── the verdict ───────────────────────────────

/// Why a path is outside a rule, or `None` when the rule allows it. Inside
/// the role's base its globs decide (wherever the base is, a temp folder
/// included); outside it, the temp folders and `/dev` are nobody's.
fn outside(rule: &Rule, target: &Parts) -> Option<String> {
    let read_only = || {
        format!(
            "the {} role is read-only in this team: it changes no files",
            rule.role_title
        )
    };
    if device(target) {
        return None;
    }
    let base = Parts::absolute(&rule.base);
    let globs = rule.globs.join(", ");
    let Some(rel) = base.as_ref().and_then(|b| target.below(b)) else {
        if exempt(target) {
            return None;
        }
        if rule.read_only {
            return Some(read_only());
        }
        if rule.globs.is_empty() {
            return None;
        }
        return Some(format!(
            "the {} role may change only {globs} inside {}, and {} is not in that folder",
            rule.role_title,
            base.map(|b| b.display())
                .unwrap_or_else(|| rule.base.clone()),
            target.display()
        ));
    };
    if rule.read_only {
        return Some(read_only());
    }
    if rule.globs.is_empty()
        || rule
            .globs
            .iter()
            .any(|g| glob_matches(g, &rel, target.windows))
    {
        return None;
    }
    let shown = if rel.is_empty() {
        ".".to_string()
    } else {
        rel.join("/")
    };
    Some(format!(
        "the {} role may change only {globs} (relative to {}), and {shown} is outside them",
        rule.role_title,
        base.map(|b| b.display()).unwrap_or_default()
    ))
}

/// This machine's form of a path with its symlinks resolved, where it (or
/// a folder above it) exists.
fn resolved_here(p: &Parts) -> Option<Parts> {
    if p.windows != cfg!(windows) {
        return None;
    }
    let display = p.display();
    let mut existing = std::path::PathBuf::from(&display);
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = crate::claude::paths::canonicalize(&existing) {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Parts::absolute(&out.to_string_lossy());
        }
        let name = existing.file_name()?.to_os_string();
        let parent = existing.parent()?.to_path_buf();
        if parent.as_os_str().is_empty() {
            return None;
        }
        rest.push(name);
        existing = parent;
    }
}

/// A denial: why, and the path it was about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial {
    pub reason: String,
    pub target: String,
}

/// Whether `rule` lets a tool call through. `cwd` and `home` resolve
/// relative paths.
pub fn check(
    rule: &Rule,
    tool: &str,
    input: &Value,
    cwd: Option<&str>,
    home: Option<&str>,
) -> Option<Denial> {
    if !rule.limits() {
        return None;
    }
    let cwd = cwd.and_then(Parts::absolute);
    let home = home.and_then(Parts::absolute);
    let targets: Vec<Parts> = if WRITE_TOOLS.contains(&tool) {
        ["file_path", "notebook_path", "path"]
            .iter()
            .filter_map(|k| input.get(*k).and_then(Value::as_str))
            .filter_map(|p| resolve(p, cwd.as_ref(), home.as_ref()))
            .take(1)
            .collect()
    } else if SHELL_TOOLS.contains(&tool) {
        match input.get("command").and_then(Value::as_str) {
            Some(cmd) => write_targets(cmd, cwd.as_ref(), home.as_ref(), tool == "PowerShell"),
            None => Vec::new(),
        }
    } else {
        return None;
    };
    for t in &targets {
        if let Some(why) = outside(rule, t) {
            return Some(Denial {
                reason: why,
                target: t.display(),
            });
        }
        // The symlink-resolved form must pass too (this machine's paths).
        if let (Some(real), Some(base)) = (
            resolved_here(t),
            Parts::absolute(&rule.base).and_then(|b| resolved_here(&b)),
        ) {
            let real_rule = Rule {
                base: base.display(),
                ..rule.clone()
            };
            if let Some(why) = outside(&real_rule, &real) {
                return Some(Denial {
                    reason: format!("{why} (through a symbolic link)"),
                    target: t.display(),
                });
            }
        }
    }
    None
}

/// What the model reads when a call is denied.
pub fn message(rule: &Rule, tool: &str, d: &Denial) -> String {
    let ask = match &rule.lead_name {
        Some(lead) => format!(
            " If the change is needed, ask the lead (SendMessage to \"{lead}\") or say so with \
             task_update."
        ),
        None => " If the change is needed, say so with task_update.".to_string(),
    };
    format!(
        "Mother Claude's team guard stopped this {tool}: {}.{ask}",
        d.reason
    )
}

// ─────────────────────────────── where it runs ───────────────────────────────

/// The `hooks` an owned role session registers in `initialize`.
pub fn initialize_hooks() -> Value {
    json!({
        "PreToolUse": [{ "matcher": GUARDED_MATCHER, "hookCallbackIds": [HOOK_CALLBACK_ID] }]
    })
}

/// The `PreToolUse` answer that denies a call with a reason.
pub fn deny_output(reason: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    })
}

fn home_dir() -> Option<String> {
    crate::claude::user_home_dir().map(|h| h.to_string_lossy().into_owned())
}

/// Decide a call of session `session`; a denial is audited and told to the
/// desktop. The model's message, or `None` to let it through.
pub fn decide(
    state: &AppState,
    session: &str,
    tool: &str,
    input: &Value,
    cwd: Option<&str>,
) -> Option<String> {
    // A role session of a team here, or one this computer runs for another
    // computer's team (its globs relative to its worktree here).
    let hosted = crate::machines::work::roles::guard_rule(state, session);
    let rule = match hosted.clone() {
        Some(r) => r,
        None => state.teams.guard_rule(session)?,
    };
    let denial = check(&rule, tool, input, cwd, home_dir().as_deref())?;
    let text = message(&rule, tool, &denial);
    if hosted.is_some() {
        crate::machines::work::roles::note_denial(state, session);
    } else {
        state.teams.note_denial(&rule.team, &rule.role);
    }
    state
        .devices
        .audit
        .record(crate::server::devices::audit::AuditEntry {
            action: "team.guard.deny".into(),
            principal: Some("session".into()),
            detail: Some(json!({
                "session": session,
                "team": rule.team,
                "role": rule.role,
                "tool": tool,
                "target": denial.target,
            })),
            ..crate::server::devices::audit::AuditEntry::now()
        });
    let shown = Path::new(&denial.target)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| denial.target.clone());
    crate::server::ws::publish_notice(
        state,
        crate::claude::model::NoticeLevel::Warning,
        format!(
            "Team guard: the {} role tried to change {shown} outside its paths; denied.",
            rule.role_title
        ),
        Some(session),
    );
    tracing::info!(session = %session, role = %rule.role, tool = %tool, "team guard denied a call");
    Some(text)
}

/// An owned session's `hook_callback` (registered by [`initialize_hooks`]):
/// the hook output.
pub fn owned_hook_callback(state: &AppState, session: &str, req: &Value) -> Value {
    if req.get("callback_id").and_then(Value::as_str) != Some(HOOK_CALLBACK_ID) {
        return json!({});
    }
    let input = req.get("input").cloned().unwrap_or(Value::Null);
    if input.get("hook_event_name").and_then(Value::as_str) != Some("PreToolUse") {
        return json!({});
    }
    let tool = input
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let cwd = input
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| state.control.cwd_of(session));
    match decide(state, session, tool, &tool_input, cwd.as_deref()) {
        Some(text) => deny_output(&text),
        None => json!({}),
    }
}

/// An owned session's `can_use_tool` request: the denial message, if the
/// guard denies it (the second line behind the hook callback).
pub fn owned_can_use_tool(state: &AppState, session: &str, req: &Value) -> Option<String> {
    let tool = req.get("tool_name").and_then(Value::as_str)?;
    let input = req.get("input").cloned().unwrap_or(Value::Null);
    let cwd = state.control.cwd_of(session);
    decide(state, session, tool, &input, cwd.as_deref())
}

/// A foreign role session's `PreToolUse` hook: the deny answer, if any. Its
/// cwd comes from the live roster, never from the hook body.
pub fn foreign_pre_tool_use(state: &AppState, p: &HookPayload) -> Option<Value> {
    let tool = p.tool_name.as_deref()?;
    if !WRITE_TOOLS.contains(&tool) && !SHELL_TOOLS.contains(&tool) {
        return None;
    }
    // Cheap first: only a session bound to a role is checked.
    state.teams.guard_rule(&p.session_id)?;
    let input = p.tool_input.clone().unwrap_or(Value::Null);
    let cwd = state
        .live
        .get(&p.session_id)
        .and_then(|l| l.cwd)
        .filter(|c| !c.is_empty());
    decide(state, &p.session_id, tool, &input, cwd.as_deref()).map(|text| deny_output(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(base: &str, globs: &[&str]) -> Rule {
        Rule {
            team: "tm".into(),
            role: "frontend".into(),
            role_title: "Frontend".into(),
            base: base.into(),
            globs: globs.iter().map(|g| g.to_string()).collect(),
            read_only: false,
            lead_name: Some("app/lead".into()),
        }
    }

    fn comps(s: &str) -> Vec<String> {
        s.split('/').map(str::to_string).collect()
    }

    #[test]
    fn globs_read_like_claude_codes_path_rules() {
        let yes = [
            ("src/app/**", "src/app/login.ts"),
            ("src/app/**", "src/app/deep/x/y.ts"),
            ("src/app", "src/app/login.ts"),
            ("src/app/", "src/app/a.ts"),
            ("./src/app/**", "src/app/a.ts"),
            ("/src/**", "src/a.ts"),
            ("*.md", "README.md"),
            ("*.md", "docs/guide/intro.md"),
            ("docs/**", "docs/a/b.md"),
            ("**/*.test.ts", "src/x/y.test.ts"),
            ("src/*.rs", "src/main.rs"),
            ("src/[a-c]*.rs", "src/build.rs"),
            ("lib?.c", "lib1.c"),
            ("**", "anything/at/all"),
        ];
        for (g, p) in yes {
            assert!(glob_matches(g, &comps(p), false), "{g} should match {p}");
        }
        let no = [
            ("src/app/**", "src/server/x.rs"),
            ("src/app/**", "src/application/x.ts"),
            ("src/*.rs", "src/nested/main.rs"),
            ("docs/**", "src/docs.rs"),
            ("*.md", "README.markdown"),
            ("src/[!a-c]*.rs", "src/build.rs"),
            ("", "x"),
        ];
        for (g, p) in no {
            assert!(
                !glob_matches(g, &comps(p), false),
                "{g} should not match {p}"
            );
        }
        // Windows paths compare without regard to case.
        assert!(glob_matches("src/App/**", &comps("SRC/app/x.ts"), true));
        assert!(!glob_matches("src/App/**", &comps("SRC/app/x.ts"), false));
        assert!(glob_matches(r"src\app\**", &comps("src/app/x.ts"), false));
    }

    #[test]
    fn both_path_forms_are_read_by_their_own_rules() {
        let mac = Parts::absolute("/Users/me/app/./src/../src/a.ts").unwrap();
        assert_eq!(mac.display(), "/Users/me/app/src/a.ts");
        let win = Parts::absolute(r"C:\Users\Me\App\src/a.ts").unwrap();
        assert_eq!(win.display(), r"C:\Users\Me\App\src\a.ts");
        let base = Parts::absolute("c:/users/me/app").unwrap();
        assert_eq!(win.below(&base).unwrap(), comps("src/a.ts"));
        let unc = Parts::absolute(r"\\nas\share\proj\x.md").unwrap();
        let unc_base = Parts::absolute(r"\\NAS\Share\proj").unwrap();
        assert_eq!(unc.below(&unc_base).unwrap(), comps("x.md"));
        let verbatim = Parts::absolute(r"\\?\C:\Users\me\app\x").unwrap();
        assert_eq!(verbatim.below(&base).unwrap(), comps("x"));
        // A Mac path is case-sensitive, and the forms never mix.
        assert!(Parts::absolute("/users/me/app/x")
            .unwrap()
            .below(&Parts::absolute("/Users/me/app").unwrap())
            .is_none());
        assert!(win.below(&mac).is_none());
        assert!(Parts::absolute("src/a.ts").is_none());
        assert!(Parts::absolute(r"D:dev").is_none());
        // Relative and ~ paths resolve against the cwd and home of the form.
        let cwd = Parts::absolute(r"C:\work\app").unwrap();
        let home = Parts::absolute(r"C:\Users\me").unwrap();
        assert_eq!(
            resolve(r"..\other\x.ts", Some(&cwd), Some(&home))
                .unwrap()
                .display(),
            r"C:\work\other\x.ts"
        );
        assert_eq!(
            resolve(r"~\notes.md", Some(&cwd), Some(&home))
                .unwrap()
                .display(),
            r"C:\Users\me\notes.md"
        );
        assert!(resolve("~bob/x", Some(&cwd), Some(&home)).is_none());
        assert!(resolve("x", None, None).is_none());
    }

    #[test]
    fn edits_outside_a_roles_paths_are_denied_on_both_forms() {
        for (base, inside, outside_path, other_folder) in [
            (
                "/Users/me/app",
                "/Users/me/app/src/app/login.ts",
                "/Users/me/app/src-tauri/src/main.rs",
                "/Users/me/elsewhere/x.ts",
            ),
            (
                r"C:\Users\Me\App",
                r"c:\users\me\app\SRC\app\login.ts",
                r"C:\Users\Me\App\src-tauri\src\main.rs",
                r"D:\elsewhere\x.ts",
            ),
        ] {
            let r = rule(base, &["src/app/**", "*.md"]);
            let edit = |p: &str| json!({ "file_path": p, "old_string": "a", "new_string": "b" });
            assert_eq!(
                check(&r, "Edit", &edit(inside), None, None),
                None,
                "{inside}"
            );
            let d = check(&r, "Write", &edit(outside_path), None, None).unwrap();
            assert!(d.reason.contains("outside them"), "{}", d.reason);
            assert!(d.reason.contains("src-tauri/src/main.rs"), "{}", d.reason);
            let d = check(&r, "MultiEdit", &edit(other_folder), None, None).unwrap();
            assert!(d.reason.contains("not in that folder"), "{}", d.reason);
            // Markdown at any depth is the role's.
            let md = format!("{base}/docs/notes.md");
            assert_eq!(check(&r, "Edit", &edit(&md), None, None), None, "{md}");
            // A notebook too.
            let nb = json!({ "notebook_path": outside_path });
            assert!(check(&r, "NotebookEdit", &nb, None, None).is_some());
            // Reads are never the guard's business.
            assert_eq!(check(&r, "Read", &edit(outside_path), None, None), None);
        }
        // A worktree role's base is its worktree.
        let wt = rule("/Users/me/app/.claude/worktrees/frontend", &["src/app/**"]);
        let main_checkout = json!({ "file_path": "/Users/me/app/src/app/login.ts" });
        assert!(check(&wt, "Edit", &main_checkout, None, None).is_some());
        let own =
            json!({ "file_path": "/Users/me/app/.claude/worktrees/frontend/src/app/login.ts" });
        assert_eq!(check(&wt, "Edit", &own, None, None), None);
        // No globs: no limit; read-only: nothing at all.
        let free = rule("/Users/me/app", &[]);
        assert_eq!(check(&free, "Write", &main_checkout, None, None), None);
        let ro = Rule {
            read_only: true,
            ..free
        };
        let d = check(&ro, "Write", &main_checkout, None, None).unwrap();
        assert!(d.reason.contains("read-only"));
        // Temp folders and /dev belong to nobody.
        let tmp = json!({ "file_path": "/tmp/scratch.txt" });
        assert_eq!(check(&ro, "Write", &tmp, None, None), None);
    }

    #[test]
    fn shell_write_targets_are_found() {
        let cwd = Parts::absolute("/w/app").unwrap();
        let t = |cmd: &str| -> Vec<String> {
            write_targets(cmd, Some(&cwd), None, false)
                .iter()
                .map(Parts::display)
                .collect()
        };
        assert_eq!(t("npm test"), Vec::<String>::new());
        // A subshell's cd stays inside it.
        assert_eq!(
            t("(cd sub && touch a) && touch b"),
            vec!["/w/app/sub/a", "/w/app/b"]
        );
        // The Mac's `sed -i ''`.
        assert_eq!(t("sed -i '' 's/a/b/' f1"), vec!["/w/app/f1"]);
        assert_eq!(t("perl -Mstrict -e 'print 1' f1"), Vec::<String>::new());
        assert_eq!(t("echo hi > out.txt"), vec!["/w/app/out.txt"]);
        assert_eq!(t("echo hi >> ../log.txt 2>&1"), vec!["/w/log.txt"]);
        assert_eq!(t("cargo build 2> err.log"), vec!["/w/app/err.log"]);
        assert_eq!(t("make &> /w/app/build.log"), vec!["/w/app/build.log"]);
        assert_eq!(t("ls 2>/dev/null"), vec!["/dev/null"]);
        assert_eq!(t("cp a.ts b.ts src/"), vec!["/w/app/src"]);
        assert_eq!(t("cp -t dest a b"), vec!["/w/app/dest"]);
        assert_eq!(t("mv -f old.rs new.rs"), vec!["/w/app/new.rs"]);
        assert_eq!(t("rm -rf build dist"), vec!["/w/app/build", "/w/app/dist"]);
        assert_eq!(
            t("mkdir -p a/b && touch a/b/c"),
            vec!["/w/app/a/b", "/w/app/a/b/c"]
        );
        assert_eq!(t("cat x | tee -a copy.txt"), vec!["/w/app/copy.txt"]);
        assert_eq!(t("sed -i 's/a/b/' f1 f2"), vec!["/w/app/f1", "/w/app/f2"]);
        assert_eq!(t("sed -i.bak -e 's/a/b/' f1"), vec!["/w/app/f1"]);
        assert_eq!(t("sed 's/a/b/' f1"), Vec::<String>::new());
        assert_eq!(t("perl -pi -e 's/a/b/' f1"), vec!["/w/app/f1"]);
        assert_eq!(
            t("dd if=/dev/zero of=disk.img bs=1M"),
            vec!["/w/app/disk.img"]
        );
        assert_eq!(t("curl -o page.html https://x"), vec!["/w/app/page.html"]);
        assert_eq!(t("chmod +x run.sh"), vec!["/w/app/run.sh"]);
        assert_eq!(t("ln -s target link"), vec!["/w/app/link"]);
        assert_eq!(t("FOO=1 sudo touch /etc/x"), vec!["/etc/x"]);
        // cd moves the cwd; an unknown cd keeps only absolute targets.
        assert_eq!(t("cd ../other && echo x > a.txt"), vec!["/w/other/a.txt"]);
        assert_eq!(
            t("cd $DIR && echo x > a.txt; echo y > /abs/b"),
            vec!["/abs/b"]
        );
        // Computed targets are not detectable.
        assert_eq!(t("echo x > \"$OUT\""), Vec::<String>::new());
        assert_eq!(t("rm src/*.ts"), Vec::<String>::new());
        assert_eq!(t("echo x > $(mktemp)"), Vec::<String>::new());
        // Quotes and escapes.
        assert_eq!(
            t("echo 'a > b' > \"my file.txt\""),
            vec!["/w/app/my file.txt"]
        );
        assert_eq!(t(r"touch my\ file"), vec!["/w/app/my file"]);
        // Comments and here-documents are not commands.
        assert_eq!(t("ls # > nope.txt"), Vec::<String>::new());
        assert_eq!(
            t("cat > notes.md <<'EOF'\nrm -rf /\necho > /etc/passwd\nEOF\necho done > d.txt"),
            vec!["/w/app/notes.md", "/w/app/d.txt"]
        );
        // rsync to another host is not a local file.
        assert_eq!(t("rsync -a out/ host:/srv/"), Vec::<String>::new());
    }

    #[test]
    fn powershell_and_windows_shell_targets_are_found() {
        let cwd = Parts::absolute(r"C:\w\app").unwrap();
        let t = |cmd: &str| -> Vec<String> {
            write_targets(cmd, Some(&cwd), None, true)
                .iter()
                .map(Parts::display)
                .collect()
        };
        // A backtick escapes in PowerShell; a backslash separates.
        assert_eq!(
            t("Set-Content my` file.txt 'x'"),
            vec![r"C:\w\app\my file.txt"]
        );
        assert_eq!(
            t(r"Set-Content -Path C:\w\app\src\x.ts -Value 'a'"),
            vec![r"C:\w\app\src\x.ts"]
        );
        assert_eq!(t(r"Out-File -FilePath ..\log.txt"), vec![r"C:\w\log.txt"]);
        assert_eq!(
            t(r"New-Item -ItemType File notes.md"),
            vec![r"C:\w\app\notes.md"]
        );
        assert_eq!(
            t(r"Copy-Item a.ts -Destination D:\out\a.ts"),
            vec![r"D:\out\a.ts"]
        );
        assert_eq!(t(r"Remove-Item -Recurse build"), vec![r"C:\w\app\build"]);
        assert_eq!(t(r"echo x > NUL"), vec![r"C:\w\app\NUL"]);
        let r = rule(r"C:\w\app", &["src/**"]);
        let cmd = |c: &str| json!({ "command": c });
        assert_eq!(
            check(
                &r,
                "PowerShell",
                &cmd(r"Set-Content src\a.ts 'x'"),
                Some(r"C:\w\app"),
                None
            ),
            None
        );
        assert!(check(
            &r,
            "PowerShell",
            &cmd(r"Set-Content docs\a.md 'x'"),
            Some(r"C:\w\app"),
            None
        )
        .is_some());
        // NUL is nobody's.
        assert_eq!(
            check(&r, "Bash", &cmd("dir > NUL"), Some(r"C:\w\app"), None),
            None
        );
    }

    #[test]
    fn shell_commands_are_judged_by_their_targets() {
        let r = rule("/w/app", &["src/app/**"]);
        let cmd = |c: &str| json!({ "command": c });
        let cwd = Some("/w/app");
        assert_eq!(
            check(&r, "Bash", &cmd("npm test && git status"), cwd, None),
            None
        );
        assert_eq!(
            check(&r, "Bash", &cmd("echo x > src/app/a.ts"), cwd, None),
            None
        );
        assert_eq!(
            check(&r, "Bash", &cmd("npm test > /tmp/out.txt 2>&1"), cwd, None),
            None
        );
        let d = check(&r, "Bash", &cmd("echo x > src/server/a.rs"), cwd, None).unwrap();
        assert!(d.reason.contains("src/server/a.rs"));
        assert!(check(
            &r,
            "Bash",
            &cmd("cd src/app && mv a.ts ../../README.md"),
            cwd,
            None
        )
        .is_some());
        assert_eq!(
            check(&r, "Bash", &cmd("cd src/app && mv a.ts b.ts"), cwd, None),
            None
        );
        let m = message(&r, "Bash", &d);
        assert!(m.starts_with("Mother Claude's team guard stopped this Bash:"));
        assert!(m.contains("SendMessage to \"app/lead\""));
    }

    #[test]
    fn a_project_in_a_temp_folder_is_still_guarded() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("app");
        let r = rule(&base.to_string_lossy(), &["src/app/**"]);
        let inside = json!({ "file_path": base.join("src/server/a.rs").to_string_lossy() });
        assert!(check(&r, "Write", &inside, None, None).is_some());
        // Temp outside the project stays nobody's.
        let other = json!({ "file_path": dir.path().join("scratch.txt").to_string_lossy() });
        assert_eq!(check(&r, "Write", &other, None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_roles_folder_is_seen_through() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("app");
        std::fs::create_dir_all(root.join("src/app")).unwrap();
        std::fs::create_dir_all(root.join("secret")).unwrap();
        std::os::unix::fs::symlink(root.join("secret"), root.join("src/app/link")).unwrap();
        let r = rule(&root.to_string_lossy(), &["src/app/**"]);
        let through = json!({ "file_path": root.join("src/app/link/x.ts").to_string_lossy() });
        let d = check(&r, "Write", &through, None, None).unwrap();
        assert!(d.reason.contains("through a symbolic link"), "{}", d.reason);
        assert!(d.reason.contains("secret/x.ts"), "{}", d.reason);
        let plain = json!({ "file_path": root.join("src/app/x.ts").to_string_lossy() });
        assert_eq!(check(&r, "Write", &plain, None, None), None);
    }

    #[test]
    fn the_hook_answers_have_the_documented_shape() {
        let h = initialize_hooks();
        assert_eq!(h["PreToolUse"][0]["matcher"], GUARDED_MATCHER);
        assert_eq!(h["PreToolUse"][0]["hookCallbackIds"][0], HOOK_CALLBACK_ID);
        let d = deny_output("no");
        assert_eq!(d["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(d["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(d["hookSpecificOutput"]["permissionDecisionReason"], "no");
    }
}
