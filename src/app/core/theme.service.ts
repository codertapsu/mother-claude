import { Injectable, computed, effect, signal } from '@angular/core';

import { storageGet, storageSet } from './platform';

export type ThemePref = 'system' | 'dark' | 'light';

const KEY = 'mc_theme';
const THEME_COLORS = { dark: '#0e0f13', light: '#f6f6f9' } as const;

/**
 * Dark (default) / light theme. "system" follows prefers-color-scheme; a
 * manual choice sets `data-theme` on <html>, which styles.scss keys off.
 */
@Injectable({ providedIn: 'root' })
export class ThemeService {
  readonly pref = signal<ThemePref>(read());
  private readonly systemLight = signal(prefersLight());
  readonly effective = computed<'dark' | 'light'>(() => {
    const p = this.pref();
    if (p === 'system') return this.systemLight() ? 'light' : 'dark';
    return p;
  });

  constructor() {
    try {
      matchMedia('(prefers-color-scheme: light)').addEventListener('change', (e) =>
        this.systemLight.set(e.matches),
      );
    } catch {
      /* old browsers: no live updates */
    }
    effect(() => this.apply(this.pref(), this.effective()));
  }

  set(pref: ThemePref): void {
    this.pref.set(pref);
    storageSet(KEY, pref);
  }

  private apply(pref: ThemePref, effective: 'dark' | 'light'): void {
    const root = document.documentElement;
    if (pref === 'system') delete root.dataset['theme'];
    else root.dataset['theme'] = pref;
    // Keep the browser chrome (status bar, tab strip) in step with a manual choice.
    for (const meta of Array.from(document.querySelectorAll('meta[name="theme-color"]'))) {
      if (pref === 'system') {
        const light = meta.getAttribute('media')?.includes('light');
        meta.setAttribute('content', light ? THEME_COLORS.light : THEME_COLORS.dark);
      } else {
        meta.setAttribute('content', THEME_COLORS[effective]);
      }
    }
  }
}

function read(): ThemePref {
  const v = storageGet(KEY);
  return v === 'dark' || v === 'light' ? v : 'system';
}

function prefersLight(): boolean {
  try {
    return matchMedia('(prefers-color-scheme: light)').matches;
  } catch {
    return false;
  }
}
