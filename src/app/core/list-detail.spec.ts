import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  LIST_PATHS,
  ListDetailService,
  WIDE_QUERY,
  besideList,
  isWorkList,
  listSection,
} from './list-detail';
import { PLACES, placeOf } from './nav';

describe('list and detail (docs/UI-REDESIGN.md §9, stage 9)', () => {
  let ld: ListDetailService;

  beforeEach(() => {
    TestBed.configureTestingModule({ providers: [provideRouter([])] });
    ld = TestBed.inject(ListDetailService);
  });

  it('starts at 1280 px', () => {
    expect(WIDE_QUERY).toBe('(min-width: 1280px)');
  });

  it('knows the lists, where they live, and the place each belongs to', () => {
    expect(Object.keys(LIST_PATHS)).toEqual(['sessions', 'inbox', 'agents', 'projects']);
    for (const list of ['sessions', 'inbox', 'agents', 'projects']) {
      expect(isWorkList(list), list).toBe(true);
    }
    expect(isWorkList('teams')).toBe(false);
    expect(isWorkList(undefined)).toBe(false);
    // The sidebar marks the list's place beside a session: the Inbox's, or
    // Sessions' for its tabs.
    expect(placeOf(listSection('inbox'))?.id).toBe('inbox');
    for (const list of ['sessions', 'agents', 'projects'] as const) {
      expect(placeOf(listSection(list))?.id, list).toBe('sessions');
    }
    // Each list is a page of the map.
    const paths = PLACES.flatMap((p) => [p.path, ...(p.tabs ?? []).map((t) => t.path)]);
    for (const path of Object.values(LIST_PATHS)) expect(paths).toContain(path);
  });

  it('shows the Sessions list beside a session opened from Projects', () => {
    expect(besideList('projects')).toBe('sessions');
    expect(besideList('inbox')).toBe('inbox');
    expect(besideList('agents')).toBe('agents');
  });

  it('splits only on a wide screen, while a session shows', () => {
    ld.wide.set(false);
    ld.showDetail({ id: 's1', machine: null, agent: null }, false);
    expect(ld.split()).toBe(false);
    expect(ld.isOpen('s1')).toBe(false);
    ld.wide.set(true);
    expect(ld.split()).toBe(true);
    expect(ld.isOpen('s1')).toBe(true);
    expect(ld.isOpen('s1', 'm1')).toBe(false);
    ld.showList('sessions', '/');
    expect(ld.split()).toBe(false);
    expect(ld.detail()).toBeNull();
  });

  it('remembers the list and its address, query and all', () => {
    ld.showList('inbox', '/inbox');
    ld.showList('sessions', '/?machine=m1');
    ld.showDetail({ id: 's1', machine: null, agent: null }, true);
    // A list was shown: a notification's session opens beside it.
    expect(ld.list()).toBe('sessions');
    expect(ld.closeUrl()).toBe('/?machine=m1');
    ld.showList('projects', '/projects');
    ld.showDetail({ id: 's1', machine: null, agent: null }, false);
    expect(ld.beside()).toBe('sessions');
    expect(ld.closeUrl()).toBe('/');
  });

  it('puts the Inbox beside a session a notification opened before any list', () => {
    ld.showDetail({ id: 's1', machine: null, agent: null }, true);
    expect(ld.list()).toBe('inbox');
    expect(ld.closeUrl()).toBe('/inbox');
  });

  it('opens a link without a request beside the Sessions list', () => {
    ld.showDetail({ id: 's1', machine: 'm1', agent: null }, false);
    expect(ld.list()).toBe('sessions');
    expect(ld.closeUrl()).toBe('/');
  });

  it('lets go of the detail when another page shows', () => {
    ld.wide.set(true);
    ld.showDetail({ id: 's1', machine: null, agent: null }, false);
    ld.leave();
    expect(ld.split()).toBe(false);
  });
});
