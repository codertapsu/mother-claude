import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiService } from './api.service';
import { PendingPair } from './models';
import { NowService } from './now.service';
import { RealtimeService } from './realtime.service';

/** A pair request that can still be approved. */
export function isStillPending(p: PendingPair, now: number): boolean {
  return (p.status ?? 'pending') === 'pending' && !(p.expiresAt && p.expiresAt <= now);
}

/**
 * Devices waiting for the desktop to approve their pairing (desktop only:
 * `GET /api/devices` and the `pairRequest` frames are the desktop's): the
 * Inbox lists them and counts them, the shell's dialog asks for the typed
 * number. Loaded once started, again on every `devices` frame (a request
 * settled here, on another window or by expiring) and after a reconnect;
 * a `pairRequest` frame adds one at once.
 */
@Injectable({ providedIn: 'root' })
export class PairRequestsService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);
  private readonly clock = inject(NowService);

  private readonly all = signal<PendingPair[]>([]);
  /** The requests still waiting, oldest first (the same list while none
   * came or went: the clock only drops the expired). */
  readonly pending = computed(
    () => {
      const now = this.clock.now();
      return this.all()
        .filter((p) => isStillPending(p, now))
        .sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
    },
    { equal: (a, b) => a.length === b.length && a.every((p, i) => p === b[i]) },
  );

  private started = false;

  constructor() {
    this.realtime.pairRequest$.subscribe((f) => {
      if (!this.started || !f.pending?.id) return;
      const p = f.pending;
      this.all.update((list) =>
        list.some((x) => x.id === p.id) ? list.map((x) => (x.id === p.id ? p : x)) : [...list, p],
      );
    });
    this.realtime.devices$.subscribe(() => {
      if (this.started) void this.load();
    });
    this.realtime.refetch$.subscribe(() => {
      if (this.started) void this.load();
    });
  }

  /** Follow the requests (the desktop's shell, once paired). */
  start(): void {
    if (this.started) return;
    this.started = true;
    void this.load();
  }

  /** One was approved or denied here: gone without waiting for the frame. */
  settled(id: string): void {
    this.all.update((list) => list.filter((p) => p.id !== id));
  }

  private async load(): Promise<void> {
    try {
      this.all.set((await this.api.listDevices()).pending ?? []);
    } catch {
      // Unknown right now: the frames still bring new requests.
    }
  }
}
