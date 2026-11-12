import type { IconName } from '../shared/icon.component';

/**
 * The app's map (docs/UI-REDESIGN.md §6.1, §10.1): six places — Sessions,
 * Inbox and Teams for the work, Computers, Accounts and Settings for what
 * you look after — with their tabs, Settings' connections, and Help. The
 * desktop sidebar, the phone's bottom bar and its More list, the phone's
 * top-bar title and a place's tabs (`mc-hub-tabs`) all read it; nothing
 * else names the places. A stage of the redesign moves routes and this map
 * together: for now every tab and connection opens today's route.
 *
 * One map for the desktop and the phone: the sidebar lists the work, a
 * line, then what you look after; the phone's bar shows the work and
 * *More* the rest, in the same order, then Help.
 *
 * Labels are English, translated where they are shown (`tr(entry.label)`).
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

/** One of Settings' *Connections*: what else works with Mother Claude,
 * each on a page of its own. */
export interface NavConnection {
  id: 'devices' | 'elsewhere' | 'browser' | 'api' | 'tools';
  /** Its route, as `routerLink` takes it. */
  path: string;
  /** The part of the page it opens at. */
  fragment?: string;
  /** Its name (English). */
  label: string;
  icon: IconName;
  /** Only the desktop app lists it. */
  desktopOnly?: boolean;
  /** The Help topic that explains it. */
  help: string;
}

/** The sidebar's two groups, a line between them: the work, and what you
 * look after. The phone's bar shows the first, More the second. */
export type NavGroup = 'work' | 'care';

/** A place of the sidebar and of the phone's navigation. */
export interface NavPlace extends NavEntry {
  group: NavGroup;
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
    group: 'work',
    help: 'sessions',
    // `new`: the home's primary button; `session`: one of them.
    sections: ['', 'agents', 'new', 'session'],
    tabs: [
      { path: '/', label: 'Sessions', exact: true },
      { path: '/agents', label: 'Agents', badge: 'agents' },
    ],
  },
  {
    id: 'inbox',
    path: '/inbox',
    label: 'Inbox',
    icon: 'inbox',
    badge: 'pending',
    group: 'work',
    help: 'approvals',
    sections: ['inbox'],
  },
  {
    id: 'teams',
    path: '/teams',
    label: 'Teams',
    icon: 'layers',
    group: 'work',
    help: 'teams',
    sections: ['teams', 'capacity'],
    tabs: [
      { path: '/teams', label: 'Teams' },
      { path: '/capacity', label: 'Capacity' },
    ],
  },
  {
    id: 'computers',
    path: '/machines',
    label: 'Computers',
    icon: 'monitor',
    badge: 'machines',
    group: 'care',
    help: 'machines-setup',
    sections: ['machines'],
  },
  {
    id: 'accounts',
    path: '/accounts',
    label: 'Accounts',
    icon: 'user',
    group: 'care',
    help: 'accounts',
    sections: ['accounts', 'backup'],
    tabs: [
      { path: '/accounts', label: 'Accounts' },
      { path: '/backup', label: 'Backups' },
    ],
  },
  {
    id: 'settings',
    path: '/settings',
    label: 'Settings',
    icon: 'settings',
    group: 'care',
    help: 'settings',
    // Its connections' pages, and the first run's macOS permissions.
    sections: ['settings', 'remote', 'browser', 'api', 'services', 'onboarding'],
  },
];

/** Settings' *Connections*, in their order (docs/UI-REDESIGN.md §6.7). */
export const CONNECTIONS: readonly NavConnection[] = [
  {
    id: 'devices',
    path: '/remote',
    label: 'Phone & devices',
    icon: 'phone',
    desktopOnly: true,
    help: 'phone',
  },
  {
    id: 'elsewhere',
    path: '/remote',
    fragment: 'foreign',
    label: 'Sessions started elsewhere',
    icon: 'plug',
    desktopOnly: true,
    help: 'foreign-sessions',
  },
  { id: 'browser', path: '/browser', label: 'Browser', icon: 'globe', help: 'browser' },
  { id: 'api', path: '/api', label: 'HTTP API', icon: 'code', help: 'api' },
  {
    id: 'tools',
    path: '/services',
    label: 'Claude Code tools',
    icon: 'activity',
    help: 'claude-code-tools',
  },
];

/** Help: the sidebar's foot, and More's last row. */
export const HELP_ENTRY: NavEntry = {
  id: 'help',
  path: '/help',
  label: 'Help',
  icon: 'question',
  sections: ['help'],
};

/** The phone's last tab: what the bar has no room for. */
export const MORE_ENTRY: NavEntry = {
  id: 'more',
  path: '/more',
  label: 'More',
  icon: 'more',
  sections: ['more'],
};

/** Titles of pages that are no place, or not the place's own (English):
 * what the phone's top bar says there. */
export const PAGE_TITLES: Readonly<Record<string, string>> = {
  more: 'More',
  pair: 'Pair this device',
  session: 'Session',
  new: 'New session',
  onboarding: 'Welcome',
  remote: 'Remote',
  browser: 'Browser',
  api: 'API',
  services: 'Services',
};

/**
 * Routes no entry of the map opens: reached by a link or a button on a
 * page, a redirect, or from outside the app (a QR code, a push
 * notification). `app.routes.ts` holds nothing else.
 */
export const LINK_ONLY: readonly string[] = [
  'new',
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
  return PLACES.filter((p) => !p.desktopOnly || desktop);
}

/** The tabs of the phone's bottom bar before More: the work. */
export function barPlaces(): NavPlace[] {
  return PLACES.filter((p) => p.group === 'work');
}

/** More's rows, in order: what you look after, then Help. */
export function moreEntries(desktop: boolean): NavEntry[] {
  return [...PLACES.filter((p) => p.group === 'care' && (!p.desktopOnly || desktop)), HELP_ENTRY];
}

/** Settings' connections this client may open. */
export function connections(desktop: boolean): NavConnection[] {
  return CONNECTIONS.filter((c) => !c.desktopOnly || desktop);
}

/** The place (or Help) a section belongs to, if any. */
export function placeOf(section: string): NavEntry | undefined {
  return [...PLACES, HELP_ENTRY].find((p) => p.sections.includes(section));
}

/** The phone's More tab is the current one: More itself, or a page of
 * what it lists. */
export function moreActive(section: string): boolean {
  return [MORE_ENTRY, ...moreEntries(true)].some((e) => e.sections.includes(section));
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
  const paths = [
    ...[...PLACES, HELP_ENTRY, MORE_ENTRY].map((e) => e.path),
    ...PLACES.flatMap((p) => (p.tabs ?? []).map((t) => t.path)),
    ...CONNECTIONS.map((c) => c.path),
  ];
  return [...new Set(paths)].map((path) => path.replace(/^\//, ''));
}
