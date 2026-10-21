import {
  ChangeDetectionStrategy,
  Component,
  computed,
  input,
  linkedSignal,
  output,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ChecklistRow, filterRows } from '../core/move-data';
import { IconComponent } from './icon.component';

/** Rows shown before "Show N more". */
const FIRST_ROWS = 50;

/**
 * A searchable checklist of conversations or projects (Move data): a
 * search box, how many are selected, *Select all shown* and *None*, and
 * one row per item with its title, a short tag ("Running", "In storage
 * only") and a muted line (its id, folder or size). Long lists show their
 * first rows and "Show N more"; `more` offers loading earlier ones.
 */
@Component({
  selector: 'mc-move-checklist',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, IconComponent],
  template: `
    <div class="clhead">
      <label class="search">
        <mc-icon name="search" [size]="15" />
        <input
          type="search"
          autocapitalize="off"
          autocomplete="off"
          spellcheck="false"
          [placeholder]="'Search ' + noun()"
          [attr.aria-label]="'Search ' + noun()"
          [ngModel]="query()"
          (ngModelChange)="query.set($event)"
        />
      </label>
      <div class="bulk">
        <span class="muted small count">{{ selectedCount() }} of {{ rows().length }} selected</span>
        <button
          class="btn ghost small"
          type="button"
          [disabled]="disabled() || !visible().length"
          (click)="setAll(true)"
        >
          Select all shown
        </button>
        <button
          class="btn ghost small"
          type="button"
          [disabled]="disabled() || !selectedCount()"
          (click)="setAll(false)"
        >
          None
        </button>
      </div>
    </div>
    <ul class="rows" [attr.aria-label]="label()">
      @for (r of shown(); track r.id) {
        <li>
          <label class="pick">
            <input
              type="checkbox"
              [checked]="selected().has(r.id)"
              [disabled]="disabled()"
              [attr.aria-label]="r.title + (r.tag ? ' (' + r.tag + ')' : '')"
              (change)="flip.emit(r.id)"
            />
            <span class="body">
              <span class="tline">
                <span class="title">{{ r.title }}</span>
                @if (r.tag) {
                  <span class="tag">{{ r.tag }}</span>
                }
              </span>
              @if (r.sub) {
                <span class="sub muted small">{{ r.sub }}</span>
              }
            </span>
          </label>
        </li>
      } @empty {
        <li class="none muted small">
          @if (loading()) {
            Loading…
          } @else if (query().trim()) {
            Nothing matches.
          } @else {
            {{ emptyText() }}
          }
        </li>
      }
    </ul>
    @if (hidden() > 0) {
      <button class="btn ghost small" type="button" (click)="limit.set(null)">
        Show {{ hidden() }} more
      </button>
    }
    @if (error()) {
      <p class="error-text small" role="alert">{{ error() }}</p>
    }
    @if (more()) {
      <button
        class="btn ghost small"
        type="button"
        [disabled]="loading()"
        (click)="loadMore.emit()"
      >
        {{ loading() ? 'Loading…' : moreText() }}
      </button>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .clhead {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
        margin-bottom: 0.35rem;
      }
      .search {
        position: relative;
        display: block;
      }
      .search mc-icon {
        position: absolute;
        left: 0.65rem;
        top: 50%;
        transform: translateY(-50%);
        color: var(--faint);
        pointer-events: none;
      }
      .search input {
        padding-left: 2.1rem;
      }
      .bulk {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.2rem 0.3rem;
      }
      .count {
        margin-right: auto;
      }
      /* A long list scrolls by itself, so How and Check stay near. */
      .rows {
        margin: 0;
        padding: 0 0.1rem 0 0;
        list-style: none;
        max-height: min(22rem, 42dvh);
        overflow-y: auto;
        overscroll-behavior: contain;
        border-bottom: 1px solid var(--border);
      }
      .rows > li {
        border-top: 1px solid var(--border);
      }
      .pick {
        display: flex;
        align-items: flex-start;
        gap: 0.6rem;
        padding: 0.5rem 0;
        min-height: var(--tap);
        cursor: pointer;
      }
      .pick input {
        margin-top: 0.2rem;
        flex: 0 0 auto;
      }
      .body {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
      }
      .tline {
        display: flex;
        align-items: baseline;
        flex-wrap: wrap;
        gap: 0.2rem 0.45rem;
      }
      .title {
        font-weight: 500;
        line-height: 1.35;
        overflow-wrap: anywhere;
      }
      .tag {
        font-size: 0.72rem;
        padding: 0.02rem 0.4rem;
        border-radius: 0.4rem;
        background: var(--panel-3);
        color: var(--muted);
        white-space: nowrap;
      }
      .sub {
        overflow-wrap: anywhere;
      }
      .none {
        padding: 0.6rem 0;
      }
      .btn.ghost.small {
        margin-top: 0.2rem;
      }
    `,
  ],
})
export class MoveChecklistComponent {
  readonly rows = input.required<ChecklistRow[]>();
  readonly selected = input.required<ReadonlySet<string>>();
  /** What the list is, for screen readers ("Conversations of Work"). */
  readonly label = input('Items');
  /** "conversations", "projects": what the search looks for. */
  readonly noun = input('conversations');
  readonly emptyText = input('There is nothing here.');
  readonly loading = input(false);
  readonly error = input('');
  /** Earlier items can be loaded. */
  readonly more = input(false);
  readonly moreText = input('Load earlier ones');
  readonly disabled = input(false);
  readonly flip = output<string>();
  /** Select (true) or clear (false) the rows a search shows. */
  readonly bulk = output<{ ids: string[]; on: boolean }>();
  readonly loadMore = output<void>();

  protected readonly query = signal('');
  /** How many rows show (null: all), back to the first ones on a search. */
  protected readonly limit = linkedSignal<string, number | null>({
    source: () => this.query(),
    computation: () => FIRST_ROWS,
  });
  protected readonly visible = computed(() => filterRows(this.rows(), this.query()));
  protected readonly shown = computed(() => {
    const n = this.limit();
    return n === null ? this.visible() : this.visible().slice(0, n);
  });
  protected readonly hidden = computed(() => this.visible().length - this.shown().length);
  protected readonly selectedCount = computed(
    () => this.rows().filter((r) => this.selected().has(r.id)).length,
  );

  protected setAll(on: boolean): void {
    this.bulk.emit({ ids: (on ? this.visible() : this.rows()).map((r) => r.id), on });
  }
}
