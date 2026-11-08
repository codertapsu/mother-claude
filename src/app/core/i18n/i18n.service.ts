import { Injectable, effect, inject } from '@angular/core';

import { AuthService } from '../auth.service';
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
  }

  /** Switch to `lang` and keep the choice on this device. */
  set(lang: Lang): Promise<void> {
    return useLanguage(lang, true);
  }
}
