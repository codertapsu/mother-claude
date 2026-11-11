import { Injectable, computed, effect, inject, signal } from '@angular/core';

import { tr } from './i18n/i18n';
import { NoticeService } from './notice.service';
import { RealtimeService } from './realtime.service';
import { RemoteSessionsService } from './remote-sessions.service';
import { storageGet, storageSet } from './platform';

const ALERTS_KEY = 'mc_alerts';
const BASE_TITLE = 'Mother Claude';

/**
 * "Something needs you" without push: the pending count in document.title
 * (and the app badge where the Badging API exists), plus an optional short
 * vibration + tone when a new request arrives while the app is open. This is
 * the fallback on LAN (self-signed certificates can't do Web Push).
 */
@Injectable({ providedIn: 'root' })
export class AttentionService {
  private realtime = inject(RealtimeService);
  private notices = inject(NoticeService);
  private remote = inject(RemoteSessionsService);

  /** Requests someone can answer from here (observed ones wait at the
   * desk), other computers' online ones included. */
  readonly pendingCount = computed(
    () =>
      this.realtime.pendingItems().filter((p) => p.answerable).length +
      this.remote.pending().filter((p) => p.answerable).length,
  );
  readonly alertsEnabled = signal(storageGet(ALERTS_KEY) !== '0');

  private readonly seen = new Set<string>();
  private primed = false;
  private audio?: AudioContext;

  constructor() {
    effect(() => {
      const n = this.pendingCount();
      document.title = n > 0 ? `(${n}) ${BASE_TITLE}` : BASE_TITLE;
      const nav = navigator as Navigator & {
        setAppBadge?: (n?: number) => Promise<void>;
        clearAppBadge?: () => Promise<void>;
      };
      try {
        if (n > 0) void nav.setAppBadge?.(n).catch(() => undefined);
        else void nav.clearAppBadge?.().catch(() => undefined);
      } catch {
        /* Badging API unavailable */
      }
    });

    effect(() => {
      const items = this.realtime.pendingItems();
      const loaded = this.realtime.sessionsLoaded();
      const fresh = [];
      for (const item of items) {
        if (!this.seen.has(item.id)) {
          this.seen.add(item.id);
          fresh.push(item);
        }
      }
      // The first snapshot is history, not news.
      if (!loaded) return;
      if (!this.primed) {
        this.primed = true;
        return;
      }
      if (!fresh.length) return;
      const first = fresh[0];
      const session = this.realtime.sessions().find((s) => s.id === first.sessionId);
      const what =
        first.kind === 'question'
          ? tr('Question')
          : first.kind === 'plan'
            ? tr('Plan to review')
            : first.kind === 'elicitation'
              ? tr('Input needed')
              : tr('Permission needed');
      const name = session?.title || session?.name || tr('a session');
      const params = { what, name, title: first.title, n: fresh.length - 1 };
      this.notices.announce(
        fresh.length > 1
          ? tr('{what} in {name}: {title} (and {n} more)', params)
          : tr('{what} in {name}: {title}', params),
      );
      if (this.alertsEnabled()) this.alert();
    });
  }

  setAlerts(on: boolean): void {
    this.alertsEnabled.set(on);
    storageSet(ALERTS_KEY, on ? '1' : '0');
  }

  private alert(): void {
    try {
      // Browsers refuse (and log) vibration before the first tap.
      if (navigator.userActivation?.hasBeenActive ?? true) navigator.vibrate?.([80, 60, 80]);
    } catch {
      /* not supported */
    }
    try {
      const Ctx =
        window.AudioContext ??
        (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!Ctx) return;
      this.audio ??= new Ctx();
      const ctx = this.audio;
      if (ctx.state === 'suspended') void ctx.resume().catch(() => undefined);
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(880, ctx.currentTime);
      osc.frequency.setValueAtTime(1320, ctx.currentTime + 0.09);
      gain.gain.setValueAtTime(0.0001, ctx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.08, ctx.currentTime + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.25);
      osc.connect(gain).connect(ctx.destination);
      osc.start();
      osc.stop(ctx.currentTime + 0.26);
    } catch {
      /* audio blocked until a user gesture — the title badge still shows */
    }
  }
}
