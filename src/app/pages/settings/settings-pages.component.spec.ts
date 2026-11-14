import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { SettingsPagesComponent } from './settings-pages.component';

describe("Settings' own pages: Claude Code tools", () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SettingsPagesComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/mcp', () => ({
      status: 200,
      body: { enabled: true, clients: 4, tools: [], toolCount: 0, recentCalls: [] },
    }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = async () => {
    fixture = TestBed.createComponent(SettingsPagesComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const rows = (el: HTMLElement) => [...el.querySelectorAll<HTMLAnchorElement>('a.list-row')];
  const state = (el: HTMLElement, id: string) =>
    text(el.querySelector(`a[data-id="${id}"] .state`));

  it('opens Claude Code tools, saying how they stand, and nothing that is a place now', async () => {
    asDesktop();
    const el = await render();
    expect(text(el.querySelector('h2'))).toBe('Claude Code');
    expect(
      rows(el).map((a) => [text(a.querySelector('.grow > span')), a.getAttribute('href')]),
    ).toEqual([['Claude Code tools', '/settings/tools']]);
    expect(state(el, 'tools')).toBe('On · used by 4 sessions in the last 15 min');
    // Phone & devices, Sessions started elsewhere, Browser and HTTP API are
    // places of their own: nothing of theirs is read here.
    for (const call of [
      'GET /devices',
      'GET /remote',
      'GET /control/foreign',
      'GET /browser',
      'GET /bridge',
    ]) {
      expect(backend.calls, call).not.toContain(call);
    }
  });

  it('gives a phone the same row', async () => {
    const el = await render();
    expect(rows(el).map((a) => a.getAttribute('data-id'))).toEqual(['tools']);
    expect(state(el, 'tools')).toBe('On · used by 4 sessions in the last 15 min');
  });

  it('says what it is while its state cannot be read, and when the tools are off', async () => {
    backend.routes.delete('/mcp');
    const el = await render();
    expect(state(el, 'tools')).toBe('Mother Claude’s tools for Claude Code sessions');
    fixture.destroy();
    backend.routes.set('/mcp', () => ({
      status: 200,
      body: { enabled: false, clients: 0, tools: [], toolCount: 0, recentCalls: [] },
    }));
    const off = await render();
    expect(state(off, 'tools')).toBe('Off: the browser extension and the team tools need them');
  });
});
