import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { SyncView } from '../../core/machines';
import { syncPair } from '../../core/mock/attention.testing';
import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { PEER_ID, machinesView, peer } from './machines.testing';
import { SyncPageComponent } from './sync-page.component';

@Component({ template: '' })
class BlankComponent {}

/** Computers › Code sync (docs/UI-REDESIGN.md §6.5): the projects kept in
 * sync, and *Keep a project in sync…* without the setup. */
describe('SyncPageComponent: Computers › Code sync', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SyncPageComponent>;
  let synced: SyncView;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([{ path: 'computers/setup', component: BlankComponent }]),
    ]);
    synced = { pairs: [], watching: true };
    backend.routes.set('/machines/sync', (method, body) => {
      if (method === 'PUT') {
        const b = body as { project: string; machine: string };
        synced = {
          watching: true,
          pairs: [
            syncPair({ folder: b.project, machine: b.machine, status: { state: 'waiting' } }),
          ],
        };
      }
      return { status: 200, body: synced };
    });
    backend.routes.set('/projects', () => ({
      status: 200,
      body: {
        projects: [
          { name: 'tray-tool', path: '/Users/me/dev/tray-tool' },
          { name: 'web', path: '/Users/me/dev/web' },
        ],
      },
    }));
    backend.routes.set('/machines/settings', () => ({
      status: 200,
      body: {
        relay: { mode: 'public', url: null },
        keepAwake: true,
        projects: [
          {
            key: '/Users/me/dev/tray-tool',
            project: '/Users/me/dev/tray-tool',
            root: null,
            code: {},
            recipe: { carryIgnored: ['.env.local'] },
            updatedAt: Date.now(),
          },
        ],
      },
    }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: Element, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button, a')].find((b) => text(b).startsWith(label));
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async (view = machinesView({ enabled: true, peers: [peer()] })) => {
    backend.routes.set('/machines', () => ({ status: 200, body: view }));
    fixture = TestBed.createComponent(SyncPageComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const pick = (el: HTMLElement, selector: string, value: string) => {
    const select = el.querySelector<HTMLSelectElement>(selector)!;
    select.value = value;
    select.dispatchEvent(new Event('change'));
  };

  it('says nothing is synced before a computer is paired, and offers the setup', async () => {
    asDesktop();
    const el = await render(machinesView());
    expect(text(el.querySelector('.empty-state strong'))).toBe('No project kept in sync');
    const setUp = button(el, 'Set up your computers')!;
    setUp.click();
    await settle();
    expect(TestBed.inject(Router).url).toBe('/computers/setup');
    expect(button(el, 'Keep a project in sync…')).toBeUndefined();
  });

  it('keeps a project in sync from a sheet: the project, the computer, its files', async () => {
    asDesktop();
    const el = await render();
    expect(text(el.querySelector('.section-head h2'))).toBe('Projects kept in sync');
    expect(text(el.querySelector('mc-machines-sync'))).toContain('No project is kept in sync.');
    button(el, 'Keep a project in sync…')!.click();
    await settle();
    const sheet = el.querySelector('mc-keep-in-sync-sheet')!;
    const submit = button(sheet, 'Keep it in sync') as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    pick(sheet as HTMLElement, 'select[name="project"]', '/Users/me/dev/tray-tool');
    await settle();
    const carried = sheet.querySelector('.carried')!;
    expect(text(carried.querySelector('.mono'))).toBe('.env.local');
    expect(carried.querySelector('a')?.getAttribute('href')).toBe(
      '/computers/options#each-project',
    );
    expect(text(sheet)).toContain(
      'DESKTOP-PC is asked once; syncing starts when someone there says yes.',
    );
    expect(sheet.querySelector<HTMLInputElement>('input[type="radio"]')?.checked).toBe(true);
    submit.click();
    await settle();
    expect(backend.bodyOf('PUT /machines/sync')).toEqual({
      project: '/Users/me/dev/tray-tool',
      machine: PEER_ID,
      on: true,
    });
    expect(text(el.querySelector('.notice.ok'))).toBe(
      'Asked DESKTOP-PC to keep it in sync: it is on once someone there confirms.',
    );
    expect(text(el.querySelector('mc-machines-sync'))).toContain('tray-tool');
  });

  it('is read-only on a phone', async () => {
    asPhoneOf('macos');
    const el = await render();
    expect(el.querySelector('mc-machines-sync')).not.toBeNull();
    expect(button(el, 'Keep a project in sync…')).toBeUndefined();
  });
});
