import type { IconName } from '../shared/icon.component';

/**
 * The app's map (docs/UI-REDESIGN.md §6.1, §10.1, §11): the places —
 * Sessions, Inbox and Teams for the work; Computers, Phone & devices,
 * Sessions started elsewhere, Browser, HTTP API, Accounts, Backups and
 * Settings for what you look after — with their tabs, Settings' own pages,
 * and Help. The desktop sidebar, the phone's bottom bar and its More list,
 * the phone's top-bar title and a place's tabs (`mc-hub-tabs`) all read
 * it; nothing else names the places. Routes and this map move together;
 * an old route redirects to where its page is now ({@link REDIRECTED}).
 *
 * One map for the desktop and the phone: the sidebar lists the work, a
 * line, then what you look after; the phone's bar shows the work and
 * *More* the rest, in the same order, then Help.
 *
 * Labels are English, translated where they are shown (`tr(entry.label)`).
 */

/** What an entry's badge shows: the requests waiting (`pending`, a count),
 * the agents running (`agents`, a count), your computers' health
 * (`machines`, a red dot when a required check fails), or the projects
 * kept in sync between computers (`sync`, a count). */
export type NavBadge = 'pending' | 'agents' | 'machines' | 'sync';

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

/** A page of Settings that a row of Settings opens, saying how it stands. */
export interface NavSettingsPage {
  id: 'tools';
  /** Its route, as `routerLink` takes it. */
  path: string;
  /** Its name (English). */
  label: string;
  icon: IconName;
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
    sections: ['', 'agents', 'projects', 'new', 'session'],
    tabs: [
      { path: '/', label: 'Sessions', exact: true },
      { path: '/agents', label: 'Agents', badge: 'agents' },
      { path: '/projects', label: 'Projects' },
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
    // A team's own page (`/teams/:id`) too.
    sections: ['teams'],
    tabs: [
      { path: '/teams', label: 'Teams', exact: true },
      { path: '/teams/templates', label: 'Templates' },
      { path: '/teams/capacity', label: 'Capacity' },
    ],
  },
  {
    id: 'computers',
    path: '/computers',
    label: 'Computers',
    icon: 'monitor',
    badge: 'machines',
    group: 'care',
    help: 'machines-setup',
    // Its setup (`/computers/setup`) too.
    sections: ['computers'],
    tabs: [
      { path: '/computers', label: 'Overview', exact: true },
      { path: '/computers/sync', label: 'Code sync', badge: 'sync' },
      { path: '/computers/activity', label: 'Activity' },
      { path: '/computers/options', label: 'Options' },
    ],
  },
  // A phone reads it: itself, and the devices paired (read only).
  {
    id: 'devices',
    path: '/devices',
    label: 'Phone & devices',
    icon: 'phone',
    group: 'care',
    help: 'phone',
    sections: ['devices'],
  },
  {
    id: 'elsewhere',
    path: '/elsewhere',
    label: 'Sessions started elsewhere',
    icon: 'plug',
    desktopOnly: true,
    group: 'care',
    help: 'foreign-sessions',
    sections: ['elsewhere'],
  },
  {
    id: 'browser',
    path: '/browser',
    label: 'Browser',
    icon: 'globe',
    group: 'care',
    help: 'browser',
    sections: ['browser'],
  },
  // `?server=` opens one of the API servers.
  {
    id: 'http-api',
    path: '/http-api',
    label: 'HTTP API',
    icon: 'code',
    group: 'care',
    help: 'api',
    sections: ['http-api'],
  },
  {
    id: 'accounts',
    path: '/accounts',
    label: 'Accounts',
    icon: 'user',
    group: 'care',
    help: 'accounts',
    sections: ['accounts'],
    tabs: [
      { path: '/accounts', label: 'Accounts', exact: true },
      { path: '/accounts/usage', label: 'Usage' },
      { path: '/accounts/changes', label: 'Recent changes' },
    ],
  },
  {
    id: 'backups',
    path: '/backups',
    label: 'Backups',
    icon: 'cloud',
    group: 'care',
    help: 'backup',
    sections: ['backups'],
  },
  {
    id: 'settings',
    path: '/settings',
    label: 'Settings',
    icon: 'settings',
    group: 'care',
    help: 'settings',
    // Its own pages (`/settings/…`), and the first run's macOS permissions.
    sections: ['settings', 'onboarding'],
  },
];

/**
 * Settings' own pages, each a row of Settings (docs/UI-REDESIGN.md §6.7).
 * Phone & devices, Sessions started elsewhere, Browser and HTTP API were
 * its *Connections* too until they became places (§11, 2026-11-14).
 */
export const SETTINGS_PAGES: readonly NavSettingsPage[] = [
  {
    id: 'tools',
    path: '/settings/tools',
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
};

/**
 * Routes no entry of the map opens: reached by a link or a button on a
 * page, or from outside the app (a QR code, a push notification).
 * `app.routes.ts` holds nothing else but the old routes ({@link
 * REDIRECTED}).
 */
export const LINK_ONLY: readonly string[] = [
  'new',
  'session/:id',
  'session/:id/agent/:agentId',
  'session/:id/on/:machine',
  'session/:id/agent/:agentId/on/:machine',
  'computers/setup',
  'pair',
  'teams/:id',
  'help/:topic',
  'onboarding',
  '**',
];

/**
 * Old routes that redirect to where their page is now (docs/UI-REDESIGN.md
 * §10.1, §10.3, §11), with the rest of the path, the query and the
 * fragment — links from before the redesign keep working. Redirects stay
 * for good.
 */
export const REDIRECTED: readonly string[] = [
  'good-to-know',
  'machines',
  'backup',
  'remote',
  'api',
  'services',
  'capacity',
  // Settings' connections and Accounts' Backups tab, until 2026-11-14.
  'settings/devices',
  'settings/elsewhere',
  'settings/browser',
  'settings/api',
  'accounts/backups',
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
    ...SETTINGS_PAGES.map((p) => p.path),
  ];
  return [...new Set(paths)].map((path) => path.replace(/^\//, ''));
}
