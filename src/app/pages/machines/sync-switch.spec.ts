import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { MachineAsk, SyncPair, SyncView } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { AsksCardComponent } from './asks-card.component';
import { PEER_ID, machinesView, peer } from './machines.testing';
import { ProjectOptionsComponent } from './project-options.component';

@Component({ template: '' })
class BlankComponent {}

const APP = '/Users/you/dev/app';
const ID = 'sy_0123456789abcdef';

const text = (el: Element | null | undefined) =>
  (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const button = (el: Element, label: string) =>
  [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));

function synced(over: Partial<SyncPair> = {}): SyncPair {
  return {
    id: ID,
    machine: PEER_ID,
    machineName: 'DESKTOP-PC',
    machineOs: 'windows',
    online: true,
    folder: APP,
    name: 'app',
    there: 'D:\\dev\\app',
    here: { on: true },
    peer: { on: true, asked: false },
    status: { state: 'inSync', at: Date.now() },
    last: null,
    conflict: null,
    kept: [],
    createdAt: Date.now(),
    updatedAt: Date.now(),
    ...over,
  };
}

describe('ProjectOptionsComponent: keeping a project in sync', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ProjectOptionsComponent>;
  let pairs: SyncPair[];
  const settle = async () => {
    for (let i = 0; i < 5; i++) {
      await flush();
      fixture.detectChanges();
    }
  };

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    pairs = [];
    backend.routes.set('/projects', () => ({
      status: 200,
      body: { projects: [{ path: APP, name: 'app' }] },
    }));
    backend.routes.set('/machines/settings', () => ({
      status: 200,
      body: { relay: { mode: 'public', url: null }, keepAwake: true, projects: [] },
    }));
    backend.routes.set('/machines/sync', (method, body) => {
      if (method === 'PUT') {
        const b = body as { project: string; machine: string; on: boolean };
        pairs = b.on
          ? [
              synced({
                folder: b.project,
                peer: { on: false, asked: true },
                status: { state: 'waiting' },
              }),
            ]
          : [];
      }
      const view: SyncView = { pairs, watching: true };
      return { status: 200, body: view };
    });
    TestBed.inject(MachinesService).view.set(machinesView({ enabled: true, peers: [peer()] }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const render = async () => {
    fixture = TestBed.createComponent(ProjectOptionsComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };

  it('is off by default, says what it does, and asks the other computer when saved', async () => {
    asDesktop();
    const el = await render();
    button(el, 'Set a project’s own options…')!.click();
    await settle();
    const select = el.querySelector<HTMLSelectElement>('select[name="project"]')!;
    select.value = APP;
    select.dispatchEvent(new Event('change'));
    await settle();
    const box = el.querySelector<HTMLInputElement>('input[name="syncOn"]')!;
    expect(box.checked).toBe(false);
    const fieldset = el.querySelector('fieldset.sync')!;
    expect(text(fieldset)).toContain('Keep this project in sync with DESKTOP-PC');
    expect(text(fieldset)).toContain(
      'Its commits, its branch and its uncommitted changes go to the other computer a few seconds after they change, both ways; files git ignores stay, but for the ones it carries. Nothing is merged: when both changed it, you choose which to keep. Saving asks the other computer: it is on once someone there confirms.',
    );
    box.click();
    await settle();
    button(el, 'Save')!.click();
    await settle();
    const order = backend.calls.filter((c) => c.startsWith('PUT '));
    expect(order).toEqual(['PUT /machines/settings', 'PUT /machines/sync']);
    expect(backend.bodyOf('PUT /machines/sync')).toEqual({
      project: APP,
      machine: PEER_ID,
      on: true,
    });
    expect(text(el)).toContain(
      'Saved. DESKTOP-PC was asked to keep it in sync: it is on once someone there confirms.',
    );
    // Listed for its sync, though it has no options of its own.
    const row = el.querySelector('.list > li')!;
    expect(text(row)).toContain('code sync with DESKTOP-PC: Waiting for DESKTOP-PC');
    expect(button(row, 'Use the defaults')).toBeUndefined();
  });

  it('turns it off from the same switch', async () => {
    asDesktop();
    pairs = [synced()];
    const el = await render();
    const row = el.querySelector('.list > li')!;
    expect(text(row)).toContain('code sync with DESKTOP-PC: In sync');
    button(row, 'Change')!.click();
    await settle();
    const box = el.querySelector<HTMLInputElement>('input[name="syncOn"]')!;
    expect(box.checked).toBe(true);
    box.click();
    await settle();
    button(el, 'Save')!.click();
    await settle();
    expect(backend.bodyOf('PUT /machines/sync')).toEqual({
      project: APP,
      machine: PEER_ID,
      on: false,
    });
  });

  it('shows no switch while no computer is paired', async () => {
    asDesktop();
    TestBed.inject(MachinesService).view.set(machinesView());
    const el = await render();
    button(el, 'Set a project’s own options…')!.click();
    await settle();
    expect(el.querySelector('fieldset.sync')).toBeNull();
  });
});

describe('AsksCardComponent: keeping a project in sync', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<AsksCardComponent>;
  const settle = async () => {
    for (let i = 0; i < 5; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const ask: MachineAsk = {
    id: 'ask_0123456789abcdef',
    from: PEER_ID,
    fromName: 'DESKTOP-PC',
    kind: 'sync',
    title: 'app',
    folder: APP,
    there: 'D:\\dev\\app',
    sync: ID,
    at: Date.now(),
    state: 'asked',
  };

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: '**', component: BlankComponent }])]);
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  it('shows the request and turns it on only when confirmed here', async () => {
    asDesktop();
    backend.routes.set('/machines/asks/ask_0123456789abcdef', () => ({
      status: 200,
      body: { ask: { ...ask, state: 'done', detail: 'Kept in sync with DESKTOP-PC.' }, next: null },
    }));
    fixture = TestBed.createComponent(AsksCardComponent);
    fixture.componentRef.setInput('asks', [ask]);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el)).toContain('Keep app in sync with DESKTOP-PC');
    // What travels, what stays, and that nothing is merged.
    expect(text(el)).toContain(
      `Keep ${APP} here the same as its copy on DESKTOP-PC, both ways: its commits, its branch and its uncommitted changes go across a few seconds after they change. Files git ignores stay on each computer, but for any the project carries. When both computers changed it, you choose which to keep: nothing is merged, and the side not kept is saved first.`,
    );
    expect(backend.calls).toEqual([]);
    button(el, 'Keep it in sync')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/asks/ask_0123456789abcdef')).toEqual({
      answer: 'confirm',
    });
  });
});
