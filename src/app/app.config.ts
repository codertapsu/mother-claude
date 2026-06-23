import { ApplicationConfig, provideZoneChangeDetection } from '@angular/core';
import { provideRouter, withHashLocation } from '@angular/router';
import { provideHttpClient, withInterceptorsFromDi } from '@angular/common/http';

import { routes } from './app.routes';

export const appConfig: ApplicationConfig = {
  providers: [
    provideZoneChangeDetection({ eventCoalescing: true }),
    // Hash location keeps deep links (e.g. /#/pair?token=...) working when the
    // SPA is served by the embedded server with a catch-all fallback.
    provideRouter(routes, withHashLocation()),
    provideHttpClient(withInterceptorsFromDi()),
  ],
};
