import { Location } from '@angular/common';
import { provideLocationMocks } from '@angular/common/testing';
import { Component, inject } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { TestBed } from '@angular/core/testing';
import { ActivatedRoute, Route, Router, RouterOutlet, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';
import { map } from 'rxjs';

import { routes } from '../../app.routes';
import { AuthService } from '../../core/auth.service';
import { ListDetailService } from '../../core/list-detail';
import { ListNavDirective } from '../../shared/list-nav.directive';
import { ListDetailComponent } from './list-detail.component';
import { ListRouteComponent } from './list-route.component';

/** Every list made, by its tag: the layout keeps the one on screen. */
let made: string[] = [];

@Component({
  selector: 'mc-sessions',
  template: `<input class="search" data-nav-entry /><a data-nav-item href="#a">a</a>`,
})
class SessionsStub {
  constructor() {
    made.push('sessions');
  }
}

@Component({ selector: 'mc-inbox', template: `<p>inbox</p>` })
class InboxStub {
  constructor() {
    made.push('inbox');
  }
}

@Component({ selector: 'mc-agents', template: `<p>agents</p>` })
class AgentsStub {
  constructor() {
    made.push('agents');
  }
}

@Component({ selector: 'mc-projects-page', template: `<p>projects</p>` })
class ProjectsStub {
  constructor() {
    made.push('projects');
  }
}

/** A session's page: says which (the router keeps it for another id). */
@Component({ selector: 'mc-detail-stub', template: `<p class="detail">{{ id() }}</p>` })
class DetailStub {
  protected readonly id = toSignal(inject(ActivatedRoute).paramMap.pipe(map((p) => p.get('id'))));
}

@Component({ selector: 'mc-elsewhere', template: `<p>elsewhere</p>` })
class ElsewhereStub {}

/** The app's own layout and paths, its pages stubbed. */
function workRoutes(): Route[] {
  const layout = routes.find((r) => r.children)!;
  const children = layout.children!.map((r) => {
    const rest: Route = { ...r };
    delete rest.loadComponent;
    return { ...rest, component: r.data?.['list'] ? ListRouteComponent : DetailStub };
  });
  return [
    { path: '', component: ListDetailComponent, children },
    { path: 'settings', component: ElsewhereStub },
  ];
}

describe('ListDetailComponent (docs/UI-REDESIGN.md §9, stage 9)', () => {
  let harness: RouterTestingHarness;
  let ld: ListDetailService;

  beforeEach(async () => {
    made = [];
    TestBed.configureTestingModule({
      providers: [provideRouter(workRoutes()), provideLocationMocks()],
    });
    TestBed.overrideComponent(ListDetailComponent, {
      set: {
        imports: [
          RouterOutlet,
          ListNavDirective,
          SessionsStub,
          InboxStub,
          AgentsStub,
          ProjectsStub,
        ],
      },
    });
    TestBed.inject(AuthService).state.set('paired');
    ld = TestBed.inject(ListDetailService);
    harness = await RouterTestingHarness.create();
  });

  const go = async (url: string) => {
    await harness.navigateByUrl(url);
    harness.detectChanges();
  };
  const layout = () => harness.fixture.nativeElement.querySelector('mc-list-detail') as HTMLElement;
  const shown = (sel: string) => !!layout()?.querySelector(sel);
  const url = () => TestBed.inject(Router).url;
  const escape = (target: EventTarget = document.body) => {
    const e = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    target.dispatchEvent(e);
    return e;
  };

  it('draws a list route’s list itself, on its own', async () => {
    ld.wide.set(true);
    await go('/inbox');
    expect(shown('mc-inbox')).toBe(true);
    expect(shown('.detail')).toBe(false);
    expect(layout().classList).not.toContain('ld-split');
    expect(ld.list()).toBe('inbox');
  });

  it('opens a session beside the list it came from on a wide screen, the same list', async () => {
    ld.wide.set(true);
    await go('/inbox');
    await go('/session/a');
    expect(layout().classList).toContain('ld-split');
    expect(shown('mc-inbox')).toBe(true);
    expect(layout().querySelector('.detail')?.textContent).toBe('a');
    // One Inbox all along: scrolled where it was, its filters kept.
    expect(made).toEqual(['inbox']);
    expect(ld.isOpen('a')).toBe(true);
    await go('/session/b');
    expect(layout().querySelector('.detail')?.textContent).toBe('b');
    expect(made).toEqual(['inbox']);
  });

  it('keeps a session on its own page below 1280 px, as before', async () => {
    ld.wide.set(false);
    await go('/');
    await go('/session/a');
    expect(shown('.ld-list')).toBe(false);
    expect(shown('.detail')).toBe(true);
    expect(layout().classList).not.toContain('ld-split');
    expect(ld.isOpen('a')).toBe(false);
    // Back on the list: it is drawn again.
    await go('/');
    expect(shown('mc-sessions')).toBe(true);
  });

  it('follows the window: wider, the list comes beside the session; narrower, it goes', async () => {
    ld.wide.set(false);
    await go('/session/a');
    expect(shown('.ld-list')).toBe(false);
    ld.wide.set(true);
    harness.detectChanges();
    expect(shown('mc-sessions')).toBe(true);
    expect(layout().classList).toContain('ld-split');
    ld.wide.set(false);
    harness.detectChanges();
    expect(shown('.ld-list')).toBe(false);
  });

  it('puts the Inbox beside a session a notification opened first', async () => {
    ld.wide.set(true);
    await go('/session/a?pending=p1');
    expect(shown('mc-inbox')).toBe(true);
    expect(url()).toBe('/session/a?pending=p1');
    await ld.close();
    expect(url()).toBe('/inbox');
  });

  it('opens another computer’s session beside the list too', async () => {
    ld.wide.set(true);
    await go('/');
    await go('/session/a/on/m1');
    expect(layout().classList).toContain('ld-split');
    expect(shown('mc-sessions')).toBe(true);
    expect(ld.detail()).toEqual({ id: 'a', machine: 'm1', agent: null });
    expect(ld.isOpen('a', 'm1')).toBe(true);
    expect(ld.isOpen('a')).toBe(false);
    await go('/session/a/agent/ag1/on/m1');
    expect(ld.detail()).toEqual({ id: 'a', machine: 'm1', agent: 'ag1' });
  });

  it('shows the Sessions list beside a session opened from Projects (its rows open none)', async () => {
    ld.wide.set(true);
    await go('/projects');
    expect(shown('mc-projects-page')).toBe(true);
    await go('/session/a');
    expect(shown('mc-sessions')).toBe(true);
    expect(shown('mc-projects-page')).toBe(false);
    expect(ld.closeUrl()).toBe('/');
  });

  it('closes the session with Esc, back at the list’s own address', async () => {
    ld.wide.set(true);
    await go('/?machine=m1');
    await go('/session/a');
    const e = escape();
    expect(e.defaultPrevented).toBe(true);
    await harness.fixture.whenStable();
    harness.detectChanges();
    expect(url()).toBe('/?machine=m1');
    expect(layout().classList).not.toContain('ld-split');
    expect(made).toEqual(['sessions']);
  });

  it('leaves Esc to a field, a dialog, and to narrow screens', async () => {
    ld.wide.set(true);
    await go('/');
    await go('/session/a');
    const field = layout().querySelector('input.search') as HTMLInputElement;
    expect(escape(field).defaultPrevented).toBe(false);
    const dialog = document.createElement('dialog');
    const inside = document.createElement('button');
    dialog.appendChild(inside);
    document.body.appendChild(dialog);
    try {
      dialog.show();
      expect(escape(inside).defaultPrevented).toBe(false);
      expect(escape().defaultPrevented).toBe(false);
    } finally {
      dialog.remove();
    }
    ld.wide.set(false);
    harness.detectChanges();
    expect(escape().defaultPrevented).toBe(false);
    await harness.fixture.whenStable();
    expect(url()).toBe('/session/a');
  });

  it('goes Back from session to session, then to the list, the list staying', async () => {
    // As the app's start does: the browser's Back reaches the router.
    TestBed.inject(Router).setUpLocationChangeListener();
    ld.wide.set(true);
    await go('/inbox');
    await go('/session/a');
    await go('/session/b');
    const back = async () => {
      TestBed.inject(Location).back();
      // The router takes the browser's step on its next task.
      await new Promise((r) => setTimeout(r));
      await harness.fixture.whenStable();
      harness.detectChanges();
    };
    await back();
    expect(url()).toBe('/session/a');
    expect(layout().querySelector('.detail')?.textContent).toBe('a');
    await back();
    expect(url()).toBe('/inbox');
    expect(layout().classList).not.toContain('ld-split');
    expect(made).toEqual(['inbox']);
  });

  it('lets go of the detail when another page shows', async () => {
    ld.wide.set(true);
    await go('/session/a');
    expect(ld.split()).toBe(true);
    await go('/settings');
    expect(ld.detail()).toBeNull();
    expect(ld.split()).toBe(false);
  });
});
