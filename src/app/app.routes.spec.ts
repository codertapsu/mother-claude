import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Route, Router, RouterOutlet, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { pairedGuard, routePaths, routes } from './app.routes';
import { AuthService } from './core/auth.service';

@Component({ template: '' })
class BlankComponent {}

@Component({ imports: [RouterOutlet], template: '<router-outlet />' })
class BlankLayoutComponent {}

describe('pairedGuard', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([
          { path: 'inbox', component: BlankComponent, canActivate: [pairedGuard] },
          { path: 'pair', component: BlankComponent },
        ]),
      ],
    });
  });

  it('sends an unpaired device back to /pair, saying why', async () => {
    const auth = TestBed.inject(AuthService);
    auth.state.set('unpaired');
    auth.reason.set('revoked');
    const harness = await RouterTestingHarness.create();
    await harness.navigateByUrl('/inbox');
    expect(TestBed.inject(Router).url).toBe('/pair?reason=revoked');
  });

  it('lets a paired device through', async () => {
    TestBed.inject(AuthService).state.set('paired');
    const harness = await RouterTestingHarness.create();
    await harness.navigateByUrl('/inbox');
    expect(TestBed.inject(Router).url).toBe('/inbox');
  });
});

/** The app's routes, each page a blank one (a layout, an outlet): where a
 * URL lands. */
function blankRoutes(list: Route[] = routes): Route[] {
  return list.map((r) => {
    if (!r.loadComponent) return r;
    const rest: Route = { ...r };
    delete rest.loadComponent;
    const children = rest.children;
    return children
      ? { ...rest, component: BlankLayoutComponent, children: blankRoutes(children) }
      : { ...rest, component: BlankComponent };
  });
}

describe('old routes (docs/UI-REDESIGN.md §10.1)', () => {
  let harness: RouterTestingHarness;

  beforeEach(async () => {
    TestBed.configureTestingModule({ providers: [provideRouter(blankRoutes())] });
    TestBed.inject(AuthService).state.set('paired');
    harness = await RouterTestingHarness.create();
  });

  const land = async (url: string) => {
    await harness.navigateByUrl(url);
    return TestBed.inject(Router).url;
  };

  it('sends Machines to Computers, with the rest of the path, the query and the fragment', async () => {
    expect(await land('/machines')).toBe('/computers');
    expect(await land('/machines/setup')).toBe('/computers/setup');
    expect(await land('/machines?machine=abc#copies')).toBe('/computers?machine=abc#copies');
    expect(await land('/machines/setup?step=2#rail')).toBe('/computers/setup?step=2#rail');
  });

  it('sends Backup & sync to Backups, with the query and the fragment', async () => {
    expect(await land('/backup')).toBe('/backups');
    expect(await land('/backup?account=acc_1#storage')).toBe('/backups?account=acc_1#storage');
  });

  it('splits Remote: its #foreign card is Sessions started elsewhere, the rest Phone & devices', async () => {
    expect(await land('/remote#foreign')).toBe('/elsewhere');
    expect(await land('/remote?x=1#foreign')).toBe('/elsewhere?x=1');
    expect(await land('/remote')).toBe('/devices');
    expect(await land('/remote?x=1#internet')).toBe('/devices?x=1#internet');
  });

  it('sends API to HTTP API and Services to Settings › Claude Code tools, the query kept', async () => {
    expect(await land('/api?server=work')).toBe('/http-api?server=work');
    expect(await land('/api?server=work#tokens')).toBe('/http-api?server=work#tokens');
    expect(await land('/services')).toBe('/settings/tools');
  });

  it("sends Settings' connections and Accounts › Backups to their own places, the query and the fragment kept", async () => {
    for (const [from, to] of [
      ['/settings/devices', '/devices'],
      ['/settings/devices?x=1#internet', '/devices?x=1#internet'],
      ['/settings/elsewhere', '/elsewhere'],
      ['/settings/elsewhere?account=acc_1#plugins', '/elsewhere?account=acc_1#plugins'],
      ['/settings/browser', '/browser'],
      ['/settings/browser?site=github.com#approvals', '/browser?site=github.com#approvals'],
      ['/settings/api', '/http-api'],
      ['/settings/api?server=work#tokens', '/http-api?server=work#tokens'],
      ['/accounts/backups', '/backups'],
      ['/accounts/backups?account=acc_1#storage', '/backups?account=acc_1#storage'],
    ]) {
      expect(await land(from), from).toBe(to);
    }
  });

  it('opens the places of their own where they are, Settings › Claude Code tools too', async () => {
    for (const url of [
      '/devices',
      '/devices#internet',
      '/elsewhere',
      '/browser',
      '/browser?x=1#approvals',
      '/http-api',
      '/http-api?server=work',
      '/backups',
      '/backups?account=acc_1#storage',
      '/settings',
      '/settings#updates',
      '/settings/tools',
    ]) {
      expect(await land(url), url).toBe(url);
    }
  });

  it('sends Capacity to Teams › Capacity, with its section and the fragment', async () => {
    expect(await land('/capacity')).toBe('/teams/capacity');
    expect(await land('/capacity?tab=budgets#x')).toBe('/teams/capacity?tab=budgets#x');
  });

  it("opens Teams' tabs, never as a team", () => {
    const paths = routes.map((r) => r.path);
    const team = paths.indexOf('teams/:id');
    for (const tab of ['teams/templates', 'teams/capacity']) {
      expect(paths.indexOf(tab), tab).toBeGreaterThanOrEqual(0);
      expect(paths.indexOf(tab), tab).toBeLessThan(team);
    }
  });

  it('opens the projects on disk as a page of their own', async () => {
    expect(await land('/projects')).toBe('/projects');
  });

  it('keeps the links that never move', async () => {
    expect(await land('/pair?c=K7QM-4R2X')).toBe('/pair?c=K7QM-4R2X');
    expect(await land('/session/abc?pending=p1')).toBe('/session/abc?pending=p1');
    expect(await land('/teams/tm_0123456789ab')).toBe('/teams/tm_0123456789ab');
    expect(await land('/good-to-know#links')).toBe('/help/good-to-know#links');
    expect(await land('/')).toBe('/');
  });

  it('keeps every list and session path as it was, inside the list and detail layout', async () => {
    for (const url of [
      '/?machine=m1',
      '/inbox',
      '/agents',
      '/projects',
      '/session/abc',
      '/session/abc?tab=info',
      '/session/abc/agent/ag1',
      '/session/abc/on/m1',
      '/session/abc/agent/ag1/on/m1',
    ]) {
      expect(await land(url), url).toBe(url);
    }
    // The places beside it are not the layout's: they still land.
    expect(await land('/new?machine=m1')).toBe('/new?machine=m1');
    expect(await land('/http-api?server=work')).toBe('/http-api?server=work');
    expect(await land('/nowhere')).toBe('/');
  });

  it('guards each page of the layout, the layout as a whole', () => {
    const layout = routes.find((r) => r.children)!;
    expect(layout.path).toBe('');
    expect(layout.canActivateChild).toEqual([pairedGuard]);
    expect(routePaths(layout.children)).toEqual([
      '',
      'inbox',
      'agents',
      'projects',
      'session/:id',
      'session/:id/agent/:agentId',
      'session/:id/on/:machine',
      'session/:id/agent/:agentId/on/:machine',
    ]);
  });

  it('sends an unpaired device to /pair from a page of the layout too', async () => {
    const auth = TestBed.inject(AuthService);
    auth.state.set('unpaired');
    expect(await land('/session/abc?pending=p1')).toBe('/pair');
    expect(await land('/inbox')).toBe('/pair');
  });
});
