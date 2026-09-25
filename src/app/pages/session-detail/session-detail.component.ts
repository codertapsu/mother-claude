import { Component, DestroyRef, computed, effect, inject, signal, untracked } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { map } from 'rxjs';

import { ApiService, errorMessage } from '../../core/api.service';
import {
  GitOverview,
  LaunchOverrides,
  LifecycleAction,
  SessionDetail,
  SessionSummary,
} from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { TranscriptStore } from '../../core/transcript-store';
import { ActivityLineComponent } from '../../shared/activity-line.component';
import { ChannelBadgeComponent } from '../../shared/channel-badge.component';
import { ChipComponent } from '../../shared/chip.component';
import { ContextMeterComponent } from '../../shared/context-meter.component';
import { CostComponent } from '../../shared/cost.component';
import { parseUnifiedDiff, toSplitRows } from '../../shared/diff';
import { EventListComponent } from '../../shared/event-list.component';
import { IconComponent } from '../../shared/icon.component';
import { LaunchOptionsComponent } from '../../shared/launch-options.component';
import { PendingCardComponent } from '../../shared/pending-card.component';
import { StatusPillComponent } from '../../shared/status-pill.component';
import {
  baseName,
  effortLabel,
  groupByDir,
  modeLabel,
  sessionName,
  shortPath,
} from '../../shared/util';

const DIFF_VIEW_KEY = 'mc_diff_view';

/** localStorage can throw on locked-down phone browsers (blocked site data,
 * private-mode quotas) — the diff-view preference must degrade, not crash. */
function readDiffViewPref(): 'unified' | 'split' {
  try {
    return localStorage.getItem(DIFF_VIEW_KEY) === 'split' ? 'split' : 'unified';
  } catch {
    return 'unified';
  }
}

function writeDiffViewPref(mode: 'unified' | 'split'): void {
  try {
    localStorage.setItem(DIFF_VIEW_KEY, mode);
  } catch {
    /* in-memory only */
  }
}

/**
 * Session detail — an interim version on the v2 contract: header, pending
 * cards, the live transcript (TranscriptStore), a composer and the Changes
 * tab. The full rebuild (tabs, agents panel, pickers, command palette…) is
 * tracked separately.
 */
@Component({
  selector: 'mc-session-detail',
  imports: [
    RouterLink,
    FormsModule,
    ActivityLineComponent,
    ChannelBadgeComponent,
    ChipComponent,
    ContextMeterComponent,
    CostComponent,
    EventListComponent,
    IconComponent,
    LaunchOptionsComponent,
    PendingCardComponent,
    StatusPillComponent,
  ],
  templateUrl: './session-detail.component.html',
  styleUrl: './session-detail.component.scss',
})
export class SessionDetailComponent {
  private route = inject(ActivatedRoute);
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
  private router = inject(Router);

  // Reactive: "Continue here" may navigate /session/A → /session/B, and
  // Angular reuses this component instance across that navigation.
  protected readonly id = toSignal(this.route.paramMap.pipe(map((p) => p.get('id') ?? '')), {
    initialValue: this.route.snapshot.paramMap.get('id') ?? '',
  });
  private readonly live = computed(() => this.realtime.sessions().find((s) => s.id === this.id()));
  /** REST copy, for sessions outside the live store's scope (older ones). */
  private readonly detail = signal<SessionDetail | null>(null);
  protected readonly session = computed<SessionSummary | undefined>(
    () => this.live() ?? this.detail() ?? undefined,
  );
  protected readonly loading = signal(true);
  protected readonly store = signal<TranscriptStore | null>(null);

  protected readonly tab = signal<'chat' | 'changes'>('chat');
  protected readonly diff = signal<GitOverview | null>(null);
  protected readonly patch = signal<{ path: string; raw: string } | null>(null);
  protected readonly diffView = signal<'unified' | 'split'>(readDiffViewPref());
  protected readonly unifiedRows = computed(() => {
    const p = this.patch();
    return p ? parseUnifiedDiff(p.raw) : [];
  });
  protected readonly splitRows = computed(() => toSplitRows(this.unifiedRows()));
  protected readonly view = signal<'list' | 'tree'>('list');
  protected readonly grouped = computed(() => groupByDir(this.diff()?.files ?? []));
  protected readonly changeCount = computed(() => this.diff()?.files.length ?? 0);

  protected readonly message = signal('');
  protected readonly busy = signal(false);
  protected readonly notice = signal('');
  private continueOverrides: LaunchOverrides = {};
  private patchReq = 0;

  protected readonly name = sessionName;
  protected readonly base = baseName;
  protected readonly short = shortPath;
  protected readonly mode = modeLabel;
  protected readonly effort = effortLabel;

  constructor() {
    const destroyRef = inject(DestroyRef);
    const sub = this.realtime.refetch$.subscribe(() => {
      if (!this.live()) void this.loadDetail(this.id());
    });
    destroyRef.onDestroy(() => {
      sub.unsubscribe();
      this.store()?.destroy();
    });

    // (Re)load whenever the routed session changes.
    effect(() => {
      const id = this.id();
      untracked(() => {
        this.store()?.destroy();
        this.store.set(id ? this.realtime.openTranscript(id) : null);
        this.detail.set(null);
        this.diff.set(null);
        this.patch.set(null);
        this.message.set('');
        this.notice.set('');
        this.continueOverrides = {};
        void this.loadDetail(id);
        void this.loadDiff(id, true);
      });
    });
  }

  private async loadDetail(id: string): Promise<void> {
    if (!id) return;
    this.loading.set(true);
    try {
      const d = await this.api.getSession(id);
      if (id === this.id()) this.detail.set(d);
    } catch (e) {
      if (id === this.id() && !this.live()) this.notice.set(errorMessage(e));
    } finally {
      if (id === this.id()) this.loading.set(false);
    }
  }

  // ────────────────────────────────────────────────────────── Composer

  async send(): Promise<void> {
    const text = this.message().trim();
    if (!text) return;
    const ok = await this.guard(async () => {
      const res = await this.api.sendMessage(this.id(), { text });
      return res?.delivery === 'queued'
        ? 'Queued — delivered when Claude is ready.'
        : res?.delivery === 'relayed'
          ? 'Sent — delivered at the next tool call or when the session goes idle.'
          : 'Sent.';
    });
    if (ok) this.message.set('');
  }

  /** Enter adds a newline (touch keyboards); Cmd/Ctrl+Enter sends. */
  onComposerKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void this.send();
    }
  }

  async interrupt(): Promise<void> {
    await this.guard(async () => {
      await this.api.interrupt(this.id());
      return 'Interrupt sent.';
    });
  }

  onLaunchOverrides(overrides: LaunchOverrides): void {
    this.continueOverrides = overrides;
  }

  /** Take over (ended: resume in place) or fork (live elsewhere) as owned. */
  async continueSession(): Promise<void> {
    const s = this.session();
    await this.guard(async () => {
      const id = this.id();
      const res = await this.api.continueSession(id, {
        ...(s?.live ? { fork: true } : {}),
        ...this.continueOverrides,
      });
      if (res?.id && res.id !== id) void this.router.navigate(['/session', res.id]);
      return 'You can now drive this session from here.';
    });
  }

  async lifecycle(action: LifecycleAction): Promise<void> {
    await this.guard(async () => {
      await this.api.lifecycle(this.id(), action);
      return `${action} requested.`;
    });
  }

  // ─────────────────────────────────────────────────────────── Changes

  async openChanges(): Promise<void> {
    this.tab.set('changes');
    if (!this.diff()) await this.loadDiff(this.id(), false);
  }

  private async loadDiff(id: string, silent: boolean): Promise<void> {
    if (!id) return;
    try {
      const overview = await this.api.getDiff(id);
      if (id === this.id()) this.diff.set(overview);
    } catch (e) {
      if (!silent && id === this.id()) this.notice.set(`Could not load diff: ${errorMessage(e)}`);
    }
  }

  async showPatch(path: string): Promise<void> {
    const id = this.id();
    const req = ++this.patchReq;
    try {
      const res = await this.api.getFilePatch(id, path);
      if (id !== this.id() || req !== this.patchReq) return;
      this.patch.set({ path, raw: res.patch ?? '' });
    } catch (e) {
      if (id === this.id() && req === this.patchReq) {
        this.notice.set(`Could not load patch: ${errorMessage(e)}`);
      }
    }
  }

  setDiffView(mode: 'unified' | 'split'): void {
    this.diffView.set(mode);
    writeDiffViewPref(mode);
  }

  /** Run an action once (double taps are ignored) and show its outcome. */
  private async guard(fn: () => Promise<string>): Promise<boolean> {
    if (this.busy()) return false;
    this.busy.set(true);
    this.notice.set('');
    try {
      this.notice.set(await fn());
      return true;
    } catch (e) {
      this.notice.set(`Failed: ${errorMessage(e)}`);
      return false;
    } finally {
      this.busy.set(false);
    }
  }
}
