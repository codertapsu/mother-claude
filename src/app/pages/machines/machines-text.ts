// Text the Machines screens share. The setup's checks come from the server
// in English (`src-tauri/src/machines/setup.rs`); the fixed ones are
// translated here by matching (docs/I18N.md, "Server text drawn from a fixed
// list"), and anything else — a detail with a name or a number in it — is
// shown as it came. Mirrors `setup.rs`: change both together.

import { computerNoun, tr } from '../../core/i18n/i18n';
import {
  CodeApply,
  CodeSettings,
  CodeTransfer,
  GrantName,
  GrantPair,
  MachineAsk,
  MachinePath,
  PairingView,
  Revocation,
  SetupCheck,
  SyncChanges,
  SyncPair,
  defaultCode,
  osLabel,
  pathLabel,
} from '../../core/machines';

const FIXED: Record<string, () => string> = {
  // Titles
  'Mother Claude': () => 'Mother Claude',
  'Stays awake while used': () => tr('Stays awake while used'),
  'Paired computers': () => tr('Paired computers'),
  'The relay': () => tr('The relay'),
  'Local Network access': () => tr('Local Network access'),
  'The firewall': () => tr('The firewall'),
  'The network type': () => tr('The network type'),
  // Why it matters
  'Every computer needs a version that speaks the same Machines protocol.': () =>
    tr('Every computer needs a version that speaks the same Machines protocol.'),
  'A computer asleep cannot answer the others.': () =>
    tr('A computer asleep cannot answer the others.'),
  'Machines works between computers you paired.': () =>
    tr('Machines works between computers you paired.'),
  'Each pair of computers needs the same Machines protocol and relay setting.': () =>
    tr('Each pair of computers needs the same Machines protocol and relay setting.'),
  'Across networks, your computers meet through the relay.': () =>
    tr('Across networks, your computers meet through the relay.'),
  'Without it this Mac reaches your other computers only through the relay, even at home.': () =>
    tr('Without it this Mac reaches your other computers only through the relay, even at home.'),
  'If it blocks Mother Claude, your other computers reach this Mac only through the relay.': () =>
    tr('If it blocks Mother Claude, your other computers reach this Mac only through the relay.'),
  'On a Public network, Windows blocks incoming connections.': () =>
    tr('On a Public network, Windows blocks incoming connections.'),
  'Without a rule, your other computers reach this PC only through the relay.': () =>
    tr('Without a rule, your other computers reach this PC only through the relay.'),
  // Fixes
  'Keep awake while used': () => tr('Keep awake while used'),
  'Pair a machine': () => tr('Pair a machine'),
  'Use this setting everywhere': () => tr('Use this setting everywhere'),
  'Check the relay': () => tr('Check the relay'),
  'Open Local Network settings': () => tr('Open Local Network settings'),
  'Open Firewall settings': () => tr('Open Firewall settings'),
  'Open network settings': () => tr('Open network settings'),
  'Allow through the firewall': () => tr('Allow through the firewall'),
  // Details that never carry a value
  'No other computer is paired yet.': () => tr('No other computer is paired yet.'),
  'The firewall is off.': () => tr('The firewall is off.'),
  'The firewall blocks all incoming connections.': () =>
    tr('The firewall blocks all incoming connections.'),
  'The firewall blocks Mother Claude.': () => tr('The firewall blocks Mother Claude.'),
  'The firewall lets Mother Claude in.': () => tr('The firewall lets Mother Claude in.'),
  'Mother Claude may reach your local network.': () =>
    tr('Mother Claude may reach your local network.'),
  'macOS does not let Mother Claude reach your local network. Allow it when macOS asks, or switch it on in System Settings › Privacy & Security › Local Network.':
    () =>
      tr(
        'macOS does not let Mother Claude reach your local network. Allow it when macOS asks, or switch it on in System Settings › Privacy & Security › Local Network.',
      ),
  'No network to test on right now.': () => tr('No network to test on right now.'),
  'Could not test the local network.': () => tr('Could not test the local network.'),
  "Could not read the firewall's state.": () => tr("Could not read the firewall's state."),
  'Could not read the network type.': () => tr('Could not read the network type.'),
  'Could not read the firewall.': () => tr('Could not read the firewall.'),
  'This network is Public, but its firewall is off.': () =>
    tr('This network is Public, but its firewall is off.'),
  'This network is Public: Windows blocks incoming connections on it. Set it to Private if it is your home or office network.':
    () =>
      tr(
        'This network is Public: Windows blocks incoming connections on it. Set it to Private if it is your home or office network.',
      ),
  'Not connected to a network right now.': () => tr('Not connected to a network right now.'),
  'The firewall is off for this network.': () => tr('The firewall is off for this network.'),
  'A firewall rule blocks Mother Claude (Windows makes one when its first prompt is cancelled).':
    () =>
      tr(
        'A firewall rule blocks Mother Claude (Windows makes one when its first prompt is cancelled).',
      ),
  'A firewall rule lets Mother Claude in.': () => tr('A firewall rule lets Mother Claude in.'),
  'No firewall rule lets your other computers reach Mother Claude.': () =>
    tr('No firewall rule lets your other computers reach Mother Claude.'),
  'No relay: your computers meet on the same network only.': () =>
    tr('No relay: your computers meet on the same network only.'),
  'Checked once Machines is in use.': () => tr('Checked once Machines is in use.'),
  "The relay does not answer: on the same network your computers still meet, across networks they can't.":
    () =>
      tr(
        "The relay does not answer: on the same network your computers still meet, across networks they can't.",
      ),
  'Online, direct on the same network.': () => tr('Online, direct on the same network.'),
  'Online, direct over the internet.': () => tr('Online, direct over the internet.'),
  'Online, through the relay.': () => tr('Online, through the relay.'),
  'Online.': () => tr('Online.'),
  // Phase 3a: git, long paths, Claude Code, accounts, free space.
  'Long paths': () => tr('Long paths'),
  'Free space': () => tr('Free space'),
  "A session's code moves through git, on both computers.": () =>
    tr("A session's code moves through git, on both computers."),
  'Windows and git leave out paths longer than 260 characters unless long paths are on.': () =>
    tr('Windows and git leave out paths longer than 260 characters unless long paths are on.'),
  'Sessions run in Claude Code; one moved to an older version may not resume.': () =>
    tr('Sessions run in Claude Code; one moved to an older version may not resume.'),
  'A session moves to the account of the same name, and runs only where it is signed in.': () =>
    tr('A session moves to the account of the same name, and runs only where it is signed in.'),
  "Moves and copies land in Mother Claude's data folder first.": () =>
    tr("Moves and copies land in Mother Claude's data folder first."),
  'Install the command line tools': () => tr('Install the command line tools'),
  'Install Git for Windows': () => tr('Install Git for Windows'),
  'Install git': () => tr('Install git'),
  'Update git': () => tr('Update git'),
  'Turn on for git': () => tr('Turn on for git'),
  'Ask an administrator to turn on long paths': () =>
    tr('Ask an administrator to turn on long paths'),
  'Install Claude Code': () => tr('Install Claude Code'),
  'Update Claude Code': () => tr('Update Claude Code'),
  'Sign in…': () => tr('Sign in…'),
  'Free some space': () => tr('Free some space'),
  "git isn't installed: code can't move without it.": () =>
    tr("git isn't installed: code can't move without it."),
  'Could not run git.': () => tr('Could not run git.'),
  'Checked once git runs.': () => tr('Checked once git runs.'),
  'Git for Windows leaves out paths longer than 260 characters: core.longpaths is off for this user.':
    () =>
      tr(
        'Git for Windows leaves out paths longer than 260 characters: core.longpaths is off for this user.',
      ),
  "Windows' own long paths setting is off: some tools can't open paths longer than 260 characters.":
    () =>
      tr(
        "Windows' own long paths setting is off: some tools can't open paths longer than 260 characters.",
      ),
  'Long paths are on for git.': () => tr('Long paths are on for git.'),
  'Long paths are on, for Windows and for git.': () =>
    tr('Long paths are on, for Windows and for git.'),
  "Claude Code isn't installed here, or can't be started: sessions can't run.": () =>
    tr("Claude Code isn't installed here, or can't be started: sessions can't run."),
  'Signed in.': () => tr('Signed in.'),
  "Not signed in: its sessions can't run or move here until it is.": () =>
    tr("Not signed in: its sessions can't run or move here until it is."),
  'A session of it failed to sign in: sign in again.': () =>
    tr('A session of it failed to sign in: sign in again.'),
  "Its token isn't on this computer.": () => tr("Its token isn't on this computer."),
  'Could not check its sign-in.': () => tr('Could not check its sign-in.'),
  'Not checked yet.': () => tr('Not checked yet.'),
  'Could not read the free space.': () => tr('Could not read the free space.'),
  // What an ask became, and a remote's answer.
  'Fixed.': () => tr('Fixed.'),
  'The sign-in started: finish it in the browser on this computer.': () =>
    tr('The sign-in started: finish it in the browser on this computer.'),
  'Accounts opened to add it.': () => tr('Accounts opened to add it.'),
  'Not now.': () => tr('Not now.'),
  'Cloning…': () => tr('Cloning…'),
  'Turning it on…': () => tr('Turning it on…'),
  'That account is not on this computer any more.': () =>
    tr('That account is not on this computer any more.'),
  'The remote asked for a sign-in: sign in to it on this computer (Git Credential Manager, or gh auth login), then try again.':
    () =>
      tr(
        'The remote asked for a sign-in: sign in to it on this computer (Git Credential Manager, or gh auth login), then try again.',
      ),
  "The remote refused this computer's SSH key.": () =>
    tr("The remote refused this computer's SSH key."),
  "Can't reach the remote from this computer.": () =>
    tr("Can't reach the remote from this computer."),
  'The remote has no such repository, or this computer may not see it.': () =>
    tr('The remote has no such repository, or this computer may not see it.'),
  'git could not reach the remote.': () => tr('git could not reach the remote.'),
  "git isn't installed on this computer.": () => tr("git isn't installed on this computer."),
  'The remote answers without a prompt.': () => tr('The remote answers without a prompt.'),
  'This project has no remote named origin.': () => tr('This project has no remote named origin.'),
  'The remote did not answer in 30 seconds.': () => tr('The remote did not answer in 30 seconds.'),
  'The project is not on this computer: clone it there first.': () =>
    tr('The project is not on this computer: clone it there first.'),
  // Phase 4: Claude Code tools, for the machine tools.
  'Claude Code tools': () => tr('Claude Code tools'),
  "Sessions here run commands on your other computers through Mother Claude's tools (machine_run).":
    () =>
      tr(
        "Sessions here run commands on your other computers through Mother Claude's tools (machine_run).",
      ),
  'On: sessions here can use machine_run.': () => tr('On: sessions here can use machine_run.'),
  "Off: sessions here can't use machine_run or the other machine tools.": () =>
    tr("Off: sessions here can't use machine_run or the other machine tools."),
  'Turn on': () => tr('Turn on'),
  // The readiness test's other lines.
  'Skipped: not connected.': () => tr('Skipped: not connected.'),
  "git isn't installed here.": () => tr("git isn't installed here."),
  "Claude Code isn't installed here.": () => tr("Claude Code isn't installed here."),
  "Claude Code's version is not known.": () => tr("Claude Code's version is not known."),
  // Phase 5: an older Mother Claude, starting at sign-in, introductions.
  'Update Mother Claude': () => tr('Update Mother Claude'),
  'Software updates opened here.': () => tr('Software updates opened here.'),
  'Starts when you sign in': () => tr('Starts when you sign in'),
  'After a restart, your other computers reach this one only once Mother Claude runs again.': () =>
    tr('After a restart, your other computers reach this one only once Mother Claude runs again.'),
  'Not available on this system.': () => tr('Not available on this system.'),
  'It starts when you sign in, in the tray.': () => tr('It starts when you sign in, in the tray.'),
  "It is turned off in the system's startup apps.": () =>
    tr("It is turned off in the system's startup apps."),
  "It doesn't start when you sign in.": () => tr("It doesn't start when you sign in."),
  'You declined.': () => tr('You declined.'),
  'The computer that introduced it is no longer paired.': () =>
    tr('The computer that introduced it is no longer paired.'),
  'It expired.': () => tr('It expired.'),
};

/** Server texts with a name or a number in them: the pattern, and what it
 * reads in the language shown. */
const PATTERNS: [RegExp, (m: RegExpExecArray) => string][] = [
  [/^Connected to (.+)\.$/, (m) => tr('Connected to {host}.', { host: m[1] })],
  // Phase 5 (`setup.rs` `app_check`, `intro.rs`).
  [
    /^Mother Claude (\S+) is older than (.+)'s (\S+): update it here\.$/,
    (m) =>
      tr("Mother Claude {v} is older than {name}'s {w}: update it here.", {
        v: m[1],
        name: m[2],
        w: m[3],
      }),
  ],
  [
    /^(.+) took the introduction back\.$/,
    (m) => tr('{name} took the introduction back.', { name: m[1] }),
  ],
  // The readiness test's `machine_run` echo, each way (`setup/readiness.rs`).
  [
    /^echo ran on (.+) in (.+)\.$/,
    (m) => tr('echo ran on {name} in {shell}.', { name: m[1], shell: m[2] }),
  ],
  [
    /^(.+) ran echo here in (.+)\.$/,
    (m) => tr('{name} ran echo here in {shell}.', { name: m[1], shell: m[2] }),
  ],
  [
    /^echo did not run on (.+?): (.+)$/s,
    (m) => tr('echo did not run on {name}: {why}', { name: m[1], why: m[2] }),
  ],
  [
    /^(.+?) could not run echo here: (.+)$/s,
    (m) => tr('{name} could not run echo here: {why}', { name: m[1], why: m[2] }),
  ],
  // What a request to keep a project in sync became (`sync/on.rs`).
  [/^Kept in sync with (.+)\.$/, (m) => tr('Kept in sync with {name}.', { name: m[1] })],
  [
    /^(.+) turned it off before it was confirmed\.$/,
    (m) => tr('{name} turned it off before it was confirmed.', { name: m[1] }),
  ],
  [
    /^On\. (.+) can't be reached right now: the first sync runs when it is back\.$/,
    (m) =>
      tr("On. {name} can't be reached right now: the first sync runs when it is back.", {
        name: m[1],
      }),
  ],
  [
    /^On\. (.+) differs on both computers: choose which one's to keep on the Machines page\.$/,
    (m) =>
      tr(
        "On. {project} differs on both computers: choose which one's to keep on the Machines page.",
        {
          project: m[1],
        },
      ),
  ],
  [
    /^On\. The first sync waits: (.+)$/,
    (m) => tr('On. The first sync waits: {why}', { why: oneText(m[1]) }),
  ],
  [
    /^git (\S+) is older than (\S+): update it\.$/,
    (m) => tr('git {v} is older than {min}: update it.', { v: m[1], min: m[2] }),
  ],
  [
    /^Claude Code (\S+) is older than (.+)'s (\S+): a session moved here from there may not resume\.$/,
    (m) =>
      tr(
        "Claude Code {v} is older than {name}'s {w}: a session moved here from there may not resume.",
        {
          v: m[1],
          name: m[2],
          w: m[3],
        },
      ),
  ],
  [
    /^Only (.+) free where Mother Claude keeps its data\.$/,
    (m) => tr('Only {size} free where Mother Claude keeps its data.', { size: m[1] }),
  ],
  [
    /^(.+) free where Mother Claude keeps its data\.$/,
    (m) => tr('{size} free where Mother Claude keeps its data.', { size: m[1] }),
  ],
  [
    /^(.+) moves code with other options: a move follows the options of the computer it starts from\.$/,
    (m) =>
      tr(
        '{name} moves code with other options: a move follows the options of the computer it starts from.',
        { name: m[1] },
      ),
  ],
  [/^Cloned into (.+)\.$/, (m) => tr('Cloned into {folder}.', { folder: m[1] })],
  [/^(.+) is there already\.$/, (m) => tr('{folder} is there already.', { folder: m[1] })],
  [
    /^No folder maps this project to (.+)\.$/,
    (m) => tr('No folder maps this project to {name}.', { name: m[1] }),
  ],
  [
    /^Update Mother Claude on (.+) to (check it there|compare its tools|test moving code|compare its accounts|compare its folders|see its accounts|see its projects|test running commands)\.$/,
    (m) => {
      const name = m[1];
      switch (m[2]) {
        case 'check it there':
          return tr('Update Mother Claude on {name} to check it there.', { name });
        case 'compare its tools':
          return tr('Update Mother Claude on {name} to compare its tools.', { name });
        case 'test moving code':
          return tr('Update Mother Claude on {name} to test moving code.', { name });
        case 'compare its accounts':
          return tr('Update Mother Claude on {name} to compare its accounts.', { name });
        case 'compare its folders':
          return tr('Update Mother Claude on {name} to compare its folders.', { name });
        case 'see its accounts':
          return tr('Update Mother Claude on {name} to see its accounts.', { name });
        case 'test running commands':
          return tr('Update Mother Claude on {name} to test running commands.', { name });
        default:
          return tr('Update Mother Claude on {name} to see its projects.', { name });
      }
    },
  ],
  // After the longer sentences that start the same way.
  [/^Update Mother Claude on (.+)$/, (m) => tr('Update Mother Claude on {name}', { name: m[1] })],
  [
    /^(.+) does not let this computer see it\.$/,
    (m) => tr('{name} does not let this computer see it.', { name: m[1] }),
  ],
  [/^git isn't installed on (.+)\.$/, (m) => tr("git isn't installed on {name}.", { name: m[1] })],
  [
    /^Claude Code isn't installed on (.+)\.$/,
    (m) => tr("Claude Code isn't installed on {name}.", { name: m[1] }),
  ],
  [
    /^git (\S+) here is older than (\S+)\.$/,
    (m) => tr('git {v} here is older than {min}.', { v: m[1], min: m[2] }),
  ],
  [
    /^git (\S+) on (.+) is older than (\S+)\.$/,
    (m) => tr('git {v} on {name} is older than {min}.', { v: m[1], name: m[2], min: m[3] }),
  ],
  [
    /^Claude Code on (.+) \((\S+)\) is older than here \((\S+)\)\.$/,
    (m) =>
      tr('Claude Code on {name} ({v}) is older than here ({w}).', { name: m[1], v: m[2], w: m[3] }),
  ],
  [
    /^Claude Code here \((\S+)\) is older than on (.+) \((\S+)\)\.$/,
    (m) =>
      tr('Claude Code here ({v}) is older than on {name} ({w}).', { v: m[1], name: m[2], w: m[3] }),
  ],
  [
    /^git and Claude Code (\S+) on both\.$/,
    (m) => tr('git and Claude Code {v} on both.', { v: m[1] }),
  ],
  [/^Both ways in ([\d.]+) s\.$/, (m) => tr('Both ways in {s} s.', { s: m[1] })],
  [
    /^1 account matches, signed in on (.+)\.$/,
    (m) => tr('1 account matches, signed in on {name}.', { name: m[1] }),
  ],
  [
    /^(\d+) accounts match, signed in on (.+)\.$/,
    (m) => tr('{n} accounts match, signed in on {name}.', { n: m[1], name: m[2] }),
  ],
  [/^(.+): not on (.+)$/, (m) => tr('{account}: not on {name}', { account: m[1], name: m[2] })],
  [
    /^(.+): not signed in on (.+)$/,
    (m) => tr('{account}: not signed in on {name}', { account: m[1], name: m[2] }),
  ],
  [
    /^(.+): not checked on (.+)$/,
    (m) => tr('{account}: not checked on {name}', { account: m[1], name: m[2] }),
  ],
  [/^1 folder resolves on (.+)$/, (m) => tr('1 folder resolves on {name}', { name: m[1] })],
  [
    /^(\d+) folders resolve on (.+)$/,
    (m) => tr('{n} folders resolve on {name}', { n: m[1], name: m[2] }),
  ],
  [/^(\d+) of (\d+) projects map\.$/, (m) => tr('{m} of {n} projects map.', { m: m[1], n: m[2] })],
  [
    /^(.+): its folder is missing on (.+)$/,
    (m) => tr('{root}: its folder is missing on {name}', { root: m[1], name: m[2] }),
  ],
  [
    /^(.+): not known on (.+) yet$/,
    (m) => tr('{root}: not known on {name} yet', { root: m[1], name: m[2] }),
  ],
  [
    /^(.+): mapped elsewhere on (.+)$/,
    (m) => tr('{project}: mapped elsewhere on {name}', { project: m[1], name: m[2] }),
  ],
  [
    /^1 project on both is not mapped yet \(step 5\)$/,
    () => tr('1 project on both is not mapped yet (step 5)'),
  ],
  [
    /^(\d+) projects on both are not mapped yet \(step 5\)$/,
    (m) => tr('{n} projects on both are not mapped yet (step 5)', { n: m[1] }),
  ],
  [
    /^No folder maps a project to (.+) yet: accept or add one in step 5\.$/,
    (m) =>
      tr('No folder maps a project to {name} yet: accept or add one in step 5.', { name: m[1] }),
  ],
  // Last: it matches any sentence ending so (an introduction declined
  // there, `intro.rs`).
  [/^(.+) declined\.$/, (m) => tr('{name} declined.', { name: m[1] })],
];

/** One server text (no ` · ` in it) in the language shown. */
function oneText(text: string): string {
  const known = FIXED[text];
  if (known) return known();
  const paired = /^Paired with (\d+) computers?\.$/.exec(text);
  if (paired) {
    const n = Number(paired[1]);
    return n === 1 ? tr('Paired with 1 computer.') : tr('Paired with {n} computers.', { n });
  }
  for (const [re, f] of PATTERNS) {
    const m = re.exec(text);
    if (m) return f(m);
  }
  return text;
}

/** A server text in the language shown when it is one of the fixed ones
 * (each part of a `·`-joined list on its own). */
export function serverText(text: string | null | undefined): string {
  if (!text) return '';
  if (FIXED[text]) return FIXED[text]();
  return text.split(' · ').map(oneText).join(' · ');
}

/** How code travels, in words. */
export function transferText(t: CodeTransfer | undefined): string {
  return t === 'remote' ? tr('Through your git remote') : tr('Machine to machine');
}

/** Where code lands, in words. */
export function applyText(a: CodeApply | undefined): string {
  return a === 'worktree' ? tr('Always a separate folder (worktree)') : tr('In the project folder');
}

/** The code options in one line. */
export function codeText(c: CodeSettings | null | undefined): string {
  const code = c ?? defaultCode();
  const parts = [transferText(code.transfer), applyText(code.apply)];
  if (code.transfer === 'remote' && code.keepRemoteRefs) parts.push(tr('hidden refs kept'));
  return parts.join(' · ');
}

/** How something asked of another computer went, in words. */
export function askText(a: MachineAsk, name: string): string {
  switch (a.state) {
    case 'asked':
      return tr('Asked — waiting for someone at {name}.', { name });
    case 'running':
      return tr('Under way on {name}…', { name });
    case 'done':
      return a.detail
        ? tr('Done on {name}: {what}', { name, what: serverText(a.detail) })
        : tr('Done on {name}.', { name });
    case 'declined':
      return tr('Not now (at {name}).', { name });
    case 'failed':
      return a.detail
        ? tr('It failed on {name}: {why}', { name, why: serverText(a.detail) })
        : tr('It failed on {name}.', { name });
  }
}

/** A check's title, detail, why and fix label in the language shown. */
export function checkTexts(c: SetupCheck): {
  title: string;
  detail: string;
  why: string;
  fix: string;
} {
  return {
    title: serverText(c.title),
    detail: serverText(c.detail),
    why: serverText(c.why),
    fix: serverText(c.fix?.label),
  };
}

/** A pairing's failure in words, by its code (the server's sentence for an
 * unknown one). */
export function pairErrorText(p: PairingView | null | undefined): string {
  const e = p?.error;
  if (!e) return '';
  const name = p?.other?.name || p?.intro?.other.name || tr('the other computer');
  switch (e.code) {
    // An introduced pairing (plan §5.12).
    case 'pair_declined':
      return tr('{name} declined. Nothing was paired.', { name });
    case 'intro_closed':
      return serverText(e.message);
    case 'pair_code_expired':
      return p?.role === 'host'
        ? tr('Expired — create a new code.')
        : tr('That code has expired. Make a new one on the other computer.');
    case 'pair_code_contested':
      return tr('This code was used by two computers, so neither can pair. Make a new code.');
    case 'pair_code_wrong':
      return tr(
        "That code doesn't match. Check it on the other computer, or make a new one there.",
      );
    case 'pair_not_found':
      return tr(
        'No computer showing this code was found. Check the code, and that both computers have the internet or are on the same network.',
      );
    case 'pair_unreachable':
      return tr("Can't reach {name}. The two computers need the internet, or the same network.", {
        name,
      });
    case 'pair_digits_wrong':
      return tr("That's not the number on {name}. Nothing was paired.", { name });
    case 'pair_timed_out':
      return p?.intro
        ? tr('The pairing was not confirmed in time. Ask for the introduction again.')
        : tr('The pairing was not confirmed in time. Start again.');
    case 'pair_replaced':
      return tr('A new pairing started.');
    default:
      return e.message;
  }
}

/** What a capability lets a computer do, in the language shown. */
export function capabilityText(cap: string): string {
  switch (cap) {
    case 'moves':
      return tr('move sessions');
    case 'ignoredFiles':
      return tr('take files git ignores with a move');
    case 'replaceBranch':
      return tr('replace a branch with a move');
    case 'roots':
      return tr('share folders');
    case 'sync':
      return tr('keep code in sync');
    case 'run':
      return tr('run commands for your other computers');
    case 'tasks':
      return tr('take tasks');
    case 'teamRoles':
      return tr('run a team’s roles');
    case 'introductions':
      return tr('be introduced to your other computers');
    case 'revokeEverywhere':
      return tr('take Revoke everywhere');
    default:
      return cap;
  }
}

/** Capabilities in a list: `move sessions, take tasks and share folders`. */
export function capabilityList(caps: readonly string[]): string {
  const words = caps.map(capabilityText);
  if (words.length <= 1) return words.join('');
  return tr('{list} and {last}', {
    list: words.slice(0, -1).join(', '),
    last: words[words.length - 1],
  });
}

/** Where a *Revoke everywhere* stands, in a sentence. */
export function revocationText(r: Revocation): string {
  if (!r.pending.length) {
    return r.done.length
      ? tr('Revoked here and on {names}.', { names: r.done.map((d) => d.name).join(', ') })
      : tr('Revoked here.');
  }
  const waiting = r.pending.map((p) =>
    p.tooOld
      ? tr('{name} (once it updates)', { name: p.name })
      : p.online
        ? p.name
        : tr('{name} (offline)', { name: p.name }),
  );
  return tr('Revoked here; still to be told: {names}.', { names: waiting.join(', ') });
}

/** `macOS`, `Windows`, `Linux`, or "Other" in the language shown. */
export function osName(os: string): string {
  const label = osLabel(os);
  return label === 'Other' ? tr('Other||computer') : label;
}

/** A path in the language shown. */
export function pathName(path: MachinePath | null | undefined): string {
  switch (pathLabel(path)) {
    case 'Direct (same network)':
      return tr('Direct (same network)');
    case 'Direct (internet)':
      return tr('Direct (internet)');
    case 'Through the relay (slower)':
      return tr('Through the relay (slower)');
    default:
      return '';
  }
}

/** A grant's row label in the matrix. */
export function grantRow(name: GrantName): string {
  switch (name) {
    case 'view':
      return tr('See sessions, transcripts, requests');
    case 'control':
      return tr('Drive them: send, answer, stop, model');
    case 'move':
      return tr('Move sessions here and away');
    case 'run':
      return tr('Run commands and tasks here');
    case 'dangerous':
      return tr('Dangerous actions');
  }
}

/** A grant's short name in a summary line. */
export function grantShort(name: GrantName): string {
  switch (name) {
    case 'view':
      return tr('See');
    case 'control':
      return tr('Drive');
    case 'move':
      return tr('Move');
    case 'run':
      return tr('Run');
    case 'dangerous':
      return tr('Dangerous');
  }
}

/** One grant's value in a summary: ✓ both ways, – neither, else where. */
export function grantValue(p: GrantPair): string {
  if (p.lan && p.internet) return '✓';
  if (!p.lan && !p.internet) return '–';
  return p.lan ? tr('same network only') : tr('internet only');
}

/** "This Mac", "This PC" as a heading: a capital first in every language
 * (Vietnamese puts the noun first, "máy Mac này"). */
export function thisComputerTitle(): string {
  const s = tr('This {computer}', { computer: computerNoun() });
  return s.charAt(0).toLocaleUpperCase() + s.slice(1);
}

// ─────────────────────────────────────────────────────────── Code sync

/** A synced project's state, in a word or two (its badge). */
export function syncBadge(p: SyncPair): string {
  const s = p.status;
  switch (s.state) {
    case 'inSync':
      return tr('In sync');
    case 'syncing':
      return tr('Syncing…');
    case 'waiting':
      return tr('Waiting for {name}', { name: p.machineName });
    case 'off':
      return tr('Off');
    case 'paused':
      return s.reason === 'conflict' ? tr('Changed on both') : tr('Paused');
  }
}

/** The badge's colour: `ok`, `warn` or `danger` (none: neutral). */
export function syncTone(p: SyncPair): '' | 'ok' | 'warn' | 'danger' {
  const s = p.status;
  if (s.state === 'inSync') return 'ok';
  if (s.state === 'paused') return s.reason === 'conflict' ? 'danger' : 'warn';
  return '';
}

/** What a pause, a wait or an off says, in a sentence. */
export function syncDetail(p: SyncPair): string {
  const s = p.status;
  const name = p.machineName;
  switch (s.state) {
    case 'waiting':
      return tr(
        '{name} shows the request to whoever is at it: nothing syncs until it is confirmed there.',
        {
          name,
        },
      );
    case 'off': {
      const d = s.detail ?? '';
      if (d.endsWith(' said not now.')) return tr('{name} said not now.', { name });
      if (d.startsWith('Turned off on ')) return tr('Turned off on {name}.', { name });
      return serverText(d);
    }
    case 'paused':
      switch (s.reason) {
        case 'conflict':
          return p.conflict?.first
            ? tr('The project differs on both computers: choose which one’s to keep.')
            : tr('Both computers changed it since it was last synced: choose which one’s to keep.');
        case 'offline':
          return tr('Waits until {name} is back online.', { name });
        case 'machinesOff':
          return tr('Waits until Machines is on on both computers.');
        case 'noGrant':
          return tr(
            'Waits: each computer must give the other the Move permission, as it writes into its project folder.',
          );
        case 'moveRunning':
          return tr('Waits while a session working in this project moves between computers.');
        case 'error':
          return tr('Waits: {why}', { why: serverText(s.detail) });
      }
      return '';
    default:
      return '';
  }
}

/** The last step, in a line. */
export function syncLastText(p: SyncPair): string {
  const l = p.last;
  if (!l) return '';
  const files = l.files === 1 ? tr('1 file') : tr('{n} files', { n: l.files });
  switch (l.direction) {
    case 'out':
      return tr('Last: {files} from this {computer} to {name}', {
        files,
        computer: computerNoun(),
        name: p.machineName,
      });
    case 'in':
      return tr('Last: {files} from {name}', { files, name: p.machineName });
    case 'both':
      return tr('Last: the same changes on both');
  }
}

/** One side of a conflict, in a line. */
export function syncChangesText(c: SyncChanges): string {
  const parts: string[] = [];
  if (c.files)
    parts.push(c.files === 1 ? tr('1 file changed') : tr('{n} files changed', { n: c.files }));
  if (c.branchSwitched && c.branch) parts.push(tr('now on {branch}', { branch: c.branch }));
  else if (c.headMoved) parts.push(tr('new commits'));
  return parts.join(' · ') || tr('no change to its files');
}
