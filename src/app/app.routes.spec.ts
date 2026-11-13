import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Route, Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { pairedGuard, routes } from './app.routes';
import { AuthService } from './core/auth.service';

@Component({ template: '' })
class BlankComponent {}

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

/** The app's routes, each page a blank one: where a URL lands. */
function blankRoutes(): Route[] {
  return routes.map((r) =>
    r.loadComponent ? { path: r.path, component: BlankComponent, canActivate: r.canActivate } : r,
  );
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

  it('sends Backup & sync to Accounts › Backups, with the query and the fragment', async () => {
    expect(await land('/backup')).toBe('/accounts/backups');
    expect(await land('/backup?account=acc_1#storage')).toBe(
      '/accounts/backups?account=acc_1#storage',
    );
  });

  it('splits Remote: its #foreign card is Sessions started elsewhere, the rest Phone & devices', async () => {
    expect(await land('/remote#foreign')).toBe('/settings/elsewhere');
    expect(await land('/remote')).toBe('/settings/devices');
    expect(await land('/remote?x=1#internet')).toBe('/settings/devices?x=1#internet');
  });

  it("sends Browser, API and Services to Settings' connections, the query kept", async () => {
    expect(await land('/browser')).toBe('/settings/browser');
    expect(await land('/api?server=work')).toBe('/settings/api?server=work');
    expect(await land('/services')).toBe('/settings/tools');
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
});
