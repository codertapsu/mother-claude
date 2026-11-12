import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Router, RouterOutlet, provideRouter } from '@angular/router';

import { AttentionService } from '../core/attention.service';
import { NavTab } from '../core/nav';
import { RealtimeService } from '../core/realtime.service';
import { HubTabsComponent } from './hub-tabs.component';

@Component({ template: '' })
class BlankComponent {}

const TABS: NavTab[] = [
  { path: '/', label: 'Sessions', exact: true },
  { path: '/agents', label: 'Agents', badge: 'agents' },
  { path: '/inbox', label: 'Inbox', badge: 'pending' },
];

@Component({
  imports: [HubTabsComponent, RouterOutlet],
  template: `<mc-hub-tabs [tabs]="tabs()" /><router-outlet />`,
})
class HostComponent {
  readonly tabs = signal<NavTab[]>(TABS);
}

describe('HubTabsComponent', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([
          { path: '', component: BlankComponent },
          { path: 'agents', component: BlankComponent },
          { path: 'inbox', component: BlankComponent },
        ]),
      ],
    });
  });

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = async (url: string) => {
    const fixture = TestBed.createComponent(HostComponent);
    await TestBed.inject(Router).navigateByUrl(url);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };
  const tabs = (el: HTMLElement) => [...el.querySelectorAll<HTMLAnchorElement>('nav.hub-tabs a')];

  it("links a place's pages and marks the one shown, whatever its query", async () => {
    const { el } = await render('/agents?session=s1');
    expect(tabs(el).map(text)).toEqual(['Sessions', 'Agents', 'Inbox']);
    expect(tabs(el).map((a) => a.getAttribute('href'))).toEqual(['/', '/agents', '/inbox']);
    expect(tabs(el).map((a) => a.getAttribute('aria-current'))).toEqual([null, 'page', null]);

    // The home is current on exactly its path, a filter in its query too.
    const home = await render('/?machine=abc');
    expect(tabs(home.el).map((a) => a.getAttribute('aria-current'))).toEqual(['page', null, null]);
  });

  it('counts what goes on behind a tab, and says it to screen readers', async () => {
    const realtime = TestBed.inject(RealtimeService) as unknown as {
      replaceStore(l: unknown[]): void;
    };
    realtime.replaceStore([
      {
        id: 's1',
        live: true,
        agents: { running: 2, total: 2, workflowsRunning: 0, backgroundRunning: 1 },
        pending: [],
      },
    ]);
    const { el } = await render('/');
    const agents = tabs(el)[1];
    expect(text(agents.querySelector('.tcount'))).toBe('3');
    expect(agents.getAttribute('aria-label')).toBe('Agents, 3 running');
    expect(tabs(el)[2].querySelector('.tcount')).toBeNull();
    expect(TestBed.inject(AttentionService).pendingCount()).toBe(0);
  });

  it('shows nothing for a place of one page', async () => {
    const { fixture, el } = await render('/');
    fixture.componentInstance.tabs.set([TABS[0]]);
    fixture.detectChanges();
    expect(el.querySelector('nav.hub-tabs')).toBeNull();
  });
});
