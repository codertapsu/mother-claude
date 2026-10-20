import { Injectable, NgZone, computed, effect, inject, signal, untracked } from '@angular/core';
import { Router } from '@angular/router';
import { Subscription } from 'rxjs';

import { ApiError, ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { LiveStream } from '../../core/live-stream';
import {
  AgentInfo,
  CommandInfo,
  ContinueBody,
  ImageAttachment,
  KeyName,
  LifecycleAction,
  MessageResult,
  PendingRef,
  PermissionMode,
  SessionDetail,
  SessionSummary,
  WorkflowRun,
} from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import { PendingService } from '../../core/pending.service';
import { RealtimeService } from '../../core/realtime.service';
import { SessionActionsService } from '../../core/session-actions.service';
import { TranscriptStore } from '../../core/transcript-store';
import { activityIsBusy } from '../../shared/util';

export type SessionTab = 'chat' | 'agents' | 'changes' | 'info';
export type PickerKind = 'model' | 'effort' | 'mode' | 'rename';

/** How often (in poll ticks, 2 s each) the detail refreshes while Limited. */
const DETAIL_POLL_TICKS = 5;

/**
 * Everything the session page's parts share, provided once per page:
 * the session (live store, falling back to the REST detail for sessions
 * outside the active scope), its detail (agents, workflows, tasks, queue),
 * the transcript store and the owned live stream, and the actions (send,
 * interrupt, pickers, rename, lifecycle, keys). Actions report their outcome
 * through NoticeService and return whether they worked.
 */
@Injectable()
export class SessionContext {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
  private pendings = inject(PendingService);
  private notices = inject(NoticeService);
  private auth = inject(AuthService);
  private actions = inject(SessionActionsService);
  private router = inject(Router);
  private zone = inject(NgZone);

  readonly id = signal('');
  private readonly live = computed(() => {
    const id = this.id();
    return this.realtime.sessions().find((s) => s.id === id);
  });
  readonly detail = signal<SessionDetail | null>(null);
  readonly session = computed<SessionSummary | undefined>(
    () => this.live() ?? this.detail() ?? undefined,
  );
  readonly caps = computed(() => this.session()?.control);
  readonly loading = signal(true);
  readonly loadError = signal('');
  /** The server said 404 (as opposed to not answering at all). */
  readonly notFound = signal(false);
  readonly store = signal<TranscriptStore | null>(null);
  readonly stream = signal<LiveStream | null>(null);

  readonly agents = signal<AgentInfo[]>([]);
  readonly workflows = signal<WorkflowRun[]>([]);
  readonly tasks = computed(() => this.detail()?.tasks ?? []);
  readonly queued = computed(() =>
    (this.detail()?.queuedMessages ?? []).filter((q) => q.status === 'queued'),
  );
  readonly pending = computed<PendingRef[]>(() => {
    const s = this.session();
    return s ? (s.pending ?? []).map((p) => ({ ...p, sessionId: s.id })) : [];
  });
  readonly busy = computed(() => {
    const s = this.session();
    return !!s && s.live && (s.status === 'working' || activityIsBusy(s.activity));
  });
  readonly ended = computed(() => {
    const s = this.session();
    return !!s && !s.live && ['ended', 'failed', 'stopped', 'idle'].includes(s.status);
  });
  readonly canAct = this.realtime.canAct;
  readonly canDangerous = this.auth.canDangerous;
  readonly isDesktop = this.auth.isDesktop;

  readonly tab = signal<SessionTab>('chat');
  /** A pending request to bring forward in the dock (`?pending=<id>`, e.g.
   * from a notification). Cleared once the dock showed it. */
  readonly focusPending = signal<string | null>(null);
  /** The composer asked the header to open a picker (`/model` etc.). */
  readonly openPicker = signal<PickerKind | null>(null);
  readonly terminalOpen = signal(false);
  /** The desktop's Move/Copy dialog is open, for the session or its whole
   * project (the header hosts it; the Info tab opens it too). */
  readonly moveCopy = signal<'session' | 'project' | null>(null);
  readonly composerFocused = signal(false);
  /** Text another part wants in the composer (e.g. unsent feedback). */
  readonly composerInsert = signal<{ text: string; at: number } | null>(null);
  /** `/` palette entries, loaded once per session. */
  readonly commands = signal<CommandInfo[] | null>(null);

  private subs = new Subscription();
  /** The `/commands` request in flight (a burst of keystrokes shares it). */
  private commandsLoading?: Promise<CommandInfo[]>;
  private tick = 0;
  private detailTimer?: ReturnType<typeof setTimeout>;

  constructor() {
    this.subs.add(
      this.realtime.agents$.subscribe((f) => {
        if (f.session !== this.id()) return;
        this.agents.set(f.agents ?? []);
        this.workflows.set(f.workflows ?? []);
      }),
    );
    this.subs.add(this.realtime.refetch$.subscribe(() => void this.refreshDetail()));
    this.subs.add(
      this.realtime.pollTick$.subscribe(() => {
        this.tick += 1;
        if (this.tick % DETAIL_POLL_TICKS === 0) void this.refreshDetail();
      }),
    );
    // New transcript records replace the live bubbles they finalise.
    effect(() => {
      const events = this.store()?.events() ?? [];
      untracked(() => {
        const stream = this.stream();
        if (stream && events.length) stream.settle(events.slice(-30));
      });
    });
    // Queue / agent / task counts moved: the detail's lists are stale.
    let lastSig = '';
    effect(() => {
      const s = this.live();
      const sig = s
        ? `${s.id}|${s.queued}|${s.agents?.running}|${s.agents?.backgroundRunning}|${s.agents?.workflowsRunning}|${s.pending?.length}`
        : '';
      if (!sig || sig === lastSig) return;
      const first = !lastSig || !lastSig.startsWith(`${s?.id}|`);
      lastSig = sig;
      if (!first) untracked(() => this.scheduleDetail());
    });
  }

  /** Switch to another session (the route reuses the page for /session/B).
   * Every session opens on Chat, where its requests and the composer are. */
  open(id: string): void {
    if (id === this.id()) return;
    this.close();
    this.id.set(id);
    this.tab.set('chat');
    this.focusPending.set(null);
    this.detail.set(null);
    this.loading.set(true);
    this.loadError.set('');
    this.notFound.set(false);
    this.agents.set([]);
    this.workflows.set([]);
    this.commands.set(null);
    this.commandsLoading = undefined;
    this.openPicker.set(null);
    this.terminalOpen.set(false);
    if (!id) return;
    this.store.set(this.realtime.openTranscript(id));
    this.stream.set(new LiveStream(this.realtime, id, this.zone));
    void this.refreshDetail();
  }

  destroy(): void {
    this.close();
    this.subs.unsubscribe();
  }

  private close(): void {
    clearTimeout(this.detailTimer);
    this.store()?.destroy();
    this.stream()?.destroy();
    this.store.set(null);
    this.stream.set(null);
  }

  private scheduleDetail(): void {
    clearTimeout(this.detailTimer);
    this.detailTimer = setTimeout(() => void this.refreshDetail(), 600);
  }

  async refreshDetail(): Promise<void> {
    const id = this.id();
    if (!id) return;
    try {
      const d = await this.api.getSession(id);
      if (id !== this.id()) return;
      this.detail.set(d);
      this.pendings.putFromDetail(d);
      this.agents.set(d.agentsDetail ?? []);
      this.workflows.set(d.workflows ?? []);
      this.realtime.mergeSession(d);
      this.loadError.set('');
      this.notFound.set(false);
    } catch (e) {
      if (id !== this.id()) return;
      if (!this.live()) {
        // An unreachable Mac is not a missing session (a cold start from a
        // notification while the Mac sleeps); a reconnect retries (refetch$).
        const missing = e instanceof ApiError && e.status === 404;
        this.notFound.set(missing);
        this.loadError.set(missing ? 'This session no longer exists.' : describeError(e).text);
      }
    } finally {
      if (id === this.id()) this.loading.set(false);
    }
  }

  loadCommands(): Promise<CommandInfo[]> {
    const have = this.commands();
    if (have) return Promise.resolve(have);
    if (this.commandsLoading) return this.commandsLoading;
    const id = this.id();
    const load = (async () => {
      let list: CommandInfo[];
      try {
        const res = await this.api.getCommands(id);
        list = (res.commands ?? []).filter((c) => c?.name);
      } catch {
        list = [];
      }
      if (id === this.id()) {
        this.commands.set(list);
        this.commandsLoading = undefined;
      }
      return list;
    })();
    this.commandsLoading = load;
    return load;
  }

  // ─────────────────────────────────────────────────────────── Actions

  /** Send a message. Resolves with the receipt, or null when it failed (the
   * caller keeps the text and offers Retry). */
  async send(text: string, images: ImageAttachment[] = []): Promise<MessageResult | null> {
    const id = this.id();
    try {
      const res = await this.api.sendMessage(id, {
        text,
        ...(images.length ? { images } : {}),
      });
      if (res?.delivery === 'queued' || res?.delivery === 'relayed') this.scheduleDetail();
      return res ?? { delivery: 'sent', messageId: '' };
    } catch (e) {
      this.notices.error(e, undefined, 'Not sent');
      return null;
    }
  }

  /** Ended session: resume it here with this prompt. */
  async resumeWith(prompt: string): Promise<boolean> {
    const s = this.session();
    if (!s) return false;
    const id = await this.actions.continueHere(s, { prompt });
    return !!id;
  }

  continueHere(body: ContinueBody = {}): Promise<string | null> {
    const s = this.session();
    return s ? this.actions.continueHere(s, body) : Promise.resolve(null);
  }

  async interrupt(): Promise<boolean> {
    const next = this.caps()?.interrupt === 'next-tool';
    return this.run(
      () => this.api.interrupt(this.id()),
      next ? 'Interrupt requested: it stops at the next tool call.' : 'Interrupted.',
      'Could not interrupt',
    );
  }

  setModel(model: string): Promise<boolean> {
    return this.run(() => this.api.setModel(this.id(), model), `Model: ${model}`, 'Model');
  }

  setEffort(effort: string): Promise<boolean> {
    return this.run(() => this.api.setEffort(this.id(), effort), `Effort: ${effort}`, 'Effort');
  }

  setUltracode(enabled: boolean): Promise<boolean> {
    return this.run(
      () => this.api.setUltracode(this.id(), enabled),
      enabled ? 'Ultracode on.' : 'Ultracode off.',
      'Ultracode',
    );
  }

  setThinking(enabled: boolean): Promise<boolean> {
    return this.run(
      () => this.api.setThinking(this.id(), enabled),
      enabled ? 'Thinking on.' : 'Thinking off.',
      'Thinking',
    );
  }

  setMode(mode: PermissionMode, label: string): Promise<boolean> {
    const onPrompt = this.caps()?.setMode === 'on-prompt';
    return this.run(
      () => this.api.setMode(this.id(), mode),
      onPrompt ? `Mode “${label}” applies with your next message.` : `Mode: ${label}`,
      'Mode',
    );
  }

  rename(name: string): Promise<boolean> {
    return this.run(() => this.api.rename(this.id(), name), 'Renamed.', 'Rename');
  }

  stopTask(taskId: string, label: string): Promise<boolean> {
    return this.run(
      () => this.api.stopTask(this.id(), taskId),
      `Asked “${label}” to stop.`,
      'Could not stop',
    );
  }

  async cancelQueued(messageId: string): Promise<boolean> {
    const ok = await this.run(
      () => this.api.cancelQueued(this.id(), messageId),
      'Taken back.',
      'Could not take it back',
    );
    if (ok) void this.refreshDetail();
    return ok;
  }

  sendKey(key: KeyName): Promise<boolean> {
    return this.run(() => this.api.sendKey(this.id(), key), '', `Key ${key}`);
  }

  async lifecycle(action: LifecycleAction): Promise<boolean> {
    const words: Record<LifecycleAction, string> = {
      stop: 'Stop requested.',
      respawn: 'Respawn requested.',
      rm: 'Removed.',
    };
    const ok = await this.run(() => this.api.lifecycle(this.id(), action), words[action], action);
    if (ok && action === 'rm') void this.router.navigate(['/']);
    return ok;
  }

  private async run(fn: () => Promise<unknown>, ok: string, what: string): Promise<boolean> {
    try {
      await fn();
      if (ok) this.notices.success(ok);
      return true;
    } catch (e) {
      this.notices.error(e, () => void this.run(fn, ok, what), what);
      return false;
    }
  }
}
