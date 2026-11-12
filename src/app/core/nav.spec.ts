import { routes } from '../app.routes';
import { helpTopic } from '../pages/help/help-topics';
import {
  HELP_ENTRY,
  LINK_ONLY,
  MORE_ENTRY,
  PLACES,
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
  const routePaths = routes.map((r) => r.path ?? '');

  it('holds every route: opened by an entry, or listed as reached only by link', () => {
    const inMap = new Set(mapPaths());
    for (const path of routePaths) {
      expect(inMap.has(path) || LINK_ONLY.includes(path), `route "${path}"`).toBe(true);
    }
  });

  it('names only routes that exist', () => {
    for (const path of [...mapPaths(), ...LINK_ONLY]) {
      expect(routePaths, `"${path}"`).toContain(path);
    }
    for (const path of mapPaths()) {
      expect(LINK_ONLY, `"${path}" is in the map: not reached only by link`).not.toContain(path);
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
    for (const e of [...PLACES, HELP_ENTRY]) {
      if (e.help) expect(helpTopic(e.help), `${e.id}: ${e.help}`).toBeDefined();
    }
  });

  it("keeps a tab's route inside its place", () => {
    for (const p of PLACES) {
      for (const t of tabsOf(p.id)) {
        expect(p.sections, `${p.id}: ${t.path}`).toContain(sectionOf(t.path));
      }
    }
    expect(tabsOf('no-such-place')).toEqual([]);
  });

  it('draws the desktop sidebar as today: the work, then Your data, Connect and System', () => {
    const side = sidebarPlaces(true);
    expect(side.map((p) => p.label)).toEqual([
      'Sessions',
      'Inbox',
      'Agents',
      'New session',
      'Teams',
      'Accounts',
      'Backup & sync',
      'Machines',
      'Remote',
      'Browser',
      'API',
      'Capacity',
      'Settings',
    ]);
    expect(side.map((p) => p.group ?? '')).toEqual([
      '',
      '',
      '',
      '',
      '',
      'Your data',
      'Your data',
      'Connect',
      'Connect',
      'Connect',
      'Connect',
      'System',
      'System',
    ]);
    // Remote is the desktop's only.
    expect(sidebarPlaces(false).map((p) => p.id)).not.toContain('remote');
  });

  it("draws the phone's bar and More as today", () => {
    expect(barPlaces().map((p) => p.label)).toEqual(['Sessions', 'Inbox', 'Agents']);
    expect(moreEntries(true).map((p) => p.moreLabel ?? p.label)).toEqual([
      'New session',
      'Teams',
      'Accounts',
      'Backup & sync',
      'Machines',
      'Remote',
      'Settings',
      'HTTP API',
      'Capacity',
      'Browser',
      'Services',
      'Help',
    ]);
    expect(moreEntries(false).map((p) => p.id)).not.toContain('remote');
    for (const e of moreEntries(true)) expect(e.hint, e.id).toBeTruthy();
  });

  it("titles the phone's top bar", () => {
    expect(titleOf('')).toBe('Sessions');
    expect(titleOf('backup')).toBe('Backup & sync');
    expect(titleOf('api')).toBe('API');
    expect(titleOf('services')).toBe('Services');
    expect(titleOf('help')).toBe('Help');
    expect(titleOf('session')).toBe('Session');
    expect(titleOf('pair')).toBe('Pair this device');
    expect(titleOf('onboarding')).toBe('Welcome');
    expect(titleOf('more')).toBe('More');
    expect(titleOf('nowhere')).toBe('Mother Claude');
  });

  it("marks the phone's More tab on the pages only More leads to", () => {
    for (const s of ['more', 'onboarding', 'machines', 'teams', 'capacity', 'help', 'services']) {
      expect(moreActive(s), s).toBe(true);
    }
    for (const s of ['', 'inbox', 'agents', 'session', 'pair']) {
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
