import { inject } from '@angular/core';
import { CanActivateFn, RedirectFunction, Router, Routes } from '@angular/router';

import { AuthService } from './core/auth.service';

/**
 * Data pages need a paired device. The shell redirects to /pair when the
 * pairing state changes; this also covers Back, a bookmark or a typed URL
 * afterwards (which would otherwise show a misleading empty page). While
 * the state is still unknown (startup) navigation proceeds and the shell
 * decides once it is known.
 */
export const pairedGuard: CanActivateFn = () => {
  const auth = inject(AuthService);
  if (auth.state() !== 'unpaired') return true;
  return inject(Router).createUrlTree(
    ['/pair'],
    auth.reason() === 'revoked' ? { queryParams: { reason: 'revoked' } } : {},
  );
};

/**
 * `/remote` was two places: its `#foreign` card is Sessions started
 * elsewhere, the rest Phone & devices — the query and any other fragment
 * kept (docs/UI-REDESIGN.md §10.3).
 */
export const remoteRedirect: RedirectFunction = ({ queryParams, fragment }) => {
  const router = inject(Router);
  return fragment === 'foreign'
    ? router.createUrlTree(['/elsewhere'], { queryParams })
    : router.createUrlTree(['/devices'], { queryParams, fragment: fragment ?? undefined });
};

/** Reachable without pairing. */
const OPEN = new Set(['pair', 'onboarding', '**']);

/** A list's route: the layout draws the list (`data.list`). */
const listRoute = () =>
  import('./pages/list-detail/list-route.component').then((m) => m.ListRouteComponent);

/**
 * The work's lists and the sessions they open, in one layout: from 1280 px
 * a session opens beside its list (docs/UI-REDESIGN.md §9, stage 9). The
 * paths are the same as ever: `#/session/<id>?pending=` from a push opens
 * the session on its own route.
 */
const work: Routes = [
  { path: '', pathMatch: 'full', loadComponent: listRoute, data: { list: 'sessions' } },
  { path: 'inbox', loadComponent: listRoute, data: { list: 'inbox' } },
  // Sessions › Agents.
  { path: 'agents', loadComponent: listRoute, data: { list: 'agents' } },
  // Every project folder that holds conversations (docs/ACCOUNTS.md §14.4).
  { path: 'projects', loadComponent: listRoute, data: { list: 'projects' } },
  {
    path: 'session/:id',
    loadComponent: () =>
      import('./pages/session-detail/session-detail.component').then(
        (m) => m.SessionDetailComponent,
      ),
  },
  {
    path: 'session/:id/agent/:agentId',
    loadComponent: () =>
      import('./pages/agent-view/agent-view.component').then((m) => m.AgentViewComponent),
  },
  // Another computer's session and its agents (docs/PLAN-MACHINES.md §6.2).
  {
    path: 'session/:id/on/:machine',
    loadComponent: () =>
      import('./pages/session-detail/remote-session.component').then(
        (m) => m.RemoteSessionComponent,
      ),
  },
  {
    path: 'session/:id/agent/:agentId/on/:machine',
    loadComponent: () =>
      import('./pages/agent-view/remote-agent.component').then((m) => m.RemoteAgentComponent),
  },
];

const pages: Routes = [
  {
    path: '',
    loadComponent: () =>
      import('./pages/list-detail/list-detail.component').then((m) => m.ListDetailComponent),
    children: work,
  },
  {
    path: 'new',
    loadComponent: () =>
      import('./pages/new-session/new-session.component').then((m) => m.NewSessionComponent),
  },
  // Computers: your other computers (docs/PLAN-MACHINES.md), in four tabs
  // (docs/UI-REDESIGN.md §6.5); the setup is a page of its own.
  {
    path: 'computers',
    loadComponent: () =>
      import('./pages/machines/machines.component').then((m) => m.MachinesComponent),
  },
  {
    path: 'computers/sync',
    loadComponent: () =>
      import('./pages/machines/sync-page.component').then((m) => m.SyncPageComponent),
  },
  {
    path: 'computers/activity',
    loadComponent: () =>
      import('./pages/machines/activity-page.component').then((m) => m.ActivityPageComponent),
  },
  {
    path: 'computers/options',
    loadComponent: () =>
      import('./pages/machines/options-page.component').then((m) => m.OptionsPageComponent),
  },
  {
    path: 'computers/setup',
    loadComponent: () =>
      import('./pages/machines/setup.component').then((m) => m.MachinesSetupComponent),
  },
  // Machines was renamed Computers: `/machines/setup` → `/computers/setup`,
  // the query and the fragment kept (a relative prefix redirect).
  { path: 'machines', redirectTo: 'computers' },
  // Phone entry: #/pair?c=CODE from a QR code, or a typed code.
  {
    path: 'pair',
    loadComponent: () => import('./pages/pair/pair.component').then((m) => m.PairComponent),
  },
  {
    path: 'settings',
    loadComponent: () =>
      import('./pages/settings/settings.component').then((m) => m.SettingsComponent),
  },
  // Settings' own page: Claude Code tools (docs/MCP.md).
  {
    path: 'settings/tools',
    loadComponent: () =>
      import('./pages/settings/tools-page.component').then((m) => m.ToolsPageComponent),
  },
  // What works with Mother Claude, each a place of its own
  // (docs/UI-REDESIGN.md §11, 2026-11-14): phones and internet access, the
  // plugin for sessions started elsewhere, the browser extension
  // (docs/BROWSER.md), the HTTP API (`?server=`).
  {
    path: 'devices',
    loadComponent: () => import('./pages/remote/remote.component').then((m) => m.RemoteComponent),
  },
  {
    path: 'elsewhere',
    loadComponent: () =>
      import('./pages/settings/elsewhere-page.component').then((m) => m.ElsewherePageComponent),
  },
  {
    path: 'browser',
    loadComponent: () =>
      import('./pages/browser/browser.component').then((m) => m.BrowserComponent),
  },
  {
    path: 'http-api',
    loadComponent: () => import('./pages/bridge/bridge.component').then((m) => m.BridgeComponent),
  },
  // Accounts: the Claude accounts. A paired phone renames them, sets the
  // defaults and copy permissions and moves data; adding, signing in and
  // removing stay on the desktop (docs/ACCOUNTS.md §13.9).
  {
    path: 'accounts',
    loadComponent: () =>
      import('./pages/accounts/accounts.component').then((m) => m.AccountsComponent),
  },
  // How much of each plan is left, how often it is read, the rate limits.
  {
    path: 'accounts/usage',
    loadComponent: () =>
      import('./pages/accounts/usage-page.component').then((m) => m.UsagePageComponent),
  },
  // Every data job, with Undo while it is possible.
  {
    path: 'accounts/changes',
    loadComponent: () =>
      import('./pages/accounts/changes-page.component').then((m) => m.ChangesPageComponent),
  },
  // Backups: a paired phone does everything but the passphrase
  // (docs/BACKUP.md).
  {
    path: 'backups',
    loadComponent: () => import('./pages/backup/backup.component').then((m) => m.BackupComponent),
  },
  // Permissions: what Claude may do without asking (`?on=here|others`).
  {
    path: 'permissions',
    loadComponent: () =>
      import('./pages/permissions/permissions.component').then((m) => m.PermissionsComponent),
  },
  {
    path: 'more',
    loadComponent: () => import('./pages/more/more.component').then((m) => m.MoreComponent),
  },
  // Places of their own since 2026-11-14 (docs/UI-REDESIGN.md §11): Settings'
  // connections, and Backups (Backup & sync, then Accounts › Backups) —
  // the query (`/api?server=`) and the fragment kept. Remote was two
  // places; Services is Settings › Claude Code tools.
  { path: 'settings/devices', redirectTo: 'devices' },
  { path: 'settings/elsewhere', redirectTo: 'elsewhere' },
  { path: 'settings/browser', redirectTo: 'browser' },
  { path: 'settings/api', redirectTo: 'http-api' },
  { path: 'accounts/backups', redirectTo: 'backups' },
  { path: 'backup', redirectTo: 'backups' },
  { path: 'remote', redirectTo: remoteRedirect },
  { path: 'api', redirectTo: 'http-api' },
  { path: 'services', redirectTo: 'settings/tools' },
  // Teams of sessions (docs/TEAMS.md): the teams, the templates they start
  // from, Capacity (role sessions, budgets, models and costs,
  // docs/ROUTING.md), one team. Templates and Capacity come before a
  // team's id, which is `tm_` and 12 hex digits: no clash.
  {
    path: 'teams',
    loadComponent: () => import('./pages/teams/teams.component').then((m) => m.TeamsComponent),
  },
  {
    path: 'teams/templates',
    loadComponent: () =>
      import('./pages/teams/templates-page.component').then((m) => m.TemplatesPageComponent),
  },
  {
    path: 'teams/capacity',
    loadComponent: () =>
      import('./pages/capacity/capacity.component').then((m) => m.CapacityComponent),
  },
  {
    path: 'teams/:id',
    loadComponent: () => import('./pages/teams/team.component').then((m) => m.TeamComponent),
  },
  // Capacity is a tab of Teams now (`?tab=` and the fragment kept).
  { path: 'capacity', redirectTo: 'teams/capacity' },
  // Help: every topic, and one topic (an unknown one falls back to /help).
  {
    path: 'help',
    loadComponent: () => import('./pages/help/help.component').then((m) => m.HelpComponent),
  },
  {
    path: 'help/:topic',
    loadComponent: () => import('./pages/help/help.component').then((m) => m.HelpComponent),
  },
  // The old "Good to know" page is a Help topic now.
  { path: 'good-to-know', redirectTo: 'help/good-to-know' },
  {
    path: 'onboarding',
    loadComponent: () =>
      import('./pages/onboarding/onboarding.component').then((m) => m.OnboardingComponent),
  },
  { path: '**', redirectTo: '' },
];

// A redirect is guarded where it leads (Angular refuses a guard on one); a
// layout, on each of its pages.
export const routes: Routes = pages.map((r) =>
  OPEN.has(r.path ?? '') || r.redirectTo !== undefined
    ? r
    : r.children
      ? { ...r, canActivateChild: [pairedGuard] }
      : { ...r, canActivate: [pairedGuard] },
);

/** Every page's path, a layout's pages under it (`session/:id`): what the
 * map (core/nav.ts) is checked against. */
export function routePaths(list: Routes = routes, base = ''): string[] {
  return list.flatMap((r) => {
    const path = [base, r.path ?? ''].filter(Boolean).join('/');
    return r.children ? routePaths(r.children, path) : [path];
  });
}
