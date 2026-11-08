import { bootstrapApplication } from '@angular/platform-browser';
import { appConfig } from './app/app.config';
import { AppComponent } from './app/app.component';

// A file dropped anywhere no drop zone takes it would make the webview open
// it in place of the app (the desktop leaves drops to the page, so that
// dragging inside it works on Windows too: `dragDropEnabled: false`).
const carriesFiles = (e: DragEvent) => !!e.dataTransfer?.types?.includes('Files');
for (const type of ['dragover', 'drop'] as const) {
  window.addEventListener(type, (e) => {
    if (carriesFiles(e) && !e.defaultPrevented) e.preventDefault();
  });
}

bootstrapApplication(AppComponent, appConfig).catch((err) => console.error(err));
