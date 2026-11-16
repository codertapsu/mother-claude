use super::*;

#[test]
fn plugin_ids_are_names_with_an_optional_marketplace() {
    for ok in ["figma", "figma@claude-plugins-official", "a.b_c-1@m.k"] {
        assert!(valid_plugin_id(ok), "{ok}");
    }
    for bad in [
        "",
        "-y",
        "--scope",
        ".hidden",
        "a b",
        "a@b@c",
        "a@",
        "@b",
        "a/b",
        "a;rm",
        &"x".repeat(200),
    ] {
        assert!(!valid_plugin_id(bad), "{bad}");
    }
}

#[test]
fn a_marketplace_source_is_github_a_git_url_or_a_folder_never_an_option() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().to_string_lossy().into_owned();
    for ok in [
        "anthropics/claude-plugins-official",
        "owner/repo#v1.2",
        "owner/.github",
        "https://github.com/o/r.git",
        "git@github.com:o/r.git",
        "ssh://git@host/o/r.git",
        folder.as_str(),
    ] {
        assert!(check_source(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "-x",
        "--scope=user",
        "http://example.com/m.git",
        "owner",
        "a/b/c",
        "../m",
        "relative/path/../x",
        "/no/such/folder/anywhere",
        "o/r\nx",
        "o/r#ref;rm",
    ] {
        assert!(check_source(bad).is_err(), "{bad}");
    }
}

#[test]
fn installed_plugins_are_read_field_by_field_without_server_secrets() {
    let v = json!([
        {
            "id": "postman@claude-plugins-official",
            "version": "2.3.0",
            "scope": "user",
            "enabled": true,
            "installPath": "/Users/me/.claude/plugins/cache/x",
            "installedAt": "2026-10-28T06:51:15.246Z",
            "lastUpdated": "2026-11-06T01:58:52.184Z",
            "mcpServers": {
                "postman": {
                    "type": "http",
                    "url": "https://user:secret@mcp.postman.com/mcp?key=abc",
                    "headers": {"Authorization": "Bearer sk-secret"}
                },
                "local": {"command": "/usr/local/bin/node", "args": ["--token", "s3cret"], "env": {"API_KEY": "x"}}
            },
            "projectEnabled": false,
            "somethingNew": {"nested": 1}
        },
        {
            "id": "mother-claude@mother-claude-local",
            "version": "0.3.0",
            "folderVersion": "0.14.0+mcp.1",
            "scope": "user",
            "enabled": true
        },
        {"id": "engineering@synced", "scope": "synced", "enabled": true, "notes": ["Synced from claude.ai"],
         "projectPath": "/not/for/synced"},
        {"id": "tokens@team", "scope": "local", "enabled": false, "projectPath": "/Users/me/dev/site"},
        {"no": "id"},
        "junk"
    ]);
    let list = parse_installed(&v);
    assert_eq!(list.len(), 4);
    let tokens = list.iter().find(|p| p.name == "tokens").unwrap();
    assert_eq!(tokens.project.as_deref(), Some("/Users/me/dev/site"));
    assert!(list
        .iter()
        .find(|p| p.name == "engineering")
        .unwrap()
        .project
        .is_none());
    let list: Vec<Installed> = list.into_iter().filter(|p| p.name != "tokens").collect();
    let engineering = &list[0];
    assert_eq!(engineering.name, "engineering");
    assert!(
        !engineering.changeable,
        "a synced plugin is changed where it comes from"
    );
    assert_eq!(engineering.notes, ["Synced from claude.ai"]);
    let ours = &list[1];
    assert!(ours.ours);
    assert_eq!(ours.version.as_deref(), Some("0.14.0+mcp.1"));
    let postman = &list[2];
    assert_eq!(
        postman.marketplace.as_deref(),
        Some("claude-plugins-official")
    );
    assert_eq!(postman.project_enabled, Some(false));
    assert!(postman.changeable && !postman.ours);
    assert_eq!(
        postman.mcp_servers,
        [
            ServerView {
                name: "local".into(),
                kind: "stdio".into(),
                host: None,
                program: Some("node".into()),
            },
            ServerView {
                name: "postman".into(),
                kind: "http".into(),
                host: Some("mcp.postman.com".into()),
                program: None,
            },
        ]
    );
    let shown = serde_json::to_string(&list).unwrap();
    for secret in [
        "secret",
        "s3cret",
        "API_KEY",
        "key=abc",
        "installPath",
        "/Users/me",
    ] {
        assert!(!shown.contains(secret), "{secret} in {shown}");
    }
    // The `--available` shape carries the same list.
    assert_eq!(parse_installed(&json!({ "installed": v })).len(), 4);
}

#[test]
fn marketplaces_say_where_they_come_from() {
    let v = json!([
        {"name": "claude-plugins-official", "source": "github", "repo": "anthropics/claude-plugins-official", "installLocation": "/x"},
        {"name": "mother-claude-local", "source": "directory", "path": "/Users/me/Library/Application Support/x"},
        {"source": "github"}
    ]);
    let list = parse_marketplaces(&v);
    assert_eq!(list.len(), 2);
    assert_eq!(
        list[0].location.as_deref(),
        Some("anthropics/claude-plugins-official")
    );
    assert!(!list[0].ours);
    assert!(list[1].ours);
}

fn offer(id: &str, description: &str, installs: u64) -> Available {
    Available {
        id: id.into(),
        name: id.split('@').next().unwrap().into(),
        description: description.into(),
        marketplace: "m".into(),
        install_count: Some(installs),
    }
}

#[test]
fn the_catalog_is_searched_by_name_first_then_most_installed() {
    let all = vec![
        offer("figma@m", "Design files", 10),
        offer("pixel@m", "Works with Figma frames", 500),
        offer("postman@m", "API testing", 900),
        offer("figma-tokens@m", "Tokens", 20),
    ];
    let (hits, total) = search(&all, "figma", 10);
    assert_eq!(total, 3);
    let ids: Vec<&str> = hits.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["figma-tokens@m", "figma@m", "pixel@m"]);
    let (hits, total) = search(&all, "", 2);
    assert_eq!(total, 4);
    assert_eq!(hits[0].id, "postman@m");
    let (hits, _) = search(&all, "api TESTING", 10);
    assert_eq!(hits.len(), 1);
}

#[test]
fn the_available_list_is_read_from_the_cli_shape() {
    let v = json!({
        "installed": [],
        "available": [
            {"pluginId": "42crunch@claude-plugins-official", "name": "42crunch", "description": "x".repeat(900), "marketplaceName": "claude-plugins-official", "source": {"source": "git-subdir"}, "installCount": 3986},
            {"name": "no-id"}
        ]
    });
    let list = parse_available(&v);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].description.chars().count(), 400);
    assert_eq!(list[0].install_count, Some(3986));
}

#[test]
fn a_change_runs_one_cli_command_and_accepts_only_the_shown_command() {
    let sha = "a".repeat(64);
    assert_eq!(
        change_argv(&Change::Install, "figma@m", Scope::Local, Some(&sha)),
        [
            "plugin",
            "install",
            "figma@m",
            "--scope",
            "local",
            "--json",
            "--accept-command",
            sha.as_str()
        ]
    );
    // Turning one off never accepts a command.
    assert_eq!(
        change_argv(&Change::Disable, "figma@m", Scope::User, Some(&sha)),
        ["plugin", "disable", "figma@m", "--scope", "user", "--json"]
    );
    let result =
        json!({"ok": false, "shownCommand": {"command": "npx some-installer", "sha256": sha}});
    assert_eq!(
        shown_command(&result),
        Some(("npx some-installer".to_string(), sha.clone()))
    );
    assert_eq!(
        shown_command(&json!({"shownCommand": {"command": "x", "sha256": "short"}})),
        None
    );
}

#[test]
fn the_cli_result_and_its_message_come_from_the_last_json_line() {
    let run = CliRun {
        ok: false,
        stdout: "Cloning…\n{\"ok\":false,\"error\":\"Plugin not found\"}\n".into(),
        stderr: "noise".into(),
    };
    assert_eq!(run.result().unwrap()["ok"], false);
    assert_eq!(run.message(), "Plugin not found");
    let quiet = CliRun {
        ok: false,
        stdout: String::new(),
        stderr: "  boom  ".into(),
    };
    assert_eq!(quiet.message(), "boom");
}
