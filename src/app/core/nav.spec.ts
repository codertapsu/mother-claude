import { routePaths as pathsOf, routes } from '../app.routes';
import { helpTopic } from '../pages/help/help-topics';
import {
  HELP_ENTRY,
  LINK_ONLY,
  MORE_ENTRY,
  PLACES,
  REDIRECTED,
  SETTINGS_PAGES,
  barPlaces,
  mapPaths,
  moreActive,
  moreEntries,
  placeOf,
  sectionOf,
  sidebarPlaces,
  tabsOf,
  titleOf,
} from './nav';

describe('the map (core/nav.ts)', () => {
  // A layout's pages under it: `session/:id` is the work layout's.
  const routePaths = pathsOf();

  it('holds every route: opened by an entry, reached only by link, or an old one', () => {
    const inMap = new Set(mapPaths());
    for (const path of routePaths) {
      expect(
        inMap.has(path) || LINK_ONLY.includes(path) || REDIRECTED.includes(path),
        `route "${path}"`,
      ).toBe(true);
    }
  });

  it('names only routes that exist, the old ones redirecting', () => {
    for (const path of [...mapPaths(), ...LINK_ONLY, ...REDIRECTED]) {
      expect(routePaths, `"${path}"`).toContain(path);
    }
    for (const path of mapPaths()) {
      expect(LINK_ONLY, `"${path}" is in the map: not reached only by link`).not.toContain(path);
    }
    for (const path of REDIRECTED) {
      const route = routes.find((r) => r.path === path);
      expect(route?.redirectTo, `"${path}" redirects`).toBeDefined();
    }
  });

  it('gives each entry its own section, and each section one place', () => {
    const all = [...PLACES, HELP_ENTRY];
    for (const e of all) {
      expect(e.sections, e.id).toContain(sectionOf(e.path));
    }
    const sections = all.flatMap((e) => e.sections);
    expect(new Set(sections).size).toBe(sections.length);
    for (const s of MORE_ENTRY.sections) {
      expect(placeOf(s), s).toBeUndefined();
    }
    const ids = all.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('points every Help topic at one that exists', () => {
    for (const e of [...PLACES, HELP_ENTRY, ...SETTINGS_PAGES]) {
      if (e.help) expect(helpTopic(e.help), `${e.id}: ${e.help}`).toBeDefined();
    }
  });

  it("keeps a place's tabs and Settings' own pages inside it", () => {
    for (const p of PLACES) {
      for (const t of tabsOf(p.id)) {
        expect(p.sections, `${p.id}: ${t.path}`).toContain(sectionOf(t.path));
      }
    }
    const settings = PLACES.find((p) => p.id === 'settings')!;
    for (const page of SETTINGS_PAGES) {
      expect(settings.sections, page.id).toContain(sectionOf(page.path));
    }
    expect(tabsOf('no-such-place')).toEqual([]);
  });

  it('has the work, then what you look after, each connection a place of its own', () => {
    expect(sidebarPlaces(true).map((p) => [p.label, p.path])).toEqual([
      ['Sessions', '/'],
      ['Inbox', '/inbox'],
      ['Teams', '/teams'],
      ['Computers', '/computers'],
      ['Phone & devices', '/devices'],
      ['Sessions started elsewhere', '/elsewhere'],
      ['Browser', '/browser'],
      ['HTTP API', '/http-api'],
      ['Accounts', '/accounts'],
      ['Backups', '/backups'],
      ['Settings', '/settings'],
    ]);
    // The plugin is installed from the desktop: a phone's sidebar leaves it
    // out (its page still reads, for a link).
    expect(sidebarPlaces(false).map((p) => p.id)).toEqual([
      'sessions',
      'inbox',
      'teams',
      'computers',
      'devices',
      'browser',
      'http-api',
      'accounts',
      'backups',
      'settings',
    ]);
    // Computers keeps Machines' health dot; Agents is a tab of Sessions.
    expect(PLACES.find((p) => p.id === 'computers')).toEqual(
      expect.objectContaining({ path: '/computers', badge: 'machines' }),
    );
    // Each keeps the icon it had as a connection; Backups its Help's cloud.
    expect(PLACES.map((p) => [p.id, p.icon])).toEqual(
      expect.arrayContaining([
        ['devices', 'phone'],
        ['elsewhere', 'plug'],
        ['browser', 'globe'],
        ['http-api', 'code'],
        ['backups', 'cloud'],
      ]),
    );
  });

  it('gives each place its tabs', () => {
    // Sessions: the list, the agents, the projects on disk (§6.2).
    expect(tabsOf('sessions').map((t) => [t.label, t.path])).toEqual([
      ['Sessions', '/'],
      ['Agents', '/agents'],
      ['Projects', '/projects'],
    ]);
    expect(tabsOf('sessions')[0].exact).toBe(true);
    expect(tabsOf('sessions')[1].badge).toBe('agents');
    // Teams: the teams, their templates, Capacity (§6.2).
    expect(tabsOf('teams').map((t) => [t.label, t.path])).toEqual([
      ['Teams', '/teams'],
      ['Templates', '/teams/templates'],
      ['Capacity', '/teams/capacity'],
    ]);
    expect(tabsOf('teams')[0].exact).toBe(true);
    // Accounts in three tabs (docs/UI-REDESIGN.md §6.6): Backups is a place.
    expect(tabsOf('accounts').map((t) => [t.label, t.path])).toEqual([
      ['Accounts', '/accounts'],
      ['Usage', '/accounts/usage'],
      ['Recent changes', '/accounts/changes'],
    ]);
    expect(tabsOf('accounts')[0].exact).toBe(true);
    // Computers in four tabs (docs/UI-REDESIGN.md §6.5), Code sync counting.
    expect(tabsOf('computers').map((t) => [t.label, t.path])).toEqual([
      ['Overview', '/computers'],
      ['Code sync', '/computers/sync'],
      ['Activity', '/computers/activity'],
      ['Options', '/computers/options'],
    ]);
    expect(tabsOf('computers')[0].exact).toBe(true);
    expect(tabsOf('computers')[1].badge).toBe('sync');
    for (const id of [
      'inbox',
      'devices',
      'elsewhere',
      'browser',
      'http-api',
      'backups',
      'settings',
    ]) {
      expect(tabsOf(id), id).toEqual([]);
    }
  });

  it("draws the phone's bar as the work, and More as the rest in the same order", () => {
    expect(barPlaces().map((p) => p.label)).toEqual(['Sessions', 'Inbox', 'Teams']);
    expect(moreEntries(true).map((p) => p.label)).toEqual([
      'Computers',
      'Phone & devices',
      'Sessions started elsewhere',
      'Browser',
      'HTTP API',
      'Accounts',
      'Backups',
      'Settings',
      'Help',
    ]);
    expect(moreEntries(false).map((p) => p.id)).toEqual([
      'computers',
      'devices',
      'browser',
      'http-api',
      'accounts',
      'backups',
      'settings',
      'help',
    ]);
  });

  it("lists Settings' own pages: Claude Code tools", () => {
    expect(SETTINGS_PAGES.map((p) => [p.label, p.path])).toEqual([
      ['Claude Code tools', '/settings/tools'],
    ]);
  });

  it("titles the phone's top bar by the place, or the page's own name", () => {
    expect(titleOf('')).toBe('Sessions');
    expect(titleOf('agents')).toBe('Sessions');
    expect(titleOf('projects')).toBe('Sessions');
    // Templates, Capacity and a team are pages of Teams.
    expect(titleOf('teams')).toBe('Teams');
    expect(titleOf('accounts')).toBe('Accounts');
    expect(titleOf('computers')).toBe('Computers');
    // Settings' own pages (Claude Code tools) are Settings'.
    expect(titleOf('settings')).toBe('Settings');
    // What used to be Settings' connections, and Backups, name themselves.
    expect(titleOf('devices')).toBe('Phone & devices');
    expect(titleOf('elsewhere')).toBe('Sessions started elsewhere');
    expect(titleOf('browser')).toBe('Browser');
    expect(titleOf('http-api')).toBe('HTTP API');
    expect(titleOf('backups')).toBe('Backups');
    expect(titleOf('new')).toBe('New session');
    expect(titleOf('help')).toBe('Help');
    expect(titleOf('session')).toBe('Session');
    expect(titleOf('pair')).toBe('Pair this device');
    expect(titleOf('onboarding')).toBe('Welcome');
    expect(titleOf('more')).toBe('More');
    expect(titleOf('nowhere')).toBe('Mother Claude');
  });

  it("marks the phone's More tab on the pages of what it lists", () => {
    for (const s of [
      'more',
      'computers',
      'devices',
      'elsewhere',
      'browser',
      'http-api',
      'accounts',
      'backups',
      'settings',
      'onboarding',
      'help',
    ]) {
      expect(moreActive(s), s).toBe(true);
    }
    for (const s of ['', 'inbox', 'agents', 'projects', 'teams', 'new', 'session', 'pair']) {
      expect(moreActive(s), s).toBe(false);
    }
  });

  it('reads the section of a URL', () => {
    expect(sectionOf('/')).toBe('');
    expect(sectionOf('/?machine=abc')).toBe('');
    expect(sectionOf('/teams/tm_1?x=1#y')).toBe('teams');
    expect(sectionOf('/remote#foreign')).toBe('remote');
    expect(sectionOf('')).toBe('');
  });
});
