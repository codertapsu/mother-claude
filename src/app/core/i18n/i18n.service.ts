import { Injectable, effect, inject } from '@angular/core';

import { AuthService } from '../auth.service';
import { isTauri } from '../platform';
import { LANGS, Lang, hostOsSignal, language, useLanguage } from './i18n';

/** The interface language for components (Settings › Appearance). */
@Injectable({ providedIn: 'root' })
export class I18nService {
  readonly lang = language;
  readonly langs = LANGS;

  constructor() {
    // The computer's name in sentences follows the desktop's system.
    const auth = inject(AuthService);
    effect(() => hostOsSignal.set(auth.hostOs()));
    // The tray's menu (a run started at sign-in) speaks the language this
    // window shows: the desktop can't read the window's choice, so the
    // window tells it — an OS concern, through invoke (`tray.rs`).
    if (isTauri()) {
      effect(() => void trayLanguage(language()));
    }
  }

  /** Switch to `lang` and keep the choice on this device. */
  set(lang: Lang): Promise<void> {
    return useLanguage(lang, true);
  }
}

async function trayLanguage(lang: Lang): Promise<void> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('tray_language', { lang });
  } catch {
    /* an older desktop build */
  }
}
