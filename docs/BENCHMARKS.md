# Benchmarks

Numbers tracked per release (docs/PLAN-BROWSER-AND-TEAMS.md §15), measured on
the Mac that builds the release. Each comes from a test anyone can rerun; the
command is next to it.

## Browser extension (0.8, 2026-11-06)

`npm run ext:e2e` in Google Chrome 156.0.8078.80, headless, on loopback (the
run writes `dist-extension/e2e-report.json`). Tokens are bytes ÷ 4.

### Snapshot size

| Fixture page | `interactiveOnly: true` | Full | Target |
|---|---|---|---|
| Typical (an issue list, 25 rows) | 9,955 bytes, ~2,500 tokens, 256 lines | 11,151 bytes, ~2,800 tokens | ≤ 4,000 / ≤ 12,000 tokens |
| Sign-up form | 783 bytes, ~200 tokens | 1,017 bytes, ~250 tokens | |
| 3,000 rows | 44,092 bytes, cut at 1,500 lines (`truncated`) | 38,337 bytes, cut | the cap holds |

### Round trips (desktop → extension → page → desktop, 30 runs each)

| Command | p50 | p95 | Target |
|---|---|---|---|
| `tabs.list` | 0.3 ms | 0.5 ms | p50 < 150 ms, p95 < 500 ms |
| `read` (text, a selector) | 0.5 ms | 0.7 ms | |
| `find` | 2.0 ms | 4.3 ms | |
| `snapshot` (interactive only, typical page) | 2.2 ms | 4.0 ms | |
| `click` with its "what changed" snapshot | 105.7 ms | 106.4 ms | (a 100 ms settle window) |

### Stop and deadlines

| What | Measured | Target |
|---|---|---|
| Stop in the side panel → `lease.revoked` | 1 ms | ≤ 1 s |
| The running command answers `stopped` | 1 ms | |
| A 3,000 ms command with no answer | `timeout` at 2,504 ms (the extension's own deadline) | before the desktop's |
| Keepalive ping | every 19,998 ms | 20 s |

## A real session through the whole chain

One `claude -p --model haiku` session (Claude Code 2.1.319) using the
`browser_*` tools through `/mcp`, the desktop and the extension, on a fixture
form: open, snapshot, type, click (two approvals), read the result. Six tool
calls, $0.0053. How it was run: `extension/README.md` ("the real desktop") for
the desktop half; the session itself was a one-off with the UI fixture's
plugin copy (`--plugin-dir`).

## Not measured yet

Session RSS per role, the scheduler's decision latency and cost per completed
task by class: they need a real team at work, which the reports on the
Capacity page will show once one has run.
