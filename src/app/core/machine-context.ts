import { Injectable, computed, inject } from '@angular/core';

import { computerNoun, tr } from './i18n/i18n';
import { grantedByPeer } from './machines';
import { MachinesService } from './machines.service';
import { RealtimeService } from './realtime.service';
import { SESSION_MACHINE } from './session-links';

/**
 * Another computer, as its session page (or one of its requests in a list)
 * sees it: its name, whether it is online, and what it lets this computer do
 * (docs/PLAN-MACHINES.md §6.2). Provided only in that computer's scope
 * (`machine-scope.ts`); inject it optionally — none means this computer's
 * own session.
 */
@Injectable()
export class MachineContext {
  /** The other computer's id. */
  readonly id = inject(SESSION_MACHINE) ?? '';
  /** This computer's live updates (never the scope's own facade). */
  private readonly host = inject(RealtimeService, { skipSelf: true });
  private readonly machines = inject(MachinesService);

  private readonly state = computed(() => this.host.remote().get(this.id));
  private readonly peer = computed(() => this.machines.peer(this.id));

  readonly name = computed(() => this.machines.nameOf(this.id));
  /** The link to it is up: its frames say so, else the Machines view. */
  readonly online = computed(() => this.state()?.online ?? this.peer()?.online ?? true);
  /** When it went offline (ms), when known. */
  readonly since = computed(() => this.state()?.since ?? this.peer()?.lastSeen ?? null);
  /** It lets this computer drive its sessions (not known yet: yes; it
   * decides, and says so if not). */
  readonly mayDrive = computed(() => grantedByPeer(this.peer(), 'control') !== false);
  /** It lets this computer take dangerous actions there. */
  readonly mayDangerous = computed(() => grantedByPeer(this.peer(), 'dangerous') !== false);
  /** Why nothing can be sent to its sessions from here ('' when it can). */
  readonly blocked = computed(() => {
    const name = this.name();
    if (!this.online()) return tr('Sending waits until {name} is back.', { name });
    if (!this.mayDrive()) {
      return tr('{name} lets this {computer} watch its sessions, not drive them.', {
        name,
        computer: computerNoun(),
      });
    }
    return '';
  });
}
