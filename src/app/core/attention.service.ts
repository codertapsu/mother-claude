import { Injectable, computed, effect, inject, signal } from '@angular/core';

import { RealtimeService } from './realtime.service';
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

  readonly pendingCount = computed(() => this.realtime.pendingItems().length);
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
      let fresh = false;
      for (const item of items) {
        if (!this.seen.has(item.id)) {
          this.seen.add(item.id);
          fresh = true;
        }
      }
      // The first snapshot is history, not news.
      if (!loaded) return;
      if (!this.primed) {
        this.primed = true;
        return;
      }
      if (fresh && this.alertsEnabled()) this.alert();
    });
  }

  setAlerts(on: boolean): void {
    this.alertsEnabled.set(on);
    storageSet(ALERTS_KEY, on ? '1' : '0');
  }

  private alert(): void {
    try {
      navigator.vibrate?.([80, 60, 80]);
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
