//! The move note (docs/PLAN-MACHINES.md §3.5, decisions 7 and 8): a session
//! that moved is told once, at the start of the next message it gets, where
//! it was and where it is now — the transcript keeps the old machine's paths
//! and is never rewritten — which shell it has here, and, when the move
//! asked for it, the project's recipe as a request to Claude: its commands
//! run through the session's own tool approvals, never by Mother Claude.
//! Plain English, in a message of the conversation (not the system prompt,
//! which a resume never changes: spike S3).

/// A system as a sentence names it.
pub fn os_name(os: &str) -> &'static str {
    match os {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        _ => "another system",
    }
}

/// The shell Claude Code runs commands in on a system.
pub fn shell_of(os: &str) -> &'static str {
    match os {
        "macos" => "zsh",
        "windows" => "PowerShell (Claude Code runs its Bash tool in Git Bash)",
        "linux" => "bash",
        _ => "this system's shell",
    }
}

/// What the note says.
#[derive(Clone, Debug, Default)]
pub struct Note<'a> {
    pub from_os: &'a str,
    pub from_path: &'a str,
    pub to_os: &'a str,
    pub to_path: &'a str,
    /// The code landed in a separate worktree of this project folder.
    pub worktree_of: Option<&'a str>,
    /// A copy of a session that goes on there too.
    pub as_copy: bool,
    /// Commands to set the project up here.
    pub recipe: &'a [String],
}

/// The note's text.
pub fn text(n: &Note<'_>) -> String {
    let mut out = format!(
        "[Mother Claude] This conversation {} from another computer: it was on {}, working in \
         {}, and goes on here on {}, working in {}. Paths in earlier messages refer to the old \
         computer; use the folder above from now on. The shell here is {}.",
        if n.as_copy { "was copied" } else { "moved" },
        os_name(n.from_os),
        n.from_path,
        os_name(n.to_os),
        n.to_path,
        shell_of(n.to_os),
    );
    if let Some(project) = n.worktree_of {
        out.push_str(&format!(
            " The code is in a separate git worktree of {project}, because that folder has \
             changes of its own; nothing was merged into it."
        ));
    }
    let commands: Vec<&str> = n
        .recipe
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect();
    if !commands.is_empty() {
        out.push_str(&format!(
            "\nBefore going on, please set the project up on this computer by running, in \
             order: {}.",
            commands
                .iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out
}

/// A message with the note before it (the note alone for a message with
/// only images).
pub fn prepend(note: &str, text: &str) -> String {
    if text.trim().is_empty() {
        note.to_string()
    } else {
        format!("{note}\n\n{text}")
    }
}

/// The recipe for `os` from a project's `.mother-claude/recipe.json`
/// (`{"afterMove": {"windows": ["npm ci"], "macos": [...]}}`): at most 10
/// commands of at most 300 characters, one line each.
pub fn recipe_for(json: &serde_json::Value, os: &str) -> Vec<String> {
    json.get("afterMove")
        .and_then(|a| a.get(os))
        .and_then(serde_json::Value::as_array)
        .map(|list| clean(list.iter().filter_map(serde_json::Value::as_str)))
        .unwrap_or_default()
}

/// A project's recipe set in Mother Claude (`setup::options`), held to the
/// same limits as [`recipe_for`].
pub fn clean_recipe(commands: &[String]) -> Vec<String> {
    clean(commands.iter().map(String::as_str))
}

fn clean<'a>(commands: impl Iterator<Item = &'a str>) -> Vec<String> {
    commands
        .map(str::trim)
        .filter(|c| !c.is_empty() && c.len() <= 300 && !c.contains(['\n', '\r']))
        .take(10)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_names_both_computers_the_shell_and_the_recipe() {
        let recipe = vec!["npm ci".to_string(), " ".to_string()];
        let t = text(&Note {
            from_os: "macos",
            from_path: "/Users/marcus/dev/app",
            to_os: "windows",
            to_path: r"D:\dev\app",
            worktree_of: None,
            as_copy: false,
            recipe: &recipe,
        });
        assert!(
            t.starts_with("[Mother Claude] This conversation moved"),
            "{t}"
        );
        assert!(
            t.contains("on macOS, working in /Users/marcus/dev/app"),
            "{t}"
        );
        assert!(t.contains(r"on Windows, working in D:\dev\app"), "{t}");
        assert!(t.contains("PowerShell"), "{t}");
        assert!(t.ends_with("in order: `npm ci`."), "{t}");
        let wt = text(&Note {
            from_os: "windows",
            from_path: r"D:\dev\app",
            to_os: "macos",
            to_path: "/tmp/wt",
            worktree_of: Some("/Users/m/app"),
            as_copy: true,
            recipe: &[],
        });
        assert!(
            wt.contains("was copied") && wt.contains("worktree of /Users/m/app"),
            "{wt}"
        );
        assert!(wt.contains("zsh") && !wt.contains("in order"), "{wt}");
        assert_eq!(prepend("N", "hello"), "N\n\nhello");
        assert_eq!(prepend("N", "  "), "N");
    }

    #[test]
    fn a_recipe_is_read_for_one_system_and_kept_short() {
        let v = serde_json::json!({"afterMove": {
            "windows": ["npm ci", "", "two\nlines", "x".repeat(400)],
            "macos": ["npm ci", "cargo fetch"]
        }});
        assert_eq!(recipe_for(&v, "windows"), vec!["npm ci"]);
        assert_eq!(recipe_for(&v, "macos").len(), 2);
        assert!(recipe_for(&v, "linux").is_empty());
        assert!(recipe_for(&serde_json::json!([1]), "macos").is_empty());
    }
}
