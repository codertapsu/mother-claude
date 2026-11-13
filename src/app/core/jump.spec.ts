import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { Component } from '@angular/core';

import {
  JumpItem,
  JumpService,
  applePlatform,
  foldItem,
  isJumpKey,
  jumpShortcut,
  matchScore,
  rankJump,
  sessionKey,
  sessionKeyOf,
  termScore,
} from './jump';
import { storageRemove } from './platform';

const item = (key: string, title: string, over: Partial<JumpItem> = {}): JumpItem => ({
  key,
  kind: 'page',
  title,
  icon: 'sessions',
  target: { kind: 'link', commands: ['/'] },
  ...over,
});

const score = (query: string, title: string, words = '') =>
  matchScore(query, foldItem({ title, words }));

describe('Jump to…: matching (docs/UI-REDESIGN.md §9, stage 11)', () => {
  it('ranks the whole name, its start, a word, anywhere, then its letters in order', () => {
    const whole = termScore('inbox', 'inbox');
    const start = termScore('inb', 'inbox');
    const word = termScore('tray', 'fix tray icon');
    const inside = termScore('ray', 'fix tray icon');
    const letters = termScore('fti', 'fix tray icon');
    expect(whole).toBeGreaterThan(start);
    expect(start).toBeGreaterThan(word);
    expect(word).toBeGreaterThan(inside);
    expect(inside).toBeGreaterThan(letters);
    expect(letters).toBeGreaterThan(0);
    expect(termScore('zzz', 'fix tray icon')).toBe(0);
  });

  it('takes letters in order only from a word’s start, placed where they fit best', () => {
    // "dpc": the initials of DESKTOP-PC beat letters spread through a name.
    expect(termScore('dpc', 'desktop-pc')).toBeGreaterThan(
      termScore('dpc', 'nightly dependency audit'),
    );
    expect(termScore('nses', 'new session')).toBeGreaterThan(0);
    // No word starts with its first letter: no match, however the rest lines up.
    expect(termScore('tray', 'explore caching strategy')).toBe(0);
  });

  it('needs every word, in the name or as written in the rest', () => {
    expect(score('fix icon', 'Fix tray icon')).toBeGreaterThan(0);
    expect(score('fix nothing', 'Fix tray icon')).toBe(0);
    // The rest is matched as written, never letter by letter.
    expect(score('mother', 'Fix tray icon', 'mother-claude')).toBeGreaterThan(0);
    expect(score('mcl', 'Fix tray icon', 'mother-claude')).toBe(0);
    // A match in the name beats one in the rest.
    expect(score('tray', 'Fix tray icon')).toBeGreaterThan(score('tray', 'Other', 'tray'));
  });

  it('forgets case and accents, as Vietnamese is typed without them', () => {
    expect(score('sao luu', 'Sao lưu')).toBeGreaterThan(0);
    expect(score('dong bo', 'Đồng bộ mã nguồn')).toBeGreaterThan(0);
    expect(score('DESKTOP', 'desktop-pc')).toBeGreaterThan(0);
  });
});

describe('Jump to…: ranking', () => {
  const items = [
    item('page:/inbox', 'Inbox'),
    item('page:/teams', 'Teams'),
    item('session::s1', 'Fix tray icon', { kind: 'session' }),
    item('help:tray', 'Tray and icon', { kind: 'help' }),
    item('project:/dev/tray', 'tray', { kind: 'project' }),
  ];

  it('puts the better match first', () => {
    expect(rankJump(items, 'tray', []).map((i) => i.key)).toEqual([
      'project:/dev/tray',
      'help:tray',
      'session::s1',
    ]);
  });

  it('puts a recent item first among equals, and a little higher', () => {
    const both = [
      item('help:a', 'Tray notes', { kind: 'help' }),
      item('help:b', 'Tray tips', { kind: 'help' }),
    ];
    expect(rankJump(both, 'tray', []).map((i) => i.key)).toEqual(['help:a', 'help:b']);
    expect(rankJump(both, 'tray', ['help:b']).map((i) => i.key)).toEqual(['help:b', 'help:a']);
  });

  it('orders equal matches: sessions, pages, projects, computers, settings, Help', () => {
    const same = [
      item('help:x', 'Build', { kind: 'help' }),
      item('setting:x', 'Build', { kind: 'setting' }),
      item('computer:x', 'Build', { kind: 'computer' }),
      item('project:x', 'Build', { kind: 'project' }),
      item('page:x', 'Build', { kind: 'page' }),
      item('session:x', 'Build', { kind: 'session' }),
    ];
    expect(rankJump(same, 'build', []).map((i) => i.kind)).toEqual([
      'session',
      'page',
      'project',
      'computer',
      'setting',
      'help',
    ]);
  });

  it('shows the recent items, then every page, with nothing typed', () => {
    expect(rankJump(items, '  ', ['session::s1', 'gone', 'page:/teams']).map((i) => i.key)).toEqual(
      ['session::s1', 'page:/teams', 'page:/inbox'],
    );
    expect(rankJump(items, '', [], 1)).toHaveLength(1);
  });
});

describe('Jump to…: keys and the recent list', () => {
  @Component({ template: '' })
  class BlankComponent {}

  beforeEach(() => {
    storageRemove('mc_jump_recent');
    TestBed.configureTestingModule({
      providers: [provideRouter([{ path: '**', component: BlankComponent }])],
    });
  });

  afterEach(() => storageRemove('mc_jump_recent'));

  it('opens with ⌘K on Apple’s systems, Ctrl+K on the others', () => {
    const key = (init: KeyboardEventInit) => new KeyboardEvent('keydown', { key: 'k', ...init });
    expect(isJumpKey(key({ metaKey: true }), true)).toBe(true);
    expect(isJumpKey(key({ ctrlKey: true }), true)).toBe(false);
    expect(isJumpKey(key({ ctrlKey: true }), false)).toBe(true);
    expect(isJumpKey(key({ metaKey: true }), false)).toBe(false);
    expect(isJumpKey(key({ ctrlKey: true, shiftKey: true }), false)).toBe(false);
    expect(isJumpKey(new KeyboardEvent('keydown', { key: 'K', ctrlKey: true }), false)).toBe(true);
    expect(jumpShortcut(true)).toBe('⌘K');
    expect(jumpShortcut(false)).toBe('Ctrl+K');
    expect(applePlatform({ userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 15_0)' })).toBe(true);
    expect(applePlatform({ userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' })).toBe(false);
  });

  it('reads the session a page shows, another computer’s and an agent’s too', () => {
    expect(sessionKeyOf('/session/abc?pending=p1')).toBe(sessionKey('abc'));
    expect(sessionKeyOf('/session/abc/on/m1')).toBe(sessionKey('abc', 'm1'));
    expect(sessionKeyOf('/session/abc/agent/a1/on/m1')).toBe(sessionKey('abc', 'm1'));
    expect(sessionKeyOf('/inbox')).toBeNull();
  });

  it('keeps what was opened, newest first, once each, on this device', async () => {
    const jump = TestBed.inject(JumpService);
    jump.remember('page:/inbox');
    jump.remember('page:/teams');
    jump.remember('page:/inbox');
    expect(jump.recent()).toEqual(['page:/inbox', 'page:/teams']);
    for (let i = 0; i < 20; i++) jump.remember(`page:${i}`);
    expect(jump.recent()).toHaveLength(12);
    // A session opened any way counts.
    await TestBed.inject(Router).navigateByUrl('/session/s9/on/m1');
    expect(jump.recent()[0]).toBe(sessionKey('s9', 'm1'));
    expect(JSON.parse(localStorage.getItem('mc_jump_recent') ?? '[]')[0]).toBe('session:m1:s9');
  });

  it('opens and closes, and asks the shell for the palette once', () => {
    const jump = TestBed.inject(JumpService);
    expect(jump.wanted()).toBe(false);
    jump.toggle();
    expect(jump.opened()).toBe(true);
    expect(jump.wanted()).toBe(true);
    jump.toggle();
    expect(jump.opened()).toBe(false);
  });
});
