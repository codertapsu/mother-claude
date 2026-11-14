import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { JumpService } from '../../core/jump';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import {
  PEER_ID,
  goLive,
  pairedMachines,
  remoteList,
  remoteSession,
} from '../../core/mock/remote.testing';
import { storageRemove } from '../../core/platform';
import { ProjectSettingsService } from '../../core/project-settings';
import { RealtimeService } from '../../core/realtime.service';
import { JumpToComponent } from './jump-to.component';

@Component({ template: '' })
class BlankComponent {}

describe('JumpToComponent: Jump to… (docs/UI-REDESIGN.md §9, stage 11)', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<JumpToComponent>;

  beforeEach(() => {
    storageRemove('mc_jump_recent');
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: '**', component: BlankComponent }])]);
    backend.routes.set('/data/projects', () => ({
      status: 200,
      body: {
        projects: [
          {
            account: 'default',
            cwd: '/Users/me/dev/tray-tool',
            name: 'tray-tool',
            folder: '/Users/me/.claude/projects/-Users-me-dev-tray-tool',
            sessions: 4,
            live: 0,
            bytes: 1024,
            lastActivityAt: Date.now(),
          },
        ],
      },
    }));
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: unknown[]): void }
    ).replaceStore([
      remoteSession('s1', {
        name: 'Fix tray icon',
        title: 'Fix tray icon',
        projectName: 'mother-claude',
        cwd: '/Users/me/dev/mother-claude',
        gitBranch: 'feat/session-tokens',
        status: 'idle',
      }),
    ]);
  });

  afterEach(() => {
    fixture?.destroy();
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
    storageRemove('mc_jump_recent');
  });

  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const open = async () => {
    fixture = TestBed.createComponent(JumpToComponent);
    fixture.detectChanges();
    TestBed.inject(JumpService).open();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const field = () => (fixture.nativeElement as HTMLElement).querySelector('input')!;
  const type = async (text: string) => {
    field().value = text;
    field().dispatchEvent(new Event('input'));
    await settle();
  };
  const press = async (key: string) => {
    field().dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
    await settle();
  };
  const options = () =>
    [...(fixture.nativeElement as HTMLElement).querySelectorAll('[role="option"]')].map((o) => ({
      title: o.querySelector('.title')?.textContent?.trim() ?? '',
      kind: o.querySelector('.kind')?.textContent?.trim() ?? '',
      active: o.getAttribute('aria-selected') === 'true',
      id: o.id,
    }));
  const url = () => TestBed.inject(Router).url;

  it('opens with the field focused, every page listed, the recent ones first', async () => {
    TestBed.inject(JumpService).remember('page:/teams/capacity');
    const el = await open();
    expect(document.activeElement).toBe(field());
    expect(el.querySelector('.group')?.textContent?.trim()).toBe('Recent');
    const list = options();
    expect(list[0]).toEqual(expect.objectContaining({ title: 'Capacity', active: true }));
    const titles = list.map((o) => o.title);
    for (const page of ['Sessions', 'Inbox', 'Teams', 'Computers', 'Accounts', 'Settings']) {
      expect(titles, page).toContain(page);
    }
    expect(titles).toContain('New session');
    expect(titles).toContain('Help');
    // A combobox over a listbox: the field names the option Enter opens.
    expect(field().getAttribute('role')).toBe('combobox');
    expect(field().getAttribute('aria-activedescendant')).toBe(list[0].id);
  });

  it('finds a session by its name, project, folder or branch, and opens it', async () => {
    await open();
    for (const q of ['tray icon', 'feat/session']) {
      await type(q);
      expect(options()[0], q).toEqual(expect.objectContaining({ title: 'Fix tray icon' }));
    }
    // Its project's name: the project first, its sessions after it.
    await type('mother-claude');
    expect(options().map((o) => [o.title, o.kind])).toEqual([
      ['mother-claude', 'Project'],
      ['Fix tray icon', 'Session'],
    ]);
    await type('tray icon');
    await press('Enter');
    expect(url()).toBe('/session/s1');
    const jump = TestBed.inject(JumpService);
    expect(jump.opened()).toBe(false);
    expect(jump.recent()[0]).toBe('session::s1');
  });

  it('finds another computer’s sessions, and the computer itself', async () => {
    pairedMachines();
    const socket = await goLive(backend);
    remoteList(socket, PEER_ID, [remoteSession('r1', { title: 'Release notes' })]);
    await open();
    await type('release');
    expect(options()[0]).toEqual(expect.objectContaining({ title: 'Release notes' }));
    await press('Enter');
    expect(url()).toBe(`/session/r1/on/${PEER_ID}`);
    TestBed.inject(JumpService).open();
    await settle();
    await type('desktop');
    const computer = options().find((o) => o.kind === 'Computer')!;
    expect(computer.title).toBe('DESKTOP-PC');
  });

  it('moves with ↓ and ↑, around at the ends, and opens what is chosen', async () => {
    await open();
    await type('tray');
    const n = options().length;
    expect(n).toBeGreaterThan(1);
    await press('ArrowDown');
    expect(options()[1].active).toBe(true);
    expect(field().getAttribute('aria-activedescendant')).toBe(options()[1].id);
    await press('ArrowUp');
    await press('ArrowUp');
    expect(options()[n - 1].active).toBe(true);
  });

  it("opens a project's settings", async () => {
    await open();
    await type('tray-tool');
    expect(options()[0]).toEqual(expect.objectContaining({ title: 'tray-tool', kind: 'Project' }));
    await press('Enter');
    expect(TestBed.inject(ProjectSettingsService).project()).toEqual({
      cwd: '/Users/me/dev/tray-tool',
      name: 'tray-tool',
    });
  });

  it('opens a page, a Settings section and a Help topic, old names found too', async () => {
    await open();
    await type('capacity');
    await press('Enter');
    expect(url()).toBe('/teams/capacity');

    TestBed.inject(JumpService).open();
    await settle();
    await type('appearance');
    expect(options()[0]).toEqual(expect.objectContaining({ kind: 'Setting' }));
    await press('Enter');
    expect(url()).toBe('/settings#appearance');

    TestBed.inject(JumpService).open();
    await settle();
    // Machines is Computers now.
    await type('machines');
    expect(options().map((o) => o.title)).toContain('Computers');

    await type('good to know');
    expect(options()[0]).toEqual(expect.objectContaining({ kind: 'Help topic' }));
    await press('Enter');
    expect(url()).toBe('/help/good-to-know');
  });

  it('opens the places that were Settings’ connections, and Backups, by their old names too', async () => {
    asDesktop();
    const el = await open();
    /** The page of that title among the results: [its place, the row]. */
    const pageRow = (title: string) =>
      [...el.querySelectorAll<HTMLElement>('[role="option"]')].find(
        (o) =>
          o.querySelector('.title')?.textContent?.trim() === title &&
          o.querySelector('.kind')?.textContent?.trim() === 'Page',
      );
    for (const [query, title, within, path] of [
      ['remote', 'Phone & devices', '', '/devices'],
      ['hooks plugin', 'Sessions started elsewhere', '', '/elsewhere'],
      ['chrome extension', 'Browser', '', '/browser'],
      ['bridge', 'HTTP API', '', '/http-api'],
      ['backup restore', 'Backups', '', '/backups'],
      ['mcp', 'Claude Code tools', 'Settings', '/settings/tools'],
    ]) {
      TestBed.inject(JumpService).open();
      await settle();
      await type(query);
      const row = pageRow(title);
      expect(row, query).toBeDefined();
      // A place of its own; Claude Code tools stays a page of Settings.
      expect(row?.querySelector('.detail')?.textContent?.trim() ?? '', query).toBe(within);
      row!.click();
      await settle();
      expect(url(), query).toBe(path);
    }
    // Settings no longer has a Connections section to jump to.
    TestBed.inject(JumpService).open();
    await settle();
    await type('connections');
    expect(options().filter((o) => o.kind === 'Setting')).toEqual([]);
  });

  it('says when nothing matches, and closes with Esc', async () => {
    const el = await open();
    await type('zzzzqqq');
    expect(options()).toEqual([]);
    expect(el.querySelector('.none')?.textContent?.trim()).toBe('Nothing matches “zzzzqqq”.');
    const dialog = el.querySelector('dialog')!;
    const cancel = new Event('cancel', { cancelable: true });
    dialog.dispatchEvent(cancel);
    expect(cancel.defaultPrevented).toBe(true);
    expect(TestBed.inject(JumpService).opened()).toBe(false);
    await settle();
    expect(dialog.open).toBe(false);
  });
});
