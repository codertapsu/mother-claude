# Using your browser from Claude Code

Mother Claude can let Claude Code sessions work in **your own browser** —
signed in as you, with your cookies and extensions — through the Mother
Claude browser extension. Every session gets its own tab group, every site
is off until you open it, the risky steps wait for your approval (on the
Mac, on a phone you allowed, or in the extension's side panel), and **Stop**
in the extension ends a session's browsing at once.

It complements Claude Code's own *Claude in Chrome* (`claude --chrome`): this
one works for every account kind (token accounts, API keys), for the
sessions Mother Claude starts, in `bypassPermissions` sessions (Mother
Claude's approvals still apply), and entirely on this computer.

- The extension's side (building, loading, the side panel, its options):
  [BROWSER-EXTENSION.md](BROWSER-EXTENSION.md).
- The wire protocol between the two: [BROWSER-PROTOCOL.md](BROWSER-PROTOCOL.md).
- The design and its reasons: [PLAN-BROWSER-AND-TEAMS.md](PLAN-BROWSER-AND-TEAMS.md) §5, §10.

## 1. Install the extension

The extension is not in a store yet: download
`mother-claude-chrome-<version>.zip` from the
[release](https://github.com/codertapsu/mother-claude/releases) (under
**Assets**) and unzip it in a folder you will keep, or build it yourself (see
[BROWSER-EXTENSION.md](BROWSER-EXTENSION.md)).

1. Open `chrome://extensions` (Edge: `edge://extensions`, Brave:
   `brave://extensions`).
2. Turn on **Developer mode**.
3. **Load unpacked** and choose the unzipped folder.

Chrome, Edge, Brave and Chromium (version 116 or later) work; each browser
pairs on its own. Firefox and Safari are not supported yet.

## 2. Pair it

1. In Mother Claude open **Settings › Browser** (on a phone, *More ›
   Settings › Browser*) and choose **Pair a browser**. A code like
   `K7QD-8F3B` appears; it works once, for 5 minutes.
2. In the extension's options page type the code. It connects to
   `ws://127.0.0.1:6725/ws/browser` (the port Mother Claude listens on; change
   it in the options if you changed `MOTHER_CLAUDE_PORT`).
3. The browser appears under **Browsers** as *connected*.

Pairing is done on the Mac (or on a phone you granted dangerous actions).
The extension keeps its own token (`mcx_…`); Mother Claude keeps only its
SHA-256, and the token works only from the extension that paired (its id is
pinned). To remove a browser, use the bin next to it: its connection closes
at once and the extension forgets the token.

The channel is this computer only: Mother Claude refuses a connection from
another computer, through any proxy or tunnel (including a raw TCP tunnel
pointing at its port), and from anything but a Chrome extension.

## 3. How sessions use it

Sessions call the `browser_*` tools of Mother Claude's MCP server (through
the Mother Claude plugin, or the server Mother Claude gives the sessions it
starts): `browser_navigate`, `browser_snapshot`, `browser_click`,
`browser_type`, `browser_fill_form`, `browser_screenshot`, … A session's
first browser call opens its **lease**: a tab group in your browser, titled
after the session and coloured. The session acts only in that group's tabs
(and in the active tab of a window only when you granted the extension that
tab). The lease stays however long the session goes without a browser
action — idle, busy with something else, or even ended (continued later, it
keeps its tabs) — until you disconnect it (Stop, or closing its group),
close the browser, or quit Mother Claude. If the extension loses its
connection for a moment, the session waits for it to come back.

With several browsers connected, a session uses the one you used most
recently, unless you pin one (*Use for new sessions*).

**Several sites at once.** A session can keep many tabs in its group — one
site in each, or several pages of one — and name the tab in any tool
(`tabId`). `browser_tabs` with `urls` opens up to ten sites at once, in
background tabs. Each tab runs one action at a time; actions in different
tabs run side by side, and each site keeps its own mode.

**Beyond the page.** What a person would look at in the browser's developer
tools, sessions can too:

- `browser_network`: the requests the tab made (method, address, status,
  type, size, time; one request in full with its headers and, when asked,
  its bodies; WebSocket messages). Recording starts with the session's first
  action in the tab and keeps the last 500 requests, across page loads.
- `browser_cookies`: the cookies sent to the page — names, domains, expiry and
  flags (`HttpOnly` ones too) — and, when you approve it, their values; set
  or delete one of the site's own.
- `browser_storage`: the site's local and session storage (keys and sizes;
  values when you approve it; set, remove, clear) and its IndexedDB
  databases, stores and records (read only).
- `browser_drag`: drag and drop (cards, list items, sliders, drop zones).

### Connect a session from the browser

The other way round: from the extension's side panel you choose a session
that is already running and give it the tab you are on (**Work here with a
session**). The desktop opens that session's lease in this browser — moving
it from another browser, lifting a Stop of it — and the extension moves your
tab into its group, as its active tab. What you write then reaches the
session through whatever reaches it (`routes::control::deliver_message`, as
the dashboard's composer): an owned session's stream, the hooks relay for a
foreign one (when it is idle, or at its next step), or its tmux pane. The
session keeps its conversation; nothing new starts.

- The message is your words plus your tab's **site** (its origin) and what
  the session may do there; never the page's title, path or query, which
  the page sets and which would reach the model as your own words.
- A session running in `bypassPermissions` gets the tab, but is told what to
  do from the desktop only (`needs_desktop`).
- The list says which sessions have the browser tools (listed or called them
  since they started, kept in `mcp-sessions.json`) and which Mother Claude
  can send messages.
- Each connect and message is an audit line (`browser.session.connect`,
  `browser.session.tell`, never the text) and a notice on the desktop.

## 4. Site modes

Every site has a mode, set on the Browser page or in the extension's side
panel:

| Mode | Sessions may |
|---|---|
| **Off** (the default) | nothing: not open it, not see its title |
| **Read** | open it, read it (snapshots, text, find, screenshots, the console, network requests, cookie and storage names), scroll, wait |
| **Act** | also click, type, fill forms, press keys, drag, answer dialogs, upload, run scripts, change cookies and storage — the risky ones still ask you |

A mode belongs to an origin (`https://github.com`, `http://localhost:3000`;
the port counts, `https` and `http` are different sites). A wildcard such as
`https://*.example.com` covers every subdomain (not `example.com` itself).
**Every other site** (`*`) is the mode of all the rest. A site's own entry
wins over a wildcard, and the longest wildcard over a shorter one.

Letting sessions act on **every** site, on a wildcard, or on a sensitive site
is set on the Mac (or a phone holding the dangerous grant); the side panel
and other phones can set ordinary sites only.

### Sensitive sites

A built-in list of banking, payment and sign-in sites stays **off** whatever
*Every other site* or a wildcard says, until you give one its own mode — and
opening one, or acting on one, always asks you first. The list (domain and
its subdomains):

- **Payment**: paypal.com, venmo.com, stripe.com, squareup.com, wise.com,
  revolut.com, klarna.com, pay.google.com, payments.google.com, coinbase.com,
  binance.com, kraken.com
- **Banking**: chase.com, bankofamerica.com, wellsfargo.com, citi.com,
  capitalone.com, usbank.com, americanexpress.com, discover.com, schwab.com,
  fidelity.com, vanguard.com, robinhood.com, hsbc.com, hsbc.co.uk,
  barclays.co.uk, lloydsbank.com, natwest.com, santander.com, ing.com,
  monzo.com, n26.com, chime.com
- **Sign-in**: accounts.google.com, myaccount.google.com,
  login.microsoftonline.com, login.live.com, account.microsoft.com,
  appleid.apple.com, idmsa.apple.com, account.apple.com, okta.com, auth0.com,
  login.yahoo.com, 1password.com, bitwarden.com, lastpass.com

## 5. Approvals

Mother Claude decides — on the Mac, whatever the model asks and whatever the
page says — which browser actions wait for you:

- opening a sensitive site, and any action on one;
- filling or submitting a form on a page with password, card or one-time-code
  fields (typing *into* such a field is never allowed: see §6);
- `browser_eval` (a script in the page);
- `browser_upload` (files from this computer);
- reading cookie or storage **values**, and every change to cookies or
  storage (they can hold your sign-in);
- more than the per-minute limit of actions on one site (30 by default: a
  session stuck in a loop);
- any session's action in a browser where you pressed Stop for another
  session less than a minute ago.

An approval is a card like any other approval in Mother Claude: in the
Inbox, on the session's page, on the Browser page — and in the extension's
side panel. The first answer wins. A phone can approve only when you granted
it dangerous actions (Settings › Phone & devices › Paired devices); any
device can deny. Nobody answering within 5 minutes counts as no. Every
approval allowed in the side panel also shows a notice on the Mac.

**Always allow** on a card adds a standing grant ("sessions may run scripts
on `http://localhost:3000`"): it skips that one kind of approval on that one
site from then on. Grants are listed under *Always allowed*, where any of
your devices can remove them; adding one needs the Mac (or a granted phone).
The Stop rule has no grant.

## 6. Secrets

- A model never types into a password, card number, security code,
  one-time-code or other secret field: Mother Claude refuses, and tells the
  session to ask you to type it yourself.
- Values of those fields show as `«redacted»` in what a session reads; Mother
  Claude redacts again what the extension sends (private keys, well-known
  token formats, bearer credentials, `password=…` pairs), before a model sees
  it.
- Cookie and storage **values** reach a session only when you approve it
  (or allowed "read or change cookies and site storage" on that site for
  good); names, sizes and flags need no approval. Even then, well-known token
  formats stay `«redacted»`.
- In network requests, credential headers (`Authorization`, `Cookie`,
  `Set-Cookie`, API keys, CSRF and session tokens) always show as
  `«redacted»`; bodies, addresses and values are redacted like page text.
- A script (`browser_eval`) could read anything the page can, which is why
  every script asks first (unless you allowed scripts on that site for good);
  the card says when one mentions cookies, storage or credentials.
- Screenshots show what is on screen (password fields show dots); turn them
  off under *Options* on Settings › Browser if you prefer.

## 7. Stop

**Stop** in the extension's side panel (or closing the session's tab group)
ends that session's browsing at once: the action in flight fails, its
approvals go, and its browser actions are refused for a minute — the model is
told you stopped it and not to retry on its own. *Resume* on the Browser
page, or in the side panel, lifts that early (a Resume from the side panel
shows a notice on the desktop). During that minute any **other** session
acting in the same browser needs your approval.

The Browser page has a **Stop** button for each session too.

## 8. Uploads

`browser_upload` reads files only from the session's project folder or a
folder you added under *Settings › Browser › Options › Folders sessions may
upload files from*
(adding one needs the Mac). Never from a Claude Code configuration folder,
Mother Claude's own data, `~/.ssh`, shell startup files or git hooks, and
never a file that usually holds secrets (`.env`, keys, `.npmrc`, `.netrc`,
credentials files): upload those yourself. At most 10 files and 5 MB at a
time, and every upload asks first (unless you allowed uploads to that site
for good).

## 9. What is recorded

- **Activity** (the Browser page, per session): what each session did — the
  tool, the site and path (never the query string), the outcome, how long it
  took, whether you approved it. Never what it typed or read. Kept in memory
  (the last 500 actions).
- **Audit** (`audit.jsonl` in Mother Claude's data folder, with the rest of
  its audit): one line per browser action, approval, pairing, removal, site
  mode, grant, Stop and resume — who, from where, the session, the site, the
  action, the outcome. Never values, page content or tokens.
- **Settings** (`browser-settings.json`: site modes, grants, settings) and
  **paired browsers** (`browsers.json`: names, versions, the pinned extension
  id, token hashes), both private (0600) in Mother Claude's data folder.
  Nothing is ever written under `~/.claude` or any account's folder.

## 10. Privacy

Everything stays on this computer: the extension talks only to Mother Claude
on `127.0.0.1`, and Mother Claude passes page content only to the session
that asked for it. Snapshots and screenshots are held only for the request.
What a session reads goes to that session's model like any other tool
result — so open only the sites you are happy for it to read.

## 11. Limits

- **Pages are untrusted.** A page can carry instructions meant for the model
  (prompt injection). Site modes, approvals and grants are yours alone — no
  tool changes them — and every risky step asks you; still, keep sensitive
  sites off and read approval cards before allowing.
- **Logins, CAPTCHAs, two-factor prompts**: do them yourself in the tab; the
  session continues afterwards. Cross-origin frames (most CAPTCHAs, payment
  widgets) are not read at all.
- **JavaScript dialogs** block a tab until answered; sessions see them and
  answer with `browser_dialog`.
- **Downloads** start from a click and cannot be stopped in advance; they
  show in the activity.
- **Screenshots, the console, network requests, cookies and trusted input**
  need the extension's optional *debugger* permission; Chrome shows a bar
  while it is attached. Storage, drag and drop and everything else work
  without it.
- **Network**: only requests made after the session's first action in the
  tab are recorded (reload to see a page's from the start); a response body
  can be read while the browser still holds it (recent ones, a few MB each),
  and binary bodies show only their size. Requests are observed, not changed:
  sessions cannot block or rewrite them.
- **Cookies and storage**: only the page's own site's — the cookies sent to
  the page, and storage of the page's origin. IndexedDB is read only, and
  cross-origin frames' storage is not reached.
- Going back or forward can land on a site the session may not see; the page
  then shows nothing to the model.
- The extension's token lives in your browser profile. A program running as
  you that can read that profile could act as the extension (and answer its
  approvals): that is why side-panel approvals show on the Mac too. Remove
  the browser on the Browser page if you suspect it.
- Phones cannot run the extension (no browser extensions there); a phone
  reads the Browser page, answers approvals (with the grant) and sets
  ordinary sites' modes.

## 12. When something does not work

| The session says | What to do |
|---|---|
| *No browser is paired* (`no_browser`) | Pair one (§2). |
| *No paired browser is connected* | Open the browser; the extension reconnects by itself. |
| *Browsing … is off* (`site_off`) / *read-only* (`site_read_only`) | Set the site's mode (§4) if you want the session there. |
| *The user pressed Stop* (`stopped`) | Resume it on the Browser page, or wait a minute. |
| *Nobody approved …* (`approval_timeout`) | Answer approvals sooner, or add a grant. |
| *needs the extension's optional debugger permission* | Press **Grant** next to **Debugger** on the extension's options page. |
| *Mother Claude could not tell which session is calling* (`no_session`) | The session reaches Mother Claude without its identity; check that the Mother Claude plugin is enabled in it. |
