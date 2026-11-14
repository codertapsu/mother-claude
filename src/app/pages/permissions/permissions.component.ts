import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { ActivatedRoute, Router } from '@angular/router';
import { map } from 'rxjs';

import { describeError } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  PermissionRule,
  SCOPES,
  familyOf,
  ruleAction,
  scopeLabel,
} from '../../core/permission-rules';
import { PermissionRulesService } from '../../core/permission-rules.service';
import { NoticeService } from '../../core/notice.service';
import { EmptyStateComponent } from '../../shared/empty-state.component';
import { IconComponent, IconName } from '../../shared/icon.component';
import { PageHeadComponent } from '../../shared/page-head.component';
import { relativeTime } from '../../shared/util';
import { RuleSheetComponent } from './rule-sheet.component';

/** What the chips show (`?on=`): every permission, this computer's, or the
 * ones for commands and tasks on the other computers. */
type Show = 'all' | 'here' | 'others';

/** The chips, in order (English labels: translated where shown). */
const CHIPS: { id: Show; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'here', label: 'This {computer}' },
  { id: 'others', label: 'Other computers' },
];

/** The permissions of one scope: everywhere, a project, a session. */
export interface ScopeGroup {
  key: string;
  title: string;
  rules: PermissionRule[];
}

/** The permissions of one computer: this one (a session's tools), or
 * another (the commands and tasks sessions here send there). */
export interface WhereGroup {
  key: string;
  here: boolean;
  title: string;
  hint: string;
  scopes: ScopeGroup[];
}

/** Every permission by where it applies — this computer first, then each
 * other one — and in each by scope: everywhere, then each project, then
 * each session; oldest first. */
export function groupRules(rules: readonly PermissionRule[]): WhereGroup[] {
  const wheres = new Map<string, { here: boolean; name: string; rules: PermissionRule[] }>();
  for (const r of rules) {
    const here = r.kind === 'tool';
    const key = here ? 'here' : `m:${r.machine ?? r.machineName ?? ''}`;
    const w = wheres.get(key) ?? { here, name: r.machineName ?? '', rules: [] };
    w.rules.push(r);
    wheres.set(key, w);
  }
  const computer = computerNoun();
  return [...wheres.entries()]
    .sort(([, a], [, b]) => Number(b.here) - Number(a.here) || a.name.localeCompare(b.name))
    .map(([key, w]) => {
      const name = w.name || tr('another computer');
      return {
        key,
        here: w.here,
        title: w.here ? tr('On this {computer}', { computer }) : tr('On {name}', { name }),
        hint: w.here
          ? tr('What sessions on this {computer} may do without asking.', { computer })
          : tr('Commands and tasks sessions here may send to {name} without asking.', { name }),
        scopes: scopeGroups(w.rules),
      };
    });
}

function scopeGroups(rules: PermissionRule[]): ScopeGroup[] {
  const groups = new Map<string, ScopeGroup & { rank: number; sort: string }>();
  for (const r of rules) {
    const key =
      r.scope === 'computer'
        ? 'computer'
        : r.scope === 'project'
          ? `p:${r.project ?? ''}`
          : `s:${r.session ?? ''}`;
    let g = groups.get(key);
    if (!g) {
      const session = r.sessionName || r.session?.slice(0, 8);
      g = {
        key,
        title: scopeLabel(r.kind, r.scope, { project: r.projectName, session }),
        rules: [],
        // Everywhere first, then projects, then sessions.
        rank: SCOPES.length - SCOPES.indexOf(r.scope),
        sort: (r.scope === 'project' ? r.projectName : session) ?? '',
      };
      groups.set(key, g);
    }
    g.rules.push(r);
  }
  return [...groups.values()]
    .sort((a, b) => a.rank - b.rank || a.sort.localeCompare(b.sort))
    .map(({ key, title, rules: list }) => ({
      key,
      title,
      rules: [...list].sort((a, b) => a.createdAt - b.createdAt),
    }));
}

/**
 * *Permissions*: what Claude may do without asking you, every answer kept
 * with *Allow for…* on a card. A switch pauses them all (*Ask for
 * everything*: the rules are kept). They are grouped by where they apply —
 * this computer (a session's tools), then each other computer (the
 * commands and tasks sessions here send there) — then by scope:
 * everywhere, a project, a session. Each says what it allows in words, its
 * patterns as chips, how often it was used and when last, with *Edit*
 * (patterns and scope, in a sheet) and *Remove* (confirmed). Chips show
 * one place (`?on=here|others`: Computers › Activity links here with
 * `others`). Live: a `permissions` frame reads them again.
 */
@Component({
  selector: 'mc-permissions',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [EmptyStateComponent, IconComponent, PageHeadComponent, RuleSheetComponent, TPipe],
  template: `
    <div class="page narrow">
      <mc-page-head
        [heading]="'Permissions' | t"
        [lede]="
          'What Claude may do without asking you. Anything else — and anything dangerous — still asks.'
            | t
        "
        help="permissions"
      />

      <section class="card pause" [attr.aria-label]="'Ask for everything' | t">
        <label class="switch">
          <input
            type="checkbox"
            [checked]="permissions.paused()"
            [disabled]="busy() === 'pause' || !permissions.loaded()"
            (change)="setPaused($any($event.target))"
          />
          <span class="switch-text">
            <strong>{{ 'Ask for everything' | t }}</strong>
            <span class="muted small">
              @if (permissions.paused()) {
                {{
                  'On: Claude asks before everything. Your permissions are kept for when you switch this off.'
                    | t
                }}
              } @else {
                {{
                  'Pauses every permission below: Claude asks before everything, until you switch it off.'
                    | t
                }}
              }
            </span>
          </span>
        </label>
      </section>

      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      @if (permissions.error()) {
        <p class="notice error small" role="alert">{{ permissions.error() }}</p>
      }

      @if (!permissions.loaded()) {
        @if (!permissions.error()) {
          <p class="muted small">{{ 'Loading…' | t }}</p>
        }
      } @else if (!permissions.rules().length) {
        <mc-empty-state
          icon="shield"
          [heading]="'No permissions yet' | t"
          [text]="'When Claude asks, choose Allow for… to remember your answer.' | t"
          help="permissions"
        />
      } @else {
        @if (hasOthers()) {
          <div class="chips" role="group" [attr.aria-label]="'Show' | t">
            @for (c of chips; track c.id) {
              <button
                type="button"
                class="chip-btn"
                [attr.aria-pressed]="show() === c.id"
                (click)="pick(c.id)"
              >
                {{ c.label | t: { computer: computer() } }}
              </button>
            }
          </div>
        }
        @if (permissions.paused()) {
          <p class="notice warn small">
            {{ 'Paused: none of these is used until you switch Ask for everything off.' | t }}
          </p>
        }
        @for (g of groups(); track g.key) {
          <section class="card where" [attr.data-where]="g.key">
            <div class="ghead">
              <mc-icon [name]="g.here ? 'shield' : 'monitor'" [size]="18" />
              <h2>{{ g.title }}</h2>
            </div>
            <p class="muted small ghint">{{ g.hint }}</p>
            @for (s of g.scopes; track s.key) {
              <h3 class="scope">{{ s.title }}</h3>
              <ul class="rules">
                @for (r of s.rules; track r.id) {
                  <li class="rule" [attr.data-id]="r.id">
                    <div class="what">
                      <mc-icon class="kind" [name]="icon(r)" [size]="15" />
                      <span class="action">{{ action(r) }}</span>
                      <!-- A space between the words and each chip, for whoever
                           reads the text (the flex gap only shows it). -->
                      @for (p of r.patterns; track p) {
                        {{ ' ' }}<code class="pat">{{ p }}</code>
                      }
                    </div>
                    <span class="muted small meta">{{ meta(r) }}</span>
                    @if (confirming() === r.id) {
                      <p class="small confirm">
                        {{ 'Remove it? What it allowed asks again.' | t }}
                      </p>
                      <div class="acts">
                        <button
                          type="button"
                          class="btn danger small"
                          [disabled]="busy() === r.id"
                          (click)="remove(r)"
                        >
                          {{ busy() === r.id ? ('Removing…' | t) : ('Remove' | t) }}
                        </button>
                        <button
                          type="button"
                          class="btn ghost small"
                          (click)="confirming.set(null)"
                        >
                          {{ 'Keep it' | t }}
                        </button>
                      </div>
                    } @else {
                      <div class="acts">
                        <button
                          type="button"
                          class="btn ghost small"
                          [attr.aria-label]="'Edit {name}' | t: { name: label(r) }"
                          (click)="editing.set(r)"
                        >
                          <mc-icon name="edit" [size]="14" /> {{ 'Edit' | t }}
                        </button>
                        <button
                          type="button"
                          class="btn ghost small"
                          [attr.aria-label]="'Remove {name}' | t: { name: label(r) }"
                          (click)="confirming.set(r.id)"
                        >
                          <mc-icon name="trash" [size]="14" /> {{ 'Remove…' | t }}
                        </button>
                      </div>
                    }
                  </li>
                }
              </ul>
            }
          </section>
        } @empty {
          <p class="muted small">{{ 'None here.' | t }}</p>
        }
      }

      <mc-rule-sheet [rule]="editing()" (closed)="editing.set(null)" />
    </div>
  `,
  styles: [
    `
      .pause {
        margin-bottom: 0.75rem;
      }
      .pause strong {
        font-weight: 500;
      }
      .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        margin: 0 0 0.75rem;
      }
      .chip-btn {
        font: inherit;
        font-size: 0.82rem;
        padding: 0.25rem 0.7rem;
        border-radius: 1rem;
        border: 1px solid var(--border);
        background: var(--panel);
        color: var(--muted);
        cursor: pointer;
        min-height: 32px;
      }
      .chip-btn[aria-pressed='true'] {
        border-color: var(--accent);
        background: var(--accent-soft);
        color: var(--accent-text);
      }
      .notice {
        margin: 0 0 0.75rem;
      }
      .ghead {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        color: var(--accent-text);
      }
      .ghead h2 {
        margin: 0;
        color: var(--text);
      }
      .ghint {
        margin: 0.2rem 0 0;
      }
      .scope {
        margin: 0.9rem 0 0.1rem;
        font-size: 0.9rem;
        overflow-wrap: anywhere;
      }
      .rules {
        list-style: none;
        margin: 0;
        padding: 0;
      }
      .rule {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        padding: 0.55rem 0;
        border-bottom: 1px solid var(--border);
      }
      .rule:last-child {
        border-bottom: 0;
      }
      .what {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.3rem 0.4rem;
        min-width: 0;
      }
      .kind {
        color: var(--muted);
      }
      .action {
        overflow-wrap: anywhere;
      }
      .pat {
        font-size: 0.78rem;
        padding: 0.05rem 0.45rem;
        border-radius: 0.4rem;
        border: 1px solid var(--border-strong);
        background: var(--code-bg);
        overflow-wrap: anywhere;
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
      }
      .confirm {
        margin: 0.2rem 0 0;
      }
    `,
  ],
})
export class PermissionsComponent {
  protected readonly permissions = inject(PermissionRulesService);
  private readonly notices = inject(NoticeService);
  private readonly router = inject(Router);
  private readonly route = inject(ActivatedRoute);

  protected readonly chips = CHIPS;
  protected readonly computer = computerNoun;
  /** What is in flight: `pause`, or a permission's id. */
  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');
  /** The permission whose *Remove* waits for a second tap. */
  protected readonly confirming = signal<string | null>(null);
  /** The permission in the *Edit* sheet. */
  protected readonly editing = signal<PermissionRule | null>(null);

  /** The chip shown, from `?on=` (anything else: all). */
  protected readonly show = toSignal(
    this.route.queryParamMap.pipe(
      map((q) => {
        const on = q.get('on');
        return on === 'here' || on === 'others' ? (on as Show) : 'all';
      }),
    ),
    { initialValue: 'all' as Show },
  );
  /** Some are for other computers: the chips are worth showing. */
  protected readonly hasOthers = computed(() =>
    this.permissions.rules().some((r) => r.kind !== 'tool'),
  );
  protected readonly groups = computed(() => {
    const show = this.show();
    return groupRules(this.permissions.rules()).filter(
      (g) => show === 'all' || g.here === (show === 'here'),
    );
  });

  constructor() {
    inject(DestroyRef).onDestroy(this.permissions.watch());
  }

  protected pick(id: Show): void {
    void this.router.navigate([], {
      relativeTo: this.route,
      queryParams: { on: id === 'all' ? null : id },
      queryParamsHandling: 'merge',
      replaceUrl: true,
    });
  }

  protected icon(r: PermissionRule): IconName {
    if (r.kind === 'run') return 'terminal';
    if (r.kind === 'task') return 'send';
    switch (familyOf(r.tool)) {
      case 'shell':
        return 'terminal';
      case 'edit':
        return 'edit';
      case 'read':
        return 'file';
      case 'fetch':
      case 'search':
        return 'globe';
      default:
        return 'plug';
    }
  }

  protected action(r: PermissionRule): string {
    return ruleAction(r);
  }

  /** What it allows, its patterns too (a button's name). */
  protected label(r: PermissionRule): string {
    return r.patterns.length ? `${ruleAction(r)} ${r.patterns.join(', ')}` : ruleAction(r);
  }

  /** How often it was used and when last; who added it, when. */
  protected meta(r: PermissionRule): string {
    const time = r.usedAt ? relativeTime(r.usedAt) : '';
    const used = !r.uses
      ? tr('Not used yet')
      : r.uses === 1
        ? time
          ? tr('Used once, {time}', { time })
          : tr('Used once')
        : time
          ? tr('Used {n} times, last {time}', { n: r.uses, time })
          : tr('Used {n} times', { n: r.uses });
    const who = r.createdBy === 'desktop' ? computerNoun() : r.createdBy;
    const added = !r.createdAt
      ? ''
      : who
        ? tr('added {time} by {who}', { time: relativeTime(r.createdAt), who })
        : tr('added {time}', { time: relativeTime(r.createdAt) });
    return [used, added].filter(Boolean).join(' · ');
  }

  /** *Ask for everything*: the switch shows what the server keeps (back
   * as it was when the change fails). */
  protected async setPaused(input: HTMLInputElement): Promise<void> {
    const paused = input.checked;
    await this.run('pause', async () => {
      await this.permissions.setPaused(paused);
      this.notices.announce(
        paused ? tr('Claude asks for everything now.') : tr('Your permissions are used again.'),
      );
    });
    input.checked = this.permissions.paused();
  }

  protected async remove(r: PermissionRule): Promise<void> {
    await this.run(r.id, async () => {
      await this.permissions.remove(r.id);
      this.confirming.set(null);
      this.notices.success(tr('Permission removed'));
    });
  }

  private async run(key: string, fn: () => Promise<void>): Promise<void> {
    this.busy.set(key);
    this.error.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
