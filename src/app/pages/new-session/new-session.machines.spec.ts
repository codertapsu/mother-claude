import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { MachinesService } from '../../core/machines.service';
import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { PEER2_ID, PEER_ID, pairedMachines } from '../../core/mock/remote.testing';
import { machinesView, peer } from '../machines/machines.testing';
import { NewSessionComponent } from './new-session.component';

@Component({ template: '' })
class BlankComponent {}

const PROXY = `/machines/${PEER_ID}/proxy`;

describe('NewSessionComponent on another computer', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<NewSessionComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: 'session/:id/on/:machine', component: BlankComponent },
        { path: 'session/:id', component: BlankComponent },
        { path: '', component: BlankComponent },
      ]),
    ]);
    backend.routes.set('/projects', () => ({
      status: 200,
      body: { projects: [{ name: 'api', path: '/Users/me/dev/api', lastActivityAt: 2 }] },
    }));
    backend.routes.set(`${PROXY}/projects`, () => ({
      status: 200,
      body: {
        projects: [
          {
            name: 'mother-claude',
            path: 'D:\\development\\projects\\mother-claude',
            lastActivityAt: 5,
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
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async () => {
    fixture = TestBed.createComponent(NewSessionComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const wheres = (el: HTMLElement) => [...el.querySelectorAll<HTMLButtonElement>('.where')];
  const startButton = (el: HTMLElement) =>
    el.querySelector('.actions .btn.primary') as HTMLButtonElement;

  it('starts it there, with its projects, through the proxy', async () => {
    pairedMachines();
    backend.routes.set(`${PROXY}/sessions`, () => ({ status: 201, body: { id: 'r-new' } }));
    const el = await render();
    expect(wheres(el).map((w) => text(w))).toEqual(['This computer', 'DESKTOP-PC']);
    expect(text(el.querySelector('.project .pname'))).toBe('api');

    wheres(el)[1].click();
    await settle();
    expect(backend.calls).toContain(`GET ${PROXY}/projects`);
    expect(text(el.querySelector('.project .pname'))).toBe('mother-claude');
    // Its one project is picked; it runs under that computer's account.
    expect(text(startButton(el))).toBe('Start on DESKTOP-PC');
    expect(text(el)).toContain('It starts on DESKTOP-PC, under its account for the folder');

    startButton(el).click();
    await settle();
    expect(backend.bodyOf(`POST ${PROXY}/sessions`)).toMatchObject({
      cwd: 'D:\\development\\projects\\mother-claude',
    });
    expect(backend.calls).not.toContain('POST /sessions');
    expect(TestBed.inject(Router).url).toBe(`/session/r-new/on/${PEER_ID}`);
  });

  it('says what the other computer refused', async () => {
    pairedMachines();
    backend.routes.set(`${PROXY}/sessions`, () => ({
      status: 403,
      body: {
        error: {
          code: 'grant_missing',
          message: 'This computer has not given yours the "control" permission.',
          grant: 'control',
        },
      },
    }));
    const el = await render();
    wheres(el)[1].click();
    await settle();
    startButton(el).click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe(
      'Could not start the session: DESKTOP-PC lets this computer watch its sessions, not drive them.',
    );
  });

  it("can't pick an offline computer, or one that lets this one watch only", async () => {
    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [
          peer({ online: false }),
          peer({
            id: PEER2_ID,
            name: 'BUILD-BOX',
            path: 'lan',
            theirGrants: {
              view: { lan: true, internet: true },
              control: { lan: false, internet: false },
              move: { lan: false, internet: false },
              run: { lan: false, internet: false },
              dangerous: { lan: false, internet: false },
            },
          }),
        ],
      }),
    );
    const el = await render();
    const [, desktop, build] = wheres(el);
    expect(text(desktop)).toBe('DESKTOP-PC (offline)');
    expect(desktop.disabled).toBe(true);
    expect(text(build)).toBe('BUILD-BOX (watch only)');
    expect(build.disabled).toBe(true);
  });

  it('is this computer only while no other is paired', async () => {
    const el = await render();
    expect(wheres(el)).toEqual([]);
    expect(text(startButton(el))).toBe('Start session');
    expect(backend.calls.filter((c) => c.includes('/machines'))).toEqual([]);
  });
});
