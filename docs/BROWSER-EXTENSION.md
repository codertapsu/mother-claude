# The Mother Claude browser extension

The extension lets your Claude Code sessions read pages and act in **your own
browser** — the one you are signed in to — through the Mother Claude app on
this computer. It works in Chrome, Edge and Brave. It talks only to the
Mother Claude app on this computer; nothing goes through the internet on its
way, and it does nothing on a site until you let it.

- [Install it](#install-it)
- [Pair it with Mother Claude](#pair-it-with-mother-claude)
- [What you see while a session works](#what-you-see-while-a-session-works)
- [Stop](#stop)
- [Site modes: off, read, act](#site-modes-off-read-act)
- [Permissions, and why each is asked](#permissions-and-why-each-is-asked)
- [Passwords and other secrets](#passwords-and-other-secrets)
- [Privacy](#privacy)
- [Troubleshooting](#troubleshooting)

## Install it

The extension is not in the Chrome Web Store yet. You install it by
"side-loading" the folder (or zip) that Mother Claude's build produces.

1. Get the extension: `npm run ext:build` in the Mother Claude repository
   makes the folder `dist-extension/chrome` and the zip
   `dist-extension/mother-claude-chrome-<version>.zip` (unzip it anywhere you
   will keep it: the browser loads it from there every time it starts).
2. Open the browser's extensions page:
   - Chrome: `chrome://extensions`
   - Edge: `edge://extensions`
   - Brave: `brave://extensions`
3. Turn on **Developer mode** (a switch on that page).
4. Press **Load unpacked** and choose the folder.
5. Pin **Mother Claude** to the toolbar (the puzzle-piece menu → the pin).

The browser may remind you now and then that a developer-mode extension is
installed. That is expected for side-loaded extensions.

Each browser is paired separately: install it in Edge and in Chrome, and both
show up in Mother Claude.

## Pair it with Mother Claude

1. In Mother Claude: **Settings › Browser › Pair a browser**. An 8-character
   code appears (like `ABCD-EFGH`).
2. Click the Mother Claude button in the browser's toolbar: the side panel
   opens. Type the code, check the name the desktop will show for this
   browser (for example "Chrome on Mac"), and press **Pair**.
3. The side panel says **Connected to Mother Claude**.

The options page (right-click the toolbar button → **Options**) has the same
form, the app's address (always this computer; port 6725 unless you changed
it) and **Forget this pairing**. If you remove the browser in Mother Claude,
the extension forgets its pairing too and asks for a new code.

The extension keeps reconnecting on its own while it is paired — after the
app restarts, after the computer sleeps — waiting a little longer each time,
up to 30 seconds.

## What you see while a session works

**A tab group per session.** When a Claude Code session starts using the
browser, a colored tab group appears, named after the session (for example
"Frontend · my-app"). The session works only in the tabs of its group. At
first the group holds one small page that says whose group it is; that page
becomes the session's first tab.

- A tab you drag **into** the group joins the session; a tab you drag **out**
  leaves it.
- The rest of your tabs are never touched, with one exception you choose:
  after you click the Mother Claude button on a tab, the side panel can add
  that tab to a session's group (**Let a session use this tab**).
- A session can open new tabs, but always inside its own group.

**The side panel** (click the toolbar button):

- **Connection**: whether the extension is connected to Mother Claude.
- **Sessions in this browser**: each session working here, with what it is
  doing right now ("Reading github.com…", "Typing on github.com…"), how many
  tabs its group has, and its **Stop** button.
- **Waiting for you**: browser actions Mother Claude wants you to approve
  (submitting a form on a sensitive site, uploading a file…), with **Allow**
  and **Deny**. The same requests show in Mother Claude and on your phone: the
  first answer wins.
- **This tab**: the site you are looking at, and what sessions may do there
  (below).

The toolbar button shows a number: how many sessions work here, or — in
orange — how many approvals wait for you. A grey **!** means the extension is
paired but cannot reach Mother Claude.

## Stop

**Stop** in the side panel ends what the session is doing in the browser at
once: the command it is running fails right away, and its next commands are
refused for a minute. Its tabs stay as they are; nothing is closed. **Resume**
(the same button, a second after Stop) lets it continue before the minute is
up; Mother Claude lifts its own stop too when it supports that, otherwise
resume the session there.

Closing a session's tab group (or its last tab) does the same as Stop.

## Site modes: off, read, act

Every site has a mode, and the extension checks it before every action (Mother
Claude checks first; the extension checks again):

| Mode     | What sessions may do                                                               |
| -------- | ---------------------------------------------------------------------------------- |
| **Off**  | Nothing. They cannot open, read or touch the site.                                 |
| **Read** | Open and read pages, take snapshots and screenshots, scroll. No clicks, no typing. |
| **Act**  | Read, click, type, choose, upload. Risky actions still wait for your approval.     |

- Sites you never chose are **off** (unless you changed the default in Mother
  Claude).
- Change the current site's mode in the side panel's **This tab** section, or
  in Mother Claude's settings. **Use the default** removes the site's own
  choice.
- **Sensitive sites** (banks, payment, sign-in pages, as listed in Mother
  Claude) can be raised to **Act** only on the desktop; the side panel says so
  instead of offering it.

A mode is half of it: the browser must also let the extension into the site
(the next section). The side panel offers **Allow on this site** when it is
missing.

## Permissions, and why each is asked

A fresh install can do nothing in your pages. The extension asks for each
optional permission only when you press its button, in the side panel or on
the options page, and you can take each back on the options page.

Always granted (asked at install):

| Permission | Why                                                                        |
| ---------- | -------------------------------------------------------------------------- |
| Storage    | Keeps the pairing, the app's address and the sessions' tab groups.         |
| Tabs       | Sees the sessions' tabs (their address and title) and opens tabs for them. |
| Tab groups | One group per session, named and colored.                                  |
| Scripting  | Reads and acts in pages of the sites you allowed.                          |
| Active tab | Lets you share the tab you clicked the button on.                          |
| Side panel | The panel with Stop, approvals and site modes.                             |
| Alarms     | Reconnects to Mother Claude after the browser put the extension to sleep.  |

Asked for when needed:

| Permission          | Why                                                                                                                                                                                                                   |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Sites (one, or all) | Read and change data on that site. Without it a session cannot read the site even in Read or Act mode.                                                                                                                |
| Debugger            | Clicks and keys the page treats as real (for sites that ignore scripted ones), screenshots of tabs in the background, the console, JavaScript dialogs, network-idle waits, and running a script the desktop approved. |
| Navigation events   | Notices page loads earlier and reports why a page failed to load.                                                                                                                                                     |
| Downloads           | Tells the session when a page it works on starts a download (the file name only).                                                                                                                                     |

**The debugging bar.** While the debugger permission is in use, the browser
shows "Mother Claude started debugging this browser" at the top. That is this
extension, working in a session's tabs; the bar goes away when the sessions'
groups close. If you press **Cancel** on it, the extension stops using the
debugger in those tabs until the session's lease ends; everything else keeps
working.

## Passwords and other secrets

- Snapshots and page reads never contain the value of a password field, of a
  field whose name or label says it holds a secret (card number, security
  code, one-time code, SSN, API key, token, secret…), or of anything a site
  marks with `data-mc-redact`. They show `«redacted»` instead.
- The extension does not type into such a field, unless you allowed it in
  Mother Claude for that site; even then the value comes from you, not from
  the session.
- It never reads cookies or site storage.

## Privacy

- The extension connects only to Mother Claude on **this computer**
  (`127.0.0.1`). It has no server of its own, no analytics and no remote code;
  everything it runs is inside the extension.
- What it sends to Mother Claude: the address and title of the sessions' tabs,
  and — when a session asks — a snapshot, a page's text, a screenshot, console
  messages, or the result of an action, of a site whose mode allows it.
  Secrets are redacted before they leave the page, and Mother Claude redacts
  again before anything reaches a model.
- What it keeps: the pairing token and the app's address (in the browser's
  extension storage), the sessions' tab groups while they work. Snapshots and
  screenshots are not stored.
- Forget the pairing (options page) or remove the extension, and the token is
  gone; removing the browser in Mother Claude does the same from the other
  side.

## Troubleshooting

- **"Mother Claude is not reachable"**: is the app running on this computer?
  The address on the options page must be `127.0.0.1` and the app's port
  (6725 unless you changed `MOTHER_CLAUDE_PORT`).
- **"That code is not right" / "expired"**: show a new code in Mother Claude
  (codes are short-lived) and type all 8 characters.
- **"Mother Claude cannot read this site yet"**: press **Allow on this site**
  in the side panel, and give the site a mode (Read or Act).
- **A session says a dialog blocks the page**: answer the dialog in the tab, or
  grant the debugger permission so the session can.
- **"Another connection of this browser took over"**: the extension runs twice
  with the same pairing (two profiles copied from one?). Press **Reconnect now**
  on the options page in the one you want.

Other browsers: Firefox has a separate build that has not been tested yet and
lacks the debugger features; Safari is not supported. Mobile browsers cannot
run it.
