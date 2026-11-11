// Text the Machines screens share. The setup's checks come from the server
// in English (`src-tauri/src/machines/setup.rs`); the fixed ones are
// translated here by matching (docs/I18N.md, "Server text drawn from a fixed
// list"), and anything else — a detail with a name or a number in it — is
// shown as it came. Mirrors `setup.rs`: change both together.

import { computerNoun, tr } from '../../core/i18n/i18n';
import {
  GrantName,
  GrantPair,
  MachinePath,
  PairingView,
  SetupCheck,
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
};

/** A server text in the language shown when it is one of the fixed ones. */
export function serverText(text: string | null | undefined): string {
  if (!text) return '';
  const known = FIXED[text];
  if (known) return known();
  const update = /^Update Mother Claude on (.+)$/.exec(text);
  if (update) return tr('Update Mother Claude on {name}', { name: update[1] });
  const paired = /^Paired with (\d+) computers?\.$/.exec(text);
  if (paired) {
    const n = Number(paired[1]);
    return n === 1 ? tr('Paired with 1 computer.') : tr('Paired with {n} computers.', { n });
  }
  const connected = /^Connected to (.+)\.$/.exec(text);
  if (connected) return tr('Connected to {host}.', { host: connected[1] });
  return text;
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
  const name = p?.other?.name || tr('the other computer');
  switch (e.code) {
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
      return tr('The pairing was not confirmed in time. Start again.');
    case 'pair_replaced':
      return tr('A new pairing started.');
    default:
      return e.message;
  }
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
