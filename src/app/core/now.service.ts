import { Injectable, NgZone, inject, signal } from '@angular/core';

/**
 * One app-wide clock for "12s ago" / "running 3m" labels. Ticks every second
 * outside the Angular zone (only views that read `now()` refresh) and pauses
 * while the page is hidden.
 */
@Injectable({ providedIn: 'root' })
export class NowService {
  private zone = inject(NgZone);
  private readonly nowS = signal(Date.now());
  readonly now = this.nowS.asReadonly();
  private timer?: ReturnType<typeof setInterval>;

  constructor() {
    this.zone.runOutsideAngular(() => {
      const start = () => {
        clearInterval(this.timer);
        this.nowS.set(Date.now());
        this.timer = setInterval(() => this.nowS.set(Date.now()), 1000);
      };
      document.addEventListener('visibilitychange', () => {
        if (document.visibilityState === 'visible') start();
        else clearInterval(this.timer);
      });
      start();
    });
  }
}
