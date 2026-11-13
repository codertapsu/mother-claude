import { ApplicationConfig, provideZoneChangeDetection } from '@angular/core';
import { provideRouter, withHashLocation, withInMemoryScrolling } from '@angular/router';

import { routes } from './app.routes';

export const appConfig: ApplicationConfig = {
  providers: [
    provideZoneChangeDetection({ eventCoalescing: true }),
    // Hash location keeps deep links (e.g. /#/pair?c=…) working when the SPA
    // is served by the embedded server — and keeps pairing codes in the URL
    // fragment, which is never sent to the server or a tunnel.
    provideRouter(
      routes,
      withHashLocation(),
      // A link to a part of a page (`/computers#copies`, `/settings#updates`)
      // scrolls there.
      withInMemoryScrolling({ anchorScrolling: 'enabled' }),
    ),
  ],
};
