import { Routes } from '@angular/router';

export const routes: Routes = [
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
