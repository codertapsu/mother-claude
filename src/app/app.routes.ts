import { inject } from '@angular/core';
import { CanActivateFn, Router, Routes } from '@angular/router';

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

/** Reachable without pairing. */
const OPEN = new Set(['pair', 'onboarding', '**']);

const pages: Routes = [
  {
    path: '',
    loadComponent: () =>
      import('./pages/sessions/sessions.component').then((m) => m.SessionsComponent),
  },
  {
    path: 'inbox',
    loadComponent: () => import('./pages/inbox/inbox.component').then((m) => m.InboxComponent),
  },
  {
    path: 'agents',
    loadComponent: () => import('./pages/agents/agents.component').then((m) => m.AgentsComponent),
  },
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
  {
    path: 'new',
    loadComponent: () =>
      import('./pages/new-session/new-session.component').then((m) => m.NewSessionComponent),
  },
  {
    path: 'remote',
    loadComponent: () => import('./pages/remote/remote.component').then((m) => m.RemoteComponent),
  },
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
  // Settings › Accounts: the Claude accounts (read-only on phones).
  {
    path: 'accounts',
    loadComponent: () =>
      import('./pages/accounts/accounts.component').then((m) => m.AccountsComponent),
  },
  // Settings › Backup & sync (read-only on phones).
  {
    path: 'backup',
    loadComponent: () => import('./pages/backup/backup.component').then((m) => m.BackupComponent),
  },
  {
    path: 'more',
    loadComponent: () => import('./pages/more/more.component').then((m) => m.MoreComponent),
  },
  {
    path: 'api',
    loadComponent: () => import('./pages/bridge/bridge.component').then((m) => m.BridgeComponent),
  },
  {
    path: 'services',
    loadComponent: () =>
      import('./pages/services/services.component').then((m) => m.ServicesComponent),
  },
  {
    path: 'good-to-know',
    loadComponent: () =>
      import('./pages/good-to-know/good-to-know.component').then((m) => m.GoodToKnowComponent),
  },
  {
    path: 'onboarding',
    loadComponent: () =>
      import('./pages/onboarding/onboarding.component').then((m) => m.OnboardingComponent),
  },
  { path: '**', redirectTo: '' },
];

export const routes: Routes = pages.map((r) =>
  OPEN.has(r.path ?? '') ? r : { ...r, canActivate: [pairedGuard] },
);
