import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';

import { ApiService } from '../../core/api.service';
import { backupErrorText, normalizeBackupIndex, pullTakesSession } from '../../core/backup';
import { formatBytes, plural } from '../../core/data-jobs';
import { BackupAction, BackupIndexSession, BackupSet } from '../../core/models';
import { lastSegment } from '../../core/paths';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { SyncDrag, SyncDropTarget, dropText, targetKey } from './sync-drag';

/** An account conversations can be brought into. */
export interface BrowserTarget {
  account: string;
  name: string;
  color: string;
}

/** What a drop (or its button) asks the page to do. */
export interface BrowserBring {
  set: string;
  ids: string[];
  prune: boolean;
}
export interface BrowserRestore {
  set: string;
  into: string;
  ids: string[];
}

type Filter = 'bring' | 'all';

const SHOWN = 120;

/** How a conversation in the backup compares with its account here. */
const STATUS: Record<BackupAction, { label: string; tone: string }> = {
  new: { label: 'Not on this computer', tone: 'new' },
  update: { label: 'Newer in the backup', tone: 'update' },
  conflict: { label: 'Changed on both', tone: 'conflict' },
  same: { label: 'Up to date', tone: 'same' },
  localNewer: { label: 'Newer here', tone: 'same' },
  live: { label: 'Running here', tone: 'same' },
};

/**
 * The conversations in one account's backup, to bring back by dragging
 * (docs/BACKUP.md "Restoring"): each says how it compares with the account
 * here, grouped by project. Drag one — or every selected one — onto its own
 * account to bring it here (a pull of those conversations: new ones come,
 * newer ones replace the copy here, kept for undo, and both sides of a
 * conflict are kept), or onto another account to restore it there (the
 * Restore dialog, preset). The account chips at the top take the drop too,
 * and are buttons for the selection, for keyboards and touch screens.
 */
@Component({
  selector: 'mc-backup-browser',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TimeAgoComponent],
  template: `
    <section class="browser card" [attr.aria-label]="'Conversations in ' + set().name + ' backup'">
      <header class="bhead">
        <div class="btitle">
          <mc-icon name="cloud" [size]="18" />
          <div>
            <strong>Conversations in {{ set().name }}'s backup</strong>
            <span class="muted small"
              >Drag them onto an account to bring them to this computer. Select several to move them
              together.</span
            >
          </div>
        </div>
        <button
          class="btn ghost icon-only"
          type="button"
          aria-label="Close"
          (click)="closed.emit()"
        >
          <mc-icon name="x" />
        </button>
      </header>

      <div class="targets" role="group" aria-label="Bring the selected conversations to">
        @for (t of targets(); track t.account) {
          <button
            type="button"
            class="target"
            [class.over]="drag.over() === key(t)"
            [class.can]="!!dropLabel(t)"
            [disabled]="disabled() || !selected().size"
            [attr.aria-label]="buttonLabel(t)"
            (click)="act(t, [...selected()])"
            (dragover)="over($event, t)"
            (dragleave)="leave(t)"
            (drop)="drop($event, t)"
          >
            <span class="dot" [style.background]="t.color" aria-hidden="true"></span>
            <span class="tname">{{ t.name }}</span>
            <span class="tdo small">{{ dropLabel(t) || buttonLabel(t) }}</span>
          </button>
        }
      </div>

      <div class="tools">
        <label class="search">
          <mc-icon name="search" [size]="15" />
          <input
            type="search"
            placeholder="Search by title or folder"
            aria-label="Search the conversations"
            [value]="query()"
            (input)="query.set($any($event.target).value)"
          />
        </label>
        <div class="chips" role="radiogroup" aria-label="Show">
          <button
            type="button"
            role="radio"
            class="chip"
            [class.on]="filter() === 'bring'"
            [attr.aria-checked]="filter() === 'bring'"
            (click)="filter.set('bring')"
          >
            Can come here <span class="n">{{ bringable().length }}</span>
          </button>
          <button
            type="button"
            role="radio"
            class="chip"
            [class.on]="filter() === 'all'"
            [attr.aria-checked]="filter() === 'all'"
            (click)="filter.set('all')"
          >
            All <span class="n">{{ sessions().length }}</span>
          </button>
        </div>
      </div>

      @if (loading()) {
        <p class="muted small pad">Opening the backup…</p>
      } @else if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
        <button class="btn small" type="button" (click)="load()">Try again</button>
      } @else if (!groups().length) {
        <p class="muted small pad">
          {{
            filter() === 'bring' && !query()
              ? 'Everything in this backup is on this computer already.'
              : 'No conversation matches.'
          }}
        </p>
      } @else {
        <div class="selbar small">
          <span>{{ selected().size ? plural(selected().size, 'selected') : '' }}</span>
          <span class="spacer"></span>
          <button class="btn ghost small" type="button" (click)="selectShown()">
            Select {{ filter() === 'bring' ? 'all that can come' : 'all shown' }}
          </button>
          @if (selected().size) {
            <button class="btn ghost small" type="button" (click)="clearSelection()">Clear</button>
          }
        </div>
        @for (g of groups(); track g.key) {
          <div class="group">
            <div class="gname">
              <mc-icon name="folder" [size]="14" />
              <strong>{{ g.name }}</strong>
              <span class="muted small mono gpath">{{ g.key }}</span>
            </div>
            <ul class="rows">
              @for (s of g.sessions; track s.id) {
                <li
                  class="row"
                  [class.sel]="selected().has(s.id)"
                  [class.fixed]="!movable(s)"
                  [attr.draggable]="movable(s) && !disabled()"
                  (dragstart)="start($event, s)"
                  (dragend)="drag.end()"
                >
                  <input
                    type="checkbox"
                    [checked]="selected().has(s.id)"
                    [disabled]="!movable(s) || disabled()"
                    [attr.aria-label]="'Select ' + title(s)"
                    (change)="flip(s.id)"
                  />
                  @if (movable(s)) {
                    <span class="grip" aria-hidden="true">⠿</span>
                  }
                  <span class="rmain">
                    <span class="rtitle">{{ title(s) }}</span>
                    <span class="muted small rmeta"
                      >{{ formatBytes(s.remoteBytes) }} · from {{ s.host || 'a computer' }}
                      <mc-time [ms]="s.pushedAt" />
                      @if (s.archived) {
                        · in storage only
                      }
                    </span>
                  </span>
                  <span class="badge small" [attr.data-tone]="status(s).tone">{{
                    status(s).label
                  }}</span>
                </li>
              }
            </ul>
          </div>
        }
        @if (hidden()) {
          <button class="btn small more" type="button" (click)="limit.set(limit() + 400)">
            Show {{ plural(hidden(), 'more conversation') }}
          </button>
        }
      }
    </section>
  `,
  styles: [
    `
      .browser {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        margin-top: 1rem;
      }
      .bhead {
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
      }
      .btitle {
        flex: 1;
        display: flex;
        gap: 0.6rem;
        align-items: flex-start;
        min-width: 0;
      }
      .btitle > mc-icon {
        color: var(--accent-text);
        margin-top: 0.15rem;
      }
      .btitle div {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
      }
      .targets {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
      }
      .target {
        appearance: none;
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: var(--tap);
        padding: 0.45rem 0.8rem;
        border: 1.5px dashed var(--border);
        border-radius: var(--radius);
        background: var(--panel-2);
        color: var(--text);
        font: inherit;
        cursor: pointer;
        transition:
          border-color 0.15s,
          background 0.15s,
          transform 0.15s;
      }
      .target:disabled {
        cursor: default;
        opacity: 0.75;
      }
      .target.can {
        border-color: var(--accent);
        opacity: 1;
      }
      .target.over {
        background: var(--accent-soft);
        transform: scale(1.03);
      }
      .dot {
        width: 0.65rem;
        height: 0.65rem;
        border-radius: 50%;
        flex: 0 0 auto;
      }
      .tname {
        font-weight: 600;
      }
      .tdo {
        color: var(--muted);
      }
      .tools {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
        align-items: center;
      }
      .search {
        flex: 1 1 14rem;
        display: flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0 0.6rem;
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        background: var(--bg, var(--panel));
        color: var(--muted);
      }
      .search input {
        flex: 1;
        min-width: 0;
        border: 0;
        background: transparent;
        padding: 0.55rem 0;
        color: var(--text);
        font: inherit;
        outline: none;
      }
      .chips {
        display: flex;
        gap: 0.35rem;
      }
      .chip {
        appearance: none;
        border: 1px solid var(--border);
        background: var(--panel-2);
        color: var(--text);
        border-radius: 999px;
        padding: 0.35rem 0.8rem;
        font: inherit;
        font-size: 0.85rem;
        cursor: pointer;
      }
      .chip.on {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .chip .n {
        color: var(--muted);
        margin-left: 0.2rem;
      }
      .selbar {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        color: var(--muted);
      }
      .spacer {
        flex: 1;
      }
      .group + .group {
        margin-top: 0.4rem;
      }
      .gname {
        display: flex;
        align-items: baseline;
        gap: 0.4rem;
        padding: 0.25rem 0;
        min-width: 0;
      }
      .gname mc-icon {
        color: var(--muted);
        align-self: center;
      }
      .gpath {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
      }
      .rows {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
      }
      .row {
        display: flex;
        align-items: center;
        gap: 0.55rem;
        padding: 0.5rem 0.65rem;
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        background: var(--panel-2);
        cursor: grab;
        transition:
          border-color 0.15s,
          background 0.15s;
      }
      .row:hover {
        border-color: var(--accent);
      }
      .row.sel {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .row.fixed {
        cursor: default;
        opacity: 0.7;
      }
      .row.fixed:hover {
        border-color: var(--border);
      }
      .grip {
        color: var(--muted);
        font-size: 0.95rem;
        line-height: 1;
      }
      .rmain {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
      }
      .rtitle {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .rmeta {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .badge {
        flex: 0 0 auto;
        border-radius: 999px;
        padding: 0.15rem 0.55rem;
        border: 1px solid var(--border);
        color: var(--muted);
        white-space: nowrap;
      }
      .badge[data-tone='new'] {
        color: var(--green, #3fb950);
        border-color: currentColor;
      }
      .badge[data-tone='update'] {
        color: var(--accent-text);
        border-color: currentColor;
      }
      .badge[data-tone='conflict'] {
        color: var(--amber, #d29922);
        border-color: currentColor;
      }
      .pad {
        margin: 0.25rem 0;
      }
      .more {
        align-self: center;
      }
      @media (max-width: 560px) {
        .badge {
          display: none;
        }
        .row {
          cursor: default;
        }
      }
    `,
  ],
})
export class BackupBrowserComponent {
  private api = inject(ApiService);
  protected readonly drag = inject(SyncDrag);

  /** The backup browsed. */
  readonly set = input.required<BackupSet>();
  /** The accounts its conversations can be brought into. */
  readonly targets = input<BrowserTarget[]>([]);
  readonly disabled = input(false);
  readonly closed = output<void>();
  /** Bring these to their own account (a pull of them). */
  readonly bring = output<BrowserBring>();
  /** Restore these into another account (the Restore dialog, preset). */
  readonly restoreInto = output<BrowserRestore>();

  protected readonly formatBytes = formatBytes;
  protected readonly plural = plural;
  protected readonly loading = signal(true);
  protected readonly error = signal('');
  protected readonly sessions = signal<BackupIndexSession[]>([]);
  protected readonly selected = signal<ReadonlySet<string>>(new Set());
  protected readonly query = signal('');
  protected readonly filter = signal<Filter>('bring');
  protected readonly limit = signal(SHOWN);
  private seq = 0;

  protected readonly bringable = computed(() =>
    this.sessions().filter((s) => pullTakesSession(s, false)),
  );
  private readonly matching = computed(() => {
    const q = this.query().trim().toLowerCase();
    const base = this.filter() === 'bring' ? this.bringable() : this.sessions();
    const found = q
      ? base.filter((s) =>
          `${this.title(s)} ${s.folder ?? ''} ${s.cwd ?? ''}`.toLowerCase().includes(q),
        )
      : base;
    return [...found].sort((a, b) => b.newestMtime - a.newestMtime);
  });
  protected readonly hidden = computed(() => Math.max(0, this.matching().length - this.limit()));
  protected readonly groups = computed(() => {
    const out = new Map<string, { key: string; name: string; sessions: BackupIndexSession[] }>();
    for (const s of this.matching().slice(0, this.limit())) {
      const key = s.mapped || s.folder || s.cwd || 'Other';
      const g = out.get(key) ?? { key, name: lastSegment(key) || key, sessions: [] };
      g.sessions.push(s);
      out.set(key, g);
    }
    return [...out.values()];
  });

  constructor() {
    effect(() => {
      this.set();
      untracked(() => void this.load());
    });
  }

  async load(): Promise<void> {
    const seq = ++this.seq;
    this.loading.set(true);
    this.error.set('');
    this.selected.set(new Set());
    try {
      const ix = normalizeBackupIndex(await this.api.getBackupIndex(this.set().account));
      if (!ix) throw new Error('The desktop sent a backup list this app could not read.');
      if (seq !== this.seq) return;
      this.sessions.set(ix.sessions);
    } catch (e) {
      if (seq !== this.seq) return;
      this.error.set(backupErrorText(e));
    } finally {
      if (seq === this.seq) this.loading.set(false);
    }
  }

  protected title(s: BackupIndexSession): string {
    return s.title || `Conversation ${s.id.slice(0, 8)}`;
  }

  protected status(s: BackupIndexSession): { label: string; tone: string } {
    return s.archived && s.action === 'new'
      ? { label: 'In storage only', tone: 'new' }
      : STATUS[s.action];
  }

  /** It can be dragged: something would happen to it here. */
  protected movable(s: BackupIndexSession): boolean {
    return pullTakesSession(s, false) || s.action === 'new';
  }

  protected flip(id: string): void {
    this.selected.update((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  protected clearSelection(): void {
    this.selected.set(new Set());
  }

  protected selectShown(): void {
    this.selected.set(
      new Set(
        this.groups()
          .flatMap((g) => g.sessions)
          .filter((s) => this.movable(s))
          .map((s) => s.id),
      ),
    );
  }

  /** Dragging a selected row drags the selection; another row, itself. */
  protected start(e: DragEvent, s: BackupIndexSession): void {
    const ids = this.selected().has(s.id) ? [...this.selected()] : [s.id];
    const chosen = this.sessions().filter((x) => ids.includes(x.id));
    this.drag.start(
      e,
      {
        kind: 'sessions',
        set: this.set().account,
        setName: this.set().name,
        ids,
        prune: chosen.some((x) => !!x.pruneWarning),
      },
      ids.length === 1 ? this.title(s) : `${ids.length} conversations`,
    );
  }

  protected key(t: BrowserTarget): string {
    return targetKey(this.target(t));
  }

  protected dropLabel(t: BrowserTarget): string {
    const item = this.drag.item();
    return item?.kind === 'sessions' ? dropText(item, this.target(t), () => t.name) : '';
  }

  protected buttonLabel(t: BrowserTarget): string {
    const n = this.selected().size;
    const what = n ? plural(n, 'conversation') : 'the selected conversations';
    return t.account === this.set().account ? `Bring ${what} here` : `Restore ${what} here…`;
  }

  protected over(e: DragEvent, t: BrowserTarget): void {
    if (!this.dropLabel(t)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
    this.drag.over.set(this.key(t));
  }

  protected leave(t: BrowserTarget): void {
    if (this.drag.over() === this.key(t)) this.drag.over.set('');
  }

  protected drop(e: DragEvent, t: BrowserTarget): void {
    const item = this.drag.item();
    this.drag.end();
    if (item?.kind !== 'sessions') return;
    e.preventDefault();
    this.act(t, item.ids);
  }

  /** Bring `ids` into `t`: their own account takes them as a pull; another
   * one through the Restore dialog. */
  protected act(t: BrowserTarget, ids: string[]): void {
    if (!ids.length) return;
    const set = this.set().account;
    if (t.account === set) {
      const chosen = this.sessions().filter((s) => ids.includes(s.id));
      this.bring.emit({ set, ids, prune: chosen.some((s) => !!s.pruneWarning) });
    } else this.restoreInto.emit({ set, into: t.account, ids });
    this.selected.set(new Set());
  }

  private target(t: BrowserTarget): SyncDropTarget {
    return { kind: 'account', account: t.account };
  }
}
