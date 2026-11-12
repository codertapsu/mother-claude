import { routes } from '../app.routes';
import { helpTopic } from '../pages/help/help-topics';
import {
  CONNECTIONS,
  HELP_ENTRY,
  LINK_ONLY,
  MORE_ENTRY,
  PLACES,
  REDIRECTED,
  barPlaces,
  connections,
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
  const routePaths = routes.map((r) => r.path ?? '');

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
    for (const e of [...PLACES, HELP_ENTRY, ...CONNECTIONS]) {
      if (e.help) expect(helpTopic(e.help), `${e.id}: ${e.help}`).toBeDefined();
    }
  });

  it("keeps a place's tabs and connections inside it", () => {
    for (const p of PLACES) {
      for (const t of tabsOf(p.id)) {
        expect(p.sections, `${p.id}: ${t.path}`).toContain(sectionOf(t.path));
      }
    }
    const settings = PLACES.find((p) => p.id === 'settings')!;
    for (const c of CONNECTIONS) {
      expect(settings.sections, c.id).toContain(sectionOf(c.path));
    }
    expect(tabsOf('no-such-place')).toEqual([]);
  });

  it('has six places: the work, then what you look after', () => {
    for (const desktop of [true, false]) {
      expect(sidebarPlaces(desktop).map((p) => p.label)).toEqual([
        'Sessions',
        'Inbox',
        'Teams',
        'Computers',
        'Accounts',
        'Settings',
      ]);
    }
    expect(PLACES.map((p) => p.group)).toEqual(['work', 'work', 'work', 'care', 'care', 'care']);
    // Computers keeps Machines' health dot; Agents is a tab of Sessions.
    expect(PLACES.find((p) => p.id === 'computers')).toEqual(
      expect.objectContaining({ path: '/computers', badge: 'machines' }),
    );
  });

  it('gives each place its tabs', () => {
    expect(tabsOf('sessions').map((t) => [t.label, t.path])).toEqual([
      ['Sessions', '/'],
      ['Agents', '/agents'],
    ]);
    expect(tabsOf('sessions')[1].badge).toBe('agents');
    expect(tabsOf('teams').map((t) => [t.label, t.path])).toEqual([
      ['Teams', '/teams'],
      ['Capacity', '/capacity'],
    ]);
    expect(tabsOf('accounts').map((t) => [t.label, t.path])).toEqual([
      ['Accounts', '/accounts'],
      ['Backups', '/backup'],
    ]);
    // Computers in four tabs (docs/UI-REDESIGN.md §6.5), Code sync counting.
    expect(tabsOf('computers').map((t) => [t.label, t.path])).toEqual([
      ['Overview', '/computers'],
      ['Code sync', '/computers/sync'],
      ['Activity', '/computers/activity'],
      ['Options', '/computers/options'],
    ]);
    expect(tabsOf('computers')[0].exact).toBe(true);
    expect(tabsOf('computers')[1].badge).toBe('sync');
    for (const id of ['inbox', 'settings']) expect(tabsOf(id), id).toEqual([]);
  });

  it("draws the phone's bar as the work, and More as the rest in the same order", () => {
    expect(barPlaces().map((p) => p.label)).toEqual(['Sessions', 'Inbox', 'Teams']);
    for (const desktop of [true, false]) {
      expect(moreEntries(desktop).map((p) => p.label)).toEqual([
        'Computers',
        'Accounts',
        'Settings',
        'Help',
      ]);
    }
  });

  it("lists Settings' connections, the desktop's own only there", () => {
    expect(connections(true).map((c) => [c.label, c.path, c.fragment ?? ''])).toEqual([
      ['Phone & devices', '/remote', ''],
      ['Sessions started elsewhere', '/remote', 'foreign'],
      ['Browser', '/browser', ''],
      ['HTTP API', '/api', ''],
      ['Claude Code tools', '/services', ''],
    ]);
    expect(connections(false).map((c) => c.id)).toEqual(['browser', 'api', 'tools']);
  });

  it("titles the phone's top bar by the place, or the page's own name", () => {
    expect(titleOf('')).toBe('Sessions');
    expect(titleOf('agents')).toBe('Sessions');
    expect(titleOf('capacity')).toBe('Teams');
    expect(titleOf('teams')).toBe('Teams');
    expect(titleOf('backup')).toBe('Accounts');
    expect(titleOf('computers')).toBe('Computers');
    expect(titleOf('settings')).toBe('Settings');
    expect(titleOf('remote')).toBe('Remote');
    expect(titleOf('api')).toBe('API');
    expect(titleOf('services')).toBe('Services');
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
      'accounts',
      'backup',
      'settings',
      'remote',
      'browser',
      'api',
      'services',
      'onboarding',
      'help',
    ]) {
      expect(moreActive(s), s).toBe(true);
    }
    for (const s of ['', 'inbox', 'agents', 'teams', 'capacity', 'new', 'session', 'pair']) {
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
