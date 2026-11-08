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
  { id: 'remote', title: 'Phone & remote' },
  { id: 'data', title: 'Accounts & data' },
  { id: 'browser-teams', title: 'Browser & teams' },
  { id: 'advanced', title: 'Advanced' },
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
    keywords: ['install', 'download', 'setup', 'first launch', 'onboarding', 'welcome'],
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
          "**Skip for now** lets you in, but the app can't read your sessions and a banner stays until you finish. Come back any time under **Settings › Permissions**.",
          'Local Network is optional: when macOS asks whether Mother Claude may find devices on your local network, click **Allow**, so your phone can reach it.',
        ],
        links: [{ label: 'Open Settings', path: '/settings' }],
      },
      {
        heading: 'Find your way around',
        points: [
          'On the desktop app, the sidebar has **Sessions**, **Inbox**, **Agents**, **New session** and **Teams**; under **Your data**, **Accounts** and **Backup & sync**; under **Connect**, **Remote**, **Browser** and **API**; under **System**, **Capacity** and **Settings**; and **Help** at the bottom.',
          'On a phone, the tabs at the bottom are **Sessions**, **Inbox**, **Agents** and **More**. Everything else is under **More**.',
          'The dot at the bottom of the sidebar (at the top on a phone) shows the connection. **Live** means updates arrive as they happen.',
          'The button beside it switches the theme between dark, light and system. **Settings › Appearance** also has the language, the accent colour and the background.',
          'The first time, a short **Welcome to Mother Claude** note explains how the app works. Press **Got it** to close it.',
        ],
      },
      {
        heading: 'What to do next',
        points: [
          'Start a session from **New session**: sessions started there can be fully controlled.',
          'Pair your phone from **Remote** on the desktop app.',
          'To answer sessions you started in a terminal or VS Code, install foreign-session control.',
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
          '**Continue here** turns any session into an owned one. A session still running elsewhere is forked into a copy; an ended one resumes in place.',
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
          'The desktop app can always do them. A phone can only after you allow it on the desktop app under **Remote › Paired devices**, separately for your home network and for the internet.',
        ],
        links: [{ label: 'See Internet access and devices', path: '/help/internet' }],
      },
    ],
  },
  {
    id: 'settings',
    title: 'Settings and updates',
    icon: 'settings',
    group: 'basics',
    summary: 'Notifications, theme, plan usage, connection and software updates.',
    intro:
      'Settings holds what applies to this device: its notifications, theme and connection. In the desktop app it also has software updates and macOS permissions.',
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
          { term: 'Claude accounts', text: 'The **Accounts** and **Backup & sync** pages.' },
          { term: 'Plan usage', text: "How often your Claude plan's limits are read." },
          {
            term: 'Connection',
            text: 'The server, how this device connects, whether it may take dangerous actions, and live updates.',
          },
          {
            term: 'Home Screen app',
            text: 'A one-time code for the iPhone Home Screen app (paired devices only).',
          },
          { term: 'Software updates', text: 'New versions of the app (desktop app only).' },
          {
            term: 'Permissions',
            text: 'macOS permissions such as Full Disk Access (desktop app only).',
          },
          { term: 'About', text: 'The version, **Help**, **Services** and the **HTTP API**.' },
        ],
        links: [{ label: 'Open Settings', path: '/settings' }],
      },
      {
        heading: 'Language, theme and colours',
        steps: [
          'Open **Settings**.',
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
        heading: 'Plan usage refresh',
        intro:
          "The Sessions page shows your Claude plan's limits (the current 5-hour window and the weekly ones) with their reset times.",
        steps: [
          'Open **Settings** in the desktop app.',
          'Under **Plan usage**, open **Plan usage refresh**.',
          'Pick **Every 3 min**, **Every 5 min (recommended)**, **Every 10 min** or **Manually only**.',
        ],
        points: [
          'It reads at that pace while a session works, every 30 minutes otherwise, and only while a dashboard is open. **Refresh** on the plan sheet reads it now.',
          'A phone shows the setting; it is changed in the desktop app.',
        ],
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
          'After the restart the HTTP API is off until you start it again on the **API** screen.',
        ],
      },
      {
        heading: 'Connection',
        points: [
          '**Connection** shows the server this device uses, how it connects, whether it may take dangerous actions, and the state of live updates.',
          '**Reconnect** connects again right away.',
          'In the desktop app, **Pair a phone** opens pairing. On a phone, **Unpair this device** removes it.',
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
      '**Sessions** lists every Claude Code session on your computer that is live or was active in the last 24 hours, whatever started it: a terminal, VS Code, JetBrains, Claude Desktop, a background job or Mother Claude itself.',
    keywords: ['list', 'filter', 'status', 'surface', 'projects', 'agents', 'fleet'],
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
        heading: 'Right now',
        intro:
          'The row at the top counts what is going on: sessions **working**, sessions that **will resume**, **agents running**, sessions that **need you**, and the cost so far today. Each part is a shortcut.',
        points: [
          "Beside it, your Claude plan's usage shows the 5-hour window and the week. Tap it for every window and its reset time.",
          'Costs are estimates, counted since midnight.',
        ],
      },
      {
        heading: 'Find a session',
        steps: [
          'Type in **Search sessions**.',
          'Choose **Live**, **Needs you** or **All**.',
          'Narrow it with **All surfaces** and **All projects** (and **All accounts**, with several accounts).',
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
          '**Projects**, beside **Sessions** at the top, lists every project folder that holds conversations, in every account: how many conversations it has, how many run, their size and the last activity.',
        points: [
          "A project's menu shows its folder on disk (desktop app), copies its path, moves or copies it, or deletes it.",
          'Deleted projects stay under **Recently deleted** for seven days, with **Undo**.',
        ],
      },
      {
        heading: 'Agents',
        intro:
          '**Agents** shows every running subagent, workflow agent, background shell and monitor, grouped by session. On a phone it is a tab of its own.',
        points: [
          'Tap an agent to read its own transcript, live while it runs. A shell or monitor shows its output.',
          'Stop one where its session allows it.',
          'The number on **Agents** counts the agents running.',
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
          'The arrow at the top left goes back to **Sessions**. The line under the name says what Claude is doing, such as the tool it runs, and for how long.',
          'The next row shows where the session runs and how it is reached (for example **VS Code · Via hooks**), then its model, permission mode and effort, its context use and cost.',
          "Tap a chip to change it. A chip that can't be changed explains why when you tap it.",
          '**⋯** opens the session menu.',
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
            text: "Name, status, id, project and folder, the transcript's path, where it runs, its Claude account, branch and pull request.",
          },
          {
            term: 'Model and usage',
            text: 'Model, permission mode, effort, ultracode, thinking, fast mode, cost and tokens.',
          },
          { term: 'Plan usage', text: "The plan of the session's account." },
          {
            term: 'From here you can',
            text: 'What works for this session, and the button that fixes each limit.',
          },
          {
            term: 'Actions',
            text: '**Terminal view**, **Open in VS Code**, **Stop**, **Respawn**, **Remove**, **Send a key**, the `claude --resume` command to copy, and **Continue here** with launch options.',
          },
          { term: 'MCP servers', text: 'The MCP servers of a session Mother Claude runs.' },
        ],
        tips: [
          "Next to the transcript's path, **Copy path** copies it; in the desktop app **Show in Finder** or **Show in File Explorer** opens its folder.",
        ],
      },
      {
        heading: 'The session menu',
        intro: 'Press **⋯** at the top right. It offers what applies to this session:',
        terms: [
          {
            term: 'Rename…',
            text: 'Renames the session. For a VS Code session the new name shows only in Mother Claude.',
          },
          {
            term: 'Continue here',
            text: 'Takes the session over as one Mother Claude runs (**Continue here (fork)** while it still runs elsewhere).',
          },
          {
            term: 'Switch account…',
            text: 'Runs it under another Claude account (with several accounts). **Continue on another account…** does it for a session started elsewhere.',
          },
          {
            term: 'Move or copy…',
            text: 'Moves or copies the conversation, or with **Move or copy this project…** its whole project, to another account or a storage.',
          },
          { term: 'Delete…', text: 'Deletes the conversation, with seven days to undo.' },
          {
            term: 'Terminal view',
            text: 'What its terminal shows (tmux sessions and background jobs).',
          },
          {
            term: 'Notifications',
            text: 'On a phone: **Follow**, **Default** or **Mute** push notifications for this session, and **Notify me when this turn finishes** while it works.',
          },
          {
            term: 'claude --resume …',
            text: '**Copy** puts the command to resume it in a terminal on your clipboard.',
          },
          { term: 'Open in VS Code', text: 'For sessions that run there.' },
          { term: 'Session info', text: 'Opens the **Info** tab.' },
          {
            term: 'Stop, Respawn, Remove',
            text: 'For background jobs. **Remove** needs the desktop app, or a device allowed dangerous actions.',
          },
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
          'Press **⋯**.',
          'Choose **Continue here**, or **Continue here (fork)** for a session still running elsewhere.',
        ],
        points: [
          'A session still running elsewhere is forked: you drive a copy with a new id, and the original keeps running where it is.',
          'An ended session resumes in place. Typing a message into it does the same: its button says **Resume**.',
          'To choose model, effort or mode first, use **Continue here** on the **Info** tab and open **Launch options (model · effort · mode)**.',
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
    summary: 'Answer permission prompts, questions and plans from any session.',
    intro:
      "When Claude needs you, a card appears: above the message box on the session's page, on its card in the list, and in the **Inbox**, which gathers everything waiting across all sessions.",
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
          '**Needs you** lists what you can answer from here, oldest first, with how long each has waited.',
          '**Waiting at the desk** lists what can only be answered where the session runs, and what would let you answer it here.',
          'The number on **Inbox** counts what waits.',
          'With several Claude accounts, the account filter shows one account at a time.',
        ],
        tips: [
          'On a keyboard, J and K move between requests; A allows, D denies, F adds feedback.',
        ],
        links: [{ label: 'Open Inbox', path: '/inbox' }],
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
          'A phone needs **Allow dangerous actions** for the network it is on, set in the desktop app under **Remote › Paired devices**.',
          'Any device can deny them.',
        ],
      },
      {
        heading: 'Sessions started elsewhere',
        points: [
          'With foreign-session control installed, a permission request from a terminal or VS Code session shows here while the session shows its own dialog. Whoever answers first wins; the other prompt goes away.',
          'Without it, those requests show under **Waiting at the desk**.',
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
          'Open **New session**.',
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

  // ───────────────────────────────────────────────────── Phone & remote
  {
    id: 'phone',
    title: 'Use it from your phone',
    icon: 'phone',
    group: 'remote',
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
    ],
    sections: [
      {
        heading: 'Pair a phone on the same Wi-Fi',
        steps: [
          'In the desktop app, open **Remote**.',
          'Under **Pair a device**, choose **Same Wi-Fi**.',
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
        links: [{ label: 'Open Remote', path: '/remote' }],
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
          "The Remote page shows the certificate's fingerprint, so you can compare it with what the phone shows.",
          'This certificate can only vouch for private addresses, so it cannot be used to impersonate a public website.',
        ],
      },
      {
        heading: 'Add it to the iPhone Home Screen',
        intro:
          'An app added to the Home Screen keeps its own storage, separate from Safari, so it pairs on its own.',
        steps: [
          'In Safari, tap Share › Add to Home Screen.',
          'In a paired Safari tab, open **Settings** and press **Set up the Home Screen app**.',
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
          'On the phone, open **Settings**.',
          'Under **Notifications**, press **Enable** next to **Push notifications**.',
          'Allow notifications when the browser asks.',
          'Pick the events you want: **Permission requests**, **Questions**, **Plans**, **Finished**, **Failures**, **When Claude asks** and **Waiting for you**.',
          'Press **Send a test**.',
        ],
        points: [
          '**Show details** adds the command or question to a notification.',
          'For one session: open its **⋯** menu and choose **Follow** or **Mute**, or **Notify me when this turn finishes**.',
          'Nothing is pushed to a device that has the app open in front of you, and bursts from one session are combined.',
          'The desktop app needs no push: it shows everything live.',
        ],
        notes: ['Your computer sends every notification, so it has to be awake and online.'],
      },
      {
        heading: 'What a phone can do',
        points: [
          'Watch every session, answer approvals, questions and plans, send messages and images, start sessions and continue them.',
          'Pairing devices, changing remote access and installing the plugin happen only in the desktop app.',
          'Dangerous actions need a grant for this device in the desktop app (**Remote › Paired devices**).',
        ],
      },
      {
        heading: 'Unpair this phone',
        steps: [
          'On the phone, open **Settings**.',
          'Under **Connection**, press **Unpair this device**.',
          'Press **Tap again to unpair**.',
        ],
        points: [
          'Its push notifications stop and the desktop app forgets it. You can also revoke it from **Remote › Paired devices**.',
        ],
      },
    ],
  },
  {
    id: 'internet',
    title: 'Internet access and devices',
    icon: 'shield',
    group: 'remote',
    summary: 'Reach your computer through a tunnel, approve devices, and manage their rights.',
    intro:
      'Away from home, your phone reaches your computer through a tunnel you run (Tailscale, ngrok or Cloudflare). Mother Claude never opens a port to the internet itself, and internet access is off until you turn it on.',
    keywords: ['tunnel', 'tailscale', 'ngrok', 'cloudflare', 'remote', 'away', 'revoke', 'grant'],
    sections: [
      {
        heading: 'Turn on internet access',
        where: 'desktop',
        steps: [
          'Open **Remote**.',
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
        links: [{ label: 'Open Remote', path: '/remote' }],
      },
      {
        heading: 'Pair a phone through the internet',
        where: 'desktop',
        steps: [
          'Under **Pair a device**, choose **Internet**.',
          'Press **Show pairing code**.',
          'Scan it with the phone (on mobile data, for example). The phone shows 4 digits.',
          'In the desktop app, press **Approve…** on the **Waiting for your approval** card.',
          'Type the 4 digits the phone shows.',
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
          'Point tunnels at port 6726, as the commands on the Remote page do. A tunnel pointed at 6725 instead keeps working after you switch this off.',
        ],
      },
      {
        heading: 'Manage paired devices',
        where: 'desktop',
        intro:
          '**Remote › Paired devices** lists every device with its platform, how and when it last connected, and its open connections.',
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
        heading: 'Warnings on the Remote page',
        terms: [
          {
            term: "ngrok's inspector is recording",
            text: 'Restart ngrok with `--inspect=false`, the way the Remote page shows it.',
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
    group: 'remote',
    summary:
      'Answer and message terminal, VS Code and JetBrains sessions with the plugin, or tmux.',
    intro:
      'Mother Claude shows every Claude Code session on your computer, whatever started it, and you can watch them right away. To answer and message the ones you started in a terminal, VS Code, JetBrains or Claude Desktop, install foreign-session control.',
    keywords: [
      'foreign',
      'plugin',
      'hooks',
      'vs code',
      'jetbrains',
      'terminal',
      'tmux',
      'reload-plugins',
      'background job',
    ],
    sections: [
      {
        heading: 'Install foreign-session control',
        where: 'desktop',
        steps: [
          'Open **Remote**.',
          'Under **Foreign-session control**, press **Install foreign-session control**.',
          'Wait until **Status** says **installed**.',
          'In each session that was already running, type `/reload-plugins` (or restart it).',
        ],
        points: [
          "It installs the Mother Claude plugin with Claude Code's own `claude plugin install`. Your `settings.json` and your own hooks are never touched.",
          'New sessions report in by themselves. The Remote page counts **Sessions reporting**, and under **Need a reload** the ones started before the install.',
          'With Mother Claude closed, your sessions behave exactly as before.',
        ],
        notes: [
          "The button is greyed out when Claude Code's `claude` command is not found. Install Claude Code, or start the app from a shell where `claude` works.",
        ],
        links: [{ label: 'Open Foreign-session control', path: '/remote', fragment: 'foreign' }],
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
          'Without the plugin, approvals there are answered with keys from the terminal view (**Send a key** on the **Info** tab).',
          '`/compact`, `/clear`, `/context`, `/usage`, `/cost`, `/status`, `/todos`, `/model`, `/effort`, `/rename` and `/btw` work from any device. Other commands, keys and a leading `!` need a device allowed dangerous actions.',
        ],
      },
      {
        heading: 'Background jobs',
        points: [
          'Jobs started with `claude --bg` can be stopped, respawned and removed from their **⋯** menu or the **Info** tab.',
          '**Terminal view** shows what the job prints.',
          'Typing into a job is experimental and needs a device allowed dangerous actions.',
        ],
      },
      {
        heading: 'Take a session over',
        intro: 'For full control of any session, use **Continue here**.',
        points: [
          'A session still running elsewhere is forked into a copy you drive here; the original keeps running.',
          'An ended session resumes in place.',
        ],
        links: [{ label: 'See Continue here', path: '/help/messaging', fragment: 'continue-here' }],
      },
      {
        heading: 'Uninstall',
        where: 'desktop',
        steps: [
          'Open **Remote › Foreign-session control**.',
          'Press **Uninstall**.',
          'Press **Tap again to uninstall**.',
        ],
        points: [
          'The hooks that running sessions already loaded go quiet first; then Claude Code removes the plugin.',
        ],
      },
    ],
  },

  // ──────────────────────────────────────────────────── Accounts & data
  {
    id: 'accounts',
    title: 'Claude accounts',
    icon: 'user',
    group: 'data',
    summary: 'Run sessions under several Claude accounts, and switch when one hits its limit.',
    intro:
      'Mother Claude can run sessions under several Claude accounts at once. When one reaches its limit, carry on under another. Accounts are under **Settings › Accounts**; while only the Default exists, the app shows no account chips or pickers.',
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
          "Open the session's **⋯** menu.",
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
          "Open the session's **⋯** menu.",
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
          "Each account's plan usage shows on its row on **Accounts**. A token account's usage shows once a session has run under it.",
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
          'A phone can rename accounts, set the defaults, allow or revoke copies, move data, pick the account of a new session and continue or switch a session.',
          'Adding, signing in, tokens and removing are done in the desktop app.',
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
          "Start from **Move or copy data…** on an account's row on **Accounts**, from **Move or copy…** in a session's **⋯** menu, or from **Select** on the Sessions list.",
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
          "Choose **Delete…** in a session's **⋯** menu, on a project's menu, or press **Select** and then **Delete N…**.",
          'Check **What goes** and **Skipped**.',
          'Leave **Keep a copy for 7 days so this can be undone** ticked.',
          'Press the button that names the delete, such as **Delete 2 conversations**.',
        ],
        notes: [
          'Deleting always needs the desktop app, or a device allowed dangerous actions. A conversation that is running is skipped.',
        ],
      },
      {
        heading: 'Undo',
        steps: [
          'Find the job: moves and copies under **Accounts › Moves and copies**, deletes under **Sessions › Projects › Recently deleted**.',
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
    title: 'Backup and sync',
    icon: 'cloud',
    group: 'data',
    summary:
      'Encrypted backups in a folder you choose, and your conversations on a second computer.',
    intro:
      "Mother Claude keeps an encrypted copy of each account's conversations in a folder you choose (iCloud Drive, Google Drive, OneDrive, a disk). Only your passphrase opens it. The same folder carries conversations to another computer. Nothing syncs on its own: you back up and pull, or set a schedule.",
    keywords: [
      'backup',
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
          'Open **Backup & sync** (in the sidebar; on a phone, under **More**).',
          'Step 1: press one of the suggested folders (iCloud Drive, Google Drive…), or **Type a folder…**, type its full path in **Backup folder** and press **Save**.',
          'Step 2: press **Set a passphrase…** and follow the dialog (below).',
          'Step 3: press **Back up now**.',
        ],
        tips: [
          'iCloud Drive: right-click the folder and choose **Keep Downloaded**, so pulls and restores never wait. OneDrive: **Always keep on this device**. Google Drive: use a folder inside My Drive.',
          'Later, the folder, the passphrase and the schedule are under **Settings** on the same page. To stop backing up, empty the folder and save.',
        ],
        links: [{ label: 'Open Backup & sync', path: '/backup' }],
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
          'Check the result under **Recent**.',
        ],
        points: [
          'A scheduled backup waits until no session of that account has been active for ten minutes. A running session is skipped.',
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
          'Under **Recent**, find the job.',
          'Press **Undo**.',
          'Press **Tap again to undo**.',
        ],
        points: ['Undo is possible for seven days. A backup itself has nothing to undo.'],
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
          'Pick the **Other machine**.',
          'Fill in **Its folder**.',
          'Press **Add pair**.',
        ],
        tips: [
          "A machine shows here once it has backed up to or pulled from these backups; **Look again** checks. A pull's report offers **Pair a folder…** with the fields filled in.",
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
          'Force actions and folder trust need a device allowed dangerous actions (**Remote › Paired devices** in the desktop app).',
        ],
      },
    ],
  },

  // ──────────────────────────────────────────────────── Browser & teams
  {
    id: 'browser',
    title: 'The browser extension',
    icon: 'globe',
    group: 'browser-teams',
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
          'In Mother Claude, open **Browser** (on a phone, **More › Browser**).',
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
          "Sessions use the browser through Mother Claude's tools, which are on by default (**Services › Claude Code tools (MCP)**).",
          'Sessions started outside Mother Claude also need foreign-session control.',
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
          'When a session ends, or after 10 minutes without a browser action, its tabs leave the group and stay open.',
        ],
      },
      {
        heading: 'Approvals',
        points: [
          "Requests appear in the Inbox, on the session's page, under **Browser › Waiting for you** and in the side panel. The first answer wins; no answer within 5 minutes counts as no.",
          'Always asked: a sensitive site, forms with password, card or one-time-code fields, running a script, uploading files, going over the per-minute limit, and any action in the minute after a Stop.',
          '**Always…** on a card saves a standing grant. Grants are listed under **Always allowed**, where **Remove** takes one back.',
        ],
      },
      {
        heading: 'Stop and Resume',
        steps: ['In the side panel, press **Stop** on the session.'],
        points: [
          'Its running command fails, its approvals are withdrawn, and its browser actions are refused for 60 seconds. Closing its tab group does the same.',
          '**Resume** lifts it early, from the side panel or from **Browser › Sessions using the browser** (which has **Stop** too).',
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
          "Uploads come only from the session's project folder or a folder you added under **Browser › Settings**, never from Claude Code's folders, `~/.ssh` or files that usually hold secrets.",
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
    group: 'browser-teams',
    summary: 'A lead and role sessions working through a task board in one project.',
    intro:
      "A team applies a template to a project: a lead session writes tasks on the project's board, and role sessions do them, each in its own files. Mother Claude starts, wakes and stops the sessions it runs, as capacity allows.",
    keywords: ['team', 'role', 'lead', 'board', 'task', 'template', 'worktree', 'workflow'],
    sections: [
      {
        heading: 'Start a team',
        steps: [
          'Open **Teams** and press **Start a team**.',
          'Choose the **Project**.',
          'Choose a **Template**.',
          'With several accounts, choose the **Account**.',
          'Optional: write a **Brief for the lead (optional)**.',
          "Under **Roles**, change a role's **Model**, **Effort**, **Permission mode** or **Start with the team** if you like.",
          'Press **Start the team**.',
        ],
        points: [
          'Built-in templates: Software delivery, Content production, Research & analysis, Browser operations and Documentation.',
          'One team per project. Roles with their own worktree need the project to be a git repository.',
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
          'A role that has to wait shows **Waiting** with the reasons. See Capacity.',
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
        heading: 'A session started elsewhere as a role',
        steps: ['On a role that is not running, open **Take role…**.', 'Pick the session.'],
        points: [
          'It needs foreign-session control and must run in the project. Mother Claude tells it about its tasks but never starts or stops it.',
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
          '**New template** writes your own; built-in ones are fixed, so press **View** and then **Duplicate to edit** to change one.',
          'Your own templates have **Edit** (JSON, with **Check** and **Save**), **Duplicate** and delete.',
        ],
      },
    ],
  },
  {
    id: 'capacity',
    title: 'Capacity, routing and budgets',
    icon: 'gauge',
    group: 'browser-teams',
    summary: 'Why a role session waits, your limits, budgets, model choice and costs.',
    intro:
      "Capacity shows how many role sessions run and why others wait, with memory, each account's plan usage, budgets, the routing policy and reports. Money figures are estimates.",
    keywords: [
      'limit',
      'budget',
      'cost',
      'routing',
      'model',
      'haiku',
      'classifier',
      'report',
      'waiting',
    ],
    sections: [
      {
        heading: 'Why a role session waits',
        intro:
          'The **Now** tab shows the role sessions that run and the ones that wait, each with its reasons in plain words. A role waits when:',
        points: [
          "Your limit of role sessions is reached (the lead does not count), or the project's.",
          'Not enough memory is free.',
          "The account's 5-hour window is nearly used up: only the lead starts at 85 %, nothing at 95 %.",
          'Claude refused a request on a rate limit: starts wait until the reset.',
          "The project's daily budget is spent.",
        ],
        tips: ['A role waiting longer than 10 minutes sends a push notification.'],
        links: [{ label: 'Open Capacity', path: '/capacity' }],
      },
      {
        heading: 'Set your limits',
        steps: [
          'Open the **Settings** tab and press **Edit**.',
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
          'Open the **Budgets** tab and press **Edit budgets**.',
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
          'Open the **Routing** tab and press **Edit the policy**.',
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
        heading: 'Reports',
        points: [
          'The **Reports** tab shows costs and outcomes for **7 days**, **30 days** or **90 days**. **CSV** and **Every task** download them.',
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

  // ────────────────────────────────────────────────────────── Advanced
  {
    id: 'claude-code-tools',
    title: 'Claude Code tools',
    icon: 'zap',
    group: 'advanced',
    summary: "Mother Claude's own tools for your sessions: the browser, the task board and more.",
    intro:
      'Mother Claude gives every Claude Code session on your computer its own tools: the browser, the team task board, teams, and Mother Claude itself. Sessions see them as `mcp__plugin_mother-claude_mc__<tool>`.',
    keywords: ['mcp', 'tools', 'server', 'services', 'plugin', 'reload-plugins'],
    sections: [
      {
        heading: 'Turn them on or off',
        where: 'desktop',
        steps: [
          'Open **Settings** and, under **About**, choose **Services (daemon, MCP servers)**.',
          "Under **Claude Code tools (MCP)**, switch **Give sessions Mother Claude's tools**.",
        ],
        points: [
          'They are on by default. Sessions started from then on get them; running ones after `/reload-plugins` or a restart.',
          'Off: sessions get no tools, and calls from running ones are refused.',
        ],
        links: [{ label: 'Open Services', path: '/services' }],
      },
      {
        heading: 'Which sessions have them',
        points: [
          'Sessions Mother Claude starts get them by themselves.',
          'Terminal, VS Code and JetBrains sessions get them through the Mother Claude plugin: install foreign-session control.',
          'Conversations started through the HTTP API do not get them yet.',
          'The panel shows **Sessions connected (last 15 min)**, whether **Sessions Mother Claude starts** get the tools, the plugin, and **Last calls**.',
        ],
        links: [{ label: 'Open Foreign-session control', path: '/remote', fragment: 'foreign' }],
      },
      {
        heading: 'Approvals',
        points: [
          'In the default permission mode Claude Code asks before using one of these tools. A session Mother Claude runs asks with a card; a session started elsewhere asks in its own window.',
          'Anything dangerous a tool would do becomes an approval card first.',
        ],
      },
      {
        heading: 'The Services page',
        terms: [
          {
            term: 'Daemon',
            text: "Claude Code's background service: reachable or not, and its status.",
          },
          { term: 'MCP servers', text: 'The MCP servers configured for Claude Code, by name.' },
          { term: 'Background jobs (daemon)', text: 'Jobs started with `claude --bg`.' },
        ],
        points: ['**Refresh** reads it all again.'],
      },
    ],
  },
  {
    id: 'api',
    title: 'The HTTP API',
    icon: 'code',
    group: 'advanced',
    summary: 'Drive Claude on this computer from scripts and tools, over HTTP.',
    intro:
      'Mother Claude can run a local HTTP API that drives Claude on this computer with your account: conversations, turns, live events and tool approvals. A conversation started through it is a normal session in the dashboard, and its approvals appear as cards.',
    keywords: ['bridge', 'http', 'rest', 'curl', 'script', 'token', 'swagger', 'port', 'server'],
    sections: [
      {
        heading: 'Start the API',
        steps: [
          'Open **API** (on a phone, **More › HTTP API**).',
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
        links: [{ label: 'Open API', path: '/api' }],
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
          'The **Authentication** line on the API screen says what is needed right now.',
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
            text: 'No tunnel was found. Start one of the commands on the Remote page, or type its address under **Or enter it yourself**.',
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
            term: 'Started before foreign control was installed',
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
        heading: 'Foreign-session control',
        terms: [
          {
            term: '"Install foreign-session control" is greyed out',
            text: "The `claude` command wasn't found. Install Claude Code, or start the app from a shell where `claude` works.",
          },
          {
            term: 'No Mother Claude tools in a terminal session',
            text: 'Install foreign-session control for that account, then type `/reload-plugins`.',
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
            text: "Install the hooks plugin (Accounts offers **Install hooks plugin**). The session's account shows after its first tool call.",
          },
          {
            term: "Can't remove an account",
            text: 'Sessions Mother Claude started or the API still use it. Stop them, or start the API with another account.',
          },
        ],
      },
      {
        heading: 'Backup and sync',
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
          { term: '`bridge_not_started`', text: 'Open **API** and press **Start the API**.' },
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
            text: "Turn **Give sessions Mother Claude's tools** back on under **Services**.",
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
