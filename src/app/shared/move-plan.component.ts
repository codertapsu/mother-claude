import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { AccountsService } from '../core/accounts.service';
import { formatBytes, plural } from '../core/data-jobs';
import {
  CustomizationPlan,
  MoveDirection,
  MovePlan,
  MovePlanSession,
  MoveSkip,
  actionLabel,
  customizationLabel,
  customizationText,
  locationKey,
  locationName,
  planLine,
  skipName,
  skipReason,
} from '../core/move-data';
import { StorageService } from '../core/storage.service';
import { shortPath } from './util';

/** Conversations listed before "and N more". */
const ROWS = 30;

/**
 * A Move data dry run (docs/ACCOUNTS.md §13.4): one line about what it
 * does, what goes (each with where it comes from when that differs, and
 * what happens to it when that differs), what is skipped and why, the
 * customizations, the warnings, and what else it writes — and whether it
 * can be undone.
 */
@Component({
  selector: 'mc-move-plan',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @let p = plan();
    <p class="planline">{{ line() }}</p>
    @if (p.jobs > 1) {
      <p class="muted small jobs">
        It runs as {{ p.jobs }} jobs, one for each place the conversations come from.
      </p>
    }
    @if (p.sessions.length) {
      <h4>What goes</h4>
      <ul class="plist" aria-label="What goes">
        @for (s of shown(); track s.id) {
          <li>
            <span class="ptitle">{{ s.title || s.id.slice(0, 8) }}</span>
            <span class="muted small">{{ rowLine(s) }}</span>
          </li>
        }
      </ul>
      @if (more() > 0) {
        <p class="muted small">and {{ more() }} more</p>
      }
    }
    @if (p.skipped.length) {
      <h4>Skipped ({{ p.skipped.length }})</h4>
      <ul class="plist skips" aria-label="Skipped">
        @for (k of p.skipped; track $index) {
          <li>
            <span class="ptitle">{{ skipNameOf(k) }}</span>
            <span class="small why">{{ reason(k) }}</span>
          </li>
        }
      </ul>
    }
    @if (p.customizations.length) {
      <h4>Customizations</h4>
      <ul class="plist" aria-label="Customizations">
        @for (c of p.customizations; track $index) {
          <li>
            <span class="ptitle">{{ itemLabel(c) }}</span>
            <span class="small" [class.muted]="c.action === 'skip'">{{ customText(c) }}</span>
          </li>
        }
      </ul>
    }
    @if (p.warnings.length) {
      <div class="notice warn small warns" role="status">
        <ul>
          @for (w of p.warnings; track $index) {
            <li>{{ w }}</li>
          }
        </ul>
      </div>
    }
    <ul class="effects small">
      @for (e of effects(); track e) {
        <li>{{ e }}</li>
      }
    </ul>
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .planline {
        margin: 0.25rem 0 0.4rem;
        line-height: 1.5;
      }
      .jobs {
        margin: 0 0 0.4rem;
      }
      h4 {
        margin: 0.85rem 0 0.3rem;
        font-size: 0.85rem;
      }
      .plist,
      .effects,
      .warns ul {
        margin: 0;
        padding: 0;
        list-style: none;
      }
      .plist > li {
        display: flex;
        flex-direction: column;
        gap: 0.05rem;
        padding: 0.4rem 0;
        border-top: 1px solid var(--border);
        overflow-wrap: anywhere;
      }
      .ptitle {
        font-weight: 500;
      }
      .skips .why {
        color: var(--muted);
      }
      .warns {
        margin-top: 0.75rem;
      }
      .warns li + li {
        margin-top: 0.35rem;
      }
      .effects {
        margin-top: 0.75rem;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        color: var(--muted);
      }
      .effects li::before {
        content: '• ';
      }
    `,
  ],
})
export class MovePlanComponent {
  private accounts = inject(AccountsService);
  private storage = inject(StorageService);

  readonly plan = input.required<MovePlan>();
  /** Way it goes (what can be undone depends on it). */
  readonly direction = input.required<MoveDirection>();
  /** Where it goes and where it comes from, as names. */
  readonly toName = input.required<string>();
  readonly fromName = input('');
  /** Titles of the conversations the sheet knows (for the skipped ones). */
  readonly titles = input<ReadonlyMap<string, string>>(new Map());

  protected readonly line = computed(() => planLine(this.plan(), this.toName()));
  protected readonly shown = computed(() => this.plan().sessions.slice(0, ROWS));
  protected readonly more = computed(() => this.plan().sessions.length - this.shown().length);
  /** The actions differ: each row says its own. */
  private readonly mixed = computed(
    () => new Set(this.plan().sessions.map((s) => s.action)).size > 1,
  );
  protected readonly effects = computed(() => {
    const p = this.plan();
    const to = this.toName();
    const out: string[] = [];
    const entries = (n: number) => plural(n, 'folder entry', 'folder entries');
    if (p.addsProjectEntries.length) {
      out.push(
        `Adds ${entries(p.addsProjectEntries.length)}, with their trust, to ${to}'s settings.`,
      );
    }
    if (p.removesProjectEntries.length) {
      out.push(
        `Takes ${entries(p.removesProjectEntries.length)} out of ${this.fromName() || 'the source'}'s settings.`,
      );
    }
    if (p.historyLines)
      out.push(`Adds ${plural(p.historyLines, 'line')} of prompt history to ${to}.`);
    if (p.writesShared) {
      out.push(
        `Writes ${to}'s shared files, so it waits until no Claude Code session uses that folder.`,
      );
    }
    if (p.truncated) out.push('The lists show the first 1000 of each; it takes them all.');
    out.push(
      this.direction() === 'toStorage' && p.mode === 'copy'
        ? 'A copy into a storage is a backup: there is nothing to undo.'
        : 'Everything it changes can be undone for seven days.',
    );
    return out;
  });

  protected rowLine(s: MovePlanSession): string {
    const p = this.plan();
    const parts = [s.id.slice(0, 8), shortPath(s.cwd)];
    if (s.files || s.bytes) parts.push(`${plural(s.files, 'file')} · ${formatBytes(s.bytes)}`);
    if (s.from && (!p.from || locationKey(s.from) !== locationKey(p.from))) {
      parts.push(`from ${locationName(s.from, this.accounts.list(), this.storage.list() ?? [])}`);
    }
    if (this.mixed()) parts.push(actionLabel(s.action));
    return parts.filter(Boolean).join(' · ');
  }

  protected skipNameOf(k: MoveSkip): string {
    const titles = this.titles();
    const fromPlan = new Map(this.plan().sessions.map((s) => [s.id, s.title ?? '']));
    return skipName(k, (id) => titles.get(id) || fromPlan.get(id) || undefined);
  }

  protected reason(k: MoveSkip): string {
    return skipReason(k);
  }

  protected itemLabel(c: CustomizationPlan): string {
    const label = customizationLabel(c.item);
    return c.name && c.name !== c.item ? `${label}: ${c.name}` : label;
  }

  protected customText(c: CustomizationPlan): string {
    const t = customizationText(c);
    return t[0].toUpperCase() + t.slice(1);
  }
}
