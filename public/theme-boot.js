/*
 * Mother Claude: the saved theme, accent, background and language, applied
 * to <html> before the first paint, so the page never flashes the default
 * dark theme first. ThemeService (src/app/core/theme.service.ts) and
 * useLanguage (src/app/core/i18n/i18n.ts) take over once the app starts;
 * keep the keys and values in step with them. A file rather than an inline
 * script: the Content-Security-Policy allows scripts from 'self' only.
 */
(function () {
  function get(key) {
    try {
      return localStorage.getItem(key);
    } catch (e) {
      return null;
    }
  }
  var root = document.documentElement;
  var theme = get('mc_theme');
  root.setAttribute('data-theme', theme === 'light' || theme === 'system' ? theme : 'dark');
  var accent = get('mc_accent');
  if (/^(blue|teal|green|orange|rose|graphite)$/.test(accent || '')) {
    root.setAttribute('data-accent', accent);
  }
  var surface = get('mc_surface');
  if (/^(midnight|black|warm)$/.test(surface || '')) {
    root.setAttribute('data-surface', surface);
  }
  if (get('mc_lang') === 'vi') root.setAttribute('lang', 'vi');
})();
