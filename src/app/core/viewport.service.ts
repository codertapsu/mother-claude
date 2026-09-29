import { Injectable, NgZone, inject, signal } from '@angular/core';

const NARROW = '(max-width: 359px)';

function media(query: string): MediaQueryList | null {
  try {
    return typeof matchMedia === 'function' ? matchMedia(query) : null;
  } catch {
    return null;
  }
}

/**
 * The visible viewport, for layouts that must fit above the on-screen
 * keyboard. `100dvh` does not shrink for the iOS keyboard, so this keeps two
 * CSS variables on <html> in sync with `window.visualViewport`:
 *
 * - `--vvh`: the visible height in px (fallback in CSS: `100dvh`).
 * - `--vv-top`: how far the visual viewport is scrolled down inside the
 *   layout viewport (iOS scrolls the page when an input takes focus).
 *
 * Also exposes the pointer type (Enter behaviour in the composer) and
 * phone-width flags.
 */
@Injectable({ providedIn: 'root' })
export class ViewportService {
  private zone = inject(NgZone);

  /** Touch-first device: Enter inserts a newline, a Send button sends. */
  readonly coarse = signal(media('(pointer: coarse)')?.matches ?? false);
  /** Phone layout (< 768 px). */
  readonly phone = signal(media('(max-width: 767px)')?.matches ?? false);
  /** The smallest phones (< 360 px, or a large display zoom): lists pack
   * their chips tighter. */
  readonly narrow = signal(media(NARROW)?.matches ?? false);
  /** The on-screen keyboard (or anything else) is covering a big part of the
   * layout viewport. */
  readonly keyboardOpen = signal(false);

  constructor() {
    const root = document.documentElement;
    const vv = window.visualViewport;
    let frame = 0;
    const apply = () => {
      frame = 0;
      const h = vv?.height ?? window.innerHeight;
      const top = vv?.offsetTop ?? 0;
      root.style.setProperty('--vvh', `${Math.round(h)}px`);
      root.style.setProperty('--vv-top', `${Math.round(top)}px`);
      const open = window.innerHeight - h > 120;
      if (open !== this.keyboardOpen()) this.zone.run(() => this.keyboardOpen.set(open));
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(apply);
    };
    this.zone.runOutsideAngular(() => {
      vv?.addEventListener('resize', schedule);
      vv?.addEventListener('scroll', schedule);
      window.addEventListener('resize', schedule);
      apply();
    });
    const watch = (q: MediaQueryList | null, set: (v: boolean) => void) =>
      q?.addEventListener?.('change', (e) => this.zone.run(() => set(e.matches)));
    watch(media('(pointer: coarse)'), (v) => this.coarse.set(v));
    watch(media('(max-width: 767px)'), (v) => this.phone.set(v));
    watch(media(NARROW), (v) => this.narrow.set(v));
  }
}
