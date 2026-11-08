import { TestBed } from '@angular/core/testing';

import { ThemeService } from './theme.service';

describe('ThemeService', () => {
  const root = document.documentElement;
  const settle = () => TestBed.tick();

  beforeEach(() => {
    localStorage.removeItem('mc_theme');
    localStorage.removeItem('mc_accent');
    localStorage.removeItem('mc_surface');
    TestBed.configureTestingModule({});
  });

  afterEach(() => {
    delete root.dataset['theme'];
    delete root.dataset['accent'];
    delete root.dataset['surface'];
    localStorage.removeItem('mc_theme');
    localStorage.removeItem('mc_accent');
    localStorage.removeItem('mc_surface');
  });

  it('is dark with the Violet accent and the Standard background by default', () => {
    const theme = TestBed.inject(ThemeService);
    settle();
    expect(theme.pref()).toBe('dark');
    expect(theme.effective()).toBe('dark');
    expect(root.dataset['theme']).toBe('dark');
    expect(root.dataset['accent']).toBeUndefined();
    expect(root.dataset['surface']).toBeUndefined();
  });

  it('applies and remembers a theme, an accent and a background', () => {
    const theme = TestBed.inject(ThemeService);
    theme.set('light');
    theme.setAccent('teal');
    theme.setSurface('midnight');
    settle();
    expect(root.dataset['theme']).toBe('light');
    expect(root.dataset['accent']).toBe('teal');
    expect(root.dataset['surface']).toBe('midnight');
    expect(localStorage.getItem('mc_theme')).toBe('light');
    expect(localStorage.getItem('mc_accent')).toBe('teal');
    expect(localStorage.getItem('mc_surface')).toBe('midnight');
    // The page's colours follow.
    expect(getComputedStyle(root).getPropertyValue('--accent').trim()).toBe('#0f766e');
    expect(getComputedStyle(root).getPropertyValue('--bg').trim()).toBe('#f2f5fa');

    theme.setAccent('violet');
    theme.setSurface('standard');
    settle();
    expect(root.dataset['accent']).toBeUndefined();
    expect(root.dataset['surface']).toBeUndefined();
  });

  it('ignores a stored value it does not know', () => {
    localStorage.setItem('mc_theme', 'sepia');
    localStorage.setItem('mc_accent', 'chartreuse');
    localStorage.setItem('mc_surface', 'neon');
    const theme = TestBed.inject(ThemeService);
    expect(theme.pref()).toBe('dark');
    expect(theme.accent()).toBe('violet');
    expect(theme.surface()).toBe('standard');
  });
});
