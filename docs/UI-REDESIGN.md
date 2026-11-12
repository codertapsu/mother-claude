# UI redesign — a proposal to choose from

**Draft for review, 2026-11-12.** How Mother Claude's dashboard (`src/`: the
desktop webview and the phone, one code path) could be reorganized so that
each thing you come to do has one obvious place. It reads the app on `main`
at `b8c0aae`; nothing here is built. Three directions are drawn;
**A is recommended** ([§9](#9-comparison-and-recommendation)), with a staged
plan ([§10](#10-migration-plan-for-a)) and the choices to make
([§11](#11-decisions-for-you)).

**How to read** — the house style of
[spikes/machines/UX.md](../spikes/machines/UX.md):

- `[Button]` a button · `[Choice ▾]` a menu · `( )` `(•)` one of · `[ ]` `[x]`
  a tick box · `‹text›` a field · `›` opens a page.
- `✓` fine · `!` works, but worse · `✕` blocks · `●` online · `○` offline ·
  `◌` greyed. `━━━━` under a tab marks the one shown.
- Desktop wireframes are about 100 columns wide, phone ones 40.
- Example computers: **MacBook** (this Mac), **DESKTOP-PC** (the Windows
  desktop), **BUILD-BOX** (sometimes offline). Accounts: **Default**,
  **Work**, **Client X**. Labels are written as the app would show them; each
  one also needs its Vietnamese (`src/app/i18n/vi/*.json`).

**Contents** — [1 In one page](#1-in-one-page) ·
[2 The app today](#2-the-app-today) ·
[3 What you come to do](#3-what-you-come-to-do) ·
[4 Principles](#4-principles) ·
[5 One page template](#5-one-page-template-for-every-direction) ·
[6 Direction A](#6-direction-a--six-places-recommended) ·
[7 Direction B](#7-direction-b--one-list-one-detail) ·
[8 Direction C](#8-direction-c--projects-first) ·
[9 Comparison](#9-comparison-and-recommendation) ·
[10 Migration](#10-migration-plan-for-a) ·
[11 Decisions](#11-decisions-for-you)

---

## 1. In one page

- **The map grew one feature at a time.** The desktop sidebar has 13
  entries under headings that name the code's areas (*Your data*, *Connect*,
  *System*); the phone has another map (4 tabs and a *More* list of 11 in a
  different order, with a *Services* page the sidebar doesn't list); Settings
  carries a third set of links; Help groups the same pages a fourth way.
- **Things you look for are spread out.** What waits for you is in seven
  places (only the Inbox counts it); *Undo* lives in four lists; plan usage in six
  places; the plugin for VS Code and terminal sessions has two homes and two
  names; the *Claude Code tools* switch, which the browser extension and the
  team tools need, sits on a page the desktop sidebar doesn't list; turning
  on code sync means going back into the Machines setup wizard (7 clicks).
- **Three directions:**
  - **A — Six places** *(recommended)*: Sessions · Inbox · Teams |
    Computers · Accounts · Settings, with tabs inside each. The Inbox holds
    everything that waits for you. The phone's bar is the sidebar's first
    group plus *More*.
  - **B — One list, one detail**: a mail-style work area — views on the
    left, one list, the open session beside it — and one *Manage* area for
    everything else.
  - **C — Projects first**: a project page gathers its sessions on every
    computer, its team, its code and sync, and its own settings.
- **Why A:** it fixes the map with the components the app already has,
  needs no server change (only the words that name pages), keeps every old
  link working through redirects, and ships in nine stages of 1–5 days, each
  green. B's split view and C's project settings can be added on top of it
  later.

---

## 2. The app today

### 2.1 Routes

| Route | Component | Notes |
|---|---|---|
| `/` | `SessionsComponent` | the home; a *Sessions / Projects* switch shows `ProjectsViewComponent` (kept in local storage `mc_sessions_view`); `?machine=` |
| `/inbox` | `InboxComponent` | |
| `/agents` | `AgentsComponent` | |
| `/session/:id` | `SessionDetailComponent` | full screen on a phone (`immersive`); push opens it with `?pending=` |
| `/session/:id/agent/:agentId` | `AgentViewComponent` | |
| `/session/:id/on/:machine`, `…/agent/:agentId/on/:machine` | `RemoteSessionComponent`, `RemoteAgentComponent` | another computer's session, through this one |
| `/new` | `NewSessionComponent` | `?machine=` |
| `/machines`, `/machines/setup` | `MachinesComponent`, `MachinesSetupComponent` | |
| `/remote` | `RemoteComponent` | `#foreign` |
| `/pair` | `PairComponent` | open without pairing; `?c=` from the QR code |
| `/settings` | `SettingsComponent` | |
| `/accounts` | `AccountsComponent` | |
| `/backup` | `BackupComponent` | |
| `/more` | `MoreComponent` | the phone's 4th tab |
| `/api` | `BridgeComponent` | `?server=` |
| `/browser` | `BrowserComponent` | |
| `/services` | `ServicesComponent` | |
| `/capacity` | `CapacityComponent` | |
| `/teams`, `/teams/:id` | `TeamsComponent`, `TeamComponent` | push opens `/teams/:id` |
| `/help`, `/help/:topic` | `HelpComponent` | 29 topics in 8 groups |
| `/good-to-know` | redirect to `help/good-to-know` | |
| `/onboarding` | `OnboardingComponent` | open without pairing |
| `**` | redirect to the home | |

Every route but `pair`, `onboarding` and the redirects is behind
`pairedGuard` (`app.routes.ts`). Links that come from outside the app: the
QR code `#/pair?c=` (`server/pairing.rs`), push notifications
`#/session/<id>?pending=<id>` and `#/teams/<id>` (`public/sw.js`), and the
Home Screen app's `start_url` `#/` (`public/manifest.webmanifest`).

### 2.2 Navigation: four maps of one app

The shell (`app.component.*`) draws a left sidebar from 768 px (232 px wide:
`nav`, 13 entries, Help and the connection in its foot) and, below, a top bar
with the page's title plus a bottom bar of 4 tabs. `MoreComponent` has its
own list (`ENTRIES`), `TITLES` and `MORE_SECTIONS` repeat the names again,
Settings links to six pages, and Help has its own groups (`HELP_GROUPS`).

| Page | Desktop sidebar | Phone | Links in Settings | Help group |
|---|---|---|---|---|
| Sessions | Sessions | tab *Sessions* | — | Your sessions |
| Inbox | Inbox (count) | tab *Inbox* | — | Your sessions |
| Agents | Agents (count) | tab *Agents* | — | — |
| New session | New session | More › New session | — | Your sessions |
| Teams | Teams | More › Teams | — | Browser & teams |
| Accounts | *Your data* › Accounts | More › Accounts | Claude accounts › Accounts | Accounts & data |
| Backup & sync | *Your data* › Backup & sync | More › Backup & sync | Claude accounts › Backup & sync | Accounts & data |
| Machines | *Connect* › Machines (red dot) | More › Machines | — | Your computers |
| Remote | *Connect* › Remote | — (desktop only) | Connection › *Pair a phone* | Phone & remote |
| Browser | *Connect* › Browser | More › Browser | — | Browser & teams |
| API | *Connect* › API | More › **HTTP API** | About › **HTTP API** | Advanced |
| Capacity | *System* › Capacity | More › Capacity | — | Browser & teams |
| Settings | *System* › Settings | More › Settings | — | Basics |
| Services | **— (missing)** | More › Services | About › Services | Advanced |
| Help | the foot | More › Help | About › Help | — |

### 2.3 Screens

| Screen | For | Main parts · actions | Who | How often | Related |
|---|---|---|---|---|---|
| **Sessions** `/` | every session and what it does now | *Right now* bar (working, will resume, agents, need you, cost today, plan meter with an account picker) · limit offer · search · Live / Needs you / All · Surface, Project, Account, Machine filters · groups: Needs you, Working, Will resume, Idle, Ended recently, Other, On computers that are offline · *Show ended sessions* · *Select* → Delete / Move or copy · *New session* | both | many times a day | Inbox, Agents, a session |
| **Projects** (switch on `/`) | every project on disk | search · account filter · *Select* → Delete / Move or copy · row menu (show in Finder, copy path) · *Recently deleted* with Undo | both | monthly | Accounts, Backup & sync |
| **A session** `/session/:id` (and `…/on/:machine`) | read and drive one session | header (status, computer, account, ⋮ menu of up to 17 items) · *Runs on* / *Lives on* / move banners · limit offer · tabs Chat · Agents · Changes · Info · chips (model, effort, thinking, ultracode, fast, mode) · pending dock · composer (send, *Send now*, interrupt, files, photos, slash commands) | both | many times a day | Inbox, Agents, Accounts, Machines |
| **An agent** `…/agent/:agentId` | one subagent's transcript | breadcrumb · result · *Stop this agent* | both | weekly | a session, Agents |
| **Agents** `/agents` | every running agent, by session | a group per session · live line · *Stop* · a shell's output | both (a phone tab) | daily | a session |
| **Inbox** `/inbox` | requests from every session | *Needs you* (oldest first; J/K, A/D/F) · *Waiting at the desk* · account filter · other computers' requests | both (a phone tab) | many times a day | a session, Remote (the plugin) |
| **New session** `/new` | start a session Mother Claude runs | Where (computer) · Project · Account · Options (model, effort, permissions) · First prompt and images · *Start* | both | daily | Sessions |
| **Teams** `/teams` | teams and their templates | *Start a team* (sheet) · Your teams · Templates · *Agent teams in your sessions* (Claude Code's own) | both | weekly, if used | a team, Capacity |
| **A team** `/teams/:id` | one team at work | Roles (state, computer, *Take role…*, *Release role*, Stop / Resume) · Board (*New task*) · *Stop the team* | both | daily, if used | Capacity, Sessions |
| **Capacity** `/capacity` | the team scheduler | Now · Budgets · Routing · Reports · Settings (how accounts pay, task classifier) · *Refresh* | mostly desktop | weekly, if teams | Teams, Accounts |
| **Accounts** `/accounts` | your Claude accounts | *Add account* · each account (sign-in, plan usage, *Install hooks plugin*, *Manage*, *Move or copy data…*) · Defaults (new sessions, the API, per project) · Copy for Continue on · Moves and copies | both (adding, signing in: desktop) | weekly | Settings, Backup & sync, Sessions |
| **Backup & sync** `/backup` | encrypted backups, and carrying them to another computer | setup in three steps · Your backups (back up, restore, pull, browse, other computers) · Settings (folder, passphrase, schedule) · Folder pairs · storages · Recent | both (passphrase: desktop) | monthly (runs by itself) | Accounts, Machines |
| **Machines** `/machines` | your other computers | *Pair a machine* · *Join…* · asks · off card · health · this computer · each computer (grants both ways, *Open its sessions*, rename, revoke) · Code sync · Runs (runs, tasks, team roles, always allowed) · Options · Recent moves · *Run the setup again* · *Turn Machines off* | desktop (phone: read only) | weekly; a glance daily | Sessions, New session, Backup & sync |
| **Set up Machines** `/machines/setup` | the seven-step wizard | rail · This Mac · Your machines · Every machine · What each may do · Accounts and folders · Options (with *Each project*) · Test → *Turn on Machines* | desktop | once per computer — and every time a folder or a project's options change | Machines |
| **Remote** `/remote` | phones, internet access, sessions started elsewhere | pairing requests · Pair a device (Same Wi-Fi / Internet, QR, code) · Internet access (address, tunnels, Tailscale) · Paired devices (rename, dangerous LAN / internet, revoke) · *Foreign-session control* (install the plugin) | desktop only | once, then when pairing | Settings, Pair, Inbox, Accounts |
| **Browser** `/browser` | the browser extension | Waiting for you · Browsers · Sessions using the browser · Site modes · Always allowed · Settings · Activity · *Refresh* | both | weekly, if used | Inbox, Services |
| **API** `/api` | the HTTP API servers | API servers (*Add API server*) · *Start the API* / *Stop* · *Manage* · Addresses · Defaults · Conversation · Access (*Open to tunnels*) · API reference · Console | desktop (some on a phone) | weekly, if used | Remote, Accounts |
| **Services** `/services` | Claude Code tools, the daemon, MCP servers, background jobs | *Claude Code tools (MCP)* (on / off, last calls) · Daemon · MCP servers · Background jobs (daemon) · *Refresh* | both | rarely — but its switch matters | Browser, Teams |
| **Settings** `/settings` | the app and this device | Notifications · Appearance · Claude accounts (two links) · Plan usage (how often it's read) · Connection (*Reconnect*, *Unpair this device*, *Pair a phone*) · Home Screen app (phone) · Software updates · Permissions · About (links to Help, Services, HTTP API) | both | monthly | almost every page |
| **Help** `/help`, `/help/:topic` | how to use it | search · 8 groups · 29 topics · buttons to the pages | both | when stuck | all |
| **More** `/more` | the phone's everything else | 11 entries, a hint each · *Live updates* state | phone | daily | all |
| **Pair** `/pair` | pair this phone | code or QR · name · the number to type on the computer · the certificate | phone, first run | once | Remote |
| **Onboarding** `/onboarding` | a missing macOS permission | grant · *Skip for now* | desktop, first run | once | Settings › Permissions |
| Shell sheets | first-run notes; a device asks to pair; a folder choice | *Welcome* sheet · pair-approval dialog (4 digits) · folder-choice sheet | both / desktop | once / when pairing | Help, Remote |

### 2.4 Help's map

`help-topics.ts` holds 29 topics in 8 groups (`HELP_GROUPS`); each topic is
a route (`/help/<id>`) with a summary, an intro, keywords and sections
(steps, points, terms, tips, notes, *Open …* links to pages, a *desktop*
label), and its Vietnamese twin in `pages/help/vi/topics-*.ts`, section for
section.

| Help group | Topics |
|---|---|
| Basics | Getting started · Good to know · Settings and updates |
| Your sessions | The Sessions list · A session's page · Messages and control · Approvals and the Inbox · Start a new session |
| Phone & remote | Use it from your phone · Internet access and devices · Sessions started elsewhere |
| Your computers | Set up Machines · Pair your computers · What another computer may do · Use another computer's sessions · Move a session to another computer · Keep a project in sync between two computers · Run commands on another computer · Hand a task to another computer |
| Accounts & data | Claude accounts · Move, copy and delete · Backup and sync |
| Browser & teams | The browser extension · Teams of sessions · Capacity, routing and budgets |
| Advanced | Claude Code tools · The HTTP API |
| Troubleshooting | Troubleshooting · Troubleshooting Machines |

Help is already closer to the jobs than the sidebar is, but it pairs the
browser with teams, files *Sessions started elsewhere* under the phone, and
puts *Claude Code tools* — which both the browser and teams need — under
*Advanced*.

### 2.5 What gets in the way

**Duplicates and overlaps**

- **F1 — Four maps.** The sidebar, the phone (tabs + *More*), Settings' own
  links and Help group the same pages four ways (§2.2). One page is *API* in
  the sidebar and its title, *HTTP API* in *More* and Settings.
- **F2 — What waits for you is in seven places.** Session requests in the Inbox
  (the only count); browser approvals again on *Browser › Waiting for you*;
  pairing requests on *Remote › Waiting for your approval* and in a dialog;
  other computers' asks and Machines health on Machines (a red dot); a
  sign-in that landed in another Claude account on Accounts; an update in the
  sidebar's foot; a failed backup in *Backup & sync › Recent*.
- **F3 — Undo in four lists.** *Accounts › Moves and copies*,
  *Backup & sync › Recent* and *Sessions › Projects › Recently deleted* are
  the same `/data/jobs` filtered three ways (`kind` move/copy, backup/restore/
  pull, delete); *Machines › Recent moves* is a fourth.
- **F4 — Plan usage in six places.** The *Right now* bar (one account at a
  time), its sheet (every account), a session's *Info*, Accounts (an
  expander per account), *Settings › Plan usage* (only how often it is
  read), *Capacity › Rate limits*. "Which account has room?" has no page.
- **F5 — Three ways to carry work to another computer, sharing words.**
  Backup & sync (*Pull*, *Folder pairs*), Machines (*Continue on another
  computer*, *Code sync*, folders) and Accounts (*Move or copy…*, also to and
  from cloud storage). "Sync" means a backup's pull on one page and git code
  sync on another; *Folder pairs* and Machines' folders are two lists of one
  idea ("Your folder pairs from Backup & sync are kept as they are").
- **F6 — One plugin, two homes, two names.** *Foreign-session control* on
  Remote (the Default account) and *Install hooks plugin* per directory
  account on Accounts. The Inbox's empty state sends you to "Remote ›
  Foreign sessions"; the server says "(Remote › …)" (`extension/connect.rs`).
  *Remote* (phones and tunnels) has nothing else to do with it.
- **F7 — A session's actions in three places.** The ⋮ menu (up to 17 items
  in one list), *Info › Actions* (Terminal view, Open in VS Code, Stop,
  Respawn, Remove, Send a key) and the composer (*Continue here*).

**Hard to find, and dead ends**

- **F8 — Services is off the desktop's map.** Only *Settings › About* links
  to it, yet its *Claude Code tools* switch is what the browser extension and
  the team tools need (`/mcp` answers `503` while it is off).
- **F9 — A project's own options live only inside the setup wizard.** The
  folders and *Each project* (how code travels and lands, ignored files that
  travel, **Keep this project in sync**) are steps 5–6 of `/machines/setup`;
  the Machines page's Options card shows only relay, code and keep awake.
  Turning on code sync — daily work for someone on two computers — is
  *Machines › Run the setup again › Options › Set a project's own options… ›
  the project › Keep this project in sync › Save*, and the server's hint
  "Machines › Options › Each project" (`machines/moves/ignored.rs`) points to
  a place that isn't there.
- **F10 — A project's settings, on four pages.** Its account (*Accounts ›
  Defaults › Per project*), its move and sync options (the setup), its
  budget and role limit (*Capacity › Budgets*), its team (Teams).
- **F11 — Projects and earlier sessions are tucked away.** *Projects* is a
  switch on Sessions, yet it holds the only project-level delete and
  *Recently deleted*; earlier sessions sit behind *Show ended sessions* at
  the bottom of the list.
- **F12 — Team work spans two headings.** Teams sits with the work, Capacity
  under *System*; the Teams page also lists Claude Code's own *agent teams*,
  which are not Mother Claude's teams.
- **F13 — Remote on a phone is a dead end:** "Remote access and devices are
  managed from the desktop app." and an *Open settings* button.

**Patterns that differ from page to page**

- **F14 —** (a) a lede on 11 pages, none on Sessions, Inbox, Agents or
  Settings; Browser's is three sentences. (b) `.page` (1100 px) or
  `.page.narrow` (760 px) with no rule: Machines wide, Accounts narrow;
  Remote wide, Browser narrow. (c) Section headings outside the cards
  (Settings, Accounts, Backup, Browser, Services) or inside them (`h2.first`:
  Remote, Machines, New session). (d) Three tab styles — the session page's
  `tablist`, Capacity's segmented "tabs", Sessions' segmented view switch —
  and none on the longest pages (Backup's 650-line template, Remote's 640,
  Machines' 470, API's 1170-line component). (e) Starting something: a page
  (New session, the Machines setup), a sheet (Start a team, Add account, Pair
  a machine, Add API server) or a card in the page (Pair a device, the first
  backup). (f) A manual *Refresh* on Browser, Capacity and Services, though
  the rest is live. (g) Empty states: a card with an icon and an action
  (Sessions, Inbox), a muted line ("No devices yet. Pair one with the QR code
  above."), or nothing. (h) "Changed on the desktop" said four ways: a card
  with a link (Remote), a notice (Machines), a line per field (Backup), a
  line per section (Settings).
- **F15 —** *New session* is an action, yet it takes a sidebar entry and a
  *More* row; *Agents* takes one of the phone's four tabs.

**Words**

- **F16 — Jargon on screen:** *foreign*, *owned*, *fork* ("Continue here
  (fork)"), *respawn*, *daemon*, *Services*, *surface* (a filter), *MCP*,
  *routing*, *escalation ladder*, *budget gate*, *task classifier*, *copy
  consents*, *hooks plugin*, *machine key*; *Remote* beside Claude Code's
  own *Remote Control* (a session chip); *Waiting at the desk*; three pairing
  buttons for two kinds of things (*Pair a machine*, *Pair a device*, *Pair
  a phone*); *Machines* on the page and its buttons, *computer* in its
  sentences and in Help's group *Your computers*.
- **F17 — Pages are named outside the app, so a rename reaches further:**
  about 130 bold mentions of a page in Help (**Remote › Paired devices**
  six times), in English and Vietnamese; server text "Machines › Set up"
  (`machines/setup/asks.rs`, `machines/sync/on.rs`), "Machines › Options ›
  Each project" (`machines/moves/ignored.rs`), "(Remote › …)"
  (`extension/connect.rs`), "the API screen" (`bridge/mod.rs`); the
  extension's "on its Browser page" (`extension/src/background/index.ts`);
  README ("Settings → Accounts", "from **Remote**") and CLAUDE.md
  ("Services › Claude Code tools").

**What works, and stays in every direction:** the session page (tabs,
chips, pending dock, composer); the pending card wherever a request shows;
the Inbox's keyboard (J/K, A/D/F); the *Right now* bar; buttons that arm
first ("Tap again to …"); data-job rows with *Undo* "possible until …"; the
Machines words (the computer's real name, *Continue on DESKTOP-PC*, ✓ ! ✕)
and its setup rail; the phone's full-screen session; Help's topic pages; the
i18n rules.

---

## 3. What you come to do

Clicks are counted from the home screen to done, typing aside.

| | Job | How often | Today on the desktop | Today on a phone | In the way |
|---|---|---|---|---|---|
| J1 | **See what needs me** | many times a day | *Inbox* — 1, but only session requests; the other kinds on five more pages | *Inbox* tab — 1; the rest under *More* | F2 |
| J2 | **Answer a request** (approval, question, plan) | many times a day | *Inbox › Allow* — 2; from a push — 2 | the same — 2 | for VS Code and terminal sessions the plugin is installed on Remote *and* Accounts (F6) |
| J3 | **Check on and steer a session** | many times a day | the card — 1; a tab or chip — 2–3 | the same | 17 menu items; actions in three places (F7) |
| J4 | **Start or continue a session** | daily | *New session* — 1, then *Start*; an ended one: scroll › *Show ended sessions* › the card › *Continue here* — 4 | *Sessions › New session* — 1 (*More › New session* — 2) | earlier sessions at the bottom (F11); "fork" (F16) |
| J5 | **Watch work in progress** (agents, background, roles) | daily | *Agents* — 1; a team — 2; why a role waits: *Capacity* — 1, under *System* | *Agents* tab — 1; *More › Teams ›* the team — 3; *More › Capacity* — 2 | F12 |
| J6 | **Work across my computers** | daily | another computer's sessions: the Machine filter — 2; move a session: the session › ⋮ › *Continue on another computer…* › *Continue on DESKTOP-PC* — 4; turn on code sync — 7 | filter — 2; move — 4; *More › Machines* (read only) — 2 | F5, F9 |
| J7 | **Run a team** | weekly | *Teams › Start a team* — 2 + the sheet; budgets: *Capacity › Budgets* — 2 | *More › Teams* — 2 | F12 |
| J8 | **Stay within plan limits** | a glance many times a day; act weekly | the bar — 0 (one account); every account: the bar's sheet — 1, or *Accounts* › expand each; switch a session: ⋮ › *Switch account…* — 3; how often usage is read: *Settings* › scroll | the same; *More › Accounts* — 2 | F4 |
| J9 | **Keep, move and undo my conversations** | monthly | back up: *Backup & sync › Back up now* — 2; move or copy: *Accounts › Move or copy data…* — 2, or *Select* on Sessions; undo: one of four lists | *More › Accounts* / *Backup & sync* — 2 | F3, F5 |
| J10 | **Reach it from my phone** | once, then rarely | *Remote* — 1, show the code; approve in the dialog; internet: *Remote* › scroll; a phone's dangerous permission: *Remote › Paired devices* | scan → *Pair*; notifications: *More › Settings › Enable* — 3; Home Screen app: *More › Settings* › scroll — 3 | F13, F16 |
| J11 | **Connect my tools** (plugin, browser, scripts, Claude Code tools) | once each | plugin: *Remote* › scroll (Default) and *Accounts* (the others); *Browser* — 1; *API* — 1; Claude Code tools: *Settings › About › Services* › switch — 3 | *More ›* … — 2 | F6, F8 |
| J12 | **Look after the app** (notifications, theme, updates, help) | monthly | *Settings* — 1; an update: the foot — 1; *Help*: the foot — 1 | *More › Settings* — 2 | F1, F14 |

---

## 4. Principles

1. **One place per job.** A feature lives in one place. Other pages link to
   it, or show its state with the one action that fixes it (as Machines
   health rows do) — never a copy.
2. **One map, desktop and phone.** The same entries, names and order; the
   phone's bar shows the first ones and *More* the rest, in the same order.
3. **Everything that waits for you is in the Inbox,** under one badge.
4. **Computers are a dimension of the work, and a place of their own.**
   Their sessions, requests and moves show where the work is (chips,
   filters, *Where*); pairing, health, code sync and options have a place.
5. **One template:** one header, tabs that are routes, sections as a heading
   and a card, settings as forms, activity as rows with *Undo*, status as a
   badge.
6. **Plain words:** what you see, not what the code calls it; the computer's
   real name; *computer*, never machine, peer or host in a sentence.
7. **Nothing breaks:** every old route redirects; push, QR and Help links
   keep working; nothing that works on a phone today stops working there.

---

## 5. One page template for every direction

```
 Computers                                ✓ All good     [Pair a computer]   [⋯]        (1)
 Use your other computers from here: see and drive their sessions.   How it works ›     (2)
  Overview    Code sync 2    Activity    Options                                       (3)
  ━━━━━━━━
 PAIRED COMPUTERS                                                       [Join…]        (4)
 ┌─────────────────────────────────────────────────────────────────────────────────────────────┐
 │ DESKTOP-PC     ● Online · Direct (same network)                         [Open its sessions] │
 │ BUILD-BOX      ○ Offline since 14:32                                                        │
 └─────────────────────────────────────────────────────────────────────────────────────────────┘
```

1. **Header** (`mc-page-head`, new): the place's name (on a phone the top bar
   shows it, as `.page-head` does today), its state as a badge in the
   Machines words (✓ ! ✕), **one** primary button, the rest under `⋯`.
2. **One sentence** saying what the place is for, and *How it works ›* to
   its Help topic. Anything longer goes to Help.
3. **Tabs are routes** (`mc-hub-tabs`, new): links with `aria-current`, a
   count when something is going on, scrolling sideways on a phone. Panels
   inside one page (a session's Chat · Agents · Changes · Info) stay a
   `tablist`.
4. **Sections:** a heading outside the card with its one action on the
   right; one card per section, rows inside (`.list-row`).

**Empty states** (`mc-empty-state`, new) say what will show here and offer
the one thing that makes it show:

```
 ┌───────────────────────────────────────────────────────────────────────┐
 │                                                                       │
 │                         No phone paired yet                           │
 │      Pair your phone to answer requests and drive sessions away       │
 │      from this Mac.                                                   │
 │                  [Pair a phone]     How it works ›                    │
 │                                                                       │
 └───────────────────────────────────────────────────────────────────────┘
```

**Settings are forms:** the label and one line of why on the left, the
control on the right; a switch saves at once, a group of fields has *Save*.
On a phone, what only the desktop changes shows its **value** and one line
(`mc-desktop-only`, new) — never a disabled control:

```
 PLAN USAGE
 ┌───────────────────────────────────────────────────────────────────────┐
 │ Read usage every                                            [5 min ▾] │
 │ While a dashboard is open and a session is active.                    │
 └───────────────────────────────────────────────────────────────────────┘
 The same on a phone:
 ┌───────────────────────────────────────────────────────────────────────┐
 │ Read usage every                                                5 min │
 │ Changed on the desktop app.                                           │
 └───────────────────────────────────────────────────────────────────────┘
```

**Activity is rows** (`mc-data-job`, `mc-move-card`): when, what, from → to,
how it ended, *Undo* while it is possible.

```
 RECENT CHANGES                                                 Undo is possible for 7 days
 ┌─────────────────────────────────────────────────────────────────────────────────────────────┐
 │ 14:05   Moved 3 conversations      Default → Work        ✓ Done                      [Undo] │
 │ 12:00   Backed up Work             312 conversations     ✓ Done                             │
 │ Mon     Deleted the project poc    41 conversations      ✓ Done                      [Undo] │
 │ Mon     Pulled from BUILD-BOX      8 conversations       ✕ The folder was offline   [Retry] │
 └─────────────────────────────────────────────────────────────────────────────────────────────┘
```

**And the rest of the rules:**

| Rule | Today's exceptions it removes |
|---|---|
| `.page.narrow` for forms and single lists, `.page` for grids and two columns | Remote wide, Browser narrow (F14b) |
| Create or confirm in one screen = a sheet; set up in steps = a page with a rail; a long form used daily on a phone = a page (*New session*) | the first backup and *Pair a device* inside their pages (F14e) |
| Nothing destructive in one tap: arm first ("Tap again to …") | — (already the rule) |
| No *Refresh* where the data is live; where it is read on demand, *Check again* | Browser, Capacity, Services (F14f) |
| Health in the Machines words: ✓ fine · ! works, but worse · ✕ blocks, each with its fix | — |

---

## 6. Direction A — Six places *(recommended)*

**In one line:** group the 13 entries into six places — **Sessions, Inbox,
Teams** for the work, **Computers, Accounts, Settings** for what you look
after — each with tabs; the Inbox holds everything that waits for you; the
phone's bar is the first group plus *More*.

### 6.1 The map

```
 Desktop sidebar                          Phone
 ┌──────────────────────┐                 bottom bar:  Sessions · Inbox · Teams · More
 │ ◆ Mother Claude      │                 More:        Computers ● · Accounts · Settings · Help
 │                      │
 │ Sessions             │   ← the home
 │ Inbox             2  │   ← one badge for everything waiting
 │ Teams                │
 │ ──────────────────── │
 │ Computers         ●  │   ← red dot: a computer needs fixing
 │ Accounts             │
 │ Settings             │
 │                      │
 │ ? Help               │
 │ ↻ Update ready       │
 │ ● Live           ☾   │
 └──────────────────────┘
```

No group headings: a line divides the work from what you look after.
*New session* leaves the map (it is the home's primary button); *Agents*
becomes a tab of Sessions. The phone keeps four tabs (`.tabbar` stays a grid
of 4), and *More* lists the sidebar's second group in the same order.

### 6.2 What lives where

| Place | Tabs (routes) | From today |
|---|---|---|
| **Sessions** | Sessions `/` · Agents `/agents` · Projects `/projects` | Sessions; Agents; the *Projects* switch; Teams' *Agent teams in your sessions* (as *Teammates*, under Agents); *New session* as the page's button |
| **Inbox** | — (sections) | Inbox, plus Browser's waiting approvals, Remote's pairing requests, Machines' asks and health problems, code sync conflicts, Accounts' sign-in warnings, failed data jobs, the update |
| **Teams** | Teams `/teams` · Templates `/teams/templates` · Capacity `/teams/capacity` | Teams; a team (`/teams/:id`, unchanged); Capacity, its sections renamed Now · Budgets · Models · Costs · Settings |
| **Computers** | Overview `/computers` · Code sync · Activity · Options (+ the setup, `/computers/setup`) | Machines and its setup; the folders and *Each project* out of the wizard; *Settings › Permissions*; Services' daemon, MCP servers and background jobs (*This Mac › Details*) |
| **Accounts** | Accounts `/accounts` · Usage · Backups · Recent changes | Accounts; the plan usage sheet, *Settings › Plan usage*, *Capacity*'s rate limits; Backup & sync; the three data-job lists |
| **Settings** | one page; *Connections* open pages: Phone & devices · Sessions started elsewhere · Browser · HTTP API · Claude Code tools | Settings minus what moved; Remote; the plugin from Remote and Accounts; Browser; API; Services' *Claude Code tools* |

### 6.3 Home: Sessions

The list as today; the view switch becomes tabs, the plan meter opens
*Accounts › Usage*, *Surface* is called *App*, and *New session* is the one
primary button.

**Shown on**: every desktop and phone. **Calls**: as today (`/ws`,
`/api/sessions`, the plan usage).

```
┌──────────────────────┬───────────────────────────────────────────────────────────────────────────┐
│ ◆ Mother Claude      │ Sessions                                ‹Search sessions›   [New session] │
│                      │  Sessions    Agents 3    Projects                                         │
│ ▸ Sessions           │  ━━━━━━━━                                                                 │
│   Inbox            2 │ ┌───────────────────────────────────────────────────────────────────────┐ │
│   Teams              │ │ 3 working · 1 will resume · 3 agents · 2 need you · $4.12 today       │ │
│ ──────────────────── │ │ Work ▾  Session 42% ▮▮▮▮▯▯▯▯▯▯ resets in 2 h 10 · Week 18%  [Usage ›] │ │
│   Computers        ● │ └───────────────────────────────────────────────────────────────────────┘ │
│   Accounts           │ [Live 5] [Needs you 2] [All 9]   Project ▾  Account ▾  Computer ▾  App ▾  │
│   Settings           │                                                                           │
│                      │ NEEDS YOU 2                                                               │
│                      │ ┌───────────────────────────────────────────────────────────────────────┐ │
│                      │ │ ● Fix tray icon                                             2 min ago │ │
│                      │ │ Bash   cargo build --release                         [Allow]   [Deny] │ │
│                      │ │ [DESKTOP-PC] [CLI · owned] [Work] [Opus] [high]                       │ │
│                      │ ├───────────────────────────────────────────────────────────────────────┤ │
│                      │ │ ● Release notes                                             1 min ago │ │
│                      │ │ Which changelog format?                                     [Answer…] │ │
│                      │ └───────────────────────────────────────────────────────────────────────┘ │
│                      │ WORKING 3                                                                 │
│                      │ ┌───────────────────────────────────────────────────────────────────────┐ │
│                      │ │ ● Nightly triage    Reading crash reports · 2 agents         just now │ │
│ ? Help               │ │ ● Build installer   Running npm run tauri:build             4 min ago │ │
│ ● Live             ☾ │ └───────────────────────────────────────────────────────────────────────┘ │
└──────────────────────┴───────────────────────────────────────────────────────────────────────────┘
```

**States**: as today (loading, offline, nothing in 24 h, no match,
*Loading DESKTOP-PC's sessions…*). *Earlier sessions* stays at the end of the
list; the search also finds them, as now.

### 6.4 Inbox: everything that waits for you

Three sections: **Needs you** (answer here), **To fix** (something stopped
working, each with its fix), **Waiting where it runs** (today's *Waiting at
the desk*). The badge counts *Needs you*; *To fix* adds a dot.

**Shown on**: every desktop and phone (a phone sees desktop-only items with
*Answer on the desktop app.*, outside its badge). **Calls**: only what the
app already reads (table below).

```
 Inbox                                                        Account: All ▾    Computer: All ▾
 Everything that waits for you: requests from every session and computer, and what to fix.

 NEEDS YOU 4                                         J/K move · A allow · D deny · F feedback
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ waiting 4 min · Fix tray icon · [DESKTOP-PC] [Work]                                             │
 │ Bash   cargo build --release                                     [Allow]   [Deny]   [Feedback…] │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────┤
 │ waiting 1 min · Release notes · [Default]                                                       │
 │ Which changelog format?   ( ) Keep a Changelog   ( ) A plain list   ‹Other…›           [Answer] │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────┤
 │ just now · iPhone wants to pair · Same Wi-Fi                                                    │
 │ Type the number the phone shows: ‹_ _ _ _›                                   [Approve]   [Deny] │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────┤
 │ 2 min ago · DESKTOP-PC asks for a fix                                                           │
 │ ! This Mac sleeps after 10 minutes: a session driven from DESKTOP-PC would stop.                │
 │                                                             [Keep awake while used]   [Not now] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘

 TO FIX 3
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ ✕ Work isn't signed in on DESKTOP-PC any more                          [Sign in on DESKTOP-PC…] │
 │ ! tray-tool changed on both computers: nothing syncs until you choose                 [Choose…] │
 │ ! The backup of Client X failed at 12:00: its folder was offline                    [Try again] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘

 WAITING WHERE IT RUNS 1
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ Refactor parser · VS Code · Bash  rm -rf dist                                                   │
 │ Answer it in VS Code, or let Mother Claude answer for VS Code and terminal sessions.            │
 │                                                                                     [Set it up] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

| Item | Read from (exists today) | Answered with | On a phone |
|---|---|---|---|
| Permission request, question, plan — here and on other computers | `RealtimeService.pendingItems()`, `RemoteSessionsService.pending()` | `mc-pending-card` | yes |
| A browser action | the same items, `source: browser` | `mc-pending-card` | yes |
| A device wants to pair | `pairRequest$`, `listDevices().pending` | *Approve…* opens `mc-pair-approval-dialog` | shown, answered on the desktop |
| Another computer asks | the Machines view's `asks` | the `mc-machine-asks` row | shown, answered on the desktop |
| A computer's health problem | `MachinesService` (the setup's checks) | its fix, or *Ask DESKTOP-PC to fix* | read only |
| Code sync changed on both | the sync card's state | *Keep this Mac's* / *Keep DESKTOP-PC's* | shown, answered on the desktop |
| A sign-in lost, or as someone else | `AccountsService` (identity warnings) | *Sign in…* / *Keep it* | shown |
| A backup, restore, pull, move or delete failed (24 h) | `/data/jobs` | *Try again*, details | yes |
| An update is ready | `UpdaterService.attentionNeeded` | *Install* | — |
| A required macOS permission is missing | `PermissionsService.missingRequired()` | *Open System Settings* | — |

Browser's own *Waiting for you* stays as a short list with "Also in the
Inbox"; Remote's pairing requests and the Machines asks card go (the dialog
and the toasts stay).

### 6.5 Computers: how Machines folds in

**A place** for pairing, health, code sync, activity and options — and **a
dimension** everywhere else: the computer's chip on other computers'
sessions (none on this Mac's), the *Computer* filter on Sessions and the
Inbox, *Where* on New session, *Runs on* / *Lives on* on a session, and its
problems in the Inbox's *To fix*. The red dot stays on *Computers*.

- **Overview** — This Mac (versions, its health: the setup's step-1 checks
  plus today's *Settings › Permissions* and the *Claude Code tools* switch;
  *Details* opens Services' daemon, MCP servers and background jobs), then
  each paired computer (state, path, versions, *Open its sessions*, what each
  may do on the other, *Change*, rename, revoke). With Machines off, This
  Mac's card and today's "Machines is off" card (UX §1.1).
- **Code sync** — the projects kept in sync and **Keep a project in sync…**,
  a sheet with that project's options: no more setup wizard (F9).
- **Activity** — one list with chips All · Moves · Commands · Tasks · Team
  roles: *Recent moves* (`mc-move-card`), `mc-machines-runs`,
  `mc-machines-tasks`, `mc-machines-roles`, *Always allowed runs*.
- **Options** — `mc-machines-options`, the folders
  (`mc-machines-accounts-folders`) and *Each project*
  (`mc-machines-project-options`), today reachable only in the wizard;
  *Run the setup again*, *Turn off*.

**Shown on**: the desktop; a phone sees it read only, under *More*.
**Calls**: as today (`/api/machines`, `/api/machines/setup`, `/api/roots`,
`/api/moves`).

```
 Computers                                   ✓ All good    [Pair a computer]   [Join…]   [⋯]
 Use your other computers from here: see and drive their sessions, move one there and back.
  Overview    Code sync 2    Activity    Options
  ━━━━━━━━
 ┌ This Mac · MacBook ─────────────────────────────────────────────────────────────────────────────┐
 │ macOS 27.2 · Mother Claude 0.11.0 · Claude Code 2.1.330                                [Rename] │
 │ ✓ Full Disk Access   ✓ Local Network   ✓ Start at login   ✓ Claude Code tools on                │
 │ ! Sleeps after 10 minutes: a move, or a session driven from DESKTOP-PC, would stop.             │
 │                                               [Keep awake while used]   [Open Battery settings] │
 │ Claude Code's background service: running · 2 background jobs                         [Details] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 ┌ DESKTOP-PC ─────────────────────────────────────────────────── ● Online · Direct (same network) ┐
 │ Windows 11 · Mother Claude 0.11.0 · Claude Code 2.1.330 · seen just now                         │
 │ 4 sessions, 1 running                                                       [Open its sessions] │
 │ DESKTOP-PC may, on this Mac    See ✓  Drive ✓  Move ✓  Run –  Dangerous –              [Change] │
 │ This Mac may, on DESKTOP-PC    See ✓  Drive ✓  Move ✓  Run –  Dangerous –                       │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 ┌ BUILD-BOX ─────────────────────────────────────────────────────────────── ○ Offline since 14:32 ┐
 │ Windows 11 · Mother Claude 0.10.9                                                               │
 │ ! Older than this Mac's: its sessions show, but nothing moves to or from it until it is         │
 │   updated.                                                                      [How to update] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

```
 Computers                                   ✓ All good    [Pair a computer]   [Join…]   [⋯]
  Overview    Code sync 2    Activity    Options
              ━━━━━━━━━━━
 PROJECTS KEPT IN SYNC                                               [Keep a project in sync…]
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ mother-claude   with DESKTOP-PC   ✓ In sync · 3 min ago, this Mac → DESKTOP-PC                  │
 │                                                                         [Sync now]   [Turn off] │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────┤
 │ tray-tool       with DESKTOP-PC   ✕ Changed on both — nothing syncs until you choose            │
 │ The side you don't keep is saved.                       [Keep this Mac's]   [Keep DESKTOP-PC's] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘

 ┌ Keep a project in sync ────────────────────────────────────────────────────────────────────── ✕ ┐
 │ Project       [codex-http-bridge ▾]    ✓ on both computers, by its git remote                   │
 │ With          (•) DESKTOP-PC   ( ) BUILD-BOX  (offline)                                         │
 │ Files git ignores that travel with it:  .env.local                                     [Change] │
 │ DESKTOP-PC is asked once; syncing starts when someone there says yes.                           │
 │                                                                    [Keep it in sync]   [Cancel] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

**States**: as the Machines page today (offline, new key, older app, setup
left half-way: *Setup: step 3 of 7 — 2 required left*).

### 6.6 Accounts: accounts, usage, backups, undo

- **Accounts** — today's page without *Moves and copies*; its *Install hooks
  plugin* buttons become one line, "Sessions started elsewhere: on ›".
- **Usage** — every account's windows (the plan usage sheet's list, as a
  page), *Read usage every* (from Settings), the rate limits teams hit (from
  Capacity › Now). The *Right now* bar's meter and the plan usage
  component's "Change in Settings" link here.
- **Backups** — today's *Backup & sync* page, renamed: "sync" now means code
  sync (F5). Folder pairs stay here (they live in the backup set); Computers ›
  Options shows them too, marked *from Backups*.
- **Recent changes** — one `/data/jobs` list (moves, copies, backups,
  restores, pulls, deletes) with filter chips; each page that started a job
  keeps a short list and *See all*. Computer moves stay in *Computers ›
  Activity*.

```
 Accounts                                                                         [Add account]
 Your Claude accounts, how much of each plan is left, their backups, and what you can undo.
  Accounts 3    Usage    Backups    Recent changes
                ━━━━━
 HOW MUCH IS LEFT                                                    Read 2 min ago   [Refresh]
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ ● Default    Session ▮▮▮▮▮▮▮▯▯▯ 71% · resets in 1 h 10    Week ▮▮▯▯▯▯▯▯▯▯ 22%                   │
 │              3 sessions running                                                                 │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────┤
 │ ● Work       Session ▮▮▯▯▯▯▯▯▯▯ 18% · resets in 3 h 40    Week ▮▮▮▮▯▯▯▯▯▯ 41%                   │
 │              1 session running · 2 role sessions waiting (Teams › Capacity)                     │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────┤
 │ ● Client X   No plan limits to show: it uses an API key.                                        │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 READING IT
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ Read usage every                                                                      [5 min ▾] │
 │ While a dashboard is open and a session is active; every 30 min while nothing is.               │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 6.7 Settings and its Connections

One page: what is set once. Simple settings stay inline; the five
*Connections* rows show their state and open a page each.

```
 Settings
 NOTIFICATIONS
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ Push notifications — on for this device                                              [Turn off] │
 │ [x] Permission requests   [x] Questions   [x] Plans   [x] Finished   [ ] Failures               │
 │ [x] In-app alerts: a short tone, and the waiting count in the tab title           [Send a test] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 APPEARANCE
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ Language  [English] [Tiếng Việt]     Theme  [Dark] [Light] [System]     Accent  ● ● ● ●         │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 CONNECTIONS — what else works with Mother Claude
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ Phone & devices               2 paired · internet access on (Tailscale)                       › │
 │ Sessions started elsewhere    VS Code and terminal sessions: on for Default and Work          › │
 │ Browser                       Chrome connected · 2 sites may act                              › │
 │ HTTP API                      Running on 127.0.0.1:5612 · 1 more server                       › │
 │ Claude Code tools             On · 4 sessions have them                                       › │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 THIS DEVICE
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ The desktop app · Live · dangerous actions allowed                                  [Reconnect] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 SOFTWARE UPDATES
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ Mother Claude 0.11.0 · ✓ up to date, checked 10 min ago                     [Check for updates] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 Help — how to use every feature ›
```

| Connection | Route | What it is |
|---|---|---|
| **Phone & devices** | `/settings/devices` | today's Remote without the plugin card: pairing (*Pair a phone*), internet access, paired devices and their dangerous permission |
| **Sessions started elsewhere** | `/settings/elsewhere` | the Mother Claude plugin per config directory in one table — Remote's card (the Default) and Accounts' *Install hooks plugin* (the others) — with *Sessions reporting* and *Last hook event* |
| **Browser** | `/settings/browser` | today's Browser page |
| **HTTP API** | `/settings/api` | today's API page (`?server=` kept) |
| **Claude Code tools** | `/settings/tools` | Services' `mc-mcp-panel`; the rest of Services goes to *Computers › This Mac › Details* |

On a phone, *Phone & devices* lists the paired devices read only, this
phone first — no dead end (F13); unpairing it and *Set up the Home Screen
app* stay under *This device*.

### 6.8 A session's page

Unchanged but for the ⋮ menu, which gets headings, and *Info*, which keeps
facts only (its *Actions* move to the menu; *Send a key* stays beside the
terminal view).

```
 ┌ Fix tray icon ───────────────────────────────────────────── ✕ ┐
 │ CONTINUE                                                      │
 │   Switch account…                                             │
 │   Continue on another account…                                │
 │   Continue on another computer…                               │
 │ ORGANIZE                                                      │
 │   Rename…                                                     │
 │   Move or copy…                                               │
 │   Move or copy this project…                                  │
 │   Delete…                                                     │
 │ THIS SESSION                                                  │
 │   Notifications                   [Follow]  [Default]  [Mute] │
 │   Notify me when this turn finishes                           │
 │   Terminal view   ·   Open in VS Code   ·   Session info      │
 │   claude --resume 3f9a2c…                              [Copy] │
 │ ITS PROCESS                                                   │
 │   [Stop]   [Restart]   [Remove]                               │
 └───────────────────────────────────────────────────────────────┘
```

For a session Mother Claude doesn't run, *Continue here, as a new session*
leads *CONTINUE*; for one on another computer, *Bring back to this Mac*.

### 6.9 Phone

```
┌──────────────────────────────────────┐
│ ◆ Sessions                         ● │
│ [Select]               [New session] │
│  Sessions   Agents 3   Projects      │
│  ━━━━━━━━                            │
│ 3 working · 2 need you · $4.12       │
│ Work  Session 42% · Week 18%         │
│ ‹Search sessions›          Filters ▾ │
│ NEEDS YOU 2                          │
│ ┌──────────────────────────────────┐ │
│ │ ● Fix tray icon            2 min │ │
│ │ Bash  cargo build --release      │ │
│ │ [DESKTOP-PC] [Work] [Opus]       │ │
│ │ [Allow]                   [Deny] │ │
│ └──────────────────────────────────┘ │
│ WORKING 3                            │
│ ┌──────────────────────────────────┐ │
│ │ ● Nightly triage        just now │ │
│ │ Reading crash reports            │ │
│ └──────────────────────────────────┘ │
├──────────────────────────────────────┤
│  Sessions   Inbox 2   Teams   More   │
└──────────────────────────────────────┘
```

```
┌──────────────────────────────────────┐
│ ◆ More                             ● │
│ ┌──────────────────────────────────┐ │
│ │ Computers                    ● › │ │
│ │ DESKTOP-PC online · 1 to fix     │ │
│ ├──────────────────────────────────┤ │
│ │ Accounts                       › │ │
│ │ Default 71% · Work 18%           │ │
│ ├──────────────────────────────────┤ │
│ │ Settings                       › │ │
│ │ Notifications on · Dark          │ │
│ ├──────────────────────────────────┤ │
│ │ Help                           › │ │
│ └──────────────────────────────────┘ │
│ Live updates: Live                   │
├──────────────────────────────────────┤
│  Sessions   Inbox 2   Teams   More   │
└──────────────────────────────────────┘
```

### 6.10 Words

| Today | In A | Why |
|---|---|---|
| Machines | **Computers** (a decision, §11) | Help's group and every sentence already say computer |
| Remote | **Phone & devices** | it pairs phones and opens internet access; *Remote* collides with Claude Code's *Remote Control* |
| Foreign-session control · Install hooks plugin | **Sessions started elsewhere** | Help's topic and KNOWN_ISSUES already say it |
| Services | — (split) | *Claude Code tools*, and *This Mac › Details* |
| API · HTTP API | **HTTP API** | one name |
| Backup & sync | **Backups** | "sync" is code sync now |
| Moves and copies · Recent backups, restores and pulls · Recently deleted | **Recent changes** | one list |
| Capacity › Routing · Reports | Capacity › **Models** · **Costs** | what they show |
| Agent teams in your sessions | **Teammates** (under Agents) | Claude Code's own, not Mother Claude's teams |
| Waiting at the desk | **Waiting where it runs** | plain |
| Continue here (fork) | **Continue here, as a new session** | says what happens |
| Respawn | **Restart** | plain |
| Surface | **App** | plain (CLI, VS Code, JetBrains, Desktop…) |
| Pair a machine · Pair a device · Pair a phone | **Pair a computer** · **Pair a phone** | two things, two buttons |
| Daemon | **Claude Code's background service** | plain |
| Escalation ladder · budget gate · task classifier | to be worded with [ROUTING.md](ROUTING.md) | jargon |

### 6.11 Merged, renamed, moved, removed

| Change | What | Fixes |
|---|---|---|
| **Merged** | everything that waits for you → the Inbox; three data-job lists → *Recent changes*; the plugin's two homes → *Sessions started elsewhere*; the plan usage in six places → *Accounts › Usage* (the bar keeps its glance); Remote's pairing card → the Inbox and *Phone & devices* | F2, F3, F4, F6 |
| **Renamed** | the words above | F16 |
| **Moved** | Agents → Sessions; Projects (a switch) → a tab; Capacity → Teams; Backup & sync → Accounts; Remote, Browser, API → *Settings › Connections*; Services → *Claude Code tools* + *This Mac › Details*; Permissions → *Computers › This Mac*; the folders and *Each project* → *Computers › Options*; code sync → its own tab | F8, F9, F11, F12, F15 |
| **Removed** | *New session* and *Agents* as nav entries; the phone's own *More* list (now the map's second group); Settings' *Claude accounts* links and About's Services / HTTP API links; the *Refresh* buttons on live pages; *Info › Actions* | F1, F7, F14f |

### 6.12 What it costs

- Familiar: every page keeps its component and its look; it moves into a
  tab. A user of today finds the same screens one level deeper at most.
- *Phone & devices*, *Browser* and *HTTP API* move one click further away
  (Settings › …): they are set up once.
- Per-project settings stay on three pages (F10), now each reachable without
  the wizard; C's project settings sheet would gather them (§9).
- No server change but the words that name pages (F17), and the Help.

---

## 7. Direction B — One list, one detail

**In one line:** turn the work area into a mail-style screen — views on the
left, one list, the open item beside it — and move everything else into one
*Manage* area.

### 7.1 The map

The left column is a list of **views**, not pages: **Needs you** (every kind,
as A's Inbox) · **Running** (working, will resume, their agents as child rows,
role sessions) · **All sessions** (search, Projects as a grouping, earlier
sessions) · **Agents** · **Teams** (a team opens its board in the detail) ·
**Computers** (one view per computer: This Mac, DESKTOP-PC, BUILD-BOX, each
with its health dot) · **Manage**. *+ New ▾* (New session, Start a team) and
*Jump to… ⌘K* sit above the list.

| Place | What it holds | From today |
|---|---|---|
| Needs you | requests answered in the list; problems with their fix | Inbox + the five other places (F2) |
| Running · All sessions · Agents | the work, one list each | Sessions, Projects, history, Agents |
| Teams | teams; the board opens beside the list | Teams, a team |
| Computers | a view per computer | the Machine filter, Machines' cards |
| **Manage** | Accounts & usage · Backups · Recent changes · Computers (pairing, setup, what each may do, options, code sync, activity) · Phone & devices · Started elsewhere · Browser · HTTP API · Claude Code tools · Team templates · Capacity · Notifications · Appearance · Updates and about · Help — with a search | every other page |

**Home**: the *Needs you* view, the oldest request open beside it (or the
last session you had open). **Phone**: the list is the home, with a view
picker at the top; the open item takes the screen, as a session does today.
Bar: **Needs you · Sessions · Teams · Manage**. **Machines**: computers are
views of the one list, with chips; their setup, grants, options, code sync
and activity are *Manage › Computers*.

### 7.2 Screens

```
┌────────────────────┬──────────────────────────────────┬──────────────────────────────────────────┐
│ ◆ Mother Claude    │ NEEDS YOU 3           [+ New ▾]  │ ‹ Fix tray icon  [DESKTOP-PC] [Work]   ⋮ │
│ ‹⌕ Jump to… ⌘K›    │ ‹Search›                         │ ● Running · Editing tray.rs              │
│                    │ ──────────────────────────────── │  Chat    Agents 3    Changes 6    Info   │
│ ▸ Needs you      3 │ Fix tray icon · DESKTOP-PC       │  ━━━━                                    │
│   Running        5 │ Bash  cargo build --release      │ You: Make the tray icon match the theme. │
│   All sessions  31 │ [Allow]   [Deny]      4 min      │ Claude: Done on macOS. On Windows the    │
│   Agents         3 │ ──────────────────────────────── │ icon is set in tray.rs; building it.     │
│   Teams          1 │ Release notes                    │ ▸ Bash  cargo build --release            │
│ COMPUTERS          │ Which changelog format?          │ ┌ Claude needs you ────────────────────┐ │
│   This Mac       4 │ [Answer…]             1 min      │ │ Bash  cargo build --release          │ │
│   DESKTOP-PC ●   4 │ ──────────────────────────────── │ │                     [Allow]   [Deny] │ │
│   BUILD-BOX  ○   1 │ ✕ Work signed out on DESKTOP-PC  │ └──────────────────────────────────────┘ │
│ ────────────────── │ [Sign in on DESKTOP-PC…]         │ ‹Message›                         [Send] │
│   Manage           │ ──────────────────────────────── │                                          │
│   ? Help           │                                  │                                          │
│ ● Live          ☾  │                                  │                                          │
└────────────────────┴──────────────────────────────────┴──────────────────────────────────────────┘
```

```
┌─────────────────────────┬────────────────────────────────────────────────────────────────────────┐
│ ‹ Back to the work      │ Computers                               ✓ All good   [Pair a computer] │
│ ‹⌕ Search settings›     │  Overview    Code sync 2    Activity    Options                        │
│                         │  ━━━━━━━━                                                              │
│ YOUR CLAUDE             │ ┌ This Mac · MacBook ────────────────────────────────────────────────┐ │
│   Accounts & usage      │ │ macOS 27.2 · Mother Claude 0.11.0 · Claude Code 2.1.330            │ │
│   Backups               │ │ ✓ Full Disk Access   ✓ Local Network   ! Sleeps after 10 min       │ │
│   Recent changes        │ └────────────────────────────────────────────────────────────────────┘ │
│ YOUR COMPUTERS          │ ┌ DESKTOP-PC ───────────────────────────────────── ● Online · Direct ┐ │
│ ▸ Computers        ●    │ │ Windows 11 · 4 sessions, 1 running             [Open its sessions] │ │
│   Phone & devices       │ └────────────────────────────────────────────────────────────────────┘ │
│ TEAMS                   │                                                                        │
│   Templates             │                                                                        │
│   Capacity              │                                                                        │
│ CONNECTIONS             │                                                                        │
│   Started elsewhere     │                                                                        │
│   Browser               │                                                                        │
│   HTTP API              │                                                                        │
│   Claude Code tools     │                                                                        │
│ THIS DEVICE             │                                                                        │
│   Notifications         │                                                                        │
│   Appearance            │                                                                        │
│   Updates and about     │                                                                        │
│   Help                  │                                                                        │
└─────────────────────────┴────────────────────────────────────────────────────────────────────────┘
```

```
┌──────────────────────────────────────┐
│ ◆ Needs you ▾                   ⌕  ● │
│ ┌──────────────────────────────────┐ │
│ │ Fix tray icon · DESKTOP-PC       │ │
│ │ waiting 4 min · Work             │ │
│ │ Bash  cargo build --release      │ │
│ │ [Allow]                   [Deny] │ │
│ └──────────────────────────────────┘ │
│ ┌──────────────────────────────────┐ │
│ │ Release notes                    │ │
│ │ Which changelog format?          │ │
│ │ [Answer…]                        │ │
│ └──────────────────────────────────┘ │
│ ┌──────────────────────────────────┐ │
│ │ ✕ Work signed out on DESKTOP-PC  │ │
│ │ Sign in there, at DESKTOP-PC.    │ │
│ └──────────────────────────────────┘ │
├──────────────────────────────────────┤
│ Needs you 3  Sessions  Teams  Manage │
└──────────────────────────────────────┘
```

### 7.3 What changes, and what it costs

- **Merged**: Inbox, Sessions and Agents into one list with views; every
  setting and set-up page into *Manage* (Accounts, Backup & sync, Machines'
  configuration, Remote, Browser, API, Services, Capacity's policy,
  Settings).
- **Removed**: the sidebar of pages, the phone's *More* (now *Manage*), the
  Inbox and Agents pages (now views), the session page's own back button on
  wide screens (the list stays). **Renamed**: as A (§6.10).
- **Best at** driving many sessions on a big screen: answering, reading and
  switching sessions without leaving the list; the phone stays much as it is.
- **Costs**: the session page must work in a pane (today it owns the screen:
  its header, `sd-scroll`, `sd-dock`); the list needs keyboard and focus
  rules across three columns; routes become a layout with a child
  (`/:view` and `session/:id` inside it), and `#/session/<id>` from a push
  must still open on its own; *Manage* is long (its search helps); nearly
  every page path the Help names changes. A 1280-pixel laptop leaves the
  open session about 720 px — tight for the *Changes* tab's split diff.

---

## 8. Direction C — Projects first

**In one line:** make the project the main object: a project page gathers
its sessions on every computer, its team, its code and sync, and its own
settings; the Inbox, Computers, Accounts and Settings complete the map.

### 8.1 The map

Sidebar: **Inbox · Projects** (the pinned ones listed under it) **·
Computers · Accounts · Settings**. Phone bar: **Inbox · Projects · Computers ·
More**.

| Place | What it holds | From today |
|---|---|---|
| Inbox | everything waiting, as A | Inbox + the five other places |
| **Projects** | the overview, an *All sessions* lens (today's list) and an *Agents* lens; a **project page** with tabs Sessions · Team · Code · Settings | Sessions, Projects, Agents, Teams (by project), *Capacity*'s budgets and *why it waits*, the project options from the wizard, Accounts' per-project account, code sync |
| Computers | Overview · Activity · Options · the setup | Machines, minus code sync (now per project) |
| Accounts | Accounts · Usage · Backups · Recent changes | as A |
| Settings | as A, plus Team templates and the Capacity policy | as A |

**Home**: *Projects*. **Machines**: a project shows where it lives on each
computer (the roots, or folder pairs) and its code sync; *Computers* keeps
pairing, health, what each may do, options and activity.

### 8.2 Screens

```
┌──────────────────────┬───────────────────────────────────────────────────────────────────────────┐
│ ◆ Mother Claude      │ Projects   [Projects] [All sessions]           ‹Search›   [New session ▾] │
│                      │                                                                           │
│   Inbox            2 │ ┌ mother-claude ─────────────────────────────────────────── 1 needs you ┐ │
│ ▸ Projects           │ │ This Mac     ● 2 working   ~/development/projects/mother-claude       │ │
│     mother-claude  1 │ │ DESKTOP-PC   ● 1 working   D:\development\projects\mother-claude      │ │
│     tray-tool        │ │ Team “Ship 0.12”: 5 of 9 tasks done · 2 roles working                 │ │
│     win-tools        │ │ main · in sync with DESKTOP-PC · Work                          [Open] │ │
│     + 11 more        │ └───────────────────────────────────────────────────────────────────────┘ │
│   Computers        ● │ ┌ tray-tool ────────────────────────────────────────────────────────────┐ │
│   Accounts           │ │ This Mac     ○ idle · 2 h ago · Default                        [Open] │ │
│   Settings           │ └───────────────────────────────────────────────────────────────────────┘ │
│                      │ ┌ win-tools ────────────────────────────────────────────────────────────┐ │
│                      │ │ DESKTOP-PC only · ○ ended yesterday · Work                     [Open] │ │
│ ? Help               │ └───────────────────────────────────────────────────────────────────────┘ │
│ ● Live             ☾ │                                                                           │
└──────────────────────┴───────────────────────────────────────────────────────────────────────────┘
```

```
 ‹ Projects   mother-claude     [Work] [In sync with DESKTOP-PC]       [New session ▾]   [⋯]
  Sessions 3    Team    Code    Settings
  ━━━━━━━━━━
 THIS MAC — ~/development/projects/mother-claude
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ ● Release notes       Writing CHANGELOG.md                                             just now │
 │ ○ Fix flaky test      The test passes now.                                              1 h ago │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 DESKTOP-PC — D:\development\projects\mother-claude                          ● Online · direct
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ ● Fix tray icon       Bash waits for you: cargo build --release                       2 min ago │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 TEAM “SHIP 0.12” — 5 of 9 tasks done                                                 [Board ›]
   lead ● This Mac    ui ● DESKTOP-PC    tests ◌ waits: Work's session window is at 92%
```

```
  Sessions 3    Team    Code    Settings
                                ━━━━━━━━
 ┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ Account            [Work ▾]   New sessions and the team of this project run under it.           │
 │ On each computer   This Mac     ~/development/projects/mother-claude                            │
 │                    DESKTOP-PC   D:\development\projects\mother-claude                    [Edit] │
 │ Code sync          [x] Keep it in sync with DESKTOP-PC                       ✓ Synced 3 min ago │
 │ When it moves      Code travels [Machine to machine ▾]    Lands [In the project folder ▾]       │
 │                    Files git ignores that travel with it: .env.local                   [Change] │
 │ Team limits        $20 a day · 3 role sessions at once in this project                   [Edit] │
 └─────────────────────────────────────────────────────────────────────────────────────────────────┘
 Today these are on four pages: Accounts › Defaults › Per project, the setup's Each project,
 Capacity › Budgets, and Teams.
```

```
┌──────────────────────────────────────┐
│ ◆ Projects                         ● │
│ [Projects] [All sessions]          ⌕ │
│ ┌──────────────────────────────────┐ │
│ │ mother-claude        1 needs you │ │
│ │ This Mac ● 2 · DESKTOP-PC ● 1    │ │
│ │ Team: 5 of 9 tasks · in sync     │ │
│ └──────────────────────────────────┘ │
│ ┌──────────────────────────────────┐ │
│ │ tray-tool                   idle │ │
│ │ This Mac · 2 h ago · Default     │ │
│ └──────────────────────────────────┘ │
├──────────────────────────────────────┤
│ Inbox 2  Projects  Computers  More   │
└──────────────────────────────────────┘
```

### 8.3 What changes, and what it costs

- **Moved**: the Sessions list and Agents become lenses of *Projects*; a
  team moves into its project (templates and the Capacity policy into
  Settings); code sync and a project's options move from Machines to the
  project; the rest as A. **Removed**: Sessions as the home, the Teams and
  Capacity pages as places of their own. **Renamed**: as A (§6.10).
- **Fixes F10 for good**: a project's account, folders, sync, move options
  and team limits in one form; code sync lives where the code is.
- **Matches how a developer thinks** ("my repo"), and makes Machines'
  folders visible instead of hidden in the wizard.
- **Costs**: opening a session gains a level unless *All sessions* is the
  default lens; sessions outside any project (a home folder) need a bucket; a
  project must be one thing across computers (the setup's step 5 matches
  projects by their git remote; the rest needs the roots and folder pairs
  joined); the *Code* tab needs a project-level git view (today's *Changes*
  reads a session's folder); the largest Help rewrite.

---

## 9. Comparison and recommendation

**Clicks for the jobs that change most** (desktop unless said; from the home):

| Job | Today | A | B | C |
|---|---|---|---|---|
| J1 see everything waiting | 1 + five other pages | 1 | 0 | 1 |
| J2 answer a request (phone) | 2 | 2 | 1 | 2 |
| J3 open a session and come back to the list | 2 | 2 | 1 | 2 (*All sessions*) · 3 (a project) |
| J5 why a role session waits | 1 | 2 | 2 | 2 |
| J6 keep a project in sync | 7 | 4 | 5 | 4 |
| J8 every account's usage | 1 (the bar's sheet) · 4+ (Accounts) | 1 (the bar's *Usage ›*) · 2 | 2 | 2 |
| J9 undo a move, copy or delete | 1–3, in one of four lists | 2 | 2 | 2 |
| J10 pair a phone | 1 | 2 | 2 | 2 |
| J11 turn on Claude Code tools | 3 | 2 (*Settings › Claude Code tools*, or *This Mac*'s row) | 2 | 2 |
| J11 the plugin, for every account | two pages | 2, one table | 2 | 2 |

**The rest:**

| | A — Six places | B — One list, one detail | C — Projects first |
|---|---|---|---|
| Entries | 6 (+ Help) | 6 views + Manage | 5 |
| Phone bar | Sessions · Inbox · Teams · More | Needs you · Sessions · Teams · Manage | Inbox · Projects · Computers · More |
| Home | Sessions, as today | *Needs you*, the item open beside it | Projects |
| Machines | a place + a dimension | views + *Manage* | each project's computers + a place |
| Findings fixed | all but F10 (now reachable, still on three pages) | all but F10 | all |
| Today's components | almost all reused unchanged, as tabs | reused; the session page must fit a pane | reused; the project page, its *Code* tab and project matching are new |
| Server changes | none — the words naming pages | none | likely: a project's identity across computers, a project-level git view |
| For today's users | the same pages, in fewer places | a new desktop layout; the phone much as today | a new main object |
| Rough size | 5–7 weeks | 8–11 weeks | 9–12 weeks |
| Risk | low | medium | high |

**Recommended: A.** It removes what users trip over today — four maps, seven
places for what waits, four undo lists, six usage places, a hidden switch,
a plugin with two homes, a long Machines page whose project options hide in
the wizard — by putting the components the app already has into a clear
map. Nothing on the server changes but words, every old link redirects, and
each stage ships on its own. B and C are bigger bets on a new main screen;
their best ideas fit on top of A afterwards:

- **from B**, *list and detail on wide screens*: opening a session from
  Sessions or the Inbox on a screen ≥ 1280 px shows it beside the list
  (stage 9);
- **from C**, a **Project settings** sheet (account, folders on each
  computer, code sync, move options, team limits) from *Sessions › Projects*
  and the session menu — F10 solved without a new main object (stage 10);
- **from B**, **Jump to… ⌘K** over sessions, projects, computers, settings
  and Help topics (stage 11).

---

## 10. Migration plan for A

### 10.1 Rules for every stage

- **The map is one file.** `src/app/core/nav.ts` (new) holds the places,
  their tabs, labels, icons, badges, desktop-only flags and Help topics. The
  sidebar, the phone's bar, *More*, the top bar's title and `mc-hub-tabs`
  all read it; it replaces `nav`, `TITLES` and `MORE_SECTIONS`
  (`app.component.ts`) and `ENTRIES` (`more.component.ts`). Each stage moves
  routes and the map together.
- **Old routes redirect, with their query and fragment.** Write redirects
  relative, as `good-to-know` does (`{ path: 'api', redirectTo:
  'settings/api' }` keeps `?server=`), or as a function returning a
  `UrlTree`. An absolute string (`'/settings/api'`) drops the query and
  fragment (Angular 22.2's `applyRedirectCommands`). A prefix redirect
  carries the rest of the path: `machines` → `computers` also turns
  `/machines/setup` into `/computers/setup`. Redirects stay for good; the
  existing map in `app.routes.ts` guards them where they lead.
- **Never moved:** `#/pair?c=` (QR codes), `#/session/<id>?pending=` and
  `#/teams/<id>` (push, `public/sw.js`), `#/` (the Home Screen app), Help
  topic ids. When a Help heading is renamed, give its section the old slug
  as `id`, so `#fragment` links keep working.
- **Every stage is green:** `npm run lint`, `npm test`, `npm run build`,
  `npm run i18n:check`; plus `cargo fmt --check`, `cargo clippy … -D
  warnings` and `cargo test` when a Rust string changes, and `npm run
  ext:lint` / `ext:test` when the extension's does. Small Conventional
  Commits, none squashed.
- **Every new string has its Vietnamese** in the area's file: page names in
  `common.json`, the shell in `shell.json`, Computers in `machines.json`,
  Accounts and Backups in `data.json`, Remote and Services' strings in
  `control.json`, Teams, Browser and the API in `platform.json`, the Inbox in
  `core.json`. A Help topic changes in English (`help-topics.ts`) and
  Vietnamese (`pages/help/vi/topics-*.ts`) in the same commit, same sections
  in the same order.
- **Moved links are updated in the stage that moves them** — about 60
  `routerLink`s, `navigate` calls and Help `links` point at `/remote`,
  `/machines`, `/api`, `/backup`, `/browser`, `/capacity` or `/services`
  today — and so are the server and extension text (F17), even though the
  redirects would carry them.

### 10.2 Stages

| # | Stage | What you see | Size |
|---|---|---|---|
| 0 | **Groundwork** | nothing yet | M · 2–3 days |
| 1 | **The new map** over today's routes | the sidebar of six, the phone's bar and *More*, Settings' *Connections* rows | M · 2–3 days |
| 2 | **The Inbox holds everything waiting** | *Needs you* · *To fix* · *Waiting where it runs* | L · 4–5 days |
| 3 | **Computers** | Machines in four tabs; options without the wizard | L · 4–5 days |
| 4 | **Accounts** | Usage, Backups and Recent changes as tabs | L · 4–5 days |
| 5 | **Settings and its Connections** | Remote, the plugin, Browser, API, Claude Code tools under Settings | L · 4–5 days |
| 6 | **Sessions and Teams tabs** | Agents and Projects as tabs; Templates and Capacity under Teams | M · 2–3 days |
| 7 | **A session's menu** | the grouped ⋮ menu; *Info* without actions | S · 1–2 days |
| 8 | **One template, Help and docs** | the same header, empty states and phone lines everywhere | M · 3 days |
| 9–11 | *Later, borrowed*: list and detail on wide screens (B) · Project settings (C) · Jump to… (B) | | M–L each |

S ≈ a day or two, M ≈ two or three, L ≈ four or five, for one developer
with Claude, Help and Vietnamese included: stages 0–8 come to about
26–34 working days.

**Stage 0 — Groundwork.** `core/nav.ts` with today's entries (the UI stays
identical) and its spec (every route in `app.routes.ts` is in the map or
listed as reached only by link); `shared/page-head.component.ts`,
`shared/hub-tabs.component.ts`, `shared/empty-state.component.ts`,
`shared/desktop-only.component.ts`, each with a spec. The shell reads the
map; nothing else changes.

**Stage 1 — The new map.** The map gets the six places and their tabs, each
pointing at **today's** routes: Computers → `/machines`, Teams' *Capacity*
tab → `/capacity`, Accounts' *Backups* tab → `/backup`, Sessions' *Agents*
tab → `/agents`. The phone's bar becomes Sessions · Inbox · Teams · More;
*More* lists Computers, Accounts, Settings and Help with a state line each.
Settings gets the *Connections* rows (to `/remote`, `/remote#foreign`,
`/browser`, `/api`, `/services`) and loses its *Claude accounts* links and
About's Services / HTTP API links. The Computers badge keeps
`MachinesService.alarm`. Help: *Getting started*, *Use it from your phone*,
*Settings and updates*. Specs: the shell's sidebar and bar from the map,
*More*.

**Stage 2 — The Inbox holds everything waiting.** `InboxComponent` gets
*To fix* and the new items of §6.4 through a small `mc-attention-card`
(title, one line of why, one action), all from services that exist;
`AttentionService` adds a to-fix count beside `pendingCount` (the badge
stays requests answerable here; pairing requests and asks count on the
desktop). A *Computer* filter beside *Account*. Remote's pairing card and
the Machines asks card are removed from those pages (the dialog and toasts
stay); Browser's *Waiting for you* gets "Also in the Inbox". Help:
*Approvals and the Inbox*. Specs: one per item kind; the badge counts.

**Stage 3 — Computers.** Routes `/computers`, `/computers/sync`,
`/computers/activity`, `/computers/options`, `/computers/setup`; redirect
`machines` → `computers`. `MachinesComponent` splits into four thin pages
over the cards it already uses (`mc-machine-asks` goes in stage 2;
`mc-machines-sync`, `mc-machines-runs`, `mc-move-card`,
`mc-machines-options`); *Options* adds `mc-machines-accounts-folders` and
`mc-machines-project-options` from the wizard; *Code sync* adds the *Keep a
project in sync…* sheet. *This Mac* takes `mc-permissions-list` (out of
Settings) and a *Details* sheet with Services' daemon, MCP servers and
background jobs. The name, if chosen (§11): page, buttons ("Pair a
computer"), `machines-text.ts`, `machines.json`, the 9 Machines Help topics,
and the server's "Machines › Set up" / "Machines › Options › Each project"
(`machines/setup/asks.rs`, `machines/sync/on.rs`,
`machines/moves/ignored.rs`). Specs: `machines.component.spec.ts` split by
page; `app.routes.spec.ts` checks the redirects keep `?` and `#`.

**Stage 4 — Accounts.** Routes `/accounts/usage`, `/accounts/backups`,
`/accounts/changes`; redirect `backup` → `accounts/backups`. *Usage* is the
plan usage sheet's list as a page, with *Read usage every* (out of
Settings) and the rate limits from Capacity; the bar's meter and the plan
usage component's "Change in Settings" link to it. *Recent changes* lists
`/data/jobs` unfiltered, with chips; Accounts, Backups and Projects keep a
short list and *See all*. The plugin buttons become one line. Help: *Claude
accounts*, *Move, copy and delete*, *Backup and sync* (renamed *Backups*).
Specs: accounts and backup specs follow their sections; one for the merged
list.

**Stage 5 — Settings and its Connections.** Routes `/settings/devices`,
`/settings/elsewhere`, `/settings/browser`, `/settings/api`,
`/settings/tools`; redirects `remote` (a function: `#foreign` →
`/settings/elsewhere`, else `/settings/devices`), `browser`, `api` (keeps
`?server=`), `services`. *Sessions started elsewhere* joins Remote's
*Foreign-session control* (`/control/foreign`) and the per-account
`/accounts/:id/hooks/install` in one table. `app.component.ts` skips the
pair dialog on `section() === 'remote'`: follow the move. On a phone,
*Phone & devices* shows this phone and the list read only. Server and
extension text: "(Remote › …)" (`extension/connect.rs`), "the API screen"
(`bridge/mod.rs`), "on its Browser page" (the extension). Help: *Use it from
your phone*, *Internet access and devices*, *Sessions started elsewhere*,
*The browser extension*, *Claude Code tools*, *The HTTP API*.

**Stage 6 — Sessions and Teams tabs.** Route `/projects` (the
`ProjectsViewComponent` behind the switch; `mc_sessions_view` dropped);
*Teammates* moves from Teams to Agents. Routes `/teams/templates` and
`/teams/capacity` before `/teams/:id` (team ids are `tm_` + 12 hex: no
clash); redirect `capacity` → `teams/capacity`; Capacity's sections renamed.
Help: *The Sessions list*, *Teams of sessions*, *Capacity, routing and
budgets*.

**Stage 7 — A session's menu.** `SessionHeaderComponent`'s sheet in four
headed groups (§6.8); *Info* keeps facts; *Respawn* → *Restart*, *Continue
here (fork)* → *Continue here, as a new session*. Help: *A session's page*.

**Stage 8 — One template, Help and docs.** `mc-page-head` on every page,
one-sentence ledes, the width rule, `mc-empty-state`, `mc-desktop-only`, no
*Refresh* on live pages. Help's groups follow the map — Basics · Your
sessions (with the Inbox) · Teams · Your computers · Accounts and data ·
Settings and connections · Troubleshooting — and `help.component.spec.ts`
follows `HELP_GROUPS`. README, CLAUDE.md and the docs that name pages
(REMOTE, BACKUP, ACCOUNTS, MACHINES, BROWSER, BRIDGE, MCP, TEAMS, ROUTING).

### 10.3 Routes after A

| Today | After A | How |
|---|---|---|
| `/`, `/inbox`, `/new`, `/agents` | the same | — |
| the *Projects* switch on `/` | `/projects` | new route |
| `/session/…` (all four) | the same | — |
| `/teams`, `/teams/:id` | the same, + `/teams/templates` | — |
| `/capacity` | `/teams/capacity` | relative redirect |
| `/machines`, `/machines/setup` | `/computers`, `/computers/setup` (+ `sync`, `activity`, `options`) | one prefix redirect |
| `/accounts` | the same, + `usage`, `changes` | — |
| `/backup` | `/accounts/backups` | relative redirect |
| `/settings` | the same, + `devices`, `elsewhere`, `browser`, `api`, `tools` | — |
| `/remote`, `/remote#foreign` | `/settings/devices`, `/settings/elsewhere` | redirect function |
| `/browser`, `/api?server=`, `/services` | `/settings/browser`, `/settings/api?server=`, `/settings/tools` | relative redirects |
| `/more`, `/help…`, `/good-to-know`, `/pair`, `/onboarding` | the same | — |

---

## 11. Decisions for you

> **Taken on 2026-11-12:** 1 — **A**; 2 — **Computers**; 3 — **Teams**;
> 4 — **everything waiting**. Decisions 5–9 follow the recommendations
> below (Backups as a tab of Accounts; Phone & devices under *Settings ›
> Connections*; one *Recent changes* list, computer moves in *Computers ›
> Activity*; permissions and Claude Code's background service under
> *Computers › This Mac*; the borrowed stages last). Built after the
> remaining Machines work (Phases 3b and 5, and the gaps), as the user
> asked: finish the features, then redesign, then one release.

1. **The direction** — **A** *(recommended)*, B or C.
2. **Machines' name** — **Computers** *(recommended: Help's group and every
   sentence already say computer; the alternative UX.md decision 3 set
   aside)*, or keep *Machines*.
3. **The phone's third tab** — **Teams** *(recommended: a team's board is
   what you check from a phone; another computer's sessions are already on
   Sessions)*, or *Computers*.
4. **The Inbox's reach** — **everything waiting**, requests and problems
   *(recommended)*, or requests only, with problems left on their pages.
5. **Backup & sync** → **Backups**, a tab of Accounts *(recommended)*, or
   keep its name and entry.
6. **Remote** → **Phone & devices**, under *Settings › Connections*
   *(recommended)*, or keep it in the map.
7. **One *Recent changes* list** for data jobs, computer moves staying in
   *Computers › Activity* *(recommended)*, or one list for both.
8. **Permissions and Claude Code's background service** under *Computers ›
   This Mac* *(recommended)*, or kept in Settings and Services.
9. **The borrowed stages** — list and detail on wide screens, the Project
   settings sheet, *Jump to… ⌘K*: which, and when.
