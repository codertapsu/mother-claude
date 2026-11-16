# Plugins

Claude Code's plugins add skills, slash commands, agents, hooks and MCP
servers. Mother Claude shows and changes them in two places:

- **Plugins** (sidebar, *Manage*): what is installed and where it is on,
  the marketplaces' catalog, and the marketplaces. Changes are the
  desktop's.
- A running session's **Info** tab: its MCP servers (with *Reconnect* and
  *Turn off / on*) and its plugins (with *Reload plugins* and *Add a plugin
  folder to this session…*).

Mother Claude never writes Claude Code's settings itself: every change is
one `claude plugin …` run by Claude Code on an explicit action from the
desktop, and a session's reload or reconnect is a control request to the
session (owned sessions) or `/reload-plugins` typed into its tmux pane.

## Where a plugin is on

| Scope | Where Claude Code saves it | Who gets it |
|---|---|---|
| `user` (*Everywhere*) | the account's `settings.json` | every session of that account |
| `project` (*One project, for everyone*) | `<project>/.claude/settings.json` | whoever works on the project and has the file |
| `local` (*One project, only you*) | `<project>/.claude/settings.local.json` | this computer only |

The project scopes run the CLI in the project's folder (`check_project`:
an existing folder, never inside a configuration directory). `claude plugin
list --json` names every project's own plugins wherever it runs (with their
`projectPath`, served as `project`), but says whether one is on only for the
folder it runs in: such a plugin's `enabledKnown` is true only when the list
was read in its project (*Show a project*), and a change of it runs in its
own project's folder. A directory
account manages its own directory: the CLI runs with that account's
environment (`accounts::AccountEnv`, `CLAUDE_CONFIG_DIR`); a token account
shares the Default's. Plugins from claude.ai (`synced`) and managed ones are
shown read only.

## The routes

Reading is for any signed-in caller; changes are `DesktopOnly` and local
(`ctx.local`). Each change is audited (`plugins.install`, `plugins.uninstall`,
`plugins.enable`, `plugins.disable`, `plugins.update`,
`plugins.marketplace.add|remove`) with the plugin and scope, never a
project's path or a folder's.

| Route | What |
|---|---|
| `GET /api/plugins?account=&project=` | `{installed, marketplaces, project}` — `claude plugin list --json` (in the project's folder when given) and `claude plugin marketplace list --json` |
| `GET /api/plugins/available?q=&account=` | `{plugins, total, catalog}` — `claude plugin list --json --available`, cached 10 min per account, searched (name first, then most installed), at most 60 |
| `POST /api/plugins/install {plugin, scope?, project?, account?, acceptCommand?}` | `claude plugin install <id> --scope <s> --json [--accept-command <sha256>]` |
| `POST /api/plugins/uninstall` · `/enabled {enabled}` · `/update` | `uninstall`, `enable`/`disable`, `update` the same way |
| `POST /api/plugins/marketplaces {source}` | `claude plugin marketplace add <source> --scope user --json` |
| `DELETE /api/plugins/marketplaces/{name}` | `claude plugin marketplace remove <name> --json` |
| `POST /api/plugins/reload {account?}` | every owned session between turns reloads (below) → `{reloaded, held, busy, failed}` |

A plugin id is `name` or `name@marketplace` (`[A-Za-z0-9._-]`, never
starting with `-` or `.`); a marketplace source is a GitHub `owner/repo`
(optionally `#ref`), an `https://`, `ssh://` or `git@` URL, or an existing
absolute path — never `http://`, never anything that reads as an option.

**A marketplace's command.** Some marketplaces install a plugin by running a
command (or fetch it through a helper command). Without a terminal the CLI
refuses and reports the command with its SHA-256 (`shownCommand`). The
server answers `409 confirm_command {command, sha256}`; the install sheet
shows the command, and only *Run it and install* sends the same request
again with `acceptCommand: <sha256>` (`--accept-command`), which the CLI
honours for exactly that command.

Mother Claude's own plugin and marketplace (`mother-claude@mother-claude-local`)
answer `409 plugin_is_ours`: they are installed and removed in *Sessions
started elsewhere*.

A plugin's MCP servers are shown as name, kind and host (an http server) or
program name (a stdio server) — never headers, environment values, arguments
or credentials in a URL.

## A session's plugins and MCP servers

| Route | Owned session | Elsewhere |
|---|---|---|
| `GET /api/sessions/{id}/mcp` | `mcp_status` → `{servers:[{name, status, scope, source, error, tools}]}` (never a server's config) | `409` |
| `POST /api/sessions/{id}/mcp/reconnect {server}` | `mcp_reconnect {serverName}`, then the servers | `409`: type `/mcp` in it |
| `POST /api/sessions/{id}/mcp/enabled {server, enabled}` | `mcp_toggle` — this session only | `409` |
| `GET /api/sessions/{id}/plugins` | `{plugins, errors, known, reload: "control"}` from `system/init` (or the last reload) | `reload: "tmux"` or `"none"` |
| `POST /api/sessions/{id}/plugins/reload {force?}` | `reload_plugins {refuse_if_busy, hold_on_cache_impact}` | tmux: `/reload-plugins` typed; else `409` |
| `POST /api/sessions/{id}/plugins/add {path}` | `reload_plugins {add_plugins:[{type:"local", path}]}` — the dangerous right, audited | `409` |

Verified on CLI 2.1.325: `reload_plugins` with `refuse_if_busy` refuses
while a turn runs or work is queued ("The session is busy, so plugins were
not reloaded"); with `hold_on_cache_impact`, a reload that would change the
session's MCP servers or LSP tools — so its next message re-reads the whole
conversation, uncached — is held and answered `held: true` with
`cache_impact {mcp_servers_added, mcp_servers_removed, lsp_tool_change}`.
The Info tab says what would change and offers *Reload anyway* (`force`).
`add_plugins` loads a plugin folder into that process until it ends (like
`--plugin-dir` at launch); the folder must hold `.claude-plugin/plugin.json`
or a plugin's folders (`skills`, `commands`, `agents`, `hooks`) or
`.mcp.json`.

*Reconnect* is the answer to an MCP server that failed at start because what
it talks to was not running yet (a Figma plugin opened after the session
started): `mcp_reconnect` connects it without restarting the session.

A paired computer reaches these session routes through the Machines proxy
(`routes::peer_route_grant`: reading with `view`, the rest with `control`;
adding a folder also needs the dangerous right, intersected there). The
plugin manager itself is each computer's own.

## Tests

`tests/plugins.rs` drives the manager and `tests/owned.rs`
(`an_owned_sessions_mcp_servers_reconnect_and_its_plugins_reload`) a
session, both against the fake CLI (`tests/fixtures/fake-claude.py`): its
`plugin …` commands keep their state in the one temp file
`FAKE_PLUGINS_STATE` names, and refuse to run without it. No test runs the
real `claude plugin` against a real configuration.
