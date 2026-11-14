import { HelpGroup, HelpTopic, HelpTopicSource, resolveTopic } from './help-model';

/*
 * The Help center's content: one entry per topic, rendered by
 * HelpComponent. Write for someone using the app, not reading the code:
 * short sentences, "you", the labels exactly as the app shows them (in
 * **bold**), one action per step. Say "your computer" or "the desktop app",
 * not "the Mac", unless quoting a label that says Mac. Check every claim
 * against the docs and the templates when the app changes.
 */

export const HELP_GROUPS: readonly HelpGroup[] = [
  { id: 'basics', title: 'Basics' },
  { id: 'sessions', title: 'Your sessions' },
  { id: 'teams', title: 'Teams' },
  { id: 'machines', title: 'Your computers' },
  { id: 'data', title: 'Accounts and data' },
  { id: 'settings', title: 'Settings and connections' },
  { id: 'troubleshooting', title: 'Troubleshooting' },
];

const TOPICS: HelpTopicSource[] = [
  // ───────────────────────────────────────────────────────────── Basics
  {
    id: 'getting-started',
    title: 'Getting started',
    icon: 'sparkle',
    group: 'basics',
    summary: 'Install the app, connect it to Claude Code and grant its permissions.',
    intro:
      'Mother Claude runs on the computer where you use Claude Code. It shows every Claude Code session there, lets you control them, and serves the same dashboard to your phone.',
    keywords: [
      'install',
      'download',
      'setup',
      'first launch',
      'onboarding',
      'welcome',
      'jump to',
      'search',
      'command palette',
    ],
    sections: [
      {
        heading: 'What you need',
        points: [
          'A Mac with Apple silicon, or a Windows PC (x64). Intel Macs are not supported yet.',
          'The **Claude Code CLI** (`claude`), installed and signed in on the same computer. Mother Claude reads its sessions and starts new ones with it.',
          'Optional: `tmux`, to type into sessions that run in a tmux pane.',
        ],
      },
      {
        heading: 'Install on a Mac',
        steps: [
          "Download the `.dmg` from the project's **Releases** page.",
          'Open it and drag **Mother Claude** to Applications.',
          'Open Mother Claude.',
        ],
        tips: ['The Mac app is signed and notarized, so macOS opens it like any other app.'],
      },
      {
        heading: 'Install on Windows',
        steps: [
          "Download the `…-setup.exe` from the project's **Releases** page (or the `.msi`, for managed installs).",
          'Run it.',
          'If SmartScreen warns you, choose **More info**, then **Run anyway**.',
          'Open Mother Claude.',
        ],
        notes: ['SmartScreen warns because the Windows installers are not signed yet.'],
      },
      {
        heading: 'Grant Full Disk Access on a Mac',
        where: 'desktop',
        intro:
          'macOS protects the folder where Claude Code keeps its sessions. The first time you open the app, the **Welcome to Mother Claude** screen asks for **Full Disk Access**. Windows needs nothing like it.',
        steps: [
          'Click **Open System Settings**.',
          "Under Privacy & Security › Full Disk Access, find **Mother Claude**. If it isn't listed, click **+** and add it; **Reveal app in Finder** shows where it is.",
          'Turn its switch on. macOS may ask you to quit and reopen the app.',
          'Come back and click **Re-check**.',
          'Click **Continue to dashboard →**.',
        ],
        tips: [
          "**Skip for now** lets you in, but the app can't read your sessions and a banner stays until you finish. Come back any time under **Computers › This Mac**: **Permissions** opens the list.",
          'Local Network is optional: when macOS asks whether Mother Claude may find devices on your local network, click **Allow**, so your phone can reach it.',
        ],
        links: [{ label: 'Open Computers', path: '/computers' }],
      },
      {
        heading: 'Find your way around',
        points: [
          'On the desktop app, the sidebar has three groups. **Work**: **Sessions**, **Inbox** and **Teams**. **Connections**: **Computers**, **Phone & devices**, **Sessions started elsewhere**, **Browser** and **HTTP API**. **Manage**: **Accounts**, **Backups** and **Settings**. **Help** is at the bottom.',
          'A place with more than one page shows them as tabs under its name: **Sessions**, **Agents** and **Projects**; **Teams**, **Templates** and **Capacity**; **Computers**: **Overview**, **Code sync**, **Activity** and **Options**; **Accounts**: **Accounts**, **Usage** and **Recent changes**. **Settings** opens **Claude Code tools** on a page of its own.',
          'On a phone, the tabs at the bottom are **Sessions**, **Inbox**, **Teams** and **More**. **More** has the other places under the same headings, **Connections** and **Manage**, then **Help**, each with a line saying how it stands or what it is for.',
          'The dot at the bottom of the sidebar (at the top on a phone) shows the connection. **Live** means updates arrive as they happen.',
          'The button beside it switches the theme between dark, light and system. **Settings › Appearance** also has the language, the accent colour and the background.',
          'The first time, a short **Welcome to Mother Claude** note explains how the app works. Press **Got it** to close it.',
        ],
      },
      {
        heading: 'Jump to…',
        intro:
          '**Jump to…** finds anything by a few letters of its name: a session on any of your computers, a project, a computer, a page, a part of **Settings** or a Help topic.',
        steps: [
          'Press ⌘K on a Mac (Ctrl+K on Windows), or **Jump to…** at the top of the sidebar; on a phone, the search button at the top.',
          "Type a few letters: of a session's name, its project, folder or branch; of a page's name (old names work too); of a Help topic.",
          'Move with ↑ and ↓, and press Enter to open what is chosen. Esc closes it.',
        ],
        points: [
          'What you opened lately comes first, and leads the list before you type.',
          'A project opens its settings; another computer opens its sessions.',
          'Accents can be left out: “sao luu” finds “sao lưu”.',
        ],
      },
      {
        heading: 'What to do next',
        points: [
          'Start a session with **New session** on the **Sessions** page: sessions started there can be fully controlled.',
          'Pair your phone on the desktop app, under **Phone & devices**.',
          'To answer sessions you started in a terminal or VS Code, install the Mother Claude plugin under **Sessions started elsewhere**.',
        ],
        links: [
          { label: 'Open New session', path: '/new' },
          { label: 'See Use it from your phone', path: '/help/phone' },
          { label: 'See Sessions started elsewhere', path: '/help/foreign-sessions' },
        ],
      },
    ],
  },
  {
    id: 'good-to-know',
    title: 'Good to know',
    icon: 'book',
    group: 'basics',
    summary: 'How Mother Claude works, and the limits worth knowing.',
    intro:
      'A few things about how Mother Claude bridges your computer to your phone, and what it can and cannot do.',
    keywords: [
      'limitations',
      'limits',
      'awake',
      'sleep',
      'laptop',
      'message by message',
      'one driver',
      'local first',
      'privacy',
    ],
    sections: [
      { heading: 'The short version', embed: 'limitations' },
      {
        heading: 'Sessions Mother Claude runs, and the others',
        intro:
          'Mother Claude can fully control a session only when it started it: such a session is **owned**. Sessions you start in a terminal, VS Code, JetBrains or Claude Desktop are started elsewhere. You can always watch them, and answer and message them through the Mother Claude plugin or tmux.',
        points: [
          'Owned sessions take messages, images, approvals, model, effort and mode changes, interrupts and slash commands, and their replies stream word by word.',
          '**Continue here** turns any session into an owned one. A session still running elsewhere goes on here as a new session, a copy; an ended one resumes in place.',
          "Each session's badge says how far it can be controlled: **Full control**, **Via hooks**, **Via tmux**, **Experimental** or **Watch only**. Tap it to see what works and why.",
        ],
        links: [
          { label: 'See Messages and control', path: '/help/messaging' },
          { label: 'See Sessions started elsewhere', path: '/help/foreign-sessions' },
        ],
      },
      {
        heading: 'What stays on your computer',
        points: [
          'Your sessions, transcripts and tokens stay on your computer. There is no Mother Claude cloud.',
          'Each paired phone has its own key, and you can revoke any device on its own.',
          "Mother Claude never edits your Claude Code settings. Its plugin is installed and removed with Claude Code's own `claude plugin` commands.",
          "It writes into Claude Code's folders only when you ask it to: adding a directory account, moving, copying or deleting conversations, copying customizations, restoring or pulling a backup.",
        ],
      },
      {
        heading: 'Risky actions stay with you',
        intro:
          'Some actions are **dangerous**: they let Claude run without approval, or change settings beyond one request.',
        points: [
          'Examples: switching a session to **Bypass permissions**, saving "always allow" rules to settings files, approving a tool call marked dangerous, removing or deleting sessions, and sending raw keys to a terminal.',
          'The desktop app can always do them. A phone can only after you allow it on the desktop app under **Phone & devices**, separately for your home network and for the internet.',
        ],
        links: [{ label: 'See Internet access and devices', path: '/help/internet' }],
      },
    ],
  },
  {
    id: 'settings',
    title: 'Settings and updates',
    icon: 'settings',
    group: 'settings',
    summary: 'Notifications, theme, connection and software updates.',
    intro:
      'Settings holds what you set once: this device’s notifications, theme and connection, and Mother Claude’s tools for Claude Code. In the desktop app it also has software updates; macOS permissions are under **Computers › This Mac**.',
    keywords: ['preferences', 'theme', 'dark mode', 'light mode', 'update', 'upgrade', 'version'],
    sections: [
      {
        heading: "What's in Settings",
        terms: [
          {
            term: 'Notifications',
            text: 'Push notifications for this device, the events that notify you, and in-app alerts.',
          },
          {
            term: 'Appearance',
            text: 'The language, the theme (**Dark**, **Light** or **System**), the accent colour and the background.',
          },
          {
            term: 'Claude Code',
            text: '**Claude Code tools**: a row saying how they stand, which opens their page, where **Settings** at the top goes back.',
          },
          {
            term: 'Connection',
            text: 'The server, how this device connects, whether it may take dangerous actions, and live updates.',
          },
          {
            term: 'Home Screen app',
            text: 'A one-time code for the iPhone Home Screen app (paired devices only).',
          },
          {
            term: 'Startup',
            text: 'Start Mother Claude when you sign in, in the tray (desktop app only).',
          },
          { term: 'Software updates', text: 'New versions of the app (desktop app only).' },
          { term: 'About', text: 'The version, and **Help**.' },
        ],
        links: [{ label: 'Open Settings', path: '/settings' }],
      },
      {
        heading: 'Language, theme and colours',
        steps: [
          'Open **Settings** (on a phone, under **More**).',
          'Under **Appearance**, pick the **Language**: **English** or **Tiếng Việt**. The whole app switches at once.',
          'Pick the **Theme**: **Dark** (the default), **Light**, or **System** to follow your computer or phone.',
          'Pick an **Accent** colour, and a **Background**: **Standard**, **Midnight**, **Black** (true black, easy on OLED screens) or **Warm**.',
        ],
        tips: [
          'On the desktop app, the button at the bottom of the sidebar switches the theme too.',
          'Each device keeps its own choices: the desktop app and your phone can differ.',
          "Some text still comes in English whatever the language: what the desktop app's own service writes (error messages, the reasons on **Capacity**, decisions in a conversation, notifications on your phone) and the browser extension.",
        ],
      },
      {
        id: 'plan-usage-refresh',
        heading: 'Plan usage moved to Accounts',
        intro:
          "Settings no longer has **Plan usage**. How often your Claude plan's usage is read is **Read usage every** on **Accounts › Usage**, beside how much of each plan is left.",
        links: [
          { label: 'Open Usage', path: '/accounts/usage' },
          {
            label: 'See How often usage is read',
            path: '/help/accounts',
            fragment: 'how-often-usage-is-read',
          },
        ],
      },
      {
        id: 'connections-moved',
        heading: 'Connections and Backups moved to the sidebar',
        intro:
          'Settings no longer lists **Connections**, and **Accounts** no longer has a **Backups** tab. **Phone & devices**, **Sessions started elsewhere**, **Browser** and **HTTP API** are places of their own under **Connections** in the sidebar, and **Backups** under **Manage**; on a phone they are in **More**, under the same headings. Old links and bookmarks still open them.',
      },
      {
        heading: 'Update the app',
        where: 'desktop',
        intro:
          'The desktop app looks for a new version when it starts. When one is ready, **Update available** shows at the bottom of the sidebar.',
        steps: [
          'Click **Update available**, or open **Settings › Software updates**.',
          'To look again right now, click **Check for updates**.',
          'Read the release notes.',
          'Click **Download & install**.',
          'Wait while it shows **Downloading…**, **Installing…** and **Update installed — restarting…**. The app restarts by itself.',
        ],
        notes: [
          'On Windows the app closes while the update installs.',
          'After the restart the HTTP API is off until you start it again: **HTTP API**.',
        ],
      },
      {
        heading: 'Start when you sign in',
        where: 'desktop',
        intro:
          'A computer nobody opens Mother Claude on after a restart, such as a build box or a desktop at home, can start it by itself, with no window, so your phone and your other computers still reach it.',
        steps: [
          'Open **Settings** in the desktop app.',
          'Under **Startup**, switch on **Start Mother Claude when you sign in, in the tray**.',
        ],
        points: [
          'The next time you sign in to the computer, Mother Claude starts with no window, only its icon in the tray (the menu bar on a Mac), and serves your phone and your other computers as usual.',
          'On Windows, click the icon to show the window, or right-click it for **Open Mother Claude** and **Quit Mother Claude**. On a Mac, click the icon in the menu bar and choose **Open Mother Claude**.',
          'Closing the window keeps it running in the tray; **Quit Mother Claude** ends it.',
          'Opening Mother Claude again while it runs brings its window to the front: a second copy never starts.',
          'It is off until you switch it on, and switching it off removes it from the computer’s startup items. On Windows, **Task Manager › Startup apps** can switch it off too: **Settings** then says so, and switching it on here starts it again.',
        ],
      },
      {
        heading: 'Connection',
        points: [
          '**Connection** shows the server this device uses, how it connects, whether it may take dangerous actions, and the state of live updates.',
          '**Reconnect** connects again right away.',
          'In the desktop app, **Pair a phone** opens **Phone & devices**. On a phone, **Unpair this device** removes it.',
        ],
        links: [{ label: 'See Use it from your phone', path: '/help/phone' }],
      },
    ],
  },

  // ─────────────────────────────────────────────────────── Your sessions
  {
    id: 'sessions',
    title: 'The Sessions list',
    icon: 'sessions',
    group: 'sessions',
    summary: "Every Claude Code session on your computer, what it's doing, and how to find one.",
    intro:
      '**Sessions** lists every Claude Code session on your computer that is live or was active in the last 24 hours, whatever started it: a terminal, VS Code, JetBrains, Claude Desktop, a background job or Mother Claude itself. Its other tabs, **Agents** and **Projects**, show the agents at work and every project on disk.',
    keywords: [
      'list',
      'filter',
      'status',
      'app',
      'surface',
      'projects',
      'agents',
      'teammates',
      'agent teams',
      'fleet',
      'project settings',
      'split view',
      'keyboard',
    ],
    sections: [
      {
        heading: 'How the list is grouped',
        terms: [
          {
            term: 'Needs you',
            text: 'A permission, question or plan waits for you. The oldest comes first.',
          },
          { term: 'Working', text: 'Claude is working on a turn, or the session is starting.' },
          {
            term: 'Will resume',
            text: "The turn ended, but work it started still runs, or a scheduled wakeup will wake it. It isn't done.",
          },
          { term: 'Idle', text: 'Waiting for your next message.' },
          { term: 'Ended recently', text: 'Ended, failed or stopped in the last 24 hours.' },
        ],
      },
      {
        heading: 'Read a session card',
        points: [
          'A card shows the status, what Claude is doing right now, and chips for where it runs, its model, effort, mode, agents, context and cost.',
          'When one simple request waits, you can answer it on the card. Otherwise press **Review**.',
          "Tap the session's name to open its page.",
          'With more than one Claude account, a coloured chip names the account.',
        ],
      },
      {
        heading: 'A session beside the list',
        intro:
          "On a wide screen, 1280 pixels or more, a session you open from **Sessions**, **Agents** or the **Inbox** shows beside the list instead of taking its place. Its address stays its own, so a bookmark, the browser's **Back** and a notification open it as before.",
        points: [
          'The open session is marked in the list. Open another and it takes its place beside the list, which stays where you left it.',
          '**✕** at the top left of the session, or **Esc**, closes it and gives the list the whole width again.',
          'A narrower window, or a phone, opens a session on its own page.',
        ],
        tips: [
          'On a keyboard, **↓** and **↑** move through the list and **Enter** opens what you are on; from **Search sessions**, **↓** goes into the list.',
        ],
      },
      {
        heading: 'Right now',
        intro:
          'The row at the top counts what is going on: sessions **working**, sessions that **will resume**, **agents running**, sessions that **need you**, and the cost so far today. Each part is a shortcut.',
        points: [
          "Beside it, your Claude plan's usage shows the 5-hour window and the week. Tap it to open **Accounts › Usage**: every window of every account, with its reset time.",
          'Costs are estimates, counted since midnight.',
        ],
      },
      {
        heading: 'Find a session',
        steps: [
          'Type in **Search sessions**.',
          'Choose **Live**, **Needs you** or **All**.',
          'Narrow it with **All apps** (the app a session runs in: the CLI, VS Code, JetBrains, Claude Desktop…) and **All projects**, and with several accounts or computers, **All accounts** and **All computers**.',
          'Under **All**, press **Show ended sessions** to include earlier ones; **Load more** goes further back.',
        ],
        tips: ['When nothing matches, **Clear filters** shows everything again.'],
      },
      {
        heading: 'Select several sessions',
        steps: [
          'Press **Select**.',
          'Tick the sessions you want, or press **Select all** to take every session the search and filters match.',
          'Press **Delete N…** or **Move or copy N…**.',
          'Press **Done** when you have finished.',
        ],
        links: [{ label: 'See Move, copy and delete', path: '/help/move-data' }],
      },
      {
        heading: 'Projects',
        intro:
          '**Projects**, a tab of **Sessions**, lists every project folder that holds conversations, in every account: how many conversations it has, how many run, their size and the last activity.',
        points: [
          "A project's menu opens its settings, shows its folder on disk (desktop app), copies its path, moves or copies it, or deletes it.",
          '**Select** picks several projects to delete, move or copy together.',
          'Deleted projects stay under **Recently deleted** for seven days, with **Undo**. It shows the newest three; **See all** opens every delete on **Accounts › Recent changes**.',
        ],
        links: [{ label: 'Open Projects', path: '/projects' }],
      },
      {
        heading: "A project's settings",
        intro:
          "A project's own settings are in one sheet: press **⚙** on its row in **Projects**, or **Project settings…** in a session's **⋮** menu. Each part is changed here as on its own page.",
        terms: [
          {
            term: 'Account',
            text: 'The account new sessions in its folder run under: its own, or the default. The same rules as **Accounts › Defaults › Per project**.',
          },
          {
            term: 'On each computer',
            text: 'Where it is on your other computers, through the folders they share (**Computers › Options**).',
          },
          {
            term: 'Code sync',
            text: 'Whether its code is kept the same on another computer, with **Sync now** and **Turn off**.',
          },
          {
            term: 'When it moves',
            text: 'How its code travels and lands, what Claude is asked to run after a move, the files git ignores that go with it, the computers that keep a copy of its sessions, and turning code sync on.',
          },
          {
            term: 'Team limits',
            text: 'How many role sessions its teams run at once and what it may spend a day, with what it spent today.',
          },
        ],
        notes: [
          'The computers’ parts show once **Computers** is in use. On a phone, what only the desktop app changes shows how it is set, and says so.',
        ],
      },
      {
        heading: 'Agents',
        intro:
          '**Agents**, a tab of **Sessions**, shows every running subagent, workflow agent, background shell and monitor, grouped by session.',
        points: [
          'Tap an agent to read its own transcript, live while it runs. A shell or monitor shows its output.',
          'Stop one where its session allows it.',
          'The number on **Agents** counts the agents running.',
          "**Teammates**, below them, lists what Claude Code's own agent teams in your interactive sessions reported: a teammate gone idle, a task created or completed. Mother Claude only observes them; they are not its teams.",
        ],
        links: [
          { label: 'Open Sessions', path: '/' },
          { label: 'Open Agents', path: '/agents' },
        ],
      },
    ],
  },
  {
    id: 'session-page',
    title: "A session's page",
    icon: 'file',
    group: 'sessions',
    summary: 'Chat, agents, changes and info for one session, and its menu.',
    intro:
      'Tap a session to open its page. The top shows its name, its status and what Claude is doing right now. Below are four tabs: **Chat**, **Agents**, **Changes** and **Info**.',
    keywords: ['transcript', 'conversation', 'diff', 'git', 'tabs', 'menu', 'status'],
    sections: [
      {
        heading: 'The top of the page',
        points: [
          'The arrow at the top left goes back to **Sessions**; beside the list on a wide screen it is **✕**, which closes the session. The line under the name says what Claude is doing, such as the tool it runs, and for how long.',
          'The next row shows where the session runs and how it is reached (for example **VS Code · Via hooks**), then its model, permission mode and effort, its context use and cost.',
          "Tap a chip to change it. A chip that can't be changed explains why when you tap it.",
          '**⋮** opens the session menu: what you can do with the session.',
        ],
      },
      {
        heading: 'What the status means',
        terms: [
          { term: 'Working', text: 'Claude is working on a turn.' },
          { term: 'Needs you', text: 'A permission, question or plan waits for you.' },
          {
            term: 'Background work',
            text: 'The turn ended, but work it started (agents, shells, monitors) still runs. It takes messages like an idle session.',
          },
          {
            term: 'Scheduled',
            text: 'A wakeup or a scheduled job will wake it. The page says when.',
          },
          { term: 'Idle', text: 'Waiting for your next message.' },
          { term: 'Starting', text: 'Claude is starting.' },
          { term: 'Ended', text: 'The session ended. You can continue it.' },
          { term: 'Failed', text: 'It stopped on an error.' },
          { term: 'Stopped', text: 'It was stopped.' },
        ],
        tips: [
          'A session that resumes by itself says so on its page, and what would stop it (for example closing VS Code).',
        ],
      },
      {
        heading: 'Chat',
        points: [
          "The conversation: your messages, Claude's replies, the tools it ran and their results.",
          'Sessions Mother Claude runs stream word by word. Sessions started elsewhere show each message when it is complete.',
          '**Load earlier** shows older messages.',
          'Waiting approvals, questions and plans appear above the message box, one at a time.',
        ],
        links: [{ label: 'See Messages and control', path: '/help/messaging' }],
      },
      {
        heading: 'Agents',
        points: [
          'Subagents as a tree, workflow runs with their phases, and background shells and monitors with their latest output.',
          'Tap an agent to read its transcript. Stop one where the session allows it.',
        ],
      },
      {
        heading: 'Changes',
        points: [
          "The session's git changes. **Uncommitted** shows work not yet committed; **Whole branch** shows everything since the branch left the default branch.",
          "Show the files as a **List** or a **Tree**, and a file's changes **Unified** or **Split**.",
          '**Refresh** reads them again.',
        ],
      },
      {
        heading: 'Info',
        terms: [
          {
            term: 'Session',
            text: "Name, status, id, project and folder, the transcript's path, where it runs, its Claude account, branch and pull request. Every path has **Copy path**.",
          },
          {
            term: 'Model and usage',
            text: 'Model, permission mode, effort, ultracode, thinking, fast mode, cost and tokens.',
          },
          {
            term: 'Plan usage',
            text: "The plan of the session's account, with **Refresh**. In the desktop app, **Change in Accounts › Usage** sets how often it is read.",
          },
          {
            term: 'From here you can',
            text: 'What works for this session, and the button that fixes each limit.',
          },
          { term: 'MCP servers', text: 'The MCP servers of a session Mother Claude runs.' },
        ],
        tips: [
          "Info holds what is known about the session; what you can do with it is the session menu. In the desktop app, **Show in Finder** or **Show in File Explorer** beside the transcript's path opens its folder.",
        ],
      },
      {
        heading: 'The session menu',
        intro:
          'Press **⋮** at the top right. It offers what applies to this session, in four groups:',
        terms: [
          {
            term: 'Continue',
            text: "**Continue here** takes over a session Mother Claude doesn't run, with launch options; while it still runs elsewhere, **Continue here, as a new session** drives a copy from here. **Bring back to this Mac** (or **this PC**) brings back one another computer runs. **Switch account…** runs it under another Claude account, **Continue on another account…** for a session started elsewhere. **Continue on another computer…** moves it, with its code, to another of your computers; **Keep a copy on…** keeps a copy there after each turn.",
          },
          {
            term: 'Organize',
            text: '**Rename…** (for a VS Code session the new name shows only in Mother Claude), **Move or copy…** the conversation, or its whole project with **Move or copy this project…**, to another account or a storage, and **Delete…**, with seven days to undo.',
          },
          {
            term: 'This session',
            text: 'On a phone, its **Notifications** (**Follow**, **Default** or **Mute**) and **Notify me when this turn finishes** while it works. **Terminal view**, what its terminal shows (tmux sessions and background jobs), with the keys that answer a dialog open there; **Open in VS Code**; **Session info**, the **Info** tab; and the `claude --resume …` command, which **Copy** puts on your clipboard.',
          },
          {
            term: 'Its process',
            text: "A background job's **Stop**, **Restart** and **Remove**. **Remove** needs the desktop app, or a device allowed dangerous actions.",
          },
        ],
        tips: [
          'Another computer’s session leaves out what only that computer does (switching its account, moving, copying, deleting it), and the menu says so.',
        ],
      },
    ],
  },
  {
    id: 'messaging',
    title: 'Messages and control',
    icon: 'send',
    group: 'sessions',
    summary: 'Send messages and files, Send now, interrupt, and switch model, effort and mode.',
    intro:
      'What you can do with a session depends on who started it. Sessions Mother Claude started (**owned**) can be fully controlled. Sessions started elsewhere are reached through the Mother Claude plugin or tmux, or taken over with **Continue here**.',
    keywords: [
      'owned',
      'foreign',
      'composer',
      'steer',
      'queue',
      'queued',
      'fork',
      'slash',
      'attach',
      'image',
      'photo',
    ],
    sections: [
      {
        heading: 'How far you can control a session',
        intro: 'Every session shows a badge for how it is reached. Tap it for what works and why.',
        terms: [
          {
            term: 'Full control',
            text: 'Started by Mother Claude (from **New session** or **Continue here**). Everything works at once, and replies stream word by word.',
          },
          {
            term: 'Via hooks',
            text: "Started in a terminal, VS Code, JetBrains or Claude Desktop and reached through the Mother Claude plugin. Approvals work; messages arrive at Claude's next stopping point.",
          },
          { term: 'Via tmux', text: 'Runs in a tmux pane; Mother Claude types into it.' },
          {
            term: 'Experimental',
            text: 'A background job (`claude --bg`) typed into through its terminal. It may not work reliably.',
          },
          {
            term: 'Watch only',
            text: 'You can watch it, not drive it. **Continue here** takes it over.',
          },
        ],
        links: [{ label: 'See Sessions started elsewhere', path: '/help/foreign-sessions' }],
      },
      {
        heading: 'Send a message',
        steps: [
          'Open the session and stay on **Chat**.',
          'Type in **Message Claude…**.',
          'Press **Send** (or Enter on a keyboard).',
        ],
        tips: [
          'Shift+Enter adds a new line. On a phone, Enter adds a new line, so use **Send**. Cmd+Enter or Ctrl+Enter always sends.',
          'Your draft is kept for each session until it is sent. If sending fails, the text stays: **Retry** sends it again.',
        ],
      },
      {
        heading: 'Send while Claude works',
        intro:
          "You don't have to wait for the turn to end. In a session Mother Claude runs, Claude reads a message you send while it works as soon as its current step ends (a command, a file edit), inside the same turn.",
        steps: [
          'While Claude works, type your message.',
          'Press **Send**. The message waits above the box, marked **after its current step**.',
          'To have Claude read it right away, press **Send now** on the waiting message (or **Send now** beside **Send** before you send).',
        ],
        points: [
          '**Send now** makes Claude read it at once: Claude Code stops the step it is on, or moves it to the background, to take your message.',
          'Press **✕** on a waiting message to take it back before Claude reads it.',
          "In a session started elsewhere, a message waits for Claude's next tool call or the end of its turn (**waiting for next step**). **Interrupt & send** interrupts the turn and sends it.",
        ],
      },
      {
        heading: 'Attach images and files',
        steps: [
          'Press the camera button to add a photo, or the paperclip to attach a file.',
          'Pick the files.',
          'Add your message and press **Send**.',
        ],
        points: [
          'You can also paste a screenshot, or an image or file you copied, into the message box (Ctrl+V; ⌘V on a Mac), or drop files on it.',
          'Sessions Mother Claude runs take images inline; photos are scaled down to at most 2048 pixels.',
          'Other files, and every file sent to a session started elsewhere, are uploaded and given to Claude as a file path.',
          'Press **✕** on a thumbnail to remove it before sending.',
        ],
      },
      {
        heading: 'Slash commands and file paths',
        points: [
          'Type `/` to open the list of commands. `/model`, `/effort`, `/mode` and `/rename` open the matching picker; `/clear` and `/compact` ask first.',
          'Type `@` to complete a file path from the project.',
          'Slash commands work in sessions Mother Claude runs and in tmux sessions. In tmux, all but the common ones (such as `/compact`, `/clear`, `/model` and `/effort`) need a device allowed dangerous actions.',
        ],
        links: [
          {
            label: 'See Sessions in tmux',
            path: '/help/foreign-sessions',
            fragment: 'sessions-in-tmux',
          },
        ],
      },
      {
        heading: 'Interrupt Claude',
        steps: ['While Claude works, press **Interrupt** under the message box.'],
        points: [
          "In a session started elsewhere the button says **Interrupt at next tool**: it lands at Claude's next tool call, not mid-sentence.",
          'With exactly one permission waiting, **Deny & send as feedback** refuses it and sends your text as the reason; **Queue after this** sends your message after it.',
        ],
      },
      {
        heading: 'Change model, effort or mode',
        steps: [
          'Open the session.',
          'Tap the model, mode or effort chip in the row under its name.',
          'Pick the new value.',
        ],
        terms: [
          { term: 'Ask', text: 'Ask before edits and commands.' },
          { term: 'Accept edits', text: 'File edits go through; commands ask.' },
          { term: 'Plan', text: 'Plan first, change nothing.' },
          { term: 'Auto', text: 'A classifier decides what needs you.' },
          {
            term: 'Bypass permissions',
            text: 'Never ask. Dangerous: only in the desktop app, or on a device allowed dangerous actions.',
          },
        ],
        points: [
          'Effort goes from **Low** through **Medium**, **High** and **Extra high** to **Max**. Ultracode and thinking have their own switches in sessions Mother Claude runs.',
          "Sessions started elsewhere can't switch model or effort through the plugin. tmux sessions take `/model` and `/effort`, which also change your default for new sessions.",
        ],
      },
      {
        heading: 'Continue here',
        intro: '**Continue here** turns a session into one Mother Claude runs, with full control.',
        steps: [
          'Open the session.',
          'Press **⋮**.',
          'Under **Continue**, choose **Continue here**, or **Continue here, as a new session** for a session still running elsewhere.',
          'To choose the model, effort or mode, open **Launch options (model · effort · mode)**.',
          'Press **Continue here** (or **Continue here, as a new session**).',
        ],
        points: [
          'A session still running elsewhere goes on here as a new session: you drive a copy with a new id, and the original keeps running where it is.',
          'An ended session resumes in place. Typing a message into it does the same: its button says **Resume**.',
          'If the conversation started on another computer, you may be asked to choose its folder here first.',
        ],
        notes: [
          'Drive a conversation from one place at a time: typing in VS Code or a terminal and here at once splits it in two.',
          'From a phone, this works only in folders Claude Code already trusts on your computer.',
        ],
        links: [
          {
            label: 'See Continue a conversation from another computer',
            path: '/help/backup',
            fragment: 'continue-a-conversation-from-another-computer',
          },
        ],
      },
    ],
  },
  {
    id: 'approvals',
    title: 'Approvals and the Inbox',
    icon: 'inbox',
    group: 'sessions',
    summary:
      'Answer permission prompts, questions and plans from any session, and fix what stopped working.',
    intro:
      "When Claude needs you, a card appears: above the message box on the session's page, on its card in the list, and in the **Inbox**, which gathers everything that waits for you: every session's requests and every computer's, and what to fix.",
    keywords: [
      'permission',
      'allow',
      'deny',
      'approve',
      'question',
      'plan',
      'dangerous',
      'pending',
    ],
    sections: [
      {
        heading: 'The Inbox',
        points: [
          "**Needs you** lists what you can answer from here, oldest first, with how long each has waited: every session's requests, your other computers' while they are online, and on the desktop app a device that wants to pair, what your other computers asked of this one, a computer one of them introduces and a setting one suggests.",
          '**To fix** lists what stopped working or works worse, each with its fix.',
          '**Waiting where it runs** lists what can only be answered where the session runs, and what would let you answer it here.',
          'The number on **Inbox** counts what you can answer from here; a dot says something waits to be fixed.',
          'With several Claude accounts, the **Account** filter shows one account at a time; with another computer paired, the **Computer** filter shows one computer.',
          'On a phone, what only the desktop app answers or fixes says so, and is not counted.',
        ],
        tips: [
          "On a keyboard, J and K (or ↓ and ↑) move between requests and Enter opens a request's session; A allows, D denies, F adds feedback.",
          '**Dismiss** puts a failure away on this device; failures leave by themselves after a day.',
        ],
        links: [{ label: 'Open Inbox', path: '/inbox' }],
      },
      {
        heading: 'What waits besides requests',
        terms: [
          {
            term: 'A device wants to pair',
            text: 'On the desktop app, press **Approve…** and type the number the device shows, or **Deny**. The same dialog also opens by itself.',
          },
          {
            term: 'Asked by your other computers',
            text: 'A fix, a sign-in, a clone or keeping a project in sync that another computer asks of this one. Nothing runs until someone here confirms it.',
          },
          {
            term: 'An introduction',
            text: 'One of your computers introduces another it is paired with: **Pair with …** pairs this one with it, after the same six digits on both; **Not now** says no. While they pair, **Show the pairing** opens it again.',
          },
          {
            term: 'A suggested setting',
            text: 'Another computer uses another relay, or moves code another way: **Use it here too** takes its setting, **Keep mine** keeps yours.',
          },
          {
            term: 'A required check fails',
            text: 'On this computer, run its fix here; on another, **Ask … to fix** shows the fix to whoever is there.',
          },
          {
            term: 'Needs an update',
            text: 'Two computers run versions apart: update the older one, or ask it to.',
          },
          {
            term: 'Changed on both computers',
            text: 'Code sync of that project is paused: **Choose…** opens **Computers › Code sync**, where you keep one side.',
          },
          {
            term: 'A copy is paused',
            text: 'A copy kept on another computer waits for something only you can change there, such as its **Move** permission.',
          },
          {
            term: 'A backup, move or delete failed',
            text: 'In the last day: **Try again** runs a backup again; **Details** opens its kind on **Accounts › Recent changes**.',
          },
          {
            term: 'Not revoked everywhere yet',
            text: 'A computer you revoked everywhere that some of yours, such as one that is offline, still have to be told of: each revokes it when it next connects, and lets it in until then. **Dismiss** puts it away.',
          },
          {
            term: 'A sign-in',
            text: 'An account signed out, refused, without its token here, or signed in as someone else: **Sign in…**, or **Keep it**.',
          },
          {
            term: 'An update or a permission',
            text: 'On the desktop app: **Install…** opens **Settings › Software updates**; a missing macOS permission opens System Settings.',
          },
        ],
      },
      {
        heading: 'Answer a permission request',
        intro: 'Read what Claude wants to run or change, then choose:',
        terms: [
          { term: 'Allow once', text: 'Lets this one call run.' },
          {
            term: 'Always…',
            text: 'Allows it now and remembers a rule. Pick where the rule lives: **This session only**, or a settings file for the project or for you.',
          },
          { term: 'Deny', text: 'Refuses it.' },
          {
            term: 'Deny with feedback…',
            text: 'Refuses it and tells Claude what to do instead. Tick **Also stop the current turn** to stop it too.',
          },
        ],
        notes: [
          'Rules saved to settings files need the desktop app, or a device allowed dangerous actions. A phone without that is offered the rule for this session only.',
        ],
      },
      {
        heading: 'Answer a question',
        steps: [
          'Pick an answer for each question, or type your own.',
          'Press **Send answer** (**Send answers** for several).',
        ],
        points: ['**Skip** declines to answer.'],
      },
      {
        heading: 'Review a plan',
        steps: [
          'Press **Read the full plan**.',
          'Press **Approve & auto-accept edits** or **Approve & review each edit**.',
        ],
        points: [
          '**Keep planning…** asks for changes instead: write what should change and press **Send feedback**.',
        ],
      },
      {
        heading: 'Dangerous requests',
        intro:
          "Mother Claude marks some requests **dangerous**: for example writes to Claude Code's settings, `.mcp.json`, shell startup files or git hooks. The card says why.",
        points: [
          'The desktop app can approve them.',
          'A phone needs **Allow dangerous actions** for the network it is on, set in the desktop app under **Phone & devices**.',
          'Any device can deny them.',
        ],
      },
      {
        heading: 'Sessions started elsewhere',
        points: [
          'With the Mother Claude plugin installed (**Sessions started elsewhere**), a permission request from a terminal or VS Code session shows here while the session shows its own dialog. Whoever answers first wins; the other prompt goes away.',
          'Without it, those requests show under **Waiting where it runs**, and while nothing waits the **Inbox** says where to install it.',
          'A session in **Bypass permissions** asks almost nothing, so few requests reach you.',
        ],
        links: [{ label: 'See Sessions started elsewhere', path: '/help/foreign-sessions' }],
      },
      {
        heading: "Hear about it when you're away",
        points: [
          'Push notifications tell your phone about requests, questions, plans and finished turns, even with the app closed.',
          'Requests from the browser extension and the HTTP API appear here too, as the same cards.',
        ],
        links: [
          {
            label: 'See Use it from your phone',
            path: '/help/phone',
            fragment: 'push-notifications',
          },
        ],
      },
    ],
  },
  {
    id: 'new-session',
    title: 'Start a new session',
    icon: 'plus',
    group: 'sessions',
    summary: 'Start a session Mother Claude runs, with full control from any device.',
    intro:
      'Sessions started from **New session** are owned by Mother Claude: approvals, questions, interrupts and model switches all work from this computer and your phone.',
    keywords: ['start', 'launch', 'create', 'spawn', 'project', 'first prompt'],
    sections: [
      {
        heading: 'Start one',
        steps: [
          'On **Sessions**, press **New session**.',
          'Pick a project under **Project**, or type a path under **Or any folder on this computer** (desktop app).',
          'With several Claude accounts, check **Runs under**.',
          'Under **Options**, pick a **Model**, **Effort** and **Permissions**, or leave them on **Default** to use your own Claude settings.',
          'Optional: write a **First prompt**, and press **Add images** to attach some.',
          'Press **Start session**.',
        ],
        tips: [
          'Images can be PNG, JPEG, GIF or WebP: up to 10, 5 MB each.',
          '**Custom…** under **Model** takes any model id.',
        ],
        links: [{ label: 'Open New session', path: '/new' }],
      },
      {
        heading: 'Which account it runs under',
        points: [
          "With several accounts, **Runs under** starts on the project's account, or on the default for new sessions.",
          "An account that can't run here (signed out, or no token on this computer yet) is greyed out.",
        ],
        links: [{ label: 'See Claude accounts', path: '/help/accounts' }],
      },
      {
        heading: 'From a phone',
        points: [
          'Pick one of the recent projects: typing any folder is for the desktop app.',
          'A new session starts only in a folder Claude Code already trusts on your computer (opened there once). Otherwise you see `untrusted_cwd`: open the folder in Claude Code on the computer once.',
          '**Bypass permissions** can be chosen only in the desktop app, or on a device allowed dangerous actions.',
        ],
      },
    ],
  },

  // ───────────────── Settings and connections: the phone, sessions elsewhere
  {
    id: 'phone',
    title: 'Use it from your phone',
    icon: 'phone',
    group: 'settings',
    summary:
      'Pair a phone on your Wi-Fi, trust the certificate, add it to the Home Screen, get push.',
    intro:
      'Your phone opens the same dashboard as the desktop app, served by your computer. Pair it once on your home Wi-Fi; for access from anywhere, see Internet access.',
    keywords: [
      'pair',
      'qr',
      'code',
      'wifi',
      'lan',
      'iphone',
      'android',
      'certificate',
      'push',
      'notifications',
      'devices',
      'remote',
    ],
    sections: [
      {
        heading: 'Pair a phone on the same Wi-Fi',
        steps: [
          'In the desktop app, open **Phone & devices**.',
          'Under **Pair a phone**, choose **Same Wi-Fi**.',
          'Press **Show pairing code**. A QR code and a 12-character code appear.',
          "Scan the QR code with the phone's camera.",
          'The phone opens your sessions, paired.',
        ],
        tips: [
          "Can't scan? Open the address shown under the code on the phone and type the code on its **Pair this device** page, then press **Pair**.",
          'A code works once and expires after 5 minutes. With several network addresses, pick the one the phone can reach under **Address**.',
        ],
        notes: [
          'A **Same Wi-Fi** code works only on your own network. Pair only on a network you control.',
        ],
        links: [{ label: 'Open Phone & devices', path: '/devices' }],
      },
      {
        heading: "Trust your computer's certificate",
        intro:
          'At home the phone connects over HTTPS with a certificate your computer made, so it warns once ("not private"). Trusting it turns on live updates on iPhone, the Home Screen app and push notifications.',
        steps: [
          "On the phone's Pair page, open **Trust this Mac (recommended)**.",
          'iPhone or iPad: tap **Download the profile**, then install it under Settings › General › VPN & Device Management.',
          'iPhone or iPad: under Settings › General › About › Certificate Trust Settings, turn on full trust for “Mother Claude”.',
          "Android: tap **Download the Mac's certificate** and install it under Settings › Security › Install a certificate › CA certificate.",
          'Reload the page.',
        ],
        points: [
          "**Phone & devices** on the desktop app shows the certificate's fingerprint, so you can compare it with what the phone shows.",
          'This certificate can only vouch for private addresses, so it cannot be used to impersonate a public website.',
        ],
      },
      {
        heading: 'Add it to the iPhone Home Screen',
        intro:
          'An app added to the Home Screen keeps its own storage, separate from Safari, so it pairs on its own.',
        steps: [
          'In Safari, tap Share › Add to Home Screen.',
          'In a paired Safari tab, open **More › Settings** and press **Set up the Home Screen app**.',
          'Open the app from the Home Screen.',
          'Type the code on its Pair page and press **Pair**.',
        ],
        tips: [
          'The code works once, for 5 minutes, and only for this device. The same code pairs this phone at another address too, such as your tunnel.',
        ],
      },
      {
        id: 'push-notifications',
        heading: 'Push notifications',
        intro:
          'Push reaches your phone even when the app is closed. It needs a trusted HTTPS address: your tunnel address, or your home address once the certificate is trusted. On iPhone it works only in the Home Screen app.',
        steps: [
          'On the phone, open **More › Settings**.',
          'Under **Notifications**, press **Enable** next to **Push notifications**.',
          'Allow notifications when the browser asks.',
          'Pick the events you want: **Permission requests**, **Questions**, **Plans**, **Finished**, **Failures**, **When Claude asks** and **Waiting for you**.',
          'Press **Send a test**.',
        ],
        points: [
          '**Show details** adds the command or question to a notification.',
          'For one session: open its **⋮** menu and choose **Follow** or **Mute**, or **Notify me when this turn finishes**.',
          'Nothing is pushed to a device that has the app open in front of you, and bursts from one session are combined.',
          'The desktop app needs no push: it shows everything live.',
        ],
        notes: ['Your computer sends every notification, so it has to be awake and online.'],
      },
      {
        heading: 'What a phone can do',
        points: [
          'The tabs at the bottom are **Sessions** (with **Agents** and **Projects** as tabs), **Inbox**, **Teams** (with **Templates** and **Capacity**) and **More**: under **Connections**, **Computers**, **Phone & devices**, **Browser** and **HTTP API**; under **Manage**, **Accounts** (with **Usage** and **Recent changes**), **Backups** and **Settings** (with **Claude Code tools**); then **Help**.',
          'Watch every session, answer approvals, questions and plans, send messages and images, start sessions and continue them.',
          'Pairing devices, internet access, what each device may do and installing the plugin are changed only in the desktop app.',
          'Dangerous actions need a grant for this device in the desktop app (**Phone & devices**, under **Paired devices**).',
        ],
      },
      {
        heading: 'Phone & devices on a phone',
        intro:
          'On a phone, **Phone & devices** shows this phone and every device paired with your computer, read only.',
        terms: [
          {
            term: 'This device',
            text: 'Its **Name**, the **Network** it reaches your computer through, and whether it may take **Dangerous actions** there.',
          },
          {
            term: 'Paired devices',
            text: 'Every paired device, this one first: online, or when it was last seen, and the dangerous actions it may take.',
          },
        ],
      },
      {
        heading: 'Unpair this phone',
        steps: [
          'On the phone, open **More › Settings**.',
          'Under **Connection**, press **Unpair this device**.',
          'Press **Tap again to unpair**.',
        ],
        points: [
          'Its push notifications stop and the desktop app forgets it. You can also revoke it on the desktop app, under **Phone & devices**.',
        ],
      },
    ],
  },
  {
    id: 'internet',
    title: 'Internet access and devices',
    icon: 'shield',
    group: 'settings',
    summary: 'Reach your computer through a tunnel, approve devices, and manage their rights.',
    intro:
      'Away from home, your phone reaches your computer through a tunnel you run (Tailscale, ngrok or Cloudflare). Mother Claude never opens a port to the internet itself, and internet access is off until you turn it on.',
    keywords: ['tunnel', 'tailscale', 'ngrok', 'cloudflare', 'remote', 'away', 'revoke', 'grant'],
    sections: [
      {
        heading: 'Turn on internet access',
        where: 'desktop',
        steps: [
          'Open **Phone & devices**.',
          'Under **Internet access**, turn on **Allow access through a tunnel**.',
          'Under **Start a tunnel**, copy the command for the tunnel you use.',
          'Run it in a terminal on your computer.',
          'Wait for the address to show under **Public address**.',
        ],
        terms: [
          {
            term: 'Tailscale Serve',
            text: 'Recommended: only devices signed in to your tailnet can reach it.',
          },
          { term: 'ngrok', text: 'A random address, or your static domain.' },
          {
            term: 'Cloudflare quick tunnel',
            text: 'A new address every run, so phones have to pair again.',
          },
          { term: 'Tailscale Funnel', text: 'Public: anyone can reach the address.' },
        ],
        tips: [
          'Mother Claude finds the address of ngrok, Cloudflare quick tunnels and Tailscale by itself. For any other tunnel, type its `https://` address under **Or enter it yourself**.',
        ],
        links: [{ label: 'Open Phone & devices', path: '/devices' }],
      },
      {
        heading: 'Pair a phone through the internet',
        where: 'desktop',
        steps: [
          'Under **Pair a phone**, choose **Internet**.',
          'Press **Show pairing code**.',
          'Scan it with the phone (on mobile data, for example). The phone shows 4 digits.',
          'In the desktop app, **Approve a new device?** opens by itself; or press **Approve…** on its card in the **Inbox**.',
          'Type the 4 digits the phone shows in **Number shown on the device**, and press **Approve**.',
        ],
        points: [
          'Nobody gets in through the internet without you typing those digits on your computer. Three wrong tries deny the request.',
          'A phone paired at home starts unpaired at the tunnel address, because browsers keep storage per address. Where it is paired, press **Settings › Set up the Home Screen app** for a code, and type it on the Pair page at the new address.',
        ],
      },
      {
        heading: 'Tailscale addresses',
        points: [
          "**Also listen on Tailscale** serves the dashboard on your computer's Tailscale address (100.x.y.z) too.",
          'A request there counts as coming from the internet: **Same Wi-Fi** codes are refused, and the internet grant applies.',
        ],
      },
      {
        heading: 'Turn internet access off',
        steps: ['Under **Internet access**, turn off **Allow access through a tunnel**.'],
        points: [
          'That stops the tunnel port and cuts every connection that came through it.',
          'Point tunnels at port 6726, as the commands under **Start a tunnel** do. A tunnel pointed at 6725 instead keeps working after you switch this off.',
        ],
      },
      {
        heading: 'Manage paired devices',
        where: 'desktop',
        intro:
          '**Paired devices**, on **Phone & devices**, lists every device with its platform, how and when it last connected, and its open connections.',
        terms: [
          { term: 'The pencil', text: 'Renames the device.' },
          {
            term: 'Revoke',
            text: 'Press it twice (**Tap again to revoke**). The device is cut off at once and has to pair again.',
          },
          {
            term: 'Allow dangerous actions (LAN)',
            text: 'Lets the device do what is otherwise kept for the desktop app, while it is on your home network.',
          },
          {
            term: '… over the internet',
            text: 'The same through a tunnel or a Tailscale address. Leave it off unless you really need it: with it, a stolen phone could run anything on your computer.',
          },
        ],
        points: [
          'Dangerous actions are bypass-permissions mode, rules saved to settings, removing sessions, raw terminal keys, and approving commands flagged dangerous. They are off for every device by default.',
          'Devices unused for 30 days are unpaired automatically.',
          'Every new device, and every dangerous action a device takes, shows a notice in the desktop app.',
        ],
      },
      {
        id: 'warnings-on-the-remote-page',
        heading: 'Warnings under Internet access',
        terms: [
          {
            term: "ngrok's inspector is recording",
            text: 'Restart ngrok with `--inspect=false`, as the ngrok command under **Start a tunnel** does.',
          },
          {
            term: "The quick tunnel's address changes",
            text: 'Phones have to pair again and push stops. Use Tailscale, an ngrok static domain or a named Cloudflare tunnel.',
          },
          { term: 'Tailscale Funnel is public', text: 'For your own devices, use Serve.' },
          {
            term: 'A tunnel points at the API port',
            text: "Anyone who finds the address could try to drive Claude, so the API's tokenless mode is off until that tunnel stops.",
          },
          { term: 'A raw TCP tunnel', text: 'Use an HTTP tunnel instead.' },
        ],
      },
    ],
  },
  {
    id: 'foreign-sessions',
    title: 'Sessions started elsewhere',
    icon: 'plug',
    group: 'settings',
    summary:
      'Answer and message terminal, VS Code and JetBrains sessions with the plugin, or tmux.',
    intro:
      'Mother Claude shows every Claude Code session on your computer, whatever started it, and you can watch them right away. To answer and message the ones you started in a terminal, VS Code, JetBrains or Claude Desktop, install the Mother Claude plugin under **Sessions started elsewhere**.',
    keywords: [
      'foreign',
      'foreign-session control',
      'plugin',
      'hooks',
      'hooks plugin',
      'vs code',
      'jetbrains',
      'terminal',
      'tmux',
      'reload-plugins',
      'background job',
    ],
    sections: [
      {
        id: 'install-foreign-session-control',
        heading: 'Install the Mother Claude plugin',
        where: 'desktop',
        steps: [
          'Open **Sessions started elsewhere**.',
          "On your Claude account's row, press **Install** (a token account uses the first row, the Default's).",
          'Wait until **Plugin** says **On**.',
          'In each session that was already running, type `/reload-plugins` (or restart it).',
        ],
        points: [
          "It installs the Mother Claude plugin with Claude Code's own `claude plugin install`. Your `settings.json` and your own hooks are never touched.",
          'New sessions report in by themselves: **Sessions reporting** counts them. Below the table, a line counts the running sessions started before the install, which still need `/reload-plugins`.',
          'With Mother Claude closed, your sessions behave exactly as before.',
        ],
        tips: [
          'From a session you can only watch, **Set up Sessions started elsewhere** (under its message box, or beside what does not work and why) opens the same page. On a phone, the same place says what to press on your computer.',
        ],
        notes: [
          "**Install** is greyed out when Claude Code's `claude` command is not found. Install Claude Code, or start the app from a shell where `claude` works.",
        ],
        links: [{ label: 'Open Sessions started elsewhere', path: '/elsewhere' }],
      },
      {
        heading: 'One table for every account',
        intro:
          "Each Claude account that keeps its own folder has its own copy of the plugin, so **Sessions started elsewhere** has a row for each: the Default first, then each directory account. A token account uses the Default's folder, and its row.",
        terms: [
          { term: 'Claude account', text: 'The account, and the folder the plugin goes in.' },
          {
            term: 'Plugin',
            text: '**On**, **Installed, switched off** (there, but doing nothing) or **Not installed**, with its version.',
          },
          {
            term: 'Sessions reporting',
            text: 'Running sessions whose plugin reports to Mother Claude.',
          },
          { term: 'Last hook event', text: 'When the plugin last reported something.' },
        ],
        points: [
          "In the desktop app, each account's **Sessions started elsewhere** line on **Accounts** leads there too.",
          'On a phone the table is read only: installing and uninstalling are done in the desktop app.',
        ],
      },
      {
        heading: 'What works through the plugin',
        points: [
          "Approve permissions and answer questions and plans. The session's own dialog stays: whoever answers first wins.",
          "Send messages. They arrive at Claude's next tool call or at the end of its turn, and an idle session is woken to read them. They reach Claude but don't show in the session's own window; only Claude's reply does.",
          'Interrupt. It lands at the next tool call.',
          'Model, effort and slash commands cannot be changed this way, and the permission mode only together with an approval (a plan, for example). **Continue here** gives you all of them.',
        ],
      },
      {
        heading: 'Sessions in tmux',
        points: [
          "Mother Claude types into the session's tmux pane, so messages, slash commands and Esc work as if you typed them.",
          'It refuses while a dialog is open, so it never answers one by accident. Answer it, or open **Terminal view**.',
          'Without the plugin, approvals there are answered with keys, under **Terminal view** in the session menu.',
          '`/compact`, `/clear`, `/context`, `/usage`, `/cost`, `/status`, `/todos`, `/model`, `/effort`, `/rename` and `/btw` work from any device. Other commands, keys and a leading `!` need a device allowed dangerous actions.',
        ],
      },
      {
        heading: 'Background jobs',
        points: [
          'Jobs started with `claude --bg` can be stopped, restarted and removed from their **⋮** menu, under **Its process**.',
          '**Terminal view** shows what the job prints.',
          'Typing into a job is experimental and needs a device allowed dangerous actions.',
        ],
      },
      {
        heading: 'Take a session over',
        intro: 'For full control of any session, use **Continue here**.',
        points: [
          'A session still running elsewhere goes on here as a new session, a copy you drive here; the original keeps running.',
          'An ended session resumes in place.',
        ],
        links: [{ label: 'See Continue here', path: '/help/messaging', fragment: 'continue-here' }],
      },
      {
        heading: 'Uninstall',
        where: 'desktop',
        steps: [
          'Open **Sessions started elsewhere**.',
          "On the account's row, press **Uninstall**.",
          'Press **Tap again to uninstall**.',
        ],
        points: [
          'The hooks that running sessions already loaded go quiet first; then Claude Code removes the plugin.',
        ],
      },
    ],
  },

  // ───────────────────────────────────────────────────── Your computers
  {
    id: 'machines-setup',
    title: 'Set up your computers',
    icon: 'monitor',
    group: 'machines',
    summary:
      'Connect your computers: the checks on each one, what fixes them, the test before it turns on, and what each tab of Computers holds.',
    intro:
      "Pair your computers, and from any of them you see and drive every computer's Claude Code sessions, and move a session, with its code, to another computer and back. It all happens on **Computers**, which is off until you set it up in the desktop app. The setup runs on each computer, because only a computer can check itself.",
    keywords: [
      'machines',
      'computers',
      'another computer',
      'second computer',
      'setup',
      'checks',
      'firewall',
      'local network',
      'relay',
      'health',
      'long paths',
      'full disk access',
      'daemon',
    ],
    sections: [
      {
        heading: 'What you need',
        points: [
          'Mother Claude, this version or newer, on each computer.',
          'Each computer switched on, with the internet or on the same network as the others.',
          "On Windows, an administrator's OK once, for the firewall.",
          'About 10 minutes, with each computer at hand.',
        ],
      },
      {
        heading: 'Run the setup',
        where: 'desktop',
        intro:
          'Open **Computers** and, on the **Off on this Mac** card (**Off on this PC** on Windows), press **Set up your computers**. Seven steps follow: **Next** and **Back** move between them, and **Go on with the setup** on that card brings you back to the one you left.',
        steps: [
          '**This Mac** (or **This PC**): fix what its checks find. Required ones come first.',
          '**Your computers**: pair each of your other computers.',
          "**Every computer**: each computer's checks, side by side.",
          '**What each may do**: what each computer may do on this one. Press **Save** after a change.',
          '**Accounts and folders**: your accounts and shared folders on every computer.',
          "**Options**: the relay, how a moving session's code travels and where it lands, and each project's own options.",
          '**Test, then turn on**: press **Run the test**, then **Turn on Computers**.',
        ],
        tips: [
          '**Turn on Computers** stays greyed out while a required check is left, and says how many. Until it is on, your computers only pair and run the setup and its test with each other: none sees, drives or moves the sessions of a computer where it is still off.',
          "The checks keep running afterwards: the head of **Computers** says how your computers stand (**All good**, or how many checks are recommended or required), each computer's card lists its checks that fail, with their fix, and a red dot on **Computers** in the sidebar says something broke again.",
        ],
        links: [
          { label: 'Open Computers', path: '/computers' },
          { label: 'See Pair your computers', path: '/help/machines-pair' },
        ],
      },
      {
        heading: 'What is checked',
        intro:
          'A **Required** check has to pass before it turns on; a **Recommended** one works, but worse. Each says why it matters, and its button fixes it or opens the right settings page.',
        terms: [
          {
            term: 'Mother Claude',
            text: 'Every computer needs a version that speaks the same protocol: a different one is required to update. A computer behind another one is recommended to: **Update Mother Claude** opens **Software updates**, and on another computer **Ask … to fix** shows it there.',
          },
          {
            term: 'Stays awake while used',
            text: 'A computer asleep cannot answer the others. **Keep awake while used** keeps it awake while a paired computer is connected; no system setting changes.',
          },
          {
            term: 'Starts when you sign in',
            text: 'After a restart, the others reach this computer only once Mother Claude runs again. **Turn on** makes it start when you sign in, in the tray, as **Settings › Startup** does.',
          },
          {
            term: 'Local Network access (Mac)',
            text: 'Without it, the Mac reaches your other computers only through the relay, even at home. Allow it when macOS asks, or press **Open Local Network settings** and switch Mother Claude on.',
          },
          {
            term: 'The firewall (Mac)',
            text: 'If it blocks Mother Claude, the others reach the Mac only through the relay. Press **Open Firewall settings** and allow Mother Claude.',
          },
          {
            term: 'The network type (Windows)',
            text: 'On a Public network, Windows blocks incoming connections. Press **Open network settings** and set your home or office network to Private.',
          },
          {
            term: 'The firewall (Windows)',
            text: "Without a rule, the others reach the PC only through the relay. **Allow through the firewall** adds one rule for Mother Claude, after Windows asks for an administrator's OK.",
          },
          {
            term: 'The relay',
            text: 'Across networks, your computers meet through it. Check its address, or pick the public relays under **Options**.',
          },
          {
            term: 'Paired computers',
            text: 'Only computers you paired work together. Each pair needs the same protocol and relay setting: **Use this setting everywhere** offers yours to the others, in their **Inbox**.',
          },
          {
            term: 'git',
            text: "A session's code moves through git, 2.30 or newer, on both computers: **Install the command line tools** on a Mac, **Install Git for Windows** on Windows.",
          },
          {
            term: 'Long paths (Windows)',
            text: "Git for Windows leaves out paths longer than 260 characters unless long paths are on. **Turn on for git** turns them on for you; Windows' own setting needs an administrator.",
          },
          {
            term: 'Claude Code',
            text: 'Sessions run in it, and one moved to an older version may not resume. **Update Claude Code** runs `claude update` when you press it.',
          },
          {
            term: 'Claude Code tools',
            text: "Sessions here run commands on your other computers through Mother Claude's tools. Recommended; once another computer lets this one run commands there, those tools wait for it, but the rest works without it. **Turn on** switches them on.",
          },
          {
            term: 'Each account',
            text: "Signed in on this computer. Only that account's sessions wait for it: the rest works without it. **Sign in…** starts its sign-in here.",
          },
          {
            term: 'Free space',
            text: "Moves land in Mother Claude's data folder first: 5 GB free is recommended.",
          },
        ],
        notes: [
          'The network checks are required only with **No relay**. With a relay they are recommended: your computers still meet, through the relay, more slowly.',
        ],
      },
      {
        heading: 'Accounts and folders',
        intro:
          'Set in the setup, and changed afterwards under **Computers › Options › Accounts and folders**.',
        points: [
          'An account is matched by its name on each computer, whatever the case of its letters, and the Default by the Default. A session moves to the account of the same name and runs only where it is signed in.',
          "Another computer sees only your accounts' names, kinds and whether they are signed in: never a token, an email or a folder.",
          'A shared folder is one folder on each computer, such as `~/development/projects` on a Mac and `D:\\development\\projects` on Windows: a project under it on one is the same project, at the same place, on the other.',
          "The setup proposes shared folders from your projects' git remotes: press **Use** to take one, or **Add a folder** to add your own. Your folder pairs from **Backups** are kept.",
          "A project only one computer has offers **Clone it on …** or **Clone it here**. A project with no git remote can't move with its code.",
          '**Sign in on …**, **Add it on …** and **Clone it on …** are asked of that computer: someone there confirms them, in its **Inbox**.',
        ],
        links: [
          { label: 'Open Options', path: '/computers/options', fragment: 'accounts-folders' },
        ],
      },
      {
        heading: 'Options',
        intro:
          'Set in the setup, and changed afterwards under **Computers › Options**: **General** (press **Change**) and **Each project**.',
        terms: [
          {
            term: 'Relay',
            text: "**Public relays (default)** work at once, anywhere. **Your own relay** is an `iroh-relay` you run, on a NAS or a small server: type its `https://` address and, if you like, its certificate's SHA-256, so only that certificate is taken (a self-signed one too). **No relay**: your computers meet on the same network only. A relay only ever carries encrypted traffic.",
          },
          {
            term: 'How code travels when a session moves',
            text: '**Computer to computer (default)**: straight to the other computer, nothing pushed anywhere. **Through your git remote**: hidden refs on your remote, deleted after the move unless you keep them; both computers need access to it without a password prompt.',
          },
          {
            term: 'Where the code lands',
            text: "**In the project folder, when it has no changes of its own (default)**, or **Always in a separate folder (a worktree), never the project folder**: a worktree in Mother Claude's data folder on the computer it goes to.",
          },
          {
            term: 'Keep awake',
            text: 'While another computer uses this one, Mother Claude keeps it awake. No system setting changes.',
          },
        ],
        points: [
          'Use the same relay on every computer (two computers on different relays may never meet), and best the same code options: a change here is offered to the others, in their **Inbox**, and each takes it when someone there presses **Use it here too** (or **Keep mine**). A move follows the options of the computer it starts from.',
          "**Each project** can move its code another way, and has a recipe: **After a move, ask Claude to run**, one command a line, for macOS, Windows and Linux. They go to Claude, which runs them through the session's own approvals. **Files git ignores that travel with it**, such as `.env.local`, go with each of its moves: they usually hold secrets, so adding one, and carrying them, need the dangerous right.",
          "**Check the remote** checks whether a project's git remote answers without a password prompt on each computer. It runs only when you press it, and again before each move through the remote.",
        ],
        links: [{ label: 'Open Options', path: '/computers/options' }],
      },
      {
        heading: 'Computers, tab by tab',
        intro: 'Once your computers are set up, **Computers** has four tabs:',
        terms: [
          {
            term: 'Overview',
            text: '**This Mac** (or **This PC**), then a card for each paired computer: how it is connected, its versions, its sessions (**Open its sessions**), its checks that fail with their fix, what each may do on the other (**Change**), **Rename**, **Revoke** and **Revoke everywhere**. Then **Pair your computers with each other** and **Copies**.',
          },
          {
            term: 'Code sync',
            text: 'The projects kept the same on two computers, and **Keep a project in sync…**.',
          },
          {
            term: 'Activity',
            text: 'What went between your computers: **Recent moves**, **Runs**, **Tasks**, **Team roles** and **Always allowed runs**. The chips at the top show one kind.',
          },
          {
            term: 'Options',
            text: '**General** (the relay, how code moves, copies, keeping awake), **Accounts and folders** and **Each project**, then **Run the setup again** and **Turn Computers off**.',
          },
        ],
        points: [
          '**This Mac** (or **This PC**) shows its versions, whether **Claude Code tools** are on, its checks that fail with their fix, and when they last ran: **Check again** runs them now. On a Mac it also shows the macOS permissions at a glance: **Permissions** opens their list.',
          '**Claude Code’s background service** says whether it runs and how many background jobs it has. **Details** shows it, with the MCP servers configured for Claude Code and the background jobs.',
          'What your other computers ask of this one, a computer one of them introduces and a setting one suggests wait in the **Inbox**.',
          'On a phone, **Computers** is under **More**: pairing, the setup, permissions and options are changed in the desktop app.',
        ],
        links: [
          { label: 'Open Code sync', path: '/computers/sync' },
          { label: 'Open Activity', path: '/computers/activity' },
        ],
      },
    ],
  },
  {
    id: 'machines-pair',
    title: 'Pair your computers',
    icon: 'key',
    group: 'machines',
    summary: 'Pair two computers with a one-time code, and check the same six digits on both.',
    intro:
      'Two computers pair once, in their desktop apps: on the same network or across the internet, with no port to open and no tunnel to run.',
    keywords: ['pair', 'join', 'code', 'nearby', 'six digits', 'revoke', 'unpair', 'rename', 'key'],
    sections: [
      {
        heading: 'Pair two computers',
        where: 'desktop',
        steps: [
          'On the first computer, open **Computers** and press **Pair a computer**. An 8-character code appears: it works once, for five minutes.',
          'On the other computer, open **Computers** and press **Join…**.',
          'In **Join another computer**, type the code in **Code** and press **Join**.',
          'Check that both screens show the same six digits.',
          'On each computer, choose what the other may do on it, then press **The numbers match — Pair**.',
        ],
        tips: [
          '**Nearby** lists the computers on your network that show a code, so you can check which one shows it: you still type its code, as joining needs it. None nearby? The code works across the internet too.',
          'The setup has the same buttons in its **Your computers** step. Before the setup, **Computers** offers only **Set up your computers**.',
        ],
        links: [{ label: 'Open Computers', path: '/computers' }],
      },
      {
        heading: 'Across the internet',
        points: [
          'When the other computer joined over the internet, the first one does not show the digits: type the six the other computer shows, then press **Pair**.',
          'A wrong number ends the pairing, and nothing is paired. Start again with a new code.',
        ],
      },
      {
        heading: 'Why it is safe',
        points: [
          'Only someone with the code can start a pairing: the code finds the other computer, and proves you have it.',
          "The six digits come from both computers' keys, so a computer in the middle, even one that saw the code, would make the two screens show different numbers.",
          'Nothing is stored until both computers confirm.',
          'A paired computer is recognised by its key alone, kept in your Keychain or Windows Credential Manager. Reinstalling Mother Claude makes a new key: pair that computer again.',
        ],
      },
      {
        heading: 'Rename or revoke a computer',
        where: 'desktop',
        points: [
          "On **Computers**, each paired computer's card has **Rename** (its name on this computer) and **Revoke**.",
          '**Revoke**, then **Tap again to revoke**: this computer forgets the other and cuts its connections at once.',
          '**Revoke** works on this computer only. For a lost or stolen computer, use **Revoke everywhere** on any of your others: it is cut from all of them.',
          "**Turn Computers off** (**Computers › Options**) does not unpair, but while it is off a paired computer can only pair and run the setup with this one: it can't see, drive or move its sessions, and this one doesn't use the others'. **Revoke** forgets a computer.",
        ],
      },
    ],
  },
  {
    id: 'machines-more',
    title: 'Three computers or more',
    icon: 'monitor',
    group: 'machines',
    summary:
      'Pair a new computer with the others through one you have, cut a lost one from all of them, and keep their versions in step.',
    intro:
      'Each pair of your computers is paired on its own: one computer can introduce two others, but nothing is trusted for them. Revoking a lost computer everywhere, and the versions each one runs, are on **Computers** too.',
    keywords: [
      'third computer',
      'introduce',
      'introduction',
      'revoke everywhere',
      'lost',
      'stolen',
      'versions',
      'update',
      'older',
    ],
    sections: [
      {
        heading: 'Pair a new computer with the others',
        where: 'desktop',
        intro:
          'Pair the new computer with one of yours first (**Pair your computers**). That one then offers **Also pair … with …?** for each of your other computers the new one is not paired with, right after pairing and on **Computers**, under **Pair your computers with each other**.',
        steps: [
          'Press **Introduce them**. Both computers are told who the other is; nothing is paired yet.',
          'On each of the two, open the **Inbox**: under **Needs you**, **… introduces …** says which computer introduced which. Press **Pair with …**.',
          'Check that both screens show the same six digits. When one came over the internet, the other asks you to type them, as when you pair with a code.',
          'On each computer, choose what the other may do on it, then press **The numbers match — Pair**.',
        ],
        tips: [
          '**Not now** on either computer ends it there and tells the other: nothing is paired. While they pair, **Show the pairing** opens it again.',
          'An introduction waits ten minutes for both answers: after that, introduce them again.',
          'The setup has the same offers in its **Your computers** step.',
        ],
        links: [
          { label: 'Open Computers', path: '/computers' },
          { label: 'Open Inbox', path: '/inbox' },
        ],
      },
      {
        heading: 'Why an introduction is safe',
        points: [
          "The computer that introduces them passes on only each one's name, id, public key and last addresses: never a secret, and never what either may do.",
          'Each of the two connects to the other by the key it was told, and nothing else gets in for that introduction.',
          "The six digits come from both computers' keys, as with a code: a computer in the middle would make the two screens show different numbers.",
          'Each desktop decides and confirms on its own; until both have, nothing is stored. Trust is never passed on: being paired with a computer gives nothing on the computers it is paired with.',
        ],
      },
      {
        heading: 'Revoke a computer everywhere',
        where: 'desktop',
        intro:
          'For a lost or stolen computer: cut it from all your computers from any one of them.',
        steps: [
          "On **Computers**, find the computer's card and press **Revoke everywhere**.",
          'Press **Tap again to revoke everywhere**.',
        ],
        points: [
          'This computer forgets it and cuts its connections at once, and asks each of your other computers to do the same. Each does it at once, and shows a notice that you revoked it on all your computers.',
          'A computer that is off or away does it when it next connects, and lets it in until then: the **Inbox** says which ones still have to, under **To fix** (**… is not revoked everywhere yet**). **Dismiss** puts it away.',
          'It only ever removes trust: it adds nothing anywhere, and a computer can never use it to revoke the one that asked.',
        ],
      },
      {
        heading: 'Versions',
        points: [
          "Each computer's card on **Computers** shows its system, its Mother Claude, its protocol and its Claude Code.",
          'A computer that runs an older Mother Claude says so, with **Ask … to update**: that computer shows its **Software updates** page once someone there confirms it. When this computer is the older one, **Update it** opens **Software updates** here.',
          "An older computer also says what it can't do until it updates, such as **take tasks** or **keep code in sync**. Two computers on different protocols don't work together at all until one updates.",
          'An older Claude Code there has **Ask … to update** too; here, **Update it** runs `claude update`.',
        ],
      },
    ],
  },
  {
    id: 'machines-permissions',
    title: 'What another computer may do',
    icon: 'shield',
    group: 'machines',
    summary:
      'The permissions each computer gives the others, on the same network and over the internet.',
    intro:
      'Each computer decides what each of the others may do on it. You choose when you pair, and can change it later on **Computers**.',
    keywords: [
      'permissions',
      'grants',
      'see',
      'drive',
      'move',
      'run',
      'dangerous',
      'audit',
      'internet',
      'same network',
    ],
    sections: [
      {
        heading: 'The permissions',
        terms: [
          {
            term: 'See',
            text: 'List its sessions, read their transcripts, see their approvals and questions. On by default.',
          },
          {
            term: 'Drive',
            text: 'Send messages, answer ordinary approvals, interrupt, stop, and set model, effort and mode. On by default.',
          },
          { term: 'Move', text: 'Move sessions to this computer and away from it. On by default.' },
          {
            term: 'Run',
            text: "Run commands here for its sessions, in a work folder of their own, and take tasks they hand over, in a worktree of their own: this computer's copy of the project is never touched. Each is approved on the other computer. Off by default.",
          },
          {
            term: 'Dangerous',
            text: "Bypass permissions, approve tool calls flagged dangerous, send raw keys, carry a folder's trust and the files git ignores with a move, and run commands flagged dangerous. Off by default.",
          },
        ],
        points: [
          'Driving needs seeing; moving, running commands and dangerous actions need driving. Ticking one ticks what it needs; unticking one unticks what needs it.',
        ],
        links: [
          { label: 'See Run commands on another computer', path: '/help/machines-run' },
          { label: 'See Hand a task to another computer', path: '/help/machines-task' },
        ],
      },
      {
        heading: 'Same network or internet',
        points: [
          'Each permission has two columns: **Same network**, for a direct path on your local network, and **Internet**, for everything else, the relay included.',
          "A computer's card on **Computers** shows how it is connected now: **Direct (same network)**, **Direct (internet)** or **Through the relay (slower)**.",
        ],
      },
      {
        heading: 'Change them',
        where: 'desktop',
        steps: [
          'Open **Computers**.',
          "On the other computer's card, next to **On this Mac, … may** (or **On this PC, … may**), press **Change**.",
          'Tick or untick the boxes, then press **Save**.',
        ],
        points: [
          '**On …, this Mac may** shows what the other computer lets this one do. Only that computer can change it.',
          'The setup has the same table in its **What each may do** step.',
        ],
        links: [{ label: 'Open Computers', path: '/computers' }],
      },
      {
        heading: 'Dangerous actions through another computer',
        points: [
          "A dangerous action on another computer needs both: that computer's **Dangerous** permission for this one, and your own right here. A phone without the dangerous permission on this computer can't reach for it through this computer. A move you start from here for another computer's session comes back to this one from it: carrying a folder's trust or the files git ignores then needs each computer's **Dangerous** permission for the other.",
          "Every change another computer makes here is written to this computer's audit log, with who acted through it; a dangerous one also shows a notice here.",
          'With **Dangerous**, another computer can run anything here: give it only to a computer you trust as much as this one.',
          'With **Run**, another computer runs commands and starts Claude Code sessions here as you, each approved on that computer, not on this one: give it only to a computer you would let type into a terminal here.',
        ],
      },
    ],
  },
  {
    id: 'machines-sessions',
    title: "Use another computer's sessions",
    icon: 'sessions',
    group: 'machines',
    summary:
      "See and drive every computer's sessions from any of them, and start one on another computer.",
    intro:
      "Once two computers are paired and both have Computers on, each lists the other's Claude Code sessions. Open one and use it as if it ran here.",
    keywords: [
      'other computer',
      'machine filter',
      'computer filter',
      'chip',
      'inbox',
      'offline',
      'where',
      'new session',
      'phone',
      'remote control',
    ],
    sections: [
      {
        heading: 'Find them',
        points: [
          "**Sessions** lists every computer's sessions. Another computer's have a chip with its name; this computer's have none.",
          'The **Computer** filter picks one computer: **All computers**, **This Mac** (or **This PC**), or another computer by name.',
          "On **Computers**, a computer's card says how many sessions it has and how many run; **Open its sessions** lists them.",
          '**Inbox** lists what waits on you everywhere, each request with its computer. Answer it here: the first answer wins, as with a phone.',
        ],
        links: [{ label: 'Open Sessions', path: '/' }],
      },
      {
        heading: 'Drive them',
        points: [
          "Open one: its page says **Runs on … — you drive it from here.** The message box, approvals, stop, model, effort and mode work as for this computer's sessions.",
          "Without the other computer's **Drive** permission, the page says **Runs on … — you can watch it from here.**, and what you try says why, for example **… lets this Mac watch its sessions, not drive them.**",
          'Switching its account, moving or copying it to another account or a storage, deleting it, showing its files and opening it in VS Code are done on the computer it runs on.',
        ],
      },
      {
        heading: 'Start a session on another computer',
        steps: [
          'Open **New session**.',
          'Under **Where**, pick the other computer.',
          'Pick a project, or type a path under **Or any folder on …** (desktop app).',
          'Press **Start on …**.',
        ],
        points: [
          "It runs under that computer's account for the folder, and you drive it from here.",
          'Claude Code must already trust the folder there: open it there with Claude Code once.',
        ],
        links: [{ label: 'Open New session', path: '/new' }],
      },
      {
        heading: 'When a computer is offline',
        points: [
          'Its sessions stay listed, greyed, under **On computers that are offline**, until it is back.',
          "A session's page says since when its computer is offline. What you see is from then, and sending waits until it is back.",
          'A computer asleep cannot answer: **Keep awake while used** keeps each one awake while another uses it.',
        ],
      },
      {
        heading: 'From a phone',
        points: [
          'A phone paired with any of your computers sees and drives all of them through it.',
          "A dangerous action on another computer needs the phone's dangerous permission on the computer it is paired with, and that computer's **Dangerous** permission on the other.",
          "A phone on **Limited** (refreshing every few seconds instead of live updates) sees the other computers' sessions too, on the same refresh.",
          'Setting up your computers, pairing them and changing their permissions are done in the desktop app; a phone finds **Computers** under **More**.',
        ],
      },
    ],
  },
  {
    id: 'machines-move',
    title: 'Move a session to another computer',
    icon: 'move',
    group: 'machines',
    summary: 'Continue a session on another computer, with its code, then bring it back or undo.',
    intro:
      'A session can go on on another of your computers: the same conversation, with its code, where it stands. A session lives on one computer at a time; the computer it left keeps its copy, read only, and says where it lives now.',
    keywords: [
      'move',
      'continue on another computer',
      'bring back',
      'take over',
      'undo',
      'worktree',
      'recipe',
      'code',
      'lives on',
    ],
    sections: [
      {
        heading: 'Move it',
        steps: [
          "Open the session's **⋮** menu and choose **Continue on another computer…**.",
          'Under **Computer**, pick where it goes. Mother Claude asks that computer first and shows what it found: nothing is written anywhere yet.',
          'Check **What goes**, **Where it goes**, **Code travels**, **Code lands** and the **Checks**.',
          'Press **Continue on …**.',
        ],
        tips: [
          "On another computer's session, the menu has **Bring back to this Mac** (or **Bring back to this PC**), or **Continue on this Mac…** for one that never lived here. A move always starts on the computer the session lives on: from here, Mother Claude asks that computer to do it.",
        ],
      },
      {
        heading: 'The choices',
        terms: [
          {
            term: 'Computer',
            text: 'The paired computers that are online and let this one move sessions to them. An offline one says since when.',
          },
          {
            term: 'What goes',
            text: "The conversation's size, with its subagents, and the code: its branch, how many files changed and how many are new.",
          },
          {
            term: 'Not in git',
            text: 'The files git ignores that the project lists (**Computers › Options › Each project**), such as `.env.local`. They usually hold secrets, so they go only with the dangerous right here and the **Dangerous** permission of the computer they go to; untick **Carry them** to leave them. A file of the same name there is kept for **Undo move**.',
          },
          {
            term: 'Where it goes',
            text: "The project's folder there, found through your shared folders, a folder pair or the same path. **Change…** picks another; when none is found, **Choose its folder on …** lists that computer's projects.",
          },
          {
            term: 'Code travels',
            text: '**Computer to computer** (the default: straight between the two, nothing pushed anywhere), **Through your git remote** (hidden refs on your remote, removed afterwards unless kept) or **Only the conversation**. Through the remote, it must answer on both computers without a password prompt: that is checked before the move. Computer to computer, code of more than 1 GB goes through your remote on its own when it answers on both computers, and the move says so.',
          },
          {
            term: 'Code lands',
            text: "**In the project folder** (the default, when it has no changes of its own there) or **In a separate folder (worktree)**: a worktree of it in Mother Claude's data folder there; the project folder is then never touched, and nothing is merged into it.",
          },
          {
            term: 'After the move',
            text: "When the project has a recipe (set under **Computers › Options › Each project**, or a `.mother-claude/recipe.json` committed in the project): ask Claude to run its commands there, such as `npm ci`. They are in Claude's first message there and go through its own approvals: Mother Claude never runs a command on the other computer.",
          },
          {
            term: 'When',
            text: 'While a turn runs: **After this turn** (the default: it finishes first) or **Now** (it interrupts the turn; what you had queued goes along).',
          },
          {
            term: 'Keep driving it from here',
            text: 'On by default: after the move, this page becomes the session where it runs now.',
          },
          {
            term: "Carry this folder's trust to …",
            text: 'Shown when Claude Code there never opened that folder. A trusted folder runs its project hooks and MCP servers without asking, so this is a dangerous action: it needs the desktop app or a device allowed dangerous actions, and the **Dangerous** permission the computer it goes to gives the one it leaves.',
          },
        ],
      },
      {
        heading: 'Checks',
        points: [
          '**Checks** lists what is fine, what blocks the move with its fix, and what only makes it worse (Claude Code older there, line endings, Git LFS, submodules). Both computers must have Computers on.',
          'While something blocks it, the button reads **Fix the checks first**. When the project folder there has changes of its own, or its branch went on there, Mother Claude offers **Use a separate folder (worktree)**.',
          "When its branch went on there, Mother Claude also offers to replace it with this computer's: the branch there's own commits are kept, and **Undo move** puts them back. If the branch there moves on before the move lands, the move stops.",
          'A session another app runs (VS Code, a terminal) cannot be stopped by Mother Claude: a copy goes on there as a new session while this one goes on here. The button reads **Continue as a copy on …**; or end it in that app first.',
        ],
      },
      {
        heading: 'While it moves',
        points: [
          'Each step shows with its time: checked, stopped here, packed, sent (how much, and about how long is left), placed there, started there.',
          "**Cancel the move** works until it is being placed there. **Close (it goes on)** leaves it running; the session's page says what it does meanwhile.",
          'A connection that drops for a moment while it sends picks up where it stopped.',
          "The first message the session gets there tells Claude it moved: from which computer and folder to which, that paths in earlier messages are the old computer's, and which shell it has now. The conversation itself is never rewritten.",
          "What is worth knowing is listed under the steps and in **Recent moves**: a file of its own the other computer had, kept for **Undo move**, the checks' warnings and the code's (Git LFS, submodules, executable bits Windows drops…).",
        ],
      },
      {
        heading: 'If something fails',
        terms: [
          {
            term: 'Before it is placed there',
            text: 'Nothing is lost: the session goes on here as it was (and starts again if it ran), and the other computer drops what it received. Mother Claude says why; **Try again** goes back to the choices.',
          },
          {
            term: "The code can't land in the project folder",
            text: 'Mother Claude offers a separate folder (worktree) and asks again.',
          },
          {
            term: "Placed, but didn't start there",
            text: 'Its account is not signed in there, for instance. It lives there now: **Try again** starts it there without sending anything twice; **Undo move** takes it back.',
          },
        ],
      },
      {
        heading: 'Lives on …',
        intro:
          'The computer a session left keeps its copy, greyed in the lists with a chip **Lives on …**. Its page says where the session lives and since when, with:',
        terms: [
          { term: 'Open there', text: 'The session where it runs now.' },
          {
            term: 'Bring it back here',
            text: "A move back to this computer, started on the one it lives on, while that one is online. Only what happened there is added: this computer's copy is the start of the same conversation.",
          },
          {
            term: 'Take over…',
            text: "Only while the computer it lives on is offline: the session goes on here from this computer's copy, which has the conversation only up to when it moved, with this computer's project folder as it is. If the other computer went on with it meanwhile, both versions are kept side by side when it next moves there.",
          },
        ],
        points: ["Messages can't be sent to the copy here; its page says why."],
      },
      {
        heading: 'Undo a move',
        steps: [
          "Find the move: in the sheet once it is done, under **Recent moves** on **Computers › Activity**, or under **Moves between computers** on the session's **Info** tab.",
          'Press **Undo move**.',
          'Press **Tap again to undo**.',
        ],
        points: [
          "Undo is offered for seven days. It puts the other computer's project folder and branch back as they were, takes the conversation's files there back out, and the session lives here again (it starts again here if it ran).",
          'An undo is refused once the session went on there: bring it back instead, so nothing it did there is lost. A copy is never undone: stop or delete it there.',
        ],
        links: [{ label: 'Open Activity', path: '/computers/activity' }],
      },
      {
        heading: 'What never moves',
        points: [
          'Files git ignores, such as `.env` files, `node_modules` and build output, stay where they are, except the few the project lists to carry (**Not in git**). Set the project up there with a recipe.',
          'Sign-ins: tokens, keys and `.credentials.json` stay on each computer. The session runs under the account of the same name there, signed in there.',
          "The worktree a move made on the other computer, in Mother Claude's data folder there, stays when the session leaves it; its work went with the session. Once the session left it or ended there, **Remove the worktree** on the move (**Recent moves**, the session's **Info** tab) removes it, from either computer. It is refused while anything in it is in no snapshot of the session: move the session first.",
        ],
      },
      {
        heading: 'From a phone',
        points: [
          'A phone paired with any of your computers moves sessions the same way, in one column.',
          "Carrying a folder's trust or the files git ignores needs the phone's dangerous permission, set in the desktop app; without it the trust option does not show, and those files stay where they are.",
        ],
      },
    ],
  },

  {
    id: 'machines-copies',
    title: 'Keep a copy on another computer',
    icon: 'copy',
    group: 'machines',
    summary:
      'A session copied to another computer after each turn: moving it is instant, and it can go on there while this computer sleeps.',
    intro:
      'A session you will likely continue elsewhere can keep a copy there: after each of its turns, what grew is copied to the other computer. Moving it there is then instant, and if this computer is asleep or away, the other one can continue from the copy. It is off until you turn it on.',
    keywords: [
      'copy',
      'keep a copy',
      'copies',
      'continue here from the copy',
      'asleep',
      'offline',
      'instant move',
      'cap',
    ],
    sections: [
      {
        heading: 'Turn it on',
        steps: [
          "Open the session's **⋮** menu and choose **Keep a copy on…**.",
          'Tick the computers that keep a copy; each says how its copy stands.',
          'Press **Save**.',
        ],
        tips: [
          "For every session of a project: **Computers › Options › Each project**, the project's options, **Keep a copy of its sessions on**. A session's own choice in its menu wins; **As its project says** goes back to the project's.",
        ],
      },
      {
        heading: 'What it needs',
        points: [
          'The other computer’s **Move** permission for this one: a copy is the first half of a move.',
          'Both computers have Computers on. While it is off on either, the other computer is offline or the permission is missing, the copy is paused and says why. Nothing is queued: once that clears, the next copy sends what grew meanwhile.',
        ],
      },
      {
        heading: 'When, and what travels',
        points: [
          'After a turn, never in the middle of one: once the session is idle and has stayed quiet for five seconds.',
          "Only what grew: the conversation's new lines, its subagents' and workflows' new lines, new file-history snapshots, its todo list. A conversation file that was cut short or rewritten goes again whole.",
          "Never its code (that moves with a move), sign-ins or settings. The copy is kept in Mother Claude's data folder there, never in Claude Code's folders.",
        ],
      },
      {
        heading: 'How it stands',
        terms: [
          {
            term: 'Copied at 14:32',
            text: 'The copy there is as the session was after its last turn.',
          },
          { term: 'Copies once its turn ends', text: 'A turn runs; it goes when it ends.' },
          {
            term: 'Paused',
            text: 'With the reason: the other computer is offline, Computers is off on one of the two, the **Move** permission is missing, Mother Claude there is older, it made room for newer copies, or the session lives on another computer now.',
          },
        ],
      },
      {
        heading: 'Move it, instantly',
        points: [
          'A move to a computer that keeps a copy brings the copy up to date, then sends only the list of the session’s files with their fingerprints: the other computer checks its copy against it and lands from it. **Recent moves** shows it came from its copy.',
          'If the copy can’t be used, the move sends the conversation as usual.',
        ],
      },
      {
        heading: 'Continue here from the copy',
        steps: [
          "On the computer that keeps the copy, while the session's own computer is offline: open the session (its computer's sessions are greyed), or the **Copies** card on **Computers**.",
          'Press **Continue here from the copy of …** (on the card: **Continue here**), and confirm.',
        ],
        points: [
          "The session lives here from now on: its copy lands in this computer's Claude Code folder, under the account of the same name and in the folder your shared folders map it to, and it starts here with the model, effort and mode it had. The code is this computer's project folder as it is.",
          'If the first computer went on with it while it was away, nothing is lost: when the session next moves there, that computer’s own turns are kept beside it.',
        ],
      },
      {
        heading: 'The Copies card',
        points: [
          'The **Copies** card, at the foot of **Computers**, lists the copies held on this computer (with **Continue here** while their computer is offline, and **Remove**) and this computer’s sessions kept on others (with **Stop keeping it on …**, which removes the copy there too).',
          'Each computer keeps at most 2 GB of another computer’s copies (**Copies of other computers’ sessions**, under **Computers › Options › General**); the copy refreshed longest ago goes first, never one of a session that lives there.',
          'A phone paired with the session’s computer can choose where it keeps copies and remove them; the cap is set in the desktop app.',
        ],
      },
    ],
  },

  {
    id: 'machines-sync',
    title: 'Keep a project in sync between two computers',
    icon: 'refresh',
    group: 'machines',
    summary:
      'A project’s code kept the same on two computers, both ways: what changes on one goes to the other.',
    intro:
      'Working on one project from two computers? Code sync keeps its code the same on both: an edit, a new file, a commit or a branch switch on one shows up on the other a few seconds later. It is off until you turn it on, for one project at a time, with one other computer.',
    keywords: [
      'code sync',
      'sync',
      'keep in sync',
      'two computers',
      'same code',
      'conflict',
      'kept',
      'mirror',
    ],
    sections: [
      {
        heading: 'Turn it on',
        where: 'desktop',
        steps: [
          'Open **Computers › Code sync** and press **Keep a project in sync…**.',
          'Pick the **Project**, and the other computer under **With**.',
          'Check the files git ignores that travel with it (**Change** opens **Options › Each project**), then press **Keep it in sync**.',
          'On the other computer, the request shows in its **Inbox** and as a notice: someone there presses **Keep it in sync** (or **Not now**).',
        ],
        tips: [
          'It is on only once both computers said yes. The first sync then runs at once: the computer with changes, or newer commits, sends them to the one that is clean and behind. When both have changes of their own, you choose which to keep (below).',
          'A project’s own options have the same choice, **Keep this project in sync with …**: in **Computers › Options › Each project**, or under **When it moves** in its settings (**Project settings…**).',
        ],
        links: [{ label: 'Open Code sync', path: '/computers/sync' }],
      },
      {
        heading: 'What it needs',
        points: [
          'A shared folder that maps the project to its folder on the other computer (**Computers › Options › Accounts and folders**). The folder there must be the same project: the same `origin`, or a commit in common. **Clone it on …** first when it is not there.',
          'The **Move** permission both ways: code sync writes into each computer’s project folder.',
          'Both computers have Computers on.',
        ],
      },
      {
        heading: 'What travels',
        points: [
          'The branch’s commits and the staged, unstaged and untracked changes, exactly as they are: what a move carries. Nothing is merged, and nothing is pushed anywhere: code goes straight between the two computers.',
          'Files git ignores, such as `.env` files, `node_modules` and build output, stay on each computer, except the ones the project lists under **Files git ignores that travel with it**, and only when the other computer gave this one its **Dangerous** permission. The copy such a file replaces there is kept in Mother Claude’s data folder there (the last ten).',
          'A few quiet seconds after a change, the computer that changed sends it; a check every minute catches anything missed. **Sync now** runs one at once.',
        ],
      },
      {
        id: 'the-code-sync-card',
        heading: 'The Code sync tab',
        intro:
          '**Computers › Code sync** lists each project kept in sync, under **Projects kept in sync**, and how it stands:',
        terms: [
          { term: 'In sync', text: 'When it last synced, and which way.' },
          { term: 'Syncing…', text: 'Its changes are on their way.' },
          {
            term: 'Waiting for …',
            text: 'The other computer was asked; nothing syncs until someone there confirms.',
          },
          {
            term: 'Paused',
            text: 'With the reason: the other computer is offline, Computers is off on one of the two, a **Move** permission is missing, or a session working in the project is moving between them. It goes on by itself once that ends, from the project as it is then: nothing is queued.',
          },
          { term: 'Changed on both', text: 'Both computers changed it: you choose (below).' },
          {
            term: 'Off',
            text: 'Turned off on the other computer, or it said not now. **Turn on again** asks again; **Remove** drops it here.',
          },
        ],
        points: ['**Turn off** stops it at once on this computer and tells the other one.'],
      },
      {
        heading: 'When both computers changed it',
        intro:
          'Code sync never overwrites: when both computers changed the project since it was last synced, it stops on both, and the card shows how many files each side changed and whether it committed or switched branch.',
        steps: [
          'Press **Keep this Mac’s** (or **Keep this PC’s**) or **Keep …’s** for the other computer.',
          'Tap it again to confirm.',
        ],
        points: [
          'The side you don’t keep is first saved in its own repository, under `refs/mother-claude/sync/<id>/kept-<time>`: its working tree with the untracked files, what was staged and the commits it made. Its project folder then goes back to the last synced state, and the kept side’s changes arrive there.',
          'The card lists each saved state. To look at one, in that computer’s project folder: `git show --stat refs/mother-claude/sync/<id>/kept-<time>`.',
          'The same change made on both computers is not a conflict. When setting a side aside could lose something (a file git ignores in the way, a changed submodule, a merge under way), nothing changes and the card says why.',
        ],
      },
      {
        heading: 'From a phone',
        points: [
          'A phone sees **Computers › Code sync** and how each project stands, read only: turning it on or off, **Sync now** and choosing a side happen in the desktop app.',
        ],
      },
    ],
  },

  {
    id: 'machines-run',
    title: 'Run commands on another computer',
    icon: 'terminal',
    group: 'machines',
    summary:
      'Let a session build and test on another of your computers, at its own code, without moving it.',
    intro:
      "A Claude Code session on one computer can run commands on another: a session on your Mac runs the tests on your Windows PC, at the code it has right now, and reads what they printed. Nothing moves, and you don't switch computers. It is off until the computer that runs the commands allows it, and you approve each run.",
    keywords: [
      'run',
      'run commands',
      'build',
      'test',
      'machine_run',
      'work folder',
      'always allow',
      'output',
      'artifacts',
      'powershell',
      'zsh',
      'cancel',
    ],
    sections: [
      {
        heading: 'Turn it on',
        where: 'desktop',
        steps: [
          'On the computer that will run the commands, open **Computers**.',
          'On the card of the computer whose sessions will ask, next to **On this PC, … may** (or **On this Mac, … may**), press **Change**.',
          'Tick **Run**, for **Same network**, **Internet** or both, and press **Save**.',
          "On the computer whose sessions ask, keep **Give sessions Mother Claude's tools** on, in **Settings › Claude Code tools**: sessions reach the other computer through those tools.",
        ],
        tips: [
          '**Run** needs **Drive** and **See**: ticking it ticks them. It stays off for every computer until you tick it.',
          'The setup checks **Claude Code tools**, and **Turn on** switches them on. Its test then runs a fixed `echo` each way the permission is given: **Run there** and **Run here**.',
        ],
        links: [
          { label: 'Open Computers', path: '/computers' },
          { label: 'See Claude Code tools', path: '/help/claude-code-tools' },
        ],
      },
      {
        heading: 'Ask a session to use it',
        intro:
          "Ask in your own words, such as “run the tests on DESKTOP-PC”. Claude finds your computers, writes the command for that computer's shell (zsh on a Mac, PowerShell on Windows) and runs it. It has five tools for it:",
        terms: [
          {
            term: '`machine_list`',
            text: 'Your other computers: their system and shell, whether they are online, and whether this computer may run commands there.',
          },
          {
            term: '`machine_run`',
            text: 'Runs one command on one of them, in the folder the session works in or another folder of the project, and waits up to 10 minutes for it.',
          },
          { term: '`machine_wait`', text: 'Waits again for a run that is still running.' },
          {
            term: '`machine_output`',
            text: 'Reads all a run printed, page by page, or brings a file it made into the project.',
          },
          { term: '`machine_cancel`', text: 'Stops a run, with everything it started.' },
        ],
        points: [
          'The session must work in a git repository: its code goes to the other computer as a git snapshot.',
          "In Claude Code's default permission mode the session first asks to use the tool, as for any of Mother Claude's tools. That only lets it ask: each run still waits for your approval, in every mode.",
        ],
      },
      {
        heading: 'Approve each run',
        intro:
          "Each run waits for your approval on the computer whose session asks: in **Inbox**, on the session's page and on your phone. The card shows:",
        terms: [
          { term: 'Computer', text: 'Where it runs.' },
          {
            term: 'Folder',
            text: "That computer's copy of the project, and the folder in it. The command runs in a work folder there, at the session's code: the copy itself is not touched.",
          },
          { term: 'Command', text: 'The command line, exactly as it will run.' },
          { term: 'Shell', text: 'zsh on a Mac, PowerShell on Windows.' },
          {
            term: 'Time limit',
            text: '30 minutes unless the session asks for another; 2 hours at most.',
          },
          {
            term: 'Files to list',
            text: 'Files the run makes that Claude wants to see, such as `target/release/*.exe`.',
          },
        ],
        points: [
          '**Allow once** runs it. **Deny** tells Claude not to try again; **Deny with feedback…** also tells it what to do instead.',
          'Nobody answers within 10 minutes: it does not run.',
        ],
      },
      {
        heading: 'Always allow a command',
        steps: [
          "On a run's card, press **Always…**.",
          'Choose **Always allow … on … for this project**. It names the first words of the command, such as `cargo test`, and the computer.',
        ],
        points: [
          'From then on, a command of this project that starts with those words runs on that computer without asking. Whole words only: `cargo test` covers `cargo test --lib`, never `cargo test-foo`.',
          'A command holding `;`, `&`, `|`, `$`, `(`, `)`, `{`, `}`, `<`, `>`, a backtick or a line break always asks, whatever it starts with: nothing can ride along after an allowed command.',
          'A dangerous command is never allowed this way. Nothing is offered for a shell or an interpreter alone, such as `bash`, `pwsh` or `python`: Mother Claude offers the program and what it runs, such as `npm run build` or `python -m pytest`.',
          '**Always…** needs the dangerous right: the desktop app, or a phone allowed dangerous actions.',
          "They are saved in Mother Claude, not in Claude Code's settings. **Computers › Activity** lists them under **Always allowed runs**; **Revoke** takes one back (desktop app), and that command asks again.",
        ],
      },
      {
        heading: 'Dangerous commands',
        points: [
          "A command that touches Claude Code's configuration (`.claude/`, `.mcp.json`), Mother Claude's data, `~/.ssh`, a shell startup file or git hooks is dangerous: only the desktop app, or a phone allowed dangerous actions, can allow it, and never always.",
          'It also needs the **Dangerous** permission the computer that runs it gives this one. That computer checks the command too, before you are asked: a command only its own rules find dangerous is a dangerous run all the same.',
        ],
      },
      {
        heading: 'Where it runs',
        points: [
          "In a work folder of its own on the other computer: one per project and per computer that asks, in Mother Claude's data folder there. It is a git worktree of that computer's copy of the project, the folder your shared folders point to.",
          "Before each run, the work folder is set to the session's code as it is now: committed or not, new files included. Files git ignores, such as `target/` and `node_modules/`, stay between runs, so the next build picks up where the last one stopped.",
          'Your own copy of the project on that computer is never changed: its files, branch and staged changes stay as they are.',
          "The project must be on that computer. If it isn't, the run is refused: clone it there first, with **Clone it on …** under **Computers › Options › Accounts and folders**.",
        ],
      },
      {
        heading: 'How it runs',
        points: [
          'On a Mac in zsh, as a login shell: what your `.zprofile` puts on the `PATH` is there, but `.zshrc` is not read. On Windows in PowerShell 7 when it is installed, else in Windows PowerShell, without your PowerShell profile.',
          'Without your tokens or API keys, and with nothing to type into: a command that asks a question fails instead of waiting.',
          'One run at a time per work folder: the next one waits its turn, and its time limit counts meanwhile.',
          'When the time limit runs out, or you cancel, the command stops with everything it started. Whatever it leaves running in the background stops when it ends.',
        ],
      },
      {
        heading: 'What comes back',
        points: [
          'Claude gets the exit code, how long it took and the last 64 KiB of what it printed. A run longer than 10 minutes answers that it is still running, and Claude waits again.',
          'All it printed stays a day on the computer that ran it, and Claude can read it page by page.',
          'Files it made are listed with their sizes. Claude can bring one, up to 64 MiB, into the project under `.mother-claude/artifacts/`, which git ignores, until the next run resets the work folder.',
        ],
      },
      {
        heading: 'Follow and stop runs',
        points: [
          "On the session's page, a run is a background task: open **Agents**, then **Background tasks**. **Output** shows what it printed; **Cancel** stops it.",
          '**Computers › Activity** lists the recent runs both ways under **Runs**: the computer, the project, the command, how it stands, its exit code and time. **Output** shows the end of what it printed; **Cancel** stops a run, from either computer.',
          'Each computer keeps its runs a day after they end.',
          'Turning Computers off, revoking a computer or taking back its **Run** permission stops what it runs here at once.',
        ],
        links: [{ label: 'Open Activity', path: '/computers/activity' }],
      },
      {
        heading: 'From a phone',
        points: [
          "A phone answers a run's approval like any other, follows runs on the session's page and under **Runs**, and cancels them.",
          "**Always…** and dangerous runs need the phone's dangerous permission, set in the desktop app. A phone sees **Always allowed runs** but can't revoke them.",
        ],
      },
      {
        heading: "If it doesn't run",
        terms: [
          {
            term: '… has not given this computer the Run permission',
            text: 'Tick **Run** for this computer in **Computers** on that computer, in the column of the network you use now.',
          },
          {
            term: "… isn't on …",
            text: 'That computer has no copy of the project, or no shared folder points to it. Clone it there (**Clone it on …**, under **Computers › Options › Accounts and folders**), then try again.',
          },
          {
            term: 'Update Mother Claude on …',
            text: "That computer's Mother Claude is older and doesn't run commands for other computers yet: update it.",
          },
          {
            term: 'running scripts is disabled on this system',
            text: 'Windows PowerShell follows your execution policy, which blocks `npm.ps1` and other scripts. Install PowerShell 7 and restart Mother Claude there, write `npm.cmd`, or allow your own scripts there: `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`.',
          },
          {
            term: 'A program is not found',
            text: 'On a Mac, put what adds it to the `PATH` in `.zprofile`, not only `.zshrc`. On Windows, restart Mother Claude after installing it: a run gets the `PATH` Mother Claude started with.',
          },
        ],
      },
    ],
  },

  {
    id: 'machines-task',
    title: 'Hand a task to another computer',
    icon: 'send',
    group: 'machines',
    summary:
      'Let a session hand a task to a Claude Code session on another of your computers, and get its work back as a branch.',
    intro:
      "A Claude Code session on one computer can hand a whole task to a session on another: your Mac's session asks your Windows PC to make the installer build on Windows. A session there does it in its own copy of the project, at the Mac session's code, and its work comes back to the Mac as a branch, with its last message as the summary. You approve each task.",
    keywords: [
      'task',
      'hand over',
      'machine_task',
      'branch',
      'worktree',
      'summary',
      'another session',
      'delegate',
    ],
    sections: [
      {
        heading: 'Turn it on',
        points: [
          'Handing tasks over uses the **Run** permission: a session that works on the other computer runs code there. Turn it on as for running commands.',
          'Both computers need this version of Mother Claude.',
        ],
        links: [{ label: 'See Run commands on another computer', path: '/help/machines-run' }],
      },
      {
        heading: 'Ask a session to use it',
        intro:
          'Ask in your own words, such as “have DESKTOP-PC make the installer build on Windows”. Claude writes the task as a full prompt, since the session there knows nothing of your conversation, and hands it over:',
        terms: [
          {
            term: '`machine_task`',
            text: "Hands the task over and waits up to 10 minutes for that session to finish its turn. It can choose the session's permission mode, model and effort.",
          },
          {
            term: '`machine_wait`',
            text: 'Waits again for a task that is still running, or for its next end once it runs again.',
          },
          {
            term: '`machine_cancel`',
            text: "Stops the task's session. What it did so far still comes back.",
          },
        ],
      },
      {
        heading: 'Approve each task',
        intro:
          "Each task waits for your approval on the computer whose session asks: in **Inbox**, on the session's page and on your phone. The card shows:",
        terms: [
          { term: 'Computer', text: 'Where the task goes.' },
          {
            term: 'Folder',
            text: "That computer's copy of the project. The task gets its own worktree of it, at the session's code: the copy itself is not touched.",
          },
          { term: 'Task', text: 'The whole prompt the session there gets.' },
          {
            term: 'Permission mode',
            text: 'How the session there asks before it acts: **default** unless Claude asked for another.',
          },
          {
            term: 'Account',
            text: "The Claude account it runs under there: the one with the same name as this session's account, else the Default.",
          },
        ],
        points: [
          'There is no **Always…**: every task is approved.',
          'A task whose session runs without approvals (`bypassPermissions` or `dontAsk`) is dangerous: only the desktop app, or a phone allowed dangerous actions, can allow it, and the other computer must have given this one its **Dangerous** permission.',
        ],
      },
      {
        heading: 'Where it works',
        points: [
          "In a worktree of its own on the other computer, in Mother Claude's data folder there: a git worktree of that computer's copy of the project, set to the session's code, committed or not.",
          'Your own copy of the project there is never changed.',
          "The session there is an ordinary Claude Code session that Mother Claude runs. Both computers list it, named after where it came from and the task's first line; you drive it and answer its approvals from either one.",
        ],
      },
      {
        heading: 'What comes back',
        points: [
          'When its turn ends with nothing waiting, what it did, its commits and the changes it left uncommitted, comes back as the branch `mother-claude/<computer>/<task>` in this project.',
          'The branch is never checked out or merged: look at it with `git log` and `git diff`, then merge what you want, or delete it.',
          "Claude also gets the session's last message as the task's summary. A task that changed nothing makes no branch.",
          'Send the session another message, from either computer, and the task runs again: when that turn ends, the branch moves forward with what it did since. It never rewrites what the branch already holds, and never moves a branch you committed onto or have checked out.',
          'A done task whose session sits idle for 30 minutes with nothing to do has its session stopped there, as team roles do; its worktree stays. A message to it from either computer resumes it in its worktree, and the task runs again.',
        ],
      },
      {
        heading: 'Follow and remove tasks',
        points: [
          '**Computers › Activity** lists the tasks both ways under **Tasks**: where, the project, the task, how it stands and the branch. **Open its session** opens it on the computer it runs on; **Summary** shows its last message; **Cancel** stops a task that still works.',
          '**Remove** takes away a task that ended, with its worktree and session on the other computer. Its branch stays.',
          'Each computer keeps a task 7 days after it last ended, then removes it with its worktree.',
          "On the asking session's page, a task is a background task too: open **Agents**, then **Background tasks**.",
          'Turning Computers off, revoking a computer or taking back its **Run** permission cancels the tasks it still works on here.',
        ],
        links: [{ label: 'Open Activity', path: '/computers/activity' }],
      },
      {
        heading: "If it doesn't go",
        terms: [
          {
            term: '… has not given this computer the Run permission',
            text: 'Tick **Run** for this computer in **Computers** on that computer.',
          },
          {
            term: '… needs its Dangerous permission',
            text: 'Tick **Dangerous** for this computer on the other computer, or let the task run in the default mode and answer its approvals.',
          },
          {
            term: "… isn't signed in on …",
            text: 'Sign that account in on the other computer, under **Accounts** there.',
          },
          {
            term: 'The branch did not move',
            text: 'You committed onto it, or a worktree has it checked out: Mother Claude never moves it under you. The task names the newer commit, and `git branch` makes a branch of it.',
          },
        ],
      },
    ],
  },

  // ────────────────────────────────────────────────── Accounts and data
  {
    id: 'accounts',
    title: 'Claude accounts',
    icon: 'user',
    group: 'data',
    summary:
      'Run sessions under several Claude accounts, see how much of each plan is left, and switch when one hits its limit.',
    intro:
      'Mother Claude can run sessions under several Claude accounts at once. When one reaches its limit, carry on under another. **Accounts** has three tabs: **Accounts**, **Usage** and **Recent changes**; their backups are on **Backups**. While only the Default exists, the app shows no account chips or pickers.',
    keywords: [
      'subscription',
      'limit',
      'token',
      'setup-token',
      'directory',
      'sign in',
      'login',
      'switch',
      'consent',
      'plan usage',
      'quota',
      'rate limit',
      'hooks plugin',
    ],
    sections: [
      {
        heading: 'Three kinds of account',
        terms: [
          {
            term: 'Default',
            text: 'The account Claude Code on this computer already uses. You can rename it, but not remove it.',
          },
          {
            term: 'Token account',
            text: 'Another subscription working on the same projects and conversations (**Shared data, another account**, recommended). Mother Claude gets its token with `claude setup-token`.',
          },
          {
            term: 'Directory account',
            text: 'A separate Claude Code setup (**Separate directory**) with its own settings, plugins, MCP servers, conversations and sign-in, such as `~/.claude-work`.',
          },
        ],
        tips: [
          'Pick a token account to share everything, and a directory account to keep work and personal fully apart.',
        ],
        links: [{ label: 'Open Accounts', path: '/accounts' }],
      },
      {
        heading: 'Accounts, tab by tab',
        terms: [
          {
            term: 'Accounts',
            text: 'Each account with its kind, its sign-in, its sessions, its plan usage and its **Sessions started elsewhere** line; then **Defaults**, **Per project**, **Copy for Continue on** and the newest **Moves and copies** (**See all** opens **Recent changes**).',
          },
          {
            term: 'Usage',
            text: "How much of each account's plan is left, how often it is read, and the rate limits teams hit.",
          },
          {
            term: 'Backups',
            text: "Encrypted backups of each account's conversations, pulls from your other computers, storages and folder pairs.",
          },
          {
            term: 'Recent changes',
            text: 'Every move, copy, backup, restore, pull and delete, newest first, with **Undo** while it is possible.',
          },
        ],
        links: [
          { label: 'See Backups', path: '/help/backup' },
          { label: 'See Move, copy and delete', path: '/help/move-data' },
        ],
      },
      {
        heading: 'Add a token account',
        where: 'desktop',
        steps: [
          'Open **Accounts** and press **Add account**.',
          'Choose **Shared data, another account** and press **Next**.',
          'Fill in **Name** and pick a **Colour**, then press **Add account**.',
          'Press **Sign in to get a token**.',
          'Choose where the sign-in page opens: a private Chrome window (recommended), **Open in browser**, **Open in Chrome**, or **Copy link**.',
          'Sign in with the Claude account you want, and approve.',
          'Paste the code the page shows into **Code from the browser** and press **Send the code**.',
          'When it says the token is saved, press **Done**.',
        ],
        tips: [
          "A private window keeps a browser that's already signed in to another Claude account from approving the wrong one.",
          'Already have a token? Press **I already have a token — paste it**.',
        ],
      },
      {
        heading: 'Add a directory account',
        where: 'desktop',
        steps: [
          'Press **Add account**.',
          'Choose **Separate directory** and press **Next**.',
          "Fill in **Name**. **Directory** suggests a folder; a new one is created if it doesn't exist yet.",
          'Press **Add account**.',
          'Press **Sign in now**. Your default browser opens the sign-in page.',
          'Sign in, then paste the code into **Code from the browser** and press **Send the code**.',
          'Press **Done**.',
        ],
        tips: [
          'If the browser opens the wrong Claude account, use a private Chrome window or **Copy link** instead.',
        ],
      },
      {
        heading: 'Sign in again',
        where: 'desktop',
        points: [
          "Press **Sign in** on the account's row, or **Get token** / **New token** for a token account. **Manage** has **Sign in again** and **Check the sign-in again** too.",
          'Only one sign-in runs at a time. **Cancel** stops it.',
          'If it signed in as a different Claude account than before, or as the same one as another account, a warning says so. Press **Keep** if that is right, or **Sign in again**.',
        ],
        notes: [
          'Tokens and sign-ins stay on the computer that made them. On another computer the account needs its own sign-in.',
        ],
      },
      {
        heading: 'Sessions started elsewhere, by account',
        points: [
          "Each account's row on **Accounts** says whether Mother Claude can answer and message the sessions you start under it in a terminal, VS Code or JetBrains: **Sessions started elsewhere: on** or **Sessions started elsewhere: off**.",
          'In the desktop app, that line opens **Sessions started elsewhere**, where the plugin of every account is installed and uninstalled.',
          'A directory account keeps the plugin in its own folder, so it has a row of its own there.',
          "A token account has no line: its sessions use the Default's plugin.",
        ],
        links: [
          { label: 'Open Sessions started elsewhere', path: '/elsewhere' },
          { label: 'See Sessions started elsewhere', path: '/help/foreign-sessions' },
        ],
      },
      {
        heading: 'Choose which account runs what',
        steps: [
          'Open **Accounts**.',
          'Under **Defaults**, set **New sessions run under** and **The API runs under**.',
          'For one project, under **Per project**, type its folder in **Project folder**, pick the **Account** and press **Add**.',
        ],
        points: [
          'Folders inside the project count too; the longest matching folder wins.',
          '**New session** starts on that account, and you can change it there under **Runs under**.',
        ],
      },
      {
        heading: 'Switch a session to another account',
        intro: 'For a session Mother Claude runs, the same conversation goes on, on the same page.',
        steps: [
          "Open the session's **⋮** menu.",
          'Choose **Switch account…**.',
          'Pick the account.',
          'If Claude is working, choose **After this turn** or **Now** under **When**.',
          'Press the **Switch to …** button, which names the account.',
        ],
        points: [
          'A switch that waits for the turn shows a line with **Cancel**.',
          'Its background tasks stop. If the other account keeps its own folder, the conversation moves there first (the first time, it asks: **Allow and go on**).',
          "A session started elsewhere can't change account while it runs there: the button continues it on the other account instead.",
        ],
      },
      {
        heading: 'Continue on another account',
        intro: 'You get a copy to drive under the other account; the original stays as it was.',
        steps: [
          "Open the session's **⋮** menu.",
          'Choose **Continue on another account…**.',
          'Pick the **Account**.',
          'Press the **Continue on …** button, which names the account.',
        ],
        points: [
          'The first copy into an account with its own folder asks first: **Copy and continue**, or **Not now**.',
        ],
      },
      {
        heading: 'Allow copies between two accounts',
        steps: [
          'Open **Accounts** and find **Copy for Continue on**.',
          'Under **Allow copies**, pick a pair (from → to).',
          'Press **Allow**.',
        ],
        points: [
          'Each pair works in one direction. **Revoke** takes it back.',
          'It covers the copies **Continue on** makes and the moves **Switch account** makes.',
        ],
      },
      {
        heading: 'When an account reaches its limit',
        points: [
          'The Sessions list and the session pages say so, and offer another account that has room: **Switch to** it, **Continue on** it, or **Choose an account**.',
          '**Not now** hides the offer until the limit resets.',
          "Each account's plan usage shows on its row on **Accounts**, and in full on **Usage**. A token account's usage shows once a session has run under it.",
        ],
      },
      {
        heading: 'How much of each plan is left',
        points: [
          "**Accounts › Usage** shows, under **How much is left**, each account's windows (the 5-hour window and the weekly ones) with their reset times, any extra usage, when they were read and how many of its sessions run.",
          "**Refresh** reads an account's usage again now.",
          'A token account cannot read its plan on its own: its usage shows once a session runs under it.',
          "The plan meter at the top of **Sessions** opens this page, and a session's **Info** tab shows the plan of its account.",
          '**Rate limits teams hit** lists the last rate limits Claude reported to team sessions; **Teams › Capacity** says which role sessions wait for a window.',
        ],
        links: [{ label: 'Open Usage', path: '/accounts/usage' }],
      },
      {
        heading: 'How often usage is read',
        steps: [
          'Open **Accounts › Usage** in the desktop app.',
          'Under **Reading it**, pick how often in **Read usage every**: **Every 3 min**, **Every 5 min (recommended)**, **Every 10 min** or **Manually only**.',
        ],
        points: [
          'It is read at that pace while a dashboard is open and a session is active, and every 30 minutes while nothing is.',
          'With **Manually only**, it is read when you press **Refresh**, or by a session Mother Claude runs when a dashboard connects or a limit is hit.',
          'A phone shows the setting; it is changed in the desktop app.',
        ],
      },
      {
        heading: 'Rename or remove an account',
        steps: [
          "Press **Manage** on the account's row.",
          'To rename it, change **Name** and press **Save**.',
          'To remove it, press the **Remove …** button with its name, then **Remove** (or **Keep it**).',
        ],
        points: [
          'Removing forgets the account. Its folder and its Claude sign-in stay; a token is deleted.',
          'An account cannot be removed while sessions Mother Claude started, or the API, still use it.',
          'Removing is done in the desktop app; a phone can rename.',
        ],
      },
      {
        heading: 'On a phone',
        points: [
          'A phone can rename accounts, set the defaults, allow or revoke copies, move data, see how much of each plan is left, pick the account of a new session and continue or switch a session.',
          'Adding, signing in, tokens, removing, the plugin and how often usage is read are done in the desktop app.',
        ],
      },
    ],
  },
  {
    id: 'move-data',
    title: 'Move, copy and delete',
    icon: 'move',
    group: 'data',
    summary: 'Move or copy conversations, projects and customizations; delete them; undo.',
    intro:
      'Move or copy any number of conversations, whole projects or everything an account has: between accounts, and to or from a storage. Each move is checked first and can be undone for seven days.',
    keywords: [
      'transfer',
      'copy',
      'move',
      'delete',
      'undo',
      'history',
      'customizations',
      'skills',
      'claude.md',
      'finder',
      'explorer',
    ],
    sections: [
      {
        heading: 'Move or copy',
        intro:
          "Start from **Move or copy data…** on an account's row on **Accounts**, from **Move or copy…** in a session's **⋮** menu, or from **Select** on the Sessions list.",
        steps: [
          'Choose **From** and **To**.',
          'Under **What**, choose **Everything**, **Projects**, **Conversations** or **Customizations**, and tick what to take.',
          'Under **How**, choose **Copy** or **Move**.',
          'Press **Check what it does**.',
          'Read **What goes** and **Skipped**.',
          'Press the button that names the move, such as **Move 3 conversations**.',
        ],
        tips: [
          "**Also trust their folders for** the target account carries Claude Code's folder trust across too.",
        ],
      },
      {
        heading: 'Copy customizations',
        points: [
          'Under **What › Customizations**, tick any of **Memory** (CLAUDE.md), **Settings**, **Skills**, **Agents**, **Commands**, **Output styles**, **Keybindings**, **MCP servers** and **Prompt history**.',
          'Customizations are copied, never moved, and only between accounts with different folders. Plugins are never copied.',
          '**Settings**, **MCP servers** and folder trust need the desktop app, or a device allowed dangerous actions.',
        ],
      },
      {
        heading: 'Delete conversations or projects',
        steps: [
          "Choose **Delete…** in a session's **⋮** menu, on a project's menu, or press **Select** and then **Delete N…**.",
          'Check **What goes** and **Skipped**.',
          'Leave **Keep a copy for 7 days so this can be undone** ticked.',
          'Press the button that names the delete, such as **Delete 2 conversations**.',
        ],
        notes: [
          'Deleting always needs the desktop app, or a device allowed dangerous actions. A conversation that is running is skipped.',
        ],
      },
      {
        heading: 'Recent changes',
        intro:
          '**Accounts › Recent changes** lists every change to your data, newest first: moves and copies between accounts, backups, restores, pulls and deletes, each with how it ended.',
        points: [
          'The chips show one kind: **All**, **Moves and copies**, **Backups** or **Deleted**.',
          'It updates by itself when a job starts or ends, from any device, and a running job shows how far it got.',
          '**Moves and copies** on **Accounts**, **Recent** on **Backups** and **Recently deleted** on **Sessions › Projects** list the newest three; their **See all** opens this list at their kind.',
          'Sessions moved between your computers are under **Computers › Activity**.',
        ],
        links: [{ label: 'Open Recent changes', path: '/accounts/changes' }],
      },
      {
        heading: 'Undo',
        steps: [
          'Find the job on **Accounts › Recent changes**, or in the short list where you started it.',
          'Press **Undo**.',
          'Press **Tap again to undo**.',
        ],
        points: [
          'Undo is possible for seven days; the job says until when.',
          'It is refused if a copy was continued since: that conversation has moved on.',
        ],
      },
      {
        heading: "Find a conversation's files",
        steps: [
          'Open the session and choose the **Info** tab.',
          'Next to **Transcript**, press **Show in Finder** or **Show in File Explorer** (desktop app), or **Copy path**.',
        ],
      },
      {
        heading: 'What is refused, and why',
        points: [
          'A conversation that is running is skipped: stop it first.',
          "While Claude Code runs in the target account's folder, files it shares are not changed. Quit those sessions and try again; copies that only add new files go ahead.",
          'Between the Default and a token account there is nothing to copy: they share the same folder, so **Move** only changes the account it belongs to.',
          'One move, copy, backup or restore runs at a time.',
        ],
      },
    ],
  },
  {
    id: 'backup',
    title: 'Backups',
    icon: 'cloud',
    group: 'data',
    summary:
      'Encrypted backups in a folder you choose, and your conversations on a second computer.',
    intro:
      "Mother Claude keeps an encrypted copy of each account's conversations in a folder you choose (iCloud Drive, Google Drive, OneDrive, a disk), on **Backups**. Only your passphrase opens it. The same folder carries conversations to another computer: back up on one, pull on the other. Nothing happens on its own: you back up and pull, or set a schedule. Keeping a project's code the same on two computers is **Computers › Code sync**.",
    keywords: [
      'backup',
      'backup & sync',
      'restore',
      'pull',
      'sync',
      'passphrase',
      'storage',
      'archive',
      'folder pairs',
      'windows',
      'mac',
      'second computer',
      'pc',
      'drag and drop',
      'conversations',
    ],
    sections: [
      {
        heading: 'Set it up',
        intro:
          'Until your first backup exists, the page opens with **Set up backups in three steps**.',
        steps: [
          'Open **Backups**.',
          'Step 1: press one of the suggested folders (iCloud Drive, Google Drive…), or **Type a folder…**, type its full path in **Backup folder** and press **Save**.',
          'Step 2: press **Set a passphrase…** and follow the dialog (below).',
          'Step 3: press **Back up now**.',
        ],
        tips: [
          'iCloud Drive: right-click the folder and choose **Keep Downloaded**, so pulls and restores never wait. OneDrive: **Always keep on this device**. Google Drive: use a folder inside My Drive.',
          'Later, the folder, the passphrase and the schedule are under **Settings** on the same page. To stop backing up, empty the folder and save.',
        ],
        links: [{ label: 'Open Backups', path: '/backups' }],
      },
      {
        heading: 'Your backups at a glance',
        intro: '**Your backups** shows one lane per account, from left to right:',
        terms: [
          {
            term: 'This Mac (or This PC)',
            text: 'The account on this computer, what kind it is and how many of its sessions run.',
          },
          {
            term: 'The line between',
            text: 'How its backup stands: **Backed up 2h ago** in green; **Not backed up yet**, or a backup older than twice the schedule, in amber; a failed backup or a missing key in red; **Working…** while a backup, pull or restore runs.',
          },
          {
            term: 'Backups',
            text: "The account's backup, with **Back up**, **Pull…**, **Restore…**, **Force backup…**, **Restore settings** and **Conversations**.",
          },
          {
            term: 'Other computers',
            text: 'The computers that back up to it too, and when they last did.',
          },
        ],
        tips: [
          'On the desktop you can drag instead of pressing buttons: an account onto the backups, a backup onto an account, conversations onto an account. Every drag has a button that does the same.',
        ],
      },
      {
        heading: 'Set the passphrase',
        where: 'desktop',
        steps: [
          'Under **Passphrase**, press **Set a passphrase…**.',
          'Press **Generate a passphrase**.',
          'Write it down: it is shown only once.',
          'Tick **I have written it down**.',
          'Press **Done**.',
        ],
        tips: [
          '**Use my own…** takes one of at least 12 characters: type it in **Passphrase** and **Type it again**, then press **Save in the Keychain**.',
          '**Change…** sets a new one later.',
        ],
        notes: [
          "Without the passphrase no backup can be read — by you or anyone. Mother Claude can't recover it.",
        ],
      },
      {
        heading: 'Back up',
        steps: [
          'Under **Settings › Schedule**, pick how often in **Back up**: from **By hand only** to **Every 24 hours**.',
          "To back up now, press **Back up now** beside **Your backups** (every account), or **Back up** on one account's backup.",
          'Or, on the desktop, drag an account from **This Mac** (or **This PC**) onto the backups: it is backed up at once.',
          'Check the result under **Recent** (**See all** lists every backup, restore and pull).',
        ],
        points: [
          'A scheduled backup waits until no session of that account has been active for ten minutes. A session Claude is working in is skipped until it is idle; an idle one is backed up even while it stays open.',
          'When a backup ends, a notice says how it went, and why when something was left out.',
          '**Pull on launch** pulls when the app starts, when no session runs.',
          'Sign-ins and tokens are never backed up: each computer signs in by itself.',
        ],
      },
      {
        heading: 'Bring conversations back by dragging',
        where: 'desktop',
        steps: [
          "On an account's backup, press **Conversations**. The list shows what the backup holds, by project.",
          'Read how each compares with the account here: **Not on this computer**, **Newer in the backup**, **Changed on both** or **Up to date**. **Can come here** shows the ones that would change something; **All** shows everything.',
          'Tick the ones you want, or none to drag a single one.',
          'Drag them onto their own account (its lane, or its chip above the list): they come here at once.',
          'Or drag them onto another account: **Restore…** opens for that account with them ticked.',
        ],
        points: [
          'Bringing them is a pull of just those: newer copies replace the ones here (kept for undo) and both sides of a conflict are kept.',
          'One that Claude Code would delete again at its next cleanup opens **Pull…** first, which says so.',
          'Without dragging: tick them and press the account chip above the list.',
        ],
      },
      {
        heading: 'Restore a backup',
        steps: [
          "On an account's backup, press **Restore…** (or drag the backup onto another account to restore into that one).",
          'Choose **Backup of** and **Restore into**.',
          'Tick the sessions to restore. New ones start ticked; each says how it compares with the copy here.',
          'Press **Restore N sessions**.',
        ],
        points: [
          'Old sessions get a warning first: Claude Code deletes conversations older than its cleanup period. Press **Restore anyway** to go on.',
          'To make the backup win over the copies the account has, tick **Force: replace the copies … has**. It needs the desktop app, or a device allowed dangerous actions.',
        ],
      },
      {
        heading: "Pull another computer's conversations",
        steps: [
          "Press **Pull…** (beside **Your backups** for every account, or on one account's backup), or drag a backup onto its own account.",
          'Check what each session will do, for example **Newer in the backup — replaces the copy here, kept for undo**, **Changed on both — kept side by side** or **Running here — skipped**.',
          'Press **Pull N sessions**.',
        ],
        points: [
          'A session changed on both computers is kept twice, never merged.',
          "To make the backup win, tick **Force: the backup's copy replaces …**. **Force backup…** on an account does the opposite. Both need the desktop app, or a device allowed dangerous actions.",
        ],
      },
      {
        heading: 'Undo a pull or a restore',
        steps: [
          'Under **Recent**, find the job: **See all** opens older ones on **Accounts › Recent changes**.',
          'Press **Undo**.',
          'Press **Tap again to undo**.',
        ],
        points: ['Undo is possible for seven days. A backup itself has nothing to undo.'],
        links: [{ label: 'Open Recent changes', path: '/accounts/changes' }],
      },
      {
        heading: 'Storages and archives',
        intro:
          'Besides the backup folder you can add storages: folders for archives and for sets of conversations you share.',
        steps: [
          'Under **Storage**, press **Add storage…**.',
          'Pick a **Suggested:** folder or type one in **Folder**, add a **Label** if you like, and press **Add storage**.',
          'On a set, press **Move here…** (or **New set…** on the storage).',
          'Under **What**, choose **Conversations** and tick them.',
          'Under **How**, choose **Copy** to keep them here too, or **Move** to archive them.',
          'Press **Check what it does**, then the button that names it, such as **Archive 3 conversations**.',
        ],
        points: [
          '**Bring back…** on a set restores its conversations the same way.',
          "When the account already has a conversation and the set's copy is different (you continued it on another computer, say), press **Replace with the set's copies**: the account's copy is kept for undo. One that is open here is never replaced.",
          "On the desktop, drag an account onto a storage's set to open **Move here…** from it, or drag a set onto an account to open **Bring back…** into it.",
          'Removing a storage leaves its files where they are.',
        ],
      },
      {
        heading: 'Folder pairs',
        intro:
          'A pair links a folder here to the folder with the same projects on another computer, so a pulled session lands in the matching project here (and back again).',
        steps: [
          'Under **Folder pairs**, type the folder on this computer in **Folder on …**.',
          'Pick the **Other computer**.',
          'Fill in **Its folder**.',
          'Press **Add pair**.',
        ],
        tips: [
          "Another computer shows here once it has backed up to or pulled from these backups; **Look again** checks. A pull's report offers **Pair a folder…** with the fields filled in.",
        ],
      },
      {
        heading: 'Continue a conversation from another computer',
        intro:
          'When you continue a conversation that another computer started and no pair says where its folder is here, Mother Claude asks: **Choose its folder on this PC** (or **on this Mac**).',
        steps: [
          'Read where the conversation worked on the other computer.',
          'Pick one of the folders here that look like it (the likeliest is picked), or choose **Another folder** and type its full path.',
          'Leave the **Pair** box ticked to remember the match, or untick it for this conversation only.',
          'Press **Continue here**.',
        ],
        points: [
          'The conversation runs in the folder you chose; it is not moved.',
          "With **Pair** ticked, the folders are paired (the ones above both projects when they share a name), so that computer's other conversations there continue here without asking.",
          '**Cancel** does nothing. A message you were sending stays unsent, with **Retry**.',
          'It appears for **Continue here**, a message to an ended conversation and **Continue on another account…**, and also when the folder is gone from this computer.',
          'From a phone you can only pick a folder Claude Code already trusts on your computer.',
        ],
      },
      {
        heading: 'Mac and Windows, step by step',
        steps: [
          'On the first computer: choose the backup folder, generate the passphrase and press **Back up now**.',
          'On the second computer: choose the same folder, the way that computer reaches it.',
          'On the second computer: press **Set a passphrase…**, then **Use my own…**, type the same passphrase and press **Save in the Keychain**.',
          "Optional: under **Folder pairs**, pair your projects folder with the other computer's.",
          'Press **Pull…**, then **Pull N sessions**.',
          'Open a conversation and press **Continue here**. Choose its folder if asked.',
          'To go back: **Back up now** on this computer, then **Pull…** on the other.',
        ],
        tips: [
          'Give each account the same name on both computers: backups are matched by account name.',
        ],
      },
      {
        heading: 'From a phone',
        points: [
          'A paired phone can do everything here except set or change the passphrase. It uses the buttons: dragging is for the desktop.',
          'Force actions and folder trust need a device allowed dangerous actions (**Phone & devices** in the desktop app).',
        ],
      },
    ],
  },

  // ─────────────────── The browser (Settings and connections), then Teams
  {
    id: 'browser',
    title: 'The browser extension',
    icon: 'globe',
    group: 'settings',
    summary: 'Let sessions read and act in your own Chrome, Edge or Brave, with your approval.',
    intro:
      'The Mother Claude extension lets your Claude Code sessions read and act in your own browser, signed in as you. Each session works only in its own coloured tab group, every site starts **Off**, and risky steps wait for your answer. Nothing leaves your computer.',
    keywords: ['chrome', 'edge', 'brave', 'extension', 'tab', 'site', 'side panel', 'lease'],
    sections: [
      {
        heading: 'Install the extension',
        steps: [
          "On the project's **Releases** page, under **Assets**, download `mother-claude-chrome-<version>.zip`.",
          'Unzip it into a folder you will keep: the browser loads it from there each time it starts.',
          'Open `chrome://extensions` (`edge://extensions` in Edge, `brave://extensions` in Brave).',
          'Turn on **Developer mode**.',
          'Press **Load unpacked** and choose the unzipped folder (the one that holds `manifest.json`).',
          'Pin **Mother Claude** from the puzzle-piece menu.',
        ],
        points: [
          'It works in Chrome, Edge, Brave and Chromium 116 or later, not in Firefox or Safari. Each browser pairs on its own.',
          'Phones cannot run it.',
        ],
      },
      {
        heading: 'Pair the browser',
        steps: [
          'In Mother Claude, open **Browser**.',
          'Press **Pair a browser**. A code appears; it works once, for 5 minutes.',
          'In the browser, click the Mother Claude button: the side panel opens on **Pair this browser**.',
          'Type the code in **Pairing code** and check **Name of this browser**.',
          'Press **Pair**.',
          'When the panel says **Connected**, press **Done** in Mother Claude.',
        ],
        notes: ['Pairing is done in the desktop app, or on a phone allowed dangerous actions.'],
        links: [{ label: 'Open Browser', path: '/browser' }],
      },
      {
        heading: 'Choose what sessions may do on a site',
        intro: 'Every site has a mode: **Off** (the default), **Read** or **Act**.',
        steps: [
          "Under **Site modes**, press **Off**, **Read** or **Act** on **Every other site** or on a site's row.",
          'To add a site, type its address (such as `https://github.com` or `https://*.example.com`), pick a mode and press **Add**.',
        ],
        points: [
          'In the side panel, **This tab** sets the mode of the site you are on. **Use the default for this site** clears it.',
          "A site's own entry beats a wildcard, and `https://*.example.com` covers the subdomains but not `example.com` itself.",
          'Sensitive sites (a built-in list of payment, banking and sign-in sites) stay **Off** until you give one its own mode, and always ask before a session opens or acts on one.',
          '**Act** on **Every other site**, on a wildcard or on a sensitive site needs the desktop app, or a device allowed dangerous actions.',
        ],
      },
      {
        heading: 'Give sessions the browser tools',
        points: [
          "Sessions use the browser through Mother Claude's tools, which are on by default (**Settings › Claude Code tools**).",
          'Sessions started outside Mother Claude also need the Mother Claude plugin (**Sessions started elsewhere**).',
          'A session that was already running needs `/reload-plugins` or a restart. In the side panel it shows **No browser tools** until then.',
        ],
        links: [{ label: 'See Claude Code tools', path: '/help/claude-code-tools' }],
      },
      {
        heading: 'Tab groups',
        points: [
          "A session's first browser action opens a coloured tab group named after the session and its project. It works only in that group's tabs.",
          'Drag a tab into the group to give it to the session, or out to take it back.',
          "The side panel's **Sessions in this browser** shows what each one is doing.",
          'A session stays connected to its tab group until you press **Stop**, close the group, close the browser or quit Mother Claude — however long it goes without a browser action, and even after it ends.',
        ],
      },
      {
        heading: 'Several sites, network, cookies and storage',
        intro:
          'Sessions can work on many sites at once and look at what you would see in the developer tools.',
        points: [
          'A session can keep several tabs in its group, one site in each, and work in them side by side. Each site keeps its own mode.',
          "**Network**: the requests a tab made (address, status, size, time), one in full with its headers and bodies, and WebSocket messages. Recording starts with the session's first action in the tab: reload a page to see its requests from the start.",
          '**Cookies**: names, expiry and flags of the cookies sent to the page. Their values, and setting or deleting one, wait for your approval.',
          "**Storage**: the site's local and session storage, and its IndexedDB databases (read only). Values, and every change, wait for your approval.",
          'Sign-in headers (such as `Authorization` and `Cookie`) always show as `«redacted»`.',
          "Network requests, cookies, screenshots and the console need the extension's **debugger** permission. Chrome shows a bar while it is in use.",
        ],
      },
      {
        heading: 'Approvals',
        points: [
          "Requests appear in the Inbox, on the session's page, under **Waiting for you** on the **Browser** page, and in the side panel. The first answer wins; no answer within 5 minutes counts as no.",
          'Always asked: a sensitive site, forms with password, card or one-time-code fields, running a script, uploading files, reading or changing cookie and storage values, going over the per-minute limit, and any action in the minute after a Stop.',
          '**Always…** on a card saves a standing grant. Grants are listed under **Always allowed**, where **Remove** takes one back.',
        ],
      },
      {
        heading: 'Stop and Resume',
        steps: ['In the side panel, press **Stop** on the session.'],
        points: [
          'Its running command fails, its approvals are withdrawn, and its browser actions are refused for 60 seconds. Closing its tab group does the same.',
          '**Resume** lifts it early, from the side panel or under **Sessions using the browser** on the **Browser** page (which has **Stop** too).',
          'During that minute, other sessions acting in that browser need approval.',
        ],
      },
      {
        heading: 'Work on the tab you are on',
        intro: 'Bring a session you already have to the page in front of you.',
        steps: [
          'Open the page and click the Mother Claude button.',
          'Under **This tab**, press **Read** or **Act**.',
          "Under **Work here with a session**, choose the session (**Refresh** if it isn't listed).",
          'Write in **What should it do here?**.',
          'Press **Connect and send**.',
        ],
        points: [
          "The tab joins the session's tab group. Later, **Tell it what to do** on its card sends more.",
          "The session gets your words and the site, never the page's title or path.",
          'A session that runs without permission prompts is told what to do from the desktop app only.',
        ],
      },
      {
        heading: 'What is never done',
        points: [
          'Sessions never type into password, card, one-time-code or other secret fields: they ask you to type them yourself.',
          "Uploads come only from the session's project folder or a folder you added under **Folders sessions may upload files from**, in **Options** on the **Browser** page, never from Claude Code's folders, `~/.ssh` or files that usually hold secrets.",
          '**Activity** records the tool, site, path, outcome and time, never what was typed or read.',
        ],
      },
      {
        heading: 'Remove a browser, or update the extension',
        points: [
          'To unpair, press **Remove this browser** (the bin) next to it under **Browsers**, then **Remove**. This needs the desktop app, or a device allowed dangerous actions.',
          "To update, unzip the new release into the same folder and press the reload arrow on the extension's card in `chrome://extensions`. The pairing stays.",
        ],
      },
    ],
  },
  {
    id: 'teams',
    title: 'Teams of sessions',
    icon: 'layers',
    group: 'teams',
    summary: 'A lead and role sessions working through a task board in one project.',
    intro:
      "A team applies a template to a project: a lead session writes tasks on the project's board, and role sessions do them, each in its own files. Mother Claude starts, wakes and stops the sessions it runs, as capacity allows. **Teams** has three tabs: your teams, the **Templates** they start from, and **Capacity**.",
    keywords: [
      'team',
      'role',
      'lead',
      'board',
      'task',
      'template',
      'worktree',
      'workflow',
      'flow',
      'handoff',
      'diagram',
    ],
    sections: [
      {
        heading: 'Start a team',
        steps: [
          'Open **Teams** and press **Start a team** (on every tab of Teams).',
          'Choose the **Project**.',
          'Choose a **Template**.',
          'With several accounts, choose the **Account**.',
          'Optional: write a **Brief for the lead (optional)**.',
          "Under **Roles**, change a role's **Model**, **Effort**, **Permission mode**, **Computer** or **Start with the team** if you like.",
          'Press **Start the team**.',
        ],
        points: [
          'Built-in templates: Software delivery, Content production, Research & analysis, Browser operations and Documentation.',
          'One team per project. Roles with their own worktree need the project to be a git repository.',
        ],
        tips: [
          "Claude Code's own agent teams, in your interactive sessions, are not Mother Claude's teams: what they report is under **Sessions › Agents › Teammates**.",
        ],
        links: [{ label: 'Open Teams', path: '/teams' }],
      },
      {
        heading: 'The board',
        points: [
          'Tasks move through **To do**, **Claimed**, **In progress**, **Review**, **Done**, **Failed** and **Cancelled**. On a phone, a **Status** picker shows one column at a time.',
          'The lead and the role sessions add and update tasks themselves. You can add them too.',
        ],
      },
      {
        heading: 'Add a task',
        steps: [
          'On the team, press **New task**.',
          'Fill in **Title**, **Goal** and **Acceptance criteria (one per line)**.',
          'Pick the **Role** and the **Priority**.',
          'Optional: pick a **Class** and fill in **Files (one per line)** and **Constraints (one per line)**.',
          'Press **Add the task**.',
        ],
      },
      {
        heading: 'Work on a task',
        intro: 'Tap a task to open it.',
        points: [
          'Change its **Priority** or **Role** until it ends.',
          'Type a **Note to the session** and press **Add note**.',
          'Press **Unblock** on a blocked task.',
          'In Review, press **Accept** or **Send back**. Tick **Sending it back: give it a stronger model or more effort** first to escalate it.',
          '**Cancel task** stops it; **Reopen** brings an ended one back.',
        ],
      },
      {
        heading: 'Role sessions',
        points: [
          "A role with work starts for its most urgent task, as capacity allows; an idle one gets the next task. With nothing to do, it stops after the template's idle minutes, never mid-turn. Its conversation is kept.",
          "A role card's **Stop**, **Start** and **Resume** act on one role.",
          "Tap the role's session to open it and message it like any other session.",
          'A role that has to wait shows **Waiting** with the reasons; **Teams › Capacity** lists every role that waits, and why.',
        ],
        links: [{ label: 'See Capacity, routing and budgets', path: '/help/capacity' }],
      },
      {
        heading: "Each role's files",
        points: [
          'A role may change only the files its template gives it (its **owns …** list); read-only roles change nothing. A denied write shows a notice, and the role card counts **guard denials**.',
          "It is a fence, not a sandbox: writes through scripts, build tools or git aren't caught.",
          'Worktree roles work in `<project>/.claude/worktrees/<role>` on their own branch, which Mother Claude keeps. The lead merges.',
        ],
        tips: ['Add `.claude/worktrees/` to your `.gitignore`.'],
      },
      {
        heading: 'A role on another computer',
        intro:
          'A role can run on another of your computers: the lead plans on your Mac while a Windows role works on your PC, on the same board.',
        steps: [
          'On the computer that will run it, give this one the **Run** permission in **Computers**.',
          'In the template, give the role `"machine": "DESKTOP-PC"` (a paired computer’s name, or `os:windows` for any Windows one), or choose its **Computer** when you start the team.',
          'Start the team. The role starts on that computer when it has work.',
        ],
        points: [
          "It works there in a worktree of its own, at this project's code. Your copy there is not touched.",
          'Its tasks are on this board. When it finishes one, its work comes back here as the branch `mother-claude/<computer>/<team>/<role>`, which the task names; the lead reviews and merges it.',
          "Its card says **on DESKTOP-PC**, and why it waits or could not start. That computer's **Teams › Capacity** counts it, not this one's; this one's lists it apart, under **On your other computers**, with that computer's last decision.",
          'It cannot message the lead with SendMessage: it writes on its task, and Mother Claude tells the lead.',
          "When its session is stopped, a message you send it from either computer starts it again with its role's settings, then reaches it. If that computer can't run it now, the message is refused with why.",
          'That computer lists it under **Computers › Activity › Team roles**, where **Stop** and **Remove** act on it there.',
        ],
        links: [{ label: 'See Hand a task to another computer', path: '/help/machines-task' }],
      },
      {
        heading: 'A session started elsewhere as a role',
        steps: ['On a role that is not running, open **Take role…**.', 'Pick the session.'],
        points: [
          'It needs the Mother Claude plugin (**Sessions started elsewhere**) and must run in the project. Mother Claude tells it about its tasks but never starts or stops it.',
          '**Release role** gives the role back; its tasks return to the board.',
        ],
      },
      {
        heading: 'Stop, resume or remove a team',
        steps: [
          'Press **Stop the team** and confirm. Every role session stops; their conversations are kept.',
          'Press **Resume** to run it again.',
          'To delete a stopped team, press **Remove** and confirm. Its board stays with the project.',
        ],
      },
      {
        heading: 'Templates',
        points: [
          'The **Templates** tab lists the built-in templates and your own, each with its roles, their models and its flow: where a role’s finished work goes next.',
          '**New template** writes your own; built-in ones are fixed, so press **View** and then **Duplicate to edit** to change one.',
          'Your own templates have **Edit**, **Duplicate** and delete. **Edit** opens the template as a diagram (**Visual**): drag a role to move it, and drag from the dot on its right edge to another role to send its finished work there next, or to **Merge** to choose who merges. **Add role** and **Auto-arrange** are above it.',
          'Select a role, an arrow or **Merge** to change it in the panel beside the diagram (below it on a phone), which also does all a drag does. **JSON** shows the same template as text; **Check** puts each problem on its role or arrow and lists it, and **Save** keeps the template.',
        ],
        links: [{ label: 'Open Templates', path: '/teams/templates' }],
      },
    ],
  },
  {
    id: 'capacity',
    title: 'Capacity, routing and budgets',
    icon: 'gauge',
    group: 'teams',
    summary: 'Why a role session waits, your limits, budgets, model choice and costs.',
    intro:
      "**Capacity**, a tab of **Teams**, shows how many role sessions run and why others wait, with memory and each account's plan usage, in five sections: **Now**, **Budgets**, **Models**, **Costs** and **Settings**. Money figures are estimates.",
    keywords: [
      'limit',
      'budget',
      'cost',
      'costs',
      'routing',
      'model',
      'models',
      'haiku',
      'classifier',
      'report',
      'waiting',
    ],
    sections: [
      {
        heading: 'Why a role session waits',
        intro:
          '**Now** shows the role sessions that run and the ones that wait, each with its reasons in plain words. A role waits when:',
        points: [
          "Your limit of role sessions is reached (the lead does not count), or the project's.",
          'Not enough memory is free.',
          "The account's 5-hour window is nearly used up: only the lead starts at 85 %, nothing at 95 %.",
          'Claude refused a request on a rate limit: starts wait until the reset.',
          "The project's daily budget is spent.",
        ],
        tips: [
          'A role waiting longer than 10 minutes sends a push notification.',
          "The rate limits Claude reported, and how much of each account's plan is left, are also on **Accounts › Usage**.",
        ],
        links: [
          { label: 'Open Capacity', path: '/teams/capacity' },
          { label: 'Open Usage', path: '/accounts/usage' },
        ],
      },
      {
        heading: 'Set your limits',
        steps: [
          'Open **Teams › Capacity › Settings** and press **Edit**.',
          "Set **Your limit** and **A project's limit** (1 to 16).",
          "For one folder, press **Add a project** under **A project's own limit**.",
          'Press **Save**.',
        ],
        points: [
          'The defaults are 3 role sessions in all and 4 per project, with 2 GB of memory kept free. The memory and plan-usage thresholds are there too.',
        ],
      },
      {
        heading: 'Budgets',
        steps: [
          'Open **Teams › Capacity › Budgets** and press **Edit budgets**.',
          'Set **Every project, per day (USD)**, **A role session (USD)** and **A task (USD)**. An empty field means no budget.',
          'Set **Alert at (%)** and how to be alerted.',
          'Press **Save budgets**.',
        ],
        points: [
          "When a project's daily budget is spent, no new workers start there until tomorrow (the lead still can). From the alert level, Opus work there runs on Sonnet.",
          'A role session budget is enforced only on accounts that pay per token; on a plan it is shown, not applied.',
        ],
      },
      {
        heading: 'How a model is chosen',
        where: 'desktop',
        intro:
          'Each task class has a model and effort: planning, architecture and review on Opus, implementing, testing and fixing on Sonnet, search and summaries on Haiku. A task that keeps failing its checks moves up a step.',
        steps: [
          'Open **Teams › Capacity › Models** and press **Edit the policy**.',
          'Change **Task classes**, **Escalation ladder**, **Budget gate** or **Fallback models**.',
          'Press **Save the policy**.',
        ],
        points: [
          '**Reset to defaults** puts the policy back.',
          'A phone shows the policy; it is edited in the desktop app.',
        ],
      },
      {
        heading: 'The task classifier',
        where: 'desktop',
        points: [
          'Under **Task classifier**, **Classify with Claude Haiku (spends tokens)** sorts tasks with a model instead of by keywords. It is off by default and needs an Anthropic API key.',
          '**How accounts pay** says whether each account is on a plan or pays per token.',
        ],
      },
      {
        id: 'reports',
        heading: 'Costs',
        points: [
          '**Costs** shows what tasks cost and how they ended, for **7 days**, **30 days** or **90 days**. **CSV** and **Every task** download them.',
          'On a plan, the cost is what the API would have charged.',
        ],
      },
      {
        heading: 'On a phone',
        points: [
          'A phone sees everything here and can change limits and budgets.',
          'The routing policy, how accounts pay and the classifier are changed in the desktop app.',
        ],
      },
    ],
  },

  // ───────────── Claude Code tools and the HTTP API (Settings and connections)
  {
    id: 'claude-code-tools',
    title: 'Claude Code tools',
    icon: 'zap',
    group: 'settings',
    summary: "Mother Claude's own tools for your sessions: the browser, the task board and more.",
    intro:
      'Mother Claude gives every Claude Code session on your computer its own tools: the browser, the team task board, teams, your other computers (`machine_run` and the rest), and Mother Claude itself. Sessions see them as `mcp__plugin_mother-claude_mc__<tool>`.',
    keywords: ['mcp', 'tools', 'server', 'services', 'plugin', 'reload-plugins'],
    sections: [
      {
        heading: 'Turn them on or off',
        where: 'desktop',
        steps: [
          'Open **Settings** and, under **Claude Code**, choose **Claude Code tools**.',
          "Under **Claude Code tools (MCP)**, switch **Give sessions Mother Claude's tools**.",
        ],
        points: [
          'They are on by default. Sessions started from then on get them; running ones after `/reload-plugins` or a restart.',
          'Off: sessions get no tools, and calls from running ones are refused.',
        ],
        links: [{ label: 'Open Claude Code tools', path: '/settings/tools' }],
      },
      {
        heading: 'Which sessions have them',
        points: [
          'Sessions Mother Claude starts get them by themselves.',
          'Terminal, VS Code and JetBrains sessions get them through the Mother Claude plugin: install it under **Sessions started elsewhere**.',
          'Conversations started through the HTTP API do not get them yet.',
          'The page shows how many **Tools** there are, **Sessions connected (last 15 min)**, whether **Sessions Mother Claude starts** get the tools, the plugin, and **Last calls**.',
          'When the plugin is not installed, or is older than this app, a line says so, with a link to **Sessions started elsewhere**.',
        ],
        links: [{ label: 'Open Sessions started elsewhere', path: '/elsewhere' }],
      },
      {
        heading: 'Approvals',
        points: [
          'In the default permission mode Claude Code asks before using one of these tools. A session Mother Claude runs asks with a card; a session started elsewhere asks in its own window.',
          'Anything dangerous a tool would do becomes an approval card first.',
        ],
      },
      {
        id: 'the-services-page',
        heading: 'What Claude Code runs besides its sessions',
        intro:
          'Besides its sessions, Claude Code runs a background service, MCP servers and background jobs. **Computers › This Mac › Details** (**This PC** on Windows) shows them, and the link at the bottom of **Settings › Claude Code tools** leads there.',
        terms: [
          {
            term: 'Claude Code’s background service',
            text: 'Its daemon: reachable or not, and its status.',
          },
          { term: 'MCP servers', text: 'The MCP servers configured for Claude Code, by name.' },
          { term: 'Background jobs', text: 'Jobs started with `claude --bg`.' },
        ],
        points: ['**Check again** reads it all again.'],
      },
    ],
  },
  {
    id: 'api',
    title: 'The HTTP API',
    icon: 'code',
    group: 'settings',
    summary: 'Drive Claude on this computer from scripts and tools, over HTTP.',
    intro:
      'Mother Claude can run a local HTTP API that drives Claude on this computer with your account: conversations, turns, live events and tool approvals. A conversation started through it is a normal session in the dashboard, and its approvals appear as cards.',
    keywords: ['bridge', 'http', 'rest', 'curl', 'script', 'token', 'swagger', 'port', 'server'],
    sections: [
      {
        heading: 'Start the API',
        steps: [
          'Open **HTTP API**.',
          'Under **Defaults**, fill in the **Working directory** (empty means your home folder).',
          'Pick **Model**, **Effort**, **Thinking** and **Permissions**, or leave them on **Default** to use your own Claude settings.',
          'Under **Conversation**, choose which conversation **Messages join**.',
          'Press **Start the API**.',
        ],
        points: [
          'The API is off every time the app opens, and nothing listens until you start it.',
          'What you pick here are only defaults: every request can override them.',
          'To apply a change later, press **Restart with these settings**. **Switch conversation** changes only the conversation.',
          '**Stop** closes the port; conversations already running carry on.',
        ],
        links: [{ label: 'Open HTTP API', path: '/http-api' }],
      },
      {
        heading: 'Try it',
        points: [
          'While the API runs, **API reference** opens its interactive reference and **Console** opens a page where you can chat, watch the stream and approve tool calls.',
          'The screen shows a ready-made `curl` command for your settings.',
          'These links work on the computer itself.',
        ],
      },
      {
        heading: 'Tokens',
        points: [
          'Programs on this computer need no token.',
          'A request that comes through a tunnel needs a token, and while a tunnel points at the API port every request does.',
          'The **Authentication** line on **HTTP API** says what is needed right now.',
        ],
      },
      {
        heading: 'Open to tunnels',
        where: 'desktop',
        points: [
          'Under **Access**, **Open to tunnels** accepts calls without a token from anyone who can reach the port, through a tunnel too. Restart the API to apply it.',
          "Anyone with the tunnel's address can then start conversations and approve ordinary tool calls. Leave it off unless you need it; it is off again after every restart of the app.",
        ],
      },
      {
        heading: 'An API server per account',
        steps: [
          'Press **Add API server**.',
          'Fill in a **Name** and, with several accounts, pick the **Account**.',
          'Leave **Port** empty to take the next free one.',
          'Press **Add API server**.',
          'Check its defaults, then start it, or press **Save without starting**.',
        ],
        points: [
          "**Only this account** makes a server run as one account: other accounts' conversations do not exist through it.",
          '**Manage** renames a server, changes its port or removes it. Removing stops it; its conversations are kept.',
        ],
      },
    ],
  },

  // ─────────────────────────────────────────────────── Troubleshooting
  {
    id: 'troubleshooting',
    title: 'Troubleshooting',
    icon: 'question',
    group: 'troubleshooting',
    summary: 'What the messages mean, and what to do about them.',
    intro: 'Find the message or symptom you see, then the fix.',
    keywords: ['error', 'problem', 'not working', 'fix', 'help'],
    sections: [
      {
        heading: 'Connection',
        terms: [
          { term: 'Live', text: 'Connected: updates arrive as they happen.' },
          {
            term: 'Limited',
            text: "Live updates can't connect, so the app refreshes every few seconds. Usually the phone doesn't trust your computer's certificate yet: trust it, or use your tunnel address.",
          },
          {
            term: "Can't reach Mac",
            text: "Your computer is asleep or offline, the tunnel stopped, or a quick tunnel's address changed. Keep the computer awake while you are away.",
          },
          {
            term: 'This device is no longer paired.',
            text: 'It was revoked in the desktop app, unused for too long, or its storage was cleared. Pair it again.',
          },
          {
            term: "The tunnel's browser warning page is in the way",
            text: "ngrok's free plan shows a warning page. Open the address once in the phone's browser, accept it and reload.",
          },
        ],
      },
      {
        heading: 'Pairing',
        terms: [
          {
            term: 'This code is for your home network and cannot be used through a tunnel or proxy.',
            text: 'You used a **Same Wi-Fi** code over mobile data, a tunnel or Tailscale. Create an **Internet** code.',
          },
          {
            term: 'This code was already used by another device.',
            text: 'Deny the request and create a new code.',
          },
          {
            term: 'Those digits do not match the ones on the phone.',
            text: 'Type the 4 digits the phone shows. After three wrong tries, start again on the phone.',
          },
          {
            term: 'No public URL yet',
            text: 'No tunnel was found. Start one of the commands under **Start a tunnel** on **Phone & devices**, or type its address under **Or enter it yourself**.',
          },
          {
            term: 'Mother Claude is not listening on your local network.',
            text: 'Your computer has no private network address (or the app was started for this computer only). Use a tunnel, or **Also listen on Tailscale**.',
          },
        ],
      },
      {
        heading: 'Messages and sessions',
        terms: [
          {
            term: 'A message waits "after its current step"',
            text: 'Claude reads it when its current step ends, in the same turn. **Send now** makes it read it at once.',
          },
          {
            term: 'A message stays "queued"',
            text: "The session (one started elsewhere) hasn't reached a tool call or the end of its turn, or nothing can wake it. Take it back from the queue if you change your mind. Unread after 30 minutes, it is dropped with a notice.",
          },
          {
            term: 'Started before the Mother Claude plugin was installed',
            text: 'Type `/reload-plugins` in that session, or restart it.',
          },
          {
            term: '`session_waiting` (tmux)',
            text: 'The session shows a dialog. Answer it first, or open the terminal view.',
          },
          {
            term: '`session_not_foreground` (tmux)',
            text: 'Claude is not the program in front in that pane. Bring Claude Code back to the front in its terminal.',
          },
          {
            term: '`session_ended`',
            text: 'The session ended. Continue it here instead.',
          },
          {
            term: '`untrusted_cwd`',
            text: 'Claude Code was never opened in that folder on your computer. Open it there once, or start the session from the desktop app.',
          },
          {
            term: '`dangerous_blocked`',
            text: 'That action needs the desktop app, or a device allowed dangerous actions on the network it uses now.',
          },
        ],
      },
      {
        heading: 'Notifications',
        points: [
          '"Blocked in the browser\'s site settings": allow notifications for the site.',
          'On iPhone: add Mother Claude to the Home Screen and enable push from there.',
          "On your home network: trust your computer's certificate first.",
          '**Send a test** under **Settings › Notifications** checks the whole path.',
        ],
      },
      {
        id: 'foreign-session-control',
        heading: 'Sessions started elsewhere',
        terms: [
          {
            term: 'Turn on Sessions started elsewhere on the … to approve and message this session from here.',
            text: "The session's account has no Mother Claude plugin yet. In the desktop app, press **Install** on its row in **Sessions started elsewhere**, then type `/reload-plugins` in the session.",
          },
          {
            term: '"Install" is greyed out',
            text: "The `claude` command wasn't found. Install Claude Code, or start the app from a shell where `claude` works.",
          },
          {
            term: 'No Mother Claude tools in a terminal session',
            text: "Install the Mother Claude plugin for that session's account (**Install** on its row in **Sessions started elsewhere**), then type `/reload-plugins`.",
          },
        ],
      },
      {
        heading: 'Accounts and sign-in',
        terms: [
          {
            term: 'The browser stops on a page that shows a code',
            text: 'That is expected: paste the code into **Code from the browser** and press **Send the code**.',
          },
          {
            term: "Claude Code didn't take that code",
            text: 'Paste the whole code. A code works once; for a new one, sign in again.',
          },
          {
            term: 'Signed in to the wrong Claude account',
            text: 'Sign in again in a private Chrome window, then press **Keep** on the warning when it is right.',
          },
          {
            term: 'A terminal session of a token account shows the Default',
            text: "Install the Mother Claude plugin on the Default's row (the first) in **Sessions started elsewhere**: a token account's sessions use the Default's plugin. The session's account shows after its first tool call.",
          },
          {
            term: "Can't remove an account",
            text: 'Sessions Mother Claude started or the API still use it. Stop them, or start the API with another account: **Open HTTP API** takes you there.',
          },
        ],
      },
      {
        id: 'backup-and-sync',
        heading: 'Backups',
        terms: [
          {
            term: 'Lost passphrase',
            text: "There is no recovery: backups made with it can't be read by anyone.",
          },
          {
            term: 'A passphrase change did not finish',
            text: 'Press **Set the passphrase again…** and set it (the same one or a new one).',
          },
          {
            term: 'A pull or restore from iCloud waits or fails',
            text: 'Right-click the backup folder and choose **Keep Downloaded**.',
          },
          {
            term: "A session from the other computer isn't in the right project",
            text: 'Add a folder pair, then pull again.',
          },
          {
            term: 'Both computers changed the same session',
            text: 'Both copies are kept. Use **Force backup…** or the force option in **Pull…** to choose which one wins.',
          },
        ],
      },
      {
        heading: 'Browser extension',
        terms: [
          {
            term: 'Mother Claude is not reachable',
            text: "Check that the app runs. On the extension's options page, **Address** should be `127.0.0.1` and **Port** `6725`; press **Save and reconnect**.",
          },
          {
            term: 'The code is not right, or has expired',
            text: 'Show a new code and type all 8 characters.',
          },
          { term: 'The user pressed Stop', text: 'Press **Resume**, or wait a minute.' },
          { term: 'Nobody approved …', text: 'Answer sooner, or add a standing grant.' },
          {
            term: 'Another connection took over',
            text: "Press **Reconnect now** on the extension's options page.",
          },
        ],
      },
      {
        heading: 'HTTP API and tools',
        terms: [
          {
            term: '`bridge_not_started`',
            text: 'Open **HTTP API** and press **Start the API**.',
          },
          {
            term: 'Not logged in · Please run /login',
            text: 'Sign Claude Code in again with `claude auth login` in a terminal.',
          },
          {
            term: 'Turns hang with no events',
            text: 'Claude waits for an approval: answer its card.',
          },
          {
            term: '`ECONNREFUSED` for the Mother Claude tools',
            text: 'Mother Claude was not running when the session started. Start the app, then reconnect in `/mcp` or restart the session.',
          },
          {
            term: '`mcp_disabled`',
            text: "Turn **Give sessions Mother Claude's tools** back on in **Settings › Claude Code tools**.",
          },
        ],
      },
    ],
  },
  {
    id: 'machines-troubleshooting',
    title: 'Troubleshooting your computers',
    icon: 'question',
    group: 'troubleshooting',
    summary: "When two computers don't find each other, refuse an action, or a move stops.",
    intro: 'Find the message or symptom you see, then the fix.',
    keywords: [
      'machines',
      'computers',
      'pairing',
      'nearby',
      'relay',
      'offline',
      'permission',
      'move',
      'trust',
      'account',
    ],
    sections: [
      {
        heading: 'Setup',
        terms: [
          {
            term: 'Turn on Computers is greyed out',
            text: 'A required check is left; the line above the button says how many. Fix them in the steps **This Mac** (or **This PC**) and **Every computer**.',
          },
          {
            term: 'Allow through the firewall changed nothing',
            text: "Windows asks for an administrator's OK first. When it was declined, the firewall was not changed: press it again.",
          },
          {
            term: 'Only its sessions wait for it: the rest works without it.',
            text: 'That account is not signed in on that computer. Its sessions wait for it there; everything else works.',
          },
        ],
      },
      {
        heading: 'Pairing',
        terms: [
          {
            term: 'The other computer is not in Nearby',
            text: 'Nearby only shows which computer shows a code: you always type the code, and it works across networks too. On a Mac, check Local Network access for Mother Claude.',
          },
          {
            term: 'No computer showing this code was found.',
            text: 'Check the code, and that both computers have the internet or share a network. A code lasts five minutes.',
          },
          {
            term: "That code doesn't match.",
            text: 'Check it on the other computer, or make a new one there.',
          },
          {
            term: 'This code was used by two computers, so neither can pair.',
            text: 'Make a new code.',
          },
          {
            term: "That's not the number on …",
            text: 'Nothing was paired. Start again, and type the six digits the other computer shows.',
          },
          {
            term: 'The pairing was not confirmed in time.',
            text: 'Confirm on both computers within ten minutes of making the code. Start again.',
          },
        ],
      },
      {
        heading: 'Connection',
        terms: [
          {
            term: 'Through the relay (slower), at home',
            text: 'On a Mac: Local Network access. On Windows: the network type and the firewall. The **This Mac** (or **This PC**) card on **Computers** says which.',
          },
          {
            term: '… uses another relay setting',
            text: 'Two computers on different relays may never meet. Press **Use it here too** in the **Inbox** of one of them, or **Use this setting everywhere** on the other.',
          },
          {
            term: 'A computer is offline',
            text: 'It is asleep, switched off, or Mother Claude is not running there. Its sessions stay listed, greyed, until it is back.',
          },
          {
            term: 'Older than this computer’s',
            text: "That computer's Mother Claude is older: its sessions show, but nothing moves to or from it until you update it.",
          },
          {
            term: 'Computers is off on …',
            text: 'Turn it on in **Computers** on that computer (or on this one, when it names this computer). While it is off, the others can only pair and run the setup with it.',
          },
        ],
      },
      {
        heading: 'Permissions',
        terms: [
          {
            term: '… lets this computer watch its sessions, not drive them.',
            text: 'Its **Drive** permission for this computer is off: change it in **Computers** on that computer.',
          },
          {
            term: "… doesn't let this computer take dangerous actions there.",
            text: 'Its **Dangerous** permission for this computer is off: change it on that computer. A phone also needs its own dangerous permission on the computer it is paired with.',
          },
          {
            term: "… doesn't let this computer move its sessions.",
            text: 'Its **Move** permission for this computer is off on the network you use now: change it on that computer.',
          },
          {
            term: '… refused this computer',
            text: 'Check on that computer that the two are still paired. If it revoked this one, pair them again.',
          },
        ],
      },
      {
        heading: 'Moving a session',
        terms: [
          {
            term: '… has no account named …',
            text: "Add the account there with the same name (**Accounts**) and sign it in. A session of the Default account goes on under that computer's Default.",
          },
          {
            term: "… isn't signed in on …",
            text: 'Sign it in on that computer (**Accounts** there), then try again.',
          },
          {
            term: "… doesn't trust … yet.",
            text: "Open that folder once with Claude Code there, or carry the folder's trust (it needs the dangerous permission).",
          },
          {
            term: '… has changes of its own',
            text: 'Use a separate folder (worktree), or commit or stash the changes there first.',
          },
          { term: '… runs this session right now.', text: 'Stop it on that computer first.' },
          {
            term: 'It runs without permission prompts',
            text: "A session in bypass-permissions mode needs the dangerous right here, and the other computer's **Dangerous** permission for this one.",
          },
          {
            term: "The remote … doesn't answer … without a prompt.",
            text: 'Sign in to the remote on that computer (Git Credential Manager, or `gh auth login`) and try again, or move the code computer to computer.',
          },
          {
            term: 'Carrying files git ignores needs …',
            text: 'They need the dangerous right where the session lives and the **Dangerous** permission of the computer they go to. Or untick **Carry them**: they stay where they are.',
          },
          {
            term: "Couldn't move …",
            text: 'Nothing was lost: it stays where it was, and the message says why. **Try again** goes back to the choices.',
          },
        ],
      },
    ],
  },
];

/** Every topic, in landing order (by group). */
export const HELP_TOPICS: readonly HelpTopic[] = HELP_GROUPS.flatMap((g) =>
  TOPICS.filter((t) => t.group === g.id),
).map(resolveTopic);

const BY_ID = new Map(HELP_TOPICS.map((t) => [t.id, t]));

/** The topic of `/help/:topic`, or undefined for one that does not exist. */
export function helpTopic(id: string | null | undefined): HelpTopic | undefined {
  return id ? BY_ID.get(id) : undefined;
}
