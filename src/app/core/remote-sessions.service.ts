import { Injectable, computed, effect, inject, untracked } from '@angular/core';

import { MachineScopes } from './machine-scope';
import { shortMachineId } from './machines';
import { MachinesService } from './machines.service';
import { PendingRef, SessionSummary } from './models';
import { RealtimeService } from './realtime.service';

/** Another computer, as a list of sessions names it. */
export interface MachineTag {
  id: string;
  name: string;
  online: boolean;
  /** When it went offline (ms), when known. */
  since: number | null;
}

/** Another computer and its sessions. */
export interface MachineSessions extends MachineTag {
  /** Its first list arrived (until then: "Loading DESKTOP-PC's sessions…"). */
  loaded: boolean;
  sessions: SessionSummary[];
}

/** A request waiting at another computer, with which one. */
export type RemotePendingRef = PendingRef & { machine: string };

/**
 * Other computers' sessions for the lists that show them beside this
 * computer's (Sessions, Inbox, the counts; docs/PLAN-MACHINES.md §6.1): from
 * the `remote` frames, named by `GET /api/machines`. None while no computer
 * is paired or Machines is off — the lists are then exactly this computer's.
 */
@Injectable({ providedIn: 'root' })
export class RemoteSessionsService {
  private readonly realtime = inject(RealtimeService);
  private readonly machines = inject(MachinesService);
  private readonly scopes = inject(MachineScopes);

  /** The computers whose sessions show, by name. */
  readonly list = computed<MachineSessions[]>(() => {
    const view = this.machines.view();
    if (view && !view.enabled) return [];
    const out: MachineSessions[] = [];
    for (const [id, m] of this.realtime.remote()) {
      const peer = view?.peers.find((p) => p.id === id);
      // Revoked meanwhile: its frames may not have said so yet.
      if (view && !peer) continue;
      out.push({
        id,
        name: peer?.name || shortMachineId(id),
        online: m.online,
        since: m.since ?? (m.online ? null : (peer?.lastSeen ?? null)),
        loaded: m.loaded,
        sessions: [...m.sessions.values()],
      });
    }
    return out.sort((a, b) => a.name.localeCompare(b.name));
  });
  readonly byId = computed(() => new Map(this.list().map((m) => [m.id, m])));
  /** Every one of their sessions (each with `machine`). */
  readonly sessions = computed(() => this.list().flatMap((m) => m.sessions));
  /** What waits on someone at the computers online now, oldest first. */
  readonly pending = computed<RemotePendingRef[]>(() =>
    this.list()
      .filter((m) => m.online)
      .flatMap((m) =>
        m.sessions.flatMap((s) =>
          (s.pending ?? []).map((p) => ({ ...p, sessionId: s.id, machine: m.id })),
        ),
      )
      .sort((a, b) => a.createdAt - b.createdAt),
  );

  constructor() {
    // Once another computer is heard: its name (`GET /api/machines`, which
    // the desktop reads anyway), and its scope, so its requests' frames are
    // kept from the first one on.
    effect(() => {
      const ids = [...this.realtime.remote().keys()];
      if (!ids.length) return;
      untracked(() => {
        this.machines.start();
        for (const id of ids) this.scopes.pending(id);
      });
    });
  }

  /** The tag of the computer `s` runs on (null: this one's own). */
  tagOf(s: Pick<SessionSummary, 'machine'>): MachineTag | null {
    return s.machine ? (this.byId().get(s.machine) ?? null) : null;
  }
}
