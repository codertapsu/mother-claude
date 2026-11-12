import { Component, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { DEFAULT_ACCOUNT } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { RateLimitNote, normalizeCapacity, rateLimitText } from '../../core/capacity';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { AccountView } from '../../core/models';
import { PLAN_INTERVALS, planIntervalLabel } from '../../core/plan-usage';
import { PlanUsageService } from '../../core/plan-usage.service';
import { RealtimeService } from '../../core/realtime.service';
import { AccountChipComponent } from '../../shared/account-chip.component';
import { TOKEN_NO_USAGE, signInLine } from '../../shared/accounts';
import { DesktopOnlyComponent } from '../../shared/desktop-only.component';
import { PlanUsageComponent } from '../../shared/plan-usage.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { AccountsHeadComponent } from './accounts-head.component';

/** Rate limits listed (the newest). */
const LIMITS_SHOWN = 5;

/**
 * *Accounts › Usage* (docs/UI-REDESIGN.md §6.6): how much of each Claude
 * account's plan is left — every window with its reset time, extra usage,
 * when it was read, *Refresh* — with its sessions running; how often the
 * usage is read (set on the desktop, a phone sees the value); and the rate
 * limits the team sessions hit (from Teams › Capacity). The *Right now*
 * bar's meter and the plan usage everywhere else link here.
 */
@Component({
  selector: 'mc-accounts-usage',
  imports: [
    AccountChipComponent,
    AccountsHeadComponent,
    DesktopOnlyComponent,
    PlanUsageComponent,
    RouterLink,
    TimeAgoComponent,
    TPipe,
  ],
  template: `
    <div class="page narrow">
      <mc-accounts-head />

      <h2>{{ 'How much is left' | t }}</h2>
      @if (!accounts.multi()) {
        <section class="card" [attr.aria-label]="'Plan usage' | t">
          <p class="muted small intro">
            {{ 'Your Claude plan: every session on this account shares it.' | t }}
          </p>
          @if (plan.usage()) {
            <mc-plan-usage [settingsLink]="false" />
          } @else {
            <p class="muted small">{{ 'No plan usage reported for this account yet.' | t }}</p>
          }
          <p class="muted small running">{{ runningText(defaultId) }}</p>
        </section>
      } @else {
        <p class="muted small intro">
          {{
            'Each Claude account has its own plan, shared by every session that runs under it.' | t
          }}
        </p>
        @for (a of accounts.list(); track a.id) {
          <section
            class="card acct"
            [attr.aria-label]="'Plan usage of {name}' | t: { name: a.name }"
          >
            <div class="ahead">
              <mc-account-chip [account]="a" />
              <span class="muted small who">{{ signIn(a) }}</span>
            </div>
            @if (plan.usageOf(a.id); as u) {
              @if (!u.available && a.kind === 'token') {
                <!-- A token can't read its plan: its running sessions report it. -->
                <p class="muted small none">{{ tokenNoUsage | t }}</p>
              } @else {
                <mc-plan-usage [account]="a.id" [settingsLink]="false" />
              }
            } @else {
              <p class="muted small none">
                {{
                  a.kind === 'token'
                    ? (tokenNoUsage | t)
                    : ('No plan usage reported for this account yet.' | t)
                }}
              </p>
            }
            <p class="muted small running">{{ runningText(a.id) }}</p>
          </section>
        }
      }

      <h2 id="interval">{{ 'Reading it' | t }}</h2>
      <section class="card" [attr.aria-label]="'Reading it' | t">
        <div class="row form-row">
          <div class="grow">
            @if (isDesktop()) {
              <label for="usage-interval"
                ><strong>{{ 'Read usage every' | t }}</strong></label
              >
            } @else {
              <strong>{{ 'Read usage every' | t }}</strong>
            }
            <span class="muted small">{{
              'While a dashboard is open and a session is active, the {computer} reads it this often.'
                | t: { computer: computer() }
            }}</span>
          </div>
          @if (plan.usage(); as u) {
            <mc-desktop-only [value]="intervalLabel(u.intervalSecs) | t">
              <select
                id="usage-interval"
                class="isel"
                [disabled]="saving()"
                (change)="setInterval($event)"
              >
                @for (o of intervals; track o.secs) {
                  <option [value]="o.secs" [selected]="o.secs === u.intervalSecs">
                    {{ o.label | t }}
                  </option>
                }
                @if (!knownInterval()) {
                  <option [value]="u.intervalSecs" [selected]="true">
                    {{ intervalLabel(u.intervalSecs) | t }}
                  </option>
                }
              </select>
            </mc-desktop-only>
          }
        </div>
        @if (plan.usage(); as u) {
          <p class="muted small why">
            {{
              'Why 5 min: usage moves only a few points in that time and sessions started here report it for free.'
                | t
            }}
            @if (u.intervalSecs > 0) {
              {{ 'While nothing is active it checks only every 30 min.' | t }}
            } @else {
              {{
                'Automatic checks are off: it is read when you tap Refresh, and by a session started here when a dashboard connects or a limit is hit.'
                  | t
              }}
            }
          </p>
        }
      </section>

      @if (limits().length) {
        <h2>{{ 'Rate limits teams hit' | t }}</h2>
        <section class="card list" [attr.aria-label]="'Rate limits teams hit' | t">
          @for (n of limits(); track n.at + n.account + n.status) {
            <div class="list-row small limit">
              <span class="grow"
                >{{ rateText(n) }}{{ n.pct !== undefined ? ' (' + n.pct + ' %)' : '' }}
                @if (accountName(n.account); as name) {
                  <span class="muted"> · {{ name }}</span>
                }
              </span>
              <span class="muted"><mc-time [ms]="n.at" /></span>
            </div>
          }
        </section>
        <p class="muted small more">
          <a routerLink="/capacity">{{ 'Teams › Capacity' | t }}</a>
          {{ 'says which role sessions wait for a window.' | t }}
        </p>
      }
    </div>
  `,
  styles: [
    `
      .intro {
        margin: 0 0 0.6rem;
      }
      section.card mc-plan-usage {
        margin-top: 0.2rem;
      }
      .acct + .acct {
        margin-top: 0.5rem;
      }
      .ahead {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.25rem 0.5rem;
        margin-bottom: 0.65rem;
      }
      .who {
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .none,
      .running {
        margin: 0.5rem 0 0;
      }
      .form-row {
        align-items: flex-start;
        flex-wrap: nowrap;
      }
      .grow {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
      }
      .isel {
        width: auto;
        min-width: 8rem;
      }
      .why {
        margin: 0.6rem 0 0;
      }
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .more {
        margin-top: 0.5rem;
      }
    `,
  ],
})
export class UsagePageComponent {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);
  protected readonly accounts = inject(AccountsService);
  protected readonly plan = inject(PlanUsageService);
  protected readonly isDesktop = inject(AuthService).isDesktop;
  protected readonly computer = computerNoun;

  protected readonly defaultId = DEFAULT_ACCOUNT;
  protected readonly tokenNoUsage = TOKEN_NO_USAGE;
  protected readonly intervals = PLAN_INTERVALS;
  protected readonly intervalLabel = planIntervalLabel;
  protected readonly rateText = rateLimitText;
  protected readonly saving = signal(false);
  /** The rate limits Claude reported to the team sessions, newest first. */
  protected readonly limits = signal<RateLimitNote[]>([]);
  /** The server's interval is one this app offers. */
  protected readonly knownInterval = computed(() => {
    const secs = this.plan.usage()?.intervalSecs;
    return PLAN_INTERVALS.some((o) => o.secs === secs);
  });
  /** Live sessions by their account. */
  private readonly running = computed(() => {
    const out = new Map<string, number>();
    for (const s of this.realtime.sessions()) {
      if (!s.live) continue;
      const id = s.account?.id ?? DEFAULT_ACCOUNT;
      out.set(id, (out.get(id) ?? 0) + 1);
    }
    return out;
  });

  constructor() {
    void this.accounts.ensureLoaded();
    void this.plan.ensureLoaded();
    void this.loadLimits();
  }

  protected signIn(a: AccountView): string {
    return signInLine(a).text;
  }

  protected runningText(account: string): string {
    const n = this.running().get(account) ?? 0;
    return n === 0
      ? tr('No session running')
      : n === 1
        ? tr('1 session running')
        : tr('{n} sessions running', { n });
  }

  protected accountName(id: string): string {
    return this.accounts.multi() ? (this.accounts.byId(id)?.name ?? '') : '';
  }

  protected async setInterval(e: Event): Promise<void> {
    const sel = e.target as HTMLSelectElement;
    const current = this.plan.usage()?.intervalSecs;
    const secs = Number(sel.value);
    if (!Number.isFinite(secs) || secs === current) return;
    this.saving.set(true);
    const ok = await this.plan.setIntervalSecs(secs);
    this.saving.set(false);
    // Refused: show what is really in effect again.
    if (!ok && current !== undefined) sel.value = String(current);
  }

  /** The team scheduler's rate limits (none when it can't be read). */
  private async loadLimits(): Promise<void> {
    try {
      const v = normalizeCapacity(await this.api.getCapacity());
      this.limits.set(
        [...(v?.rateLimits ?? [])].sort((a, b) => b.at - a.at).slice(0, LIMITS_SHOWN),
      );
    } catch {
      // The section stays away.
    }
  }
}
