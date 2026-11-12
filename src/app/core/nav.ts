import type { IconName } from '../shared/icon.component';

/**
 * The app's map (docs/UI-REDESIGN.md §10.1): every place the navigation
 * opens, with its route, name, icon, badge, Help topic and the routes that
 * belong to it. The desktop sidebar, the phone's bottom bar and its More
 * list, and the phone's top-bar title all read it; nothing else names the
 * places. A stage of the redesign moves routes and this map together.
 *
 * Labels and hints are English, translated where they are shown
 * (`tr(entry.label)`).
 */

/** What an entry's badge shows: the requests waiting (`pending`, a count),
 * the agents running (`agents`, a count), or Machines' health (`machines`,
 * a red dot when a required check fails). */
export type NavBadge = 'pending' | 'agents' | 'machines';

/** One page the navigation opens. */
export interface NavEntry {
  /** A stable name for it (specs, lookups). */
  id: string;
  /** Its route, as `routerLink` takes it. */
  path: string;
  /** Its name (English). */
  label: string;
  icon: IconName;
  badge?: NavBadge;
  /** Only the desktop app lists it. */
  desktopOnly?: boolean;
  /** The Help topic that explains it (`/help/<topic>`). */
  help?: string;
  /**
   * The first segments of the routes that belong to it: while one shows,
   * it is the current entry, and it names the phone's top bar (unless the
   * page has a title of its own, {@link PAGE_TITLES}).
   */
  sections: readonly string[];
}

/** A tab of a place (`mc-hub-tabs`): one of its pages. */
export interface NavTab {
  /** Its route, as `routerLink` takes it. */
  path: string;
  /** Its name (English). */
  label: string;
  badge?: NavBadge;
  /** Current only on exactly its path (a place's own page, when other tabs
   * live under it). */
  exact?: boolean;
}

/** An entry of the desktop sidebar and of the phone's navigation. */
export interface NavPlace extends NavEntry {
  /** The sidebar heading it sits under (none: the work itself). */
  group?: string;
  /** Where a phone shows it: a tab of the bottom bar, or a row of More. */
  phone: 'bar' | 'more';
  /** The line under it in More (English). */
  hint?: string;
  /** Its name in More, when that differs (English). */
  moreLabel?: string;
  /** Left out of the sidebar (only More lists it). */
  noSidebar?: boolean;
  /** Its pages, as tabs under its header (none: one page). */
  tabs?: readonly NavTab[];
}

/** The places, in the sidebar's order. */
export const PLACES: readonly NavPlace[] = [
  {
    id: 'sessions',
    path: '/',
    label: 'Sessions',
    icon: 'sessions',
    phone: 'bar',
    help: 'sessions',
    sections: [''],
  },
  {
    id: 'inbox',
    path: '/inbox',
    label: 'Inbox',
    icon: 'inbox',
    badge: 'pending',
    phone: 'bar',
    help: 'approvals',
    sections: ['inbox'],
  },
  {
    id: 'agents',
    path: '/agents',
    label: 'Agents',
    icon: 'agents',
    badge: 'agents',
    phone: 'bar',
    help: 'sessions',
    sections: ['agents'],
  },
  {
    id: 'new',
    path: '/new',
    label: 'New session',
    icon: 'plus',
    phone: 'more',
    hint: 'Start an owned Claude Code session',
    help: 'new-session',
    sections: ['new'],
  },
  {
    id: 'teams',
    path: '/teams',
    label: 'Teams',
    icon: 'layers',
    phone: 'more',
    hint: 'A lead and role sessions on a board',
    help: 'teams',
    sections: ['teams'],
  },
  {
    id: 'accounts',
    path: '/accounts',
    label: 'Accounts',
    icon: 'user',
    group: 'Your data',
    phone: 'more',
    hint: 'Several Claude accounts, sign-in, moving conversations',
    help: 'accounts',
    sections: ['accounts'],
  },
  {
    id: 'backup',
    path: '/backup',
    label: 'Backup & sync',
    icon: 'cloud',
    group: 'Your data',
    phone: 'more',
    hint: 'Back up, restore and carry conversations to another computer',
    help: 'backup',
    sections: ['backup'],
  },
  {
    id: 'machines',
    path: '/machines',
    label: 'Machines',
    icon: 'monitor',
    badge: 'machines',
    group: 'Connect',
    phone: 'more',
    hint: 'Your other computers: their sessions, pairing, health',
    help: 'machines-setup',
    sections: ['machines'],
  },
  {
    id: 'remote',
    path: '/remote',
    label: 'Remote',
    icon: 'phone',
    desktopOnly: true,
    group: 'Connect',
    phone: 'more',
    hint: 'Pair devices, internet access, foreign sessions',
    help: 'phone',
    sections: ['remote'],
  },
  {
    id: 'browser',
    path: '/browser',
    label: 'Browser',
    icon: 'globe',
    group: 'Connect',
    phone: 'more',
    hint: 'Sessions in your browser: site modes, approvals',
    help: 'browser',
    sections: ['browser'],
  },
  {
    id: 'api',
    path: '/api',
    label: 'API',
    moreLabel: 'HTTP API',
    icon: 'code',
    group: 'Connect',
    phone: 'more',
    hint: 'Drive Claude from scripts and tools',
    help: 'api',
    sections: ['api'],
  },
  {
    id: 'capacity',
    path: '/capacity',
    label: 'Capacity',
    icon: 'gauge',
    group: 'System',
    phone: 'more',
    hint: 'Role sessions, why they wait, budgets, routing and costs',
    help: 'capacity',
    sections: ['capacity'],
  },
  {
    id: 'settings',
    path: '/settings',
    label: 'Settings',
    icon: 'settings',
    group: 'System',
    phone: 'more',
    hint: 'Notifications, theme, plan usage, connection',
    help: 'settings',
    sections: ['settings'],
  },
  {
    id: 'services',
    path: '/services',
    label: 'Services',
    icon: 'activity',
    noSidebar: true,
    phone: 'more',
    hint: 'Daemon and MCP servers',
    help: 'claude-code-tools',
    sections: ['services'],
  },
];

/** Help: the sidebar's foot, and More's last row. */
export const HELP_ENTRY: NavPlace = {
  id: 'help',
  path: '/help',
  label: 'Help',
  icon: 'question',
  phone: 'more',
  hint: 'How to use every feature, step by step',
  sections: ['help'],
};

/** The phone's last tab: everything the bar has no room for. Its own
 * sections are the pages only More leads to. */
export const MORE_ENTRY: NavEntry = {
  id: 'more',
  path: '/more',
  label: 'More',
  icon: 'more',
  sections: ['more', 'onboarding'],
};

/** More's rows, in their order. */
const MORE_ORDER: readonly string[] = [
  'new',
  'teams',
  'accounts',
  'backup',
  'machines',
  'remote',
  'settings',
  'api',
  'capacity',
  'browser',
  'services',
  'help',
];

/** Titles of pages that are no place of their own (English). */
export const PAGE_TITLES: Readonly<Record<string, string>> = {
  more: 'More',
  onboarding: 'Welcome',
  pair: 'Pair this device',
  session: 'Session',
};

/**
 * Routes no entry of the map opens: reached by a link or a button on a
 * page, a redirect, or from outside the app (a QR code, a push
 * notification). `app.routes.ts` holds nothing else.
 */
export const LINK_ONLY: readonly string[] = [
  'session/:id',
  'session/:id/agent/:agentId',
  'session/:id/on/:machine',
  'session/:id/agent/:agentId/on/:machine',
  'machines/setup',
  'pair',
  'teams/:id',
  'help/:topic',
  'good-to-know',
  'onboarding',
  '**',
];

/** The first segment of a URL's path (`/teams/tm_1?x#y` → `teams`). */
export function sectionOf(url: string): string {
  const path = (url || '/').split(/[?#]/)[0].replace(/^\//, '');
  return path.split('/')[0] ?? '';
}

/** The sidebar, in order: what this client may open. */
export function sidebarPlaces(desktop: boolean): NavPlace[] {
  return PLACES.filter((p) => !p.noSidebar && (!p.desktopOnly || desktop));
}

/** The tabs of the phone's bottom bar before More. */
export function barPlaces(): NavPlace[] {
  return PLACES.filter((p) => p.phone === 'bar');
}

/** More's rows, in order: what this client may open. */
export function moreEntries(desktop: boolean): NavPlace[] {
  const all = [...PLACES, HELP_ENTRY];
  return MORE_ORDER.map((id) => all.find((p) => p.id === id)).filter(
    (p): p is NavPlace => !!p && (!p.desktopOnly || desktop),
  );
}

/** The place a section belongs to, if any. */
export function placeOf(section: string): NavPlace | undefined {
  return [...PLACES, HELP_ENTRY].find((p) => p.sections.includes(section));
}

/** The phone's More tab is the current one: its own pages, or a row of
 * More's. */
export function moreActive(section: string): boolean {
  return (
    MORE_ENTRY.sections.includes(section) ||
    [...PLACES, HELP_ENTRY].some((p) => p.phone === 'more' && p.sections.includes(section))
  );
}

/** The phone's top-bar title of a section (English). */
export function titleOf(section: string): string {
  return PAGE_TITLES[section] ?? placeOf(section)?.label ?? 'Mother Claude';
}

/** A place's tabs (none for a place of one page, or an unknown id). */
export function tabsOf(placeId: string): readonly NavTab[] {
  return PLACES.find((p) => p.id === placeId)?.tabs ?? [];
}

/** Every route the map's entries open (without the leading `/`). */
export function mapPaths(): string[] {
  const entries = [...PLACES, HELP_ENTRY, MORE_ENTRY].map((e) => e.path);
  const tabs = PLACES.flatMap((p) => (p.tabs ?? []).map((t) => t.path));
  return [...new Set([...entries, ...tabs])].map((path) => path.replace(/^\//, ''));
}
