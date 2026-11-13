import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { hostOsSignal } from '../../core/i18n/i18n';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { ToolsPageComponent } from './tools-page.component';

/** Settings › Claude Code tools (docs/UI-REDESIGN.md §6.7): Services' switch,
 * on a page of its own; the rest of Services is Computers › This Mac. */
describe('ToolsPageComponent: Settings › Claude Code tools', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ToolsPageComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/mcp', () => ({
      status: 200,
      body: { enabled: true, clients: 2, recentCalls: [], tools: [], toolCount: 9 },
    }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
    hostOsSignal.set(undefined);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

  it('holds the switch, and points to what Claude Code runs besides sessions', async () => {
    asDesktop({ hostOs: 'macos' });
    hostOsSignal.set('macos');
    fixture = TestBed.createComponent(ToolsPageComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el.querySelector('mc-page-head h1'))).toBe('Claude Code tools');
    expect(el.querySelector('a.back')?.getAttribute('href')).toBe('/settings');
    expect(el.querySelector<HTMLInputElement>('mc-mcp-panel input[type="checkbox"]')?.checked).toBe(
      true,
    );
    expect(text(el.querySelector('.more a'))).toBe('Computers › This Mac › Details');
    expect(el.querySelector('.more a')?.getAttribute('href')).toBe('/computers');
  });
});
