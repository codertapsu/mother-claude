import { bootstrapApplication } from '@angular/platform-browser';
import { appConfig } from './app/app.config';
import { AppComponent } from './app/app.component';
import { storedLanguage, useLanguage } from './app/core/i18n/i18n';

// A file dropped anywhere no drop zone takes it would make the webview open
// it in place of the app (the desktop leaves drops to the page, so that
// dragging inside it works on Windows too: `dragDropEnabled: false`).
const carriesFiles = (e: DragEvent) => !!e.dataTransfer?.types?.includes('Files');
for (const type of ['dragover', 'drop'] as const) {
  window.addEventListener(type, (e) => {
    if (carriesFiles(e) && !e.defaultPrevented) e.preventDefault();
  });
}

// The language this device chose is loaded first, so the app never shows
// English before switching (docs/I18N.md).
void useLanguage(storedLanguage())
  .catch((err: unknown) => console.error(err))
  .then(() => bootstrapApplication(AppComponent, appConfig))
  .catch((err: unknown) => console.error(err));
