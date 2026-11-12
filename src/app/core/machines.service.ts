import { Injectable, NgZone, computed, inject, signal } from '@angular/core';

import { ApiService, describeError } from './api.service';
import {
  MachinesFrame,
  MachinesView,
  PeerMachine,
  SetupView,
  healthAlarm,
  pairingLive,
  shortMachineId,
} from './machines';
import { RealtimeService } from './realtime.service';

/** How often a live pairing is asked about while a Machines screen shows. */
const PAIRING_POLL_MS = 1000;

/**
 * Machines' state for every screen that shows it (the Machines page, the
 * setup, the sidebar's health dot): `GET /api/machines`, refetched on each
 * `machines` frame; the setup's checks while a screen asks for them; and,
 * while a pairing is in progress and a Machines screen is visible, a poll
 * every second (frames come too, but a pairing must never look stuck).
 */
@Injectable({ providedIn: 'root' })
export class MachinesService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);
  private readonly zone = inject(NgZone);

  readonly view = signal<MachinesView | null>(null);
  readonly setup = signal<SetupView | null>(null);
  /** The last load's failure ('' when it worked). */
  readonly error = signal('');
  readonly pairing = computed(() => this.view()?.pairing ?? null);
  /** A required check fails while Machines is on or being set up. */
  readonly alarm = computed(() => healthAlarm(this.view()));

  /** Something shows Machines (the shell's summary, or a page). */
  private started = false;
  /** Pages showing Machines now (the pairing poll runs while > 0). */
  private watchers = 0;
  /** Pages showing the setup's checks now. */
  private setupWatchers = 0;
  private timer?: ReturnType<typeof setInterval>;
  private loading: Promise<MachinesView | null> | null = null;
  private again = false;

  constructor() {
    this.realtime.machines$.subscribe((f) => this.onFrame(f));
    this.realtime.refetch$.subscribe(() => {
      if (!this.started) return;
      void this.load();
      if (this.setupWatchers) void this.loadSetup();
    });
  }

  /** The shell: keep the summary (the health dot) current. */
  start(): void {
    if (this.started) return;
    this.started = true;
    void this.load();
  }

  /** A page shows Machines: returns its release. */
  watch(): () => void {
    this.started = true;
    this.watchers += 1;
    this.ensureTimer();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.watchers -= 1;
      if (this.watchers <= 0) this.stopTimer();
    };
  }

  /** A page shows the setup's checks: they follow the frames too. */
  watchSetup(): () => void {
    this.setupWatchers += 1;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.setupWatchers -= 1;
    };
  }

  /** `GET /api/machines` (one at a time; a request meanwhile asks again). */
  load(): Promise<MachinesView | null> {
    if (this.loading) {
      this.again = true;
      return this.loading;
    }
    this.loading = this.fetch().finally(() => {
      this.loading = null;
      if (this.again) {
        this.again = false;
        void this.load();
      }
    });
    return this.loading;
  }

  private async fetch(): Promise<MachinesView | null> {
    try {
      const v = await this.api.getMachines();
      this.view.set(v);
      this.error.set('');
      return v;
    } catch (e) {
      this.error.set(describeError(e).text);
      return null;
    }
  }

  /** The setup's checks (`fresh` runs them again first). */
  async loadSetup(fresh = false): Promise<SetupView | null> {
    try {
      const s = await this.api.getMachinesSetup(fresh);
      this.setup.set(s);
      return s;
    } catch {
      return null;
    }
  }

  /** A mutation's answer is the new view. */
  apply(v: MachinesView | null | undefined): void {
    if (v) this.view.set(v);
  }

  /** A paired computer, once the view is known. */
  peer(id: string): PeerMachine | undefined {
    return this.view()?.peers.find((p) => p.id === id);
  }

  /** A paired computer's name (the start of its id until the view is
   * known). Reactive. */
  nameOf(id: string): string {
    return this.peer(id)?.name || shortMachineId(id);
  }

  private onFrame(f: MachinesFrame): void {
    // Code sync's and copies' frames are `SyncService`'s and
    // `CopiesService`'s: nothing here changed.
    if (!this.started || f.what === 'sync' || f.what === 'copies') return;
    void this.load();
    if (this.setupWatchers && f.what !== 'pairing' && f.what !== 'transport') {
      void this.loadSetup();
    }
  }

  private ensureTimer(): void {
    if (this.timer) return;
    this.zone.runOutsideAngular(() => {
      this.timer = setInterval(() => {
        if (
          this.watchers > 0 &&
          document.visibilityState === 'visible' &&
          pairingLive(this.view()?.pairing)
        ) {
          this.zone.run(() => void this.load());
        }
      }, PAIRING_POLL_MS);
    });
  }

  private stopTimer(): void {
    clearInterval(this.timer);
    this.timer = undefined;
  }
}
