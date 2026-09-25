import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { DiffBase, GitOverview } from '../../core/models';
import { storageGet, storageSet } from '../../core/platform';
import { parseUnifiedDiff } from '../../shared/diff';
import { DiffViewComponent } from '../../shared/diff-view.component';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { baseName, groupByDir } from '../../shared/util';
import { SessionContext } from './session-context';

const DIFF_VIEW_KEY = 'mc_diff_view';
const DIFF_BASE_KEY = 'mc_diff_base';

/**
 * The session's git changes: uncommitted work, or everything since the
 * branch left the default branch; files as a list or a folder tree; one
 * file's patch unified or side by side. The view preferences are kept in
 * localStorage (guarded: storage can throw on locked-down phone browsers).
 */
@Component({
  selector: 'mc-changes-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DiffViewComponent, IconComponent, TimeAgoComponent],
  template: `
    <div class="bar">
      <div class="segmented" role="group" aria-label="Compare against">
        <button
          type="button"
          [attr.aria-pressed]="base() === 'working'"
          (click)="setBase('working')"
        >
          Uncommitted
        </button>
        <button type="button" [attr.aria-pressed]="base() === 'branch'" (click)="setBase('branch')">
          Whole branch
        </button>
      </div>
      <span class="spacer"></span>
      <button class="btn ghost small" type="button" [disabled]="loading()" (click)="load()">
        <mc-icon name="refresh" [size]="14" /> Refresh
      </button>
    </div>

    @if (error()) {
      <p class="notice error">
        {{ error() }}
        <button class="btn ghost small" type="button" (click)="load()">Retry</button>
      </p>
    }

    @if (diff(); as d) {
      @if (!d.isRepo) {
        <div class="card empty-state">
          <strong>Not a Git repository</strong>
          There are no changes to show for this folder.
        </div>
      } @else {
        <div class="card summary">
          <span class="badge"
            ><mc-icon name="branch" [size]="12" /> {{ d.branch || 'detached' }}</span
          >
          @if (d.head) {
            <span class="mono muted">{{ d.head }}</span>
          }
          <span class="spacer"></span>
          <span class="add">+{{ d.additions }}</span>
          <span class="del">−{{ d.deletions }}</span>
        </div>

        <div class="row files-head">
          <h2>{{ d.files.length }} changed {{ d.files.length === 1 ? 'file' : 'files' }}</h2>
          <span class="spacer"></span>
          <div class="segmented" role="group" aria-label="File list layout">
            <button
              type="button"
              [attr.aria-pressed]="view() === 'list'"
              (click)="view.set('list')"
            >
              List
            </button>
            <button
              type="button"
              [attr.aria-pressed]="view() === 'tree'"
              (click)="view.set('tree')"
            >
              Tree
            </button>
          </div>
        </div>

        @if (view() === 'list') {
          @for (f of d.files; track f.path) {
            <button
              class="card file"
              type="button"
              [class.selected]="patch()?.path === f.path"
              [attr.aria-pressed]="patch()?.path === f.path"
              (click)="showPatch(f.path)"
            >
              <span class="badge {{ f.status }}">{{ f.status }}</span>
              <span class="fpath mono">{{ f.path }}</span>
              <span class="spacer"></span>
              <span class="add">+{{ f.additions }}</span>
              <span class="del">−{{ f.deletions }}</span>
            </button>
          } @empty {
            <p class="muted">
              {{ base() === 'branch' ? 'No changes on this branch.' : 'No uncommitted changes.' }}
            </p>
          }
        } @else {
          @for (g of grouped(); track g.dir) {
            <div class="dirhead mono muted">
              <mc-icon name="folder" [size]="12" /> {{ g.dir === '.' ? '(root)' : g.dir + '/' }}
            </div>
            @for (f of g.files; track f.path) {
              <button
                class="card file treefile"
                type="button"
                [class.selected]="patch()?.path === f.path"
                [attr.aria-pressed]="patch()?.path === f.path"
                (click)="showPatch(f.path)"
              >
                <span class="badge {{ f.status }}">{{ f.status }}</span>
                <span class="fpath mono">{{ base_(f.path) }}</span>
                <span class="spacer"></span>
                <span class="add">+{{ f.additions }}</span>
                <span class="del">−{{ f.deletions }}</span>
              </button>
            }
          } @empty {
            <p class="muted">No changes.</p>
          }
        }

        @if (patch(); as p) {
          <div class="row patch-head">
            <h2 class="mono patch-title">{{ p.path }}</h2>
            <span class="spacer"></span>
            <div class="segmented" role="group" aria-label="Diff layout">
              <button
                type="button"
                [attr.aria-pressed]="diffView() === 'unified'"
                (click)="setDiffView('unified')"
              >
                Unified
              </button>
              <button
                type="button"
                [attr.aria-pressed]="diffView() === 'split'"
                (click)="setDiffView('split')"
              >
                Split
              </button>
            </div>
          </div>
          @if (!p.raw) {
            <div class="card muted">No diff for this file.</div>
          } @else {
            <mc-diff-view [rows]="rows()" [mode]="diffView()" />
          }
        }

        @if (d.commits.length) {
          <h2>Recent commits</h2>
          @for (c of d.commits; track c.id) {
            <div class="card commit">
              <span class="mono muted">{{ c.id }}</span> {{ c.summary }}
              <span class="muted small">— {{ c.author }} · <mc-time [ms]="c.time" /></span>
            </div>
          }
        }

        @if (d.worktrees.length) {
          <h2>Worktrees</h2>
          @for (w of d.worktrees; track w.name) {
            <div class="card mono small">
              {{ w.name }} → {{ w.path }} {{ w.locked ? '(locked)' : '' }}
            </div>
          }
        }
      }
    } @else if (loading()) {
      <div class="skeleton"><span></span><span></span><span></span></div>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        padding: 0.5rem 0 1rem;
      }
      .bar {
        display: flex;
        align-items: center;
        gap: 0.5rem;
      }
      .summary {
        display: flex;
        align-items: center;
        gap: 0.6rem;
      }
      .files-head,
      .patch-head {
        align-items: center;
      }
      .files-head h2,
      .patch-head h2 {
        margin: 0.6rem 0 0.2rem;
      }
      .file {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        width: 100%;
        text-align: left;
        font: inherit;
        color: inherit;
        cursor: pointer;
        margin: 0;
      }
      .card + .card {
        margin-top: 0;
      }
      .file.selected {
        border-color: var(--accent);
      }
      .treefile {
        margin-left: 0.9rem;
        width: calc(100% - 0.9rem);
      }
      .fpath {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
      }
      .dirhead {
        display: flex;
        align-items: center;
        gap: 0.3rem;
        font-size: 0.75rem;
        margin: 0.4rem 0 0;
      }
      .patch-title {
        word-break: break-all;
      }
      .add {
        color: var(--green);
      }
      .del {
        color: var(--red);
      }
      .badge.added,
      .badge.untracked {
        color: var(--green);
      }
      .badge.deleted {
        color: var(--red);
      }
      .badge.modified {
        color: var(--amber);
      }
      .commit {
        font-size: 0.85rem;
      }
      .skeleton {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
      .skeleton span {
        height: 2.6rem;
        border-radius: var(--radius);
        background: var(--panel-2);
        animation: mc-shimmer 1.2s ease-in-out infinite alternate;
      }
    `,
  ],
})
export class ChangesPanelComponent {
  private api = inject(ApiService);
  private ctx = inject(SessionContext);

  protected readonly base = signal<DiffBase>(
    storageGet(DIFF_BASE_KEY) === 'branch' ? 'branch' : 'working',
  );
  protected readonly diff = signal<GitOverview | null>(null);
  protected readonly loading = signal(false);
  protected readonly error = signal('');
  protected readonly patch = signal<{ path: string; raw: string } | null>(null);
  protected readonly view = signal<'list' | 'tree'>('list');
  protected readonly diffView = signal<'unified' | 'split'>(
    storageGet(DIFF_VIEW_KEY) === 'split' ? 'split' : 'unified',
  );
  protected readonly grouped = computed(() => groupByDir(this.diff()?.files ?? []));
  protected readonly rows = computed(() => parseUnifiedDiff(this.patch()?.raw ?? ''));
  protected readonly base_ = baseName;
  private req = 0;
  private patchReq = 0;

  constructor() {
    effect(() => {
      this.ctx.id();
      this.base();
      untracked(() => {
        this.diff.set(null);
        this.patch.set(null);
        void this.load();
      });
    });
  }

  protected setBase(b: DiffBase): void {
    this.base.set(b);
    storageSet(DIFF_BASE_KEY, b);
  }

  protected setDiffView(mode: 'unified' | 'split'): void {
    this.diffView.set(mode);
    storageSet(DIFF_VIEW_KEY, mode);
  }

  async load(): Promise<void> {
    const id = this.ctx.id();
    if (!id) return;
    const req = ++this.req;
    this.loading.set(true);
    this.error.set('');
    try {
      const d = await this.api.getDiff(id, this.base());
      if (req === this.req) this.diff.set(d);
    } catch (e) {
      if (req === this.req) this.error.set(`Could not load changes: ${describeError(e).text}`);
    } finally {
      if (req === this.req) this.loading.set(false);
    }
  }

  protected async showPatch(path: string): Promise<void> {
    const id = this.ctx.id();
    const req = ++this.patchReq;
    try {
      const res = await this.api.getFilePatch(id, path, this.base());
      if (req === this.patchReq) this.patch.set({ path, raw: res?.patch ?? '' });
    } catch (e) {
      if (req === this.patchReq)
        this.error.set(`Could not load the patch: ${describeError(e).text}`);
    }
  }
}
