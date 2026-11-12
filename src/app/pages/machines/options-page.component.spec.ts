import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { machinesView, peer } from './machines.testing';
import { OptionsPageComponent } from './options-page.component';

@Component({ template: '' })
class BlankComponent {}

/** Computers › Options (docs/UI-REDESIGN.md §6.5): the general options, the
 * accounts and folders and each project's options — without the setup. */
describe('OptionsPageComponent: Computers › Options', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<OptionsPageComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([{ path: 'computers/setup', component: BlankComponent }]),
    ]);
    backend.routes.set('/machines/sync', () => ({
      status: 200,
      body: { pairs: [], watching: true },
    }));
    backend.routes.set('/machines/settings', () => ({
      status: 200,
      body: { relay: { mode: 'public', url: null }, keepAwake: true, projects: [] },
    }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: Element, label: string) =>
    [...el.querySelectorAll<HTMLElement>('button, a')].find((b) => text(b).startsWith(label));
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async (view = machinesView({ enabled: true, peers: [peer()] })) => {
    backend.routes.set('/machines', () => ({ status: 200, body: view }));
    fixture = TestBed.createComponent(OptionsPageComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const headings = (el: HTMLElement) =>
    [...el.querySelectorAll('.section-head h2')].map((h) => text(h));

  it('sends to the setup while your computers were never set up', async () => {
    asDesktop();
    const el = await render(machinesView());
    expect(text(el.querySelector('.empty-state strong'))).toBe('Not set up yet');
    button(el, 'Set up your computers')!.click();
    await settle();
    expect(TestBed.inject(Router).url).toBe('/computers/setup');
  });

  it('holds the options, the accounts and folders and each project, out of the setup', async () => {
    asDesktop();
    const el = await render();
    expect(headings(el)).toEqual(['General', 'Accounts and folders', 'Each project']);
    expect(text(el.querySelector('.card.options'))).toContain('Public relays (default)');
    expect(el.querySelector('mc-machines-accounts-folders')).not.toBeNull();
    expect(el.querySelector('mc-machines-project-options')).not.toBeNull();
    expect(button(el, 'Set a project’s own options…')).toBeDefined();
    // Change opens the form in place.
    button(el.querySelector('.section-head')!, 'Change')!.click();
    await settle();
    expect(el.querySelector('mc-machines-options form')).not.toBeNull();
    expect(button(el, 'Run the setup again')?.getAttribute('href')).toBe('/computers/setup');
  });

  it('turns Computers off', async () => {
    asDesktop();
    backend.routes.set('/machines/enable', () => ({
      status: 200,
      body: machinesView({ enabled: false, peers: [peer()] }),
    }));
    const el = await render();
    button(el, 'Turn Computers off')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/enable')).toEqual({ enabled: false });
    expect(button(el, 'Turn Computers off')).toBeUndefined();
  });

  it('is read-only on a phone', async () => {
    asPhoneOf('macos');
    const el = await render();
    expect(text(el.querySelector('.card.options'))).toContain('Public relays (default)');
    expect(el.querySelector('mc-machines-accounts-folders')).toBeNull();
    expect(text(el)).toContain('Changed on the desktop app.');
    for (const label of ['Change', 'Run the setup again', 'Turn Computers off']) {
      expect(button(el, label), label).toBeUndefined();
    }
  });
});
