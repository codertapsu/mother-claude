import { Routes } from '@angular/router';

export const routes: Routes = [
  {
    path: '',
    loadComponent: () =>
      import('./pages/sessions/sessions.component').then((m) => m.SessionsComponent),
  },
  {
    path: 'session/:id',
    loadComponent: () =>
      import('./pages/session-detail/session-detail.component').then(
        (m) => m.SessionDetailComponent,
      ),
  },
  {
    path: 'services',
    loadComponent: () =>
      import('./pages/services/services.component').then((m) => m.ServicesComponent),
  },
  {
    path: 'settings',
    loadComponent: () =>
      import('./pages/settings/settings.component').then((m) => m.SettingsComponent),
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
  // Pairing links (.../#/pair?token=…) — token is captured by ConfigService on
  // load; this dedicated screen shows the QR + manual token entry.
  {
    path: 'pair',
    loadComponent: () => import('./pages/pair/pair.component').then((m) => m.PairComponent),
  },
  { path: '**', redirectTo: '' },
];
