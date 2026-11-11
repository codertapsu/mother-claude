import {
  EnvironmentInjector,
  Injectable,
  Injector,
  Provider,
  ProviderToken,
  Signal,
  computed,
  inject,
} from '@angular/core';
import { ActivatedRoute } from '@angular/router';
import { Observable, filter, map, merge } from 'rxjs';

import { ApiError, ApiService, MachineApiError } from './api.service';
import { DefaultsService } from './defaults.service';
import { computerNoun, tr } from './i18n/i18n';
import { MachineContext } from './machine-context';
import {
  AgentsFrame,
  PendingFrame,
  PendingResolvedFrame,
  SessionSummary,
  SessionsFrame,
  StreamFrame,
  TranscriptFrame,
  TranscriptResetFrame,
} from './models';
import { PendingService } from './pending.service';
import { RealtimeService, RefetchReason, RemoteInner } from './realtime.service';
import { SessionActionsService } from './session-actions.service';
import { SESSION_MACHINE } from './session-links';

// Another computer's sessions, driven from here (docs/PLAN-MACHINES.md §6.2).
// Each paired computer gets a scope: the very services this computer's
// session pages use — the REST client, live updates, pending requests,
// session actions, launch defaults — bound to that computer. Its requests go
// through this computer's proxy (`/api/machines/<id>/proxy/…`) and its live
// updates are the `remote` frames, so the session page, the Inbox's cards
// and New session work as they do here, without a branch of their own.

/** Another computer's refusal, in words naming it ('' for one that needs
 * none: those keep the server's own). */
export function machineErrorText(
  e: ApiError,
  m: Pick<MachineContext, 'name' | 'mayDangerous'>,
): string {
  const name = m.name();
  const computer = computerNoun();
  switch (e.code) {
    case 'machine_unreachable':
      return tr("{name} can't be reached right now.", { name });
    case 'grant_missing':
      switch (e.detail('grant')) {
        case 'view':
          return tr("{name} doesn't let this {computer} see its sessions.", { name, computer });
        case 'control':
          return tr('{name} lets this {computer} watch its sessions, not drive them.', {
            name,
            computer,
          });
        case 'dangerous':
          return tr("{name} doesn't let this {computer} take dangerous actions there.", {
            name,
            computer,
          });
        default:
          return tr("{name} hasn't given this {computer} that permission.", { name, computer });
      }
    case 'dangerous_blocked':
      return m.mayDangerous()
        ? tr('Only allowed at {name}, or from a device allowed to take dangerous actions.', {
            name,
          })
        : tr("{name} doesn't let this {computer} take dangerous actions there.", {
            name,
            computer,
          });
    case 'untrusted_cwd':
      return tr("{name} doesn't trust this folder yet: open it there with Claude Code once.", {
        name,
      });
    case 'machines_unknown':
      return tr('{name} is no longer paired with this {computer}.', { name, computer });
    case 'machine_refused':
      return tr('{name} refused this {computer}: check on {name} that they are still paired.', {
        name,
        computer,
      });
    default:
      return '';
  }
}

/** The REST client of another computer's sessions: the same calls, sent
 * through this computer's proxy. */
@Injectable()
export class MachineApiService extends ApiService {
  private readonly where = inject(MachineContext);
  protected override readonly scope = `/machines/${encodeURIComponent(this.where.id)}/proxy`;

  protected override unauthorized(): void {
    // A refusal on the way to another computer says nothing about whether
    // this device is still paired here (this computer's own calls tell).
  }

  protected override failure(e: ApiError): ApiError {
    const text = machineErrorText(e, this.where);
    return text ? new MachineApiError(e, text, this.where.id) : e;
  }
}

/**
 * Live updates of another computer's sessions, shaped as this computer's
 * own: its sessions, frames and subscriptions come from the one socket's
 * `remote` frames (`RealtimeService.remote`, `remote$`, `subscribeRemote`).
 * It never opens a socket; what it does not override stays empty.
 */
@Injectable()
export class MachineRealtime extends RealtimeService {
  private readonly host = inject(RealtimeService, { skipSelf: true });
  private readonly where = inject(MachineContext);
  private readonly own = computed(() => this.host.remote().get(this.where.id));
  private readonly frames: Observable<RemoteInner> = this.host.remote$.pipe(
    filter((e) => e.machine === this.where.id),
    map((e) => e.frame),
  );

  override readonly connection = this.host.connection;
  /** Sending and answering work while this computer is live and the other
   * one online. */
  override readonly canAct = computed(() => this.host.canAct() && this.where.online());
  override readonly polling = this.host.polling;
  override readonly lastDataAt = this.host.lastDataAt;
  override readonly sessions: Signal<SessionSummary[]> = computed(() => [
    ...(this.own()?.sessions.values() ?? []),
  ]);
  override readonly sessionsLoaded: Signal<boolean> = computed(() => !!this.own()?.loaded);

  override readonly sessions$ = this.kind<SessionsFrame>('sessions');
  override readonly transcript$ = this.kind<TranscriptFrame>('transcript');
  override readonly transcriptReset$ = this.kind<TranscriptResetFrame>('transcriptReset');
  override readonly stream$ = this.kind<StreamFrame>('stream');
  override readonly pending$ = this.kind<PendingFrame>('pending');
  override readonly pendingResolved$ = this.kind<PendingResolvedFrame>('pendingResolved');
  override readonly agents$ = this.kind<AgentsFrame>('agents');
  /** Its views refetch when this computer's do, and when it is back or
   * dropped frames for us. */
  override readonly refetch$: Observable<RefetchReason> = merge(
    this.host.refetch$,
    this.frames.pipe(
      filter((f) => f.t === 'online' || f.t === 'resync'),
      map((f): RefetchReason => (f.t === 'online' ? 'reconnect' : 'resync')),
    ),
  );
  override readonly pollTick$ = this.host.pollTick$;

  override async start(): Promise<void> {
    // This computer's socket carries every computer's frames.
  }

  override restart(): void {
    // As start().
  }

  override stop(): void {
    // As start().
  }

  override reconnectNow(): void {
    this.host.reconnectNow();
  }

  override session(id: string): Signal<SessionSummary | undefined> {
    return computed(() => this.own()?.sessions.get(id));
  }

  override mergeSession(s: SessionSummary): void {
    this.host.mergeRemoteSession(this.where.id, s);
  }

  override subscribe(sessionId: string, agentId?: string): () => void {
    return this.host.subscribeRemote(this.where.id, sessionId, agentId);
  }

  override subscribeFleet(): () => void {
    return () => undefined;
  }

  /** Its requests reach every dashboard already. */
  override subscribeInbox(): () => void {
    return () => undefined;
  }

  override noteTail(sessionId: string, tail: string | undefined, agentId?: string): void {
    this.host.noteRemoteTail(this.where.id, sessionId, tail, agentId);
  }

  private kind<T extends RemoteInner>(t: T['t']): Observable<T> {
    return this.frames.pipe(filter((f): f is T => f.t === t));
  }
}

function scopeProviders(machine: string): Provider[] {
  return [
    { provide: SESSION_MACHINE, useValue: machine },
    { provide: MachineContext, useClass: MachineContext },
    { provide: ApiService, useClass: MachineApiService },
    { provide: RealtimeService, useClass: MachineRealtime },
    { provide: PendingService, useClass: PendingService },
    { provide: SessionActionsService, useClass: SessionActionsService },
    { provide: DefaultsService, useClass: DefaultsService },
  ];
}

/** Each paired computer's scope, made once and kept: its session pages, the
 * Inbox's cards and New session share its services (answers in flight,
 * resolutions, the defaults read). */
@Injectable({ providedIn: 'root' })
export class MachineScopes {
  private readonly env = inject(EnvironmentInjector);
  private readonly made = new Map<string, Injector>();

  get<T>(machine: string, token: ProviderToken<T>): T {
    let scope = this.made.get(machine);
    if (!scope) {
      scope = Injector.create({
        providers: scopeProviders(machine),
        parent: this.env,
        name: `machine ${machine}`,
      });
      this.made.set(machine, scope);
    }
    return scope.get(token);
  }

  context(machine: string): MachineContext {
    return this.get(machine, MachineContext);
  }

  api(machine: string): ApiService {
    return this.get(machine, ApiService);
  }

  pending(machine: string): PendingService {
    return this.get(machine, PendingService);
  }
}

/**
 * Providers for another computer's session page (the route's `:machine`):
 * everything inside it uses that computer's scope.
 */
export function machineScopeProviders(): Provider[] {
  const machine = () => inject(ActivatedRoute).snapshot.paramMap.get('machine') ?? '';
  const scoped = <T>(token: ProviderToken<T>): Provider => ({
    provide: token,
    useFactory: () => inject(MachineScopes).get(machine(), token),
  });
  return [
    { provide: SESSION_MACHINE, useFactory: machine },
    scoped(MachineContext),
    scoped(ApiService),
    scoped(RealtimeService),
    scoped(PendingService),
    scoped(SessionActionsService),
    scoped(DefaultsService),
  ];
}
