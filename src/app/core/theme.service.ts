import { Injectable, computed, effect, signal } from '@angular/core';

import { storageGet, storageSet } from './platform';

export type ThemePref = 'dark' | 'light' | 'system';

/** Accent palettes: buttons, links, focus (styles.scss `$accents`). */
export type AccentId = 'violet' | 'blue' | 'teal' | 'green' | 'orange' | 'rose' | 'graphite';
/** Background palettes: surfaces, borders, text (styles.scss `$surfaces`). */
export type SurfaceId = 'standard' | 'midnight' | 'black' | 'warm';

export interface AccentOption {
  id: AccentId;
  label: string;
  /** The accent, as its swatch shows it in dark and in light. */
  dark: string;
  light: string;
}

export interface SurfaceOption {
  id: SurfaceId;
  label: string;
  /** A hint under its name. */
  hint: string;
  /** Background and panel, as its tile shows them in dark and in light. */
  dark: { bg: string; panel: string; border: string };
  light: { bg: string; panel: string; border: string };
}

/** Keep in step with styles.scss (the tokens and `$accents`). */
export const ACCENTS: readonly AccentOption[] = [
  { id: 'violet', label: 'Violet', dark: '#7c5cc9', light: '#6d4bc2' },
  { id: 'blue', label: 'Blue', dark: '#2f6fdf', light: '#2563eb' },
  { id: 'teal', label: 'Teal', dark: '#0f7f74', light: '#0f766e' },
  { id: 'green', label: 'Green', dark: '#1d7f45', light: '#15803d' },
  { id: 'orange', label: 'Orange', dark: '#c2410c', light: '#c2410c' },
  { id: 'rose', label: 'Rose', dark: '#d02a5c', light: '#be123c' },
  { id: 'graphite', label: 'Graphite', dark: '#55555f', light: '#3f3f46' },
];

/** Keep in step with styles.scss (the tokens and `$surfaces`). */
export const SURFACES: readonly SurfaceOption[] = [
  {
    id: 'standard',
    label: 'Standard',
    hint: 'Neutral greys',
    dark: { bg: '#0e0f13', panel: '#15161c', border: '#262833' },
    light: { bg: '#f6f6f9', panel: '#ffffff', border: '#dfe1e8' },
  },
  {
    id: 'midnight',
    label: 'Midnight',
    hint: 'Deep navy blues',
    dark: { bg: '#0a1020', panel: '#10182c', border: '#22304f' },
    light: { bg: '#f2f5fa', panel: '#ffffff', border: '#d6deea' },
  },
  {
    id: 'black',
    label: 'Black',
    hint: 'True black, easy on OLED screens',
    dark: { bg: '#000000', panel: '#0b0b0d', border: '#222228' },
    light: { bg: '#ffffff', panel: '#ffffff', border: '#e1e1e7' },
  },
  {
    id: 'warm',
    label: 'Warm',
    hint: 'Soft paper tones',
    dark: { bg: '#12100e', panel: '#1a1714', border: '#2e2924' },
    light: { bg: '#f8f5f0', panel: '#fffdf9', border: '#e4ddd2' },
  },
];

const KEY = 'mc_theme';
const ACCENT_KEY = 'mc_accent';
const SURFACE_KEY = 'mc_surface';

/**
 * The theme — dark (the default), light, or the system's — and two palettes
 * on top of it: the accent and the background. Each device keeps its own
 * (local storage). They are attributes on <html> that styles.scss keys off:
 * `data-theme` (dark, light, system), `data-accent` and `data-surface`
 * (absent for Violet and Standard). `public/theme-boot.js` sets the same
 * attributes from the same keys before the first paint: keep it in step.
 */
@Injectable({ providedIn: 'root' })
export class ThemeService {
  readonly pref = signal<ThemePref>(readPref());
  readonly accent = signal<AccentId>(readAccent());
  readonly surface = signal<SurfaceId>(readSurface());
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
    effect(() => this.apply(this.pref(), this.effective(), this.accent(), this.surface()));
  }

  set(pref: ThemePref): void {
    this.pref.set(pref);
    storageSet(KEY, pref);
  }

  setAccent(id: AccentId): void {
    this.accent.set(id);
    storageSet(ACCENT_KEY, id);
  }

  setSurface(id: SurfaceId): void {
    this.surface.set(id);
    storageSet(SURFACE_KEY, id);
  }

  private apply(
    pref: ThemePref,
    effective: 'dark' | 'light',
    accent: AccentId,
    surface: SurfaceId,
  ): void {
    const root = document.documentElement;
    root.dataset['theme'] = pref;
    if (accent === 'violet') delete root.dataset['accent'];
    else root.dataset['accent'] = accent;
    if (surface === 'standard') delete root.dataset['surface'];
    else root.dataset['surface'] = surface;
    // Keep the browser chrome (status bar, tab strip) in step.
    const s = SURFACES.find((x) => x.id === surface) ?? SURFACES[0];
    for (const meta of Array.from(document.querySelectorAll('meta[name="theme-color"]'))) {
      if (pref === 'system') {
        const light = meta.getAttribute('media')?.includes('light');
        meta.setAttribute('content', light ? s.light.bg : s.dark.bg);
      } else {
        meta.setAttribute('content', s[effective].bg);
      }
    }
  }
}

function readPref(): ThemePref {
  const v = storageGet(KEY);
  return v === 'light' || v === 'system' ? v : 'dark';
}

function readAccent(): AccentId {
  const v = storageGet(ACCENT_KEY);
  return ACCENTS.find((a) => a.id === v)?.id ?? 'violet';
}

function readSurface(): SurfaceId {
  const v = storageGet(SURFACE_KEY);
  return SURFACES.find((s) => s.id === v)?.id ?? 'standard';
}

function prefersLight(): boolean {
  try {
    return matchMedia('(prefers-color-scheme: light)').matches;
  } catch {
    return false;
  }
}
