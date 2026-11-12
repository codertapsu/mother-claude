import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';

import { AccountsService } from '../../core/accounts.service';
import { ApiService } from '../../core/api.service';
import { FixItem } from '../../core/attention';
import { AttentionService } from '../../core/attention.service';
import { AuthService } from '../../core/auth.service';
import { copyReasonText, copyTitle } from '../../core/copies';
import { jobIdOf, jobTitle, sentence } from '../../core/data-jobs';
import { DataJobsService } from '../../core/data-jobs.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachinesService } from '../../core/machines.service';
import { NoticeService } from '../../core/notice.service';
import { PermissionsService } from '../../core/permissions.service';
import { identityWarningText } from '../../core/sign-in';
import { StorageService } from '../../core/storage.service';
import { AttentionCardComponent } from '../../shared/attention-card.component';
import { DesktopOnlyComponent } from '../../shared/desktop-only.component';
import { capitalize } from '../../shared/util';
import { askStateText, askTitle } from '../machines/asks-card.component';
import { checkTexts, revocationText } from '../machines/machines-text';

/** Where a failed job's own list is. */
const JOB_PAGES: Record<string, string> = {
  backup: '/backup',
  restore: '/backup',
  pull: '/backup',
  move: '/accounts',
  copy: '/accounts',
};

/** One thing to fix, said in words. */
interface FixText {
  heading: string;
  why: string;
}

/**
 * One item of the Inbox's *To fix* (core/attention.ts): what stopped
 * working or works worse, and the one thing that fixes it — run here on
 * the desktop (a check's fix, asking another computer, keeping a sign-in),
 * or a link to the page where it is done. A phone sees what only the
 * desktop fixes, with where it is done.
 */
@Component({
  selector: 'mc-inbox-fix',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AttentionCardComponent, DesktopOnlyComponent, RouterLink, TPipe],
  template: `
    @let i = item();
    <mc-attention-card
      [heading]="text().heading"
      [why]="text().why"
      [tone]="i.tone"
      [machine]="chip()"
      [when]="i.at"
      [kind]="i.kind"
    >
      @switch (i.kind) {
        @case ('permission') {
          @if (i.permission.settingsUrl) {
            <button class="btn small primary" type="button" (click)="openSettings()">
              {{ 'Open System Settings' | t }}
            </button>
          } @else {
            <a class="btn small" routerLink="/onboarding">{{ 'Finish setup…' | t }}</a>
          }
        }
        @case ('check') {
          <mc-desktop-only [line]="'Fix it on the desktop app.' | t">
            @if (i.remote) {
              @if (i.check.fix && i.check.fix.kind !== 'manual') {
                <button class="btn small" type="button" [disabled]="busy()" (click)="askFix()">
                  {{ 'Ask {name} to fix' | t: { name: i.machineName } }}
                </button>
              }
            } @else if (i.check.fix && i.check.fix.kind !== 'manual') {
              <button class="btn small primary" type="button" [disabled]="busy()" (click)="fix()">
                {{ busy() ? ('Working…' | t) : checkFix() }}
              </button>
            } @else {
              <a class="btn small" routerLink="/computers">{{ 'Open Computers' | t }}</a>
            }
          </mc-desktop-only>
        }
        @case ('health') {
          <a class="btn small" routerLink="/computers">{{ 'Open Computers' | t }}</a>
        }
        @case ('signIn') {
          <mc-desktop-only [line]="'Fix it on the desktop app.' | t">
            @if (i.problem === 'identity') {
              <button class="btn small" type="button" [disabled]="busy()" (click)="keepIdentity()">
                {{ 'Keep it' | t }}
              </button>
              <a class="btn small" routerLink="/accounts">{{ 'Sign in again…' | t }}</a>
            } @else if (i.problem === 'tokenMissing') {
              <a class="btn small primary" routerLink="/accounts">{{ 'Add it…' | t }}</a>
            } @else {
              <a class="btn small primary" routerLink="/accounts">{{ 'Sign in…' | t }}</a>
            }
          </mc-desktop-only>
        }
        @case ('skew') {
          <mc-desktop-only [line]="'Fix it on the desktop app.' | t">
            @if (i.order === 'older') {
              <button class="btn small" type="button" [disabled]="busy()" (click)="askUpdate()">
                {{ 'Ask {name} to update' | t: { name: i.machineName } }}
              </button>
            } @else {
              <button class="btn small" type="button" [disabled]="busy()" (click)="updateHere()">
                {{ busy() ? ('Working…' | t) : ('Update it' | t) }}
              </button>
            }
          </mc-desktop-only>
        }
        @case ('sync') {
          <mc-desktop-only [line]="'Answer on the desktop app.' | t">
            <a class="btn small primary" routerLink="/computers/sync">{{ 'Choose…' | t }}</a>
          </mc-desktop-only>
        }
        @case ('copy') {
          <a class="btn small" routerLink="/computers" fragment="copies">{{
            'Open Computers' | t
          }}</a>
        }
        @case ('job') {
          @if (retryable()) {
            <button class="btn small primary" type="button" [disabled]="busy()" (click)="retry()">
              {{ busy() ? ('Starting…' | t) : ('Try again' | t) }}
            </button>
          } @else if (jobPage(); as page) {
            <a class="btn small" [routerLink]="page">{{ 'Details' | t }}</a>
          }
          <button class="btn small ghost" type="button" (click)="dismiss()">
            {{ 'Dismiss' | t }}
          </button>
        }
        @case ('ask') {
          <button class="btn small ghost" type="button" (click)="dismiss()">
            {{ 'Dismiss' | t }}
          </button>
        }
        @case ('revoked') {
          <button class="btn small ghost" type="button" (click)="dismiss()">
            {{ 'Dismiss' | t }}
          </button>
        }
        @case ('update') {
          <a class="btn small primary" routerLink="/settings" fragment="updates">{{
            'Install…' | t
          }}</a>
        }
      }
    </mc-attention-card>
  `,
})
export class FixItemComponent {
  private readonly api = inject(ApiService);
  private readonly router = inject(Router);
  private readonly notices = inject(NoticeService);
  private readonly attention = inject(AttentionService);
  private readonly accounts = inject(AccountsService);
  private readonly machines = inject(MachinesService);
  private readonly jobs = inject(DataJobsService);
  private readonly storage = inject(StorageService);
  private readonly permissions = inject(PermissionsService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  readonly item = input.required<FixItem>();
  protected readonly busy = signal(false);

  protected readonly text = computed<FixText>(() => this.say(this.item()));
  /** The other computer's chip, unless the title names it already. */
  protected readonly chip = computed(() => {
    const i = this.item();
    return i.kind === 'skew' || i.kind === 'copy' ? '' : (i.machineName ?? '');
  });
  /** A check's fix, as its button says it. */
  protected readonly checkFix = computed(() => {
    const i = this.item();
    return i.kind === 'check' ? checkTexts(i.check).fix : '';
  });
  /** A plain backup that failed runs again from here; a forced one is
   * redone where its dialog is. */
  protected readonly retryable = computed(() => {
    const i = this.item();
    if (i.kind !== 'job' || i.job.kind !== 'backup' || !i.job.account) return false;
    return (i.job.result as { force?: unknown } | undefined)?.force !== true;
  });
  protected readonly jobPage = computed(() => {
    const i = this.item();
    return i.kind === 'job' ? (JOB_PAGES[i.job.kind] ?? null) : null;
  });

  private say(i: FixItem): FixText {
    const computer = computerNoun();
    switch (i.kind) {
      case 'permission':
        return {
          heading: tr('{permission} is missing', { permission: i.permission.label }),
          why: i.permission.description,
        };
      case 'check': {
        const x = checkTexts(i.check);
        const why = x.detail || x.why;
        // A manual fix on another computer is done there: say how.
        if (i.remote && i.check.fix?.kind === 'manual') {
          return {
            heading: x.title,
            why: tr('On {name}: {fix}', { name: i.machineName ?? '', fix: x.fix }),
          };
        }
        return { heading: x.title, why };
      }
      case 'health':
        return {
          heading:
            i.blockers === 1
              ? tr('A required check fails on your computers')
              : tr('{n} required checks fail on your computers', { n: i.blockers }),
          why: tr('See which, and fix them, on Computers.'),
        };
      case 'signIn': {
        const a = i.accountView;
        const name = a.name;
        switch (i.problem) {
          case 'identity': {
            const w = a.identityWarning!;
            return {
              heading:
                w.kind === 'same-as'
                  ? tr('{name} signed in as another account here', { name })
                  : tr('{name} signed in as someone else', { name }),
              why: identityWarningText(w, (id) => this.accounts.byId(id)?.name),
            };
          }
          case 'tokenMissing':
            return {
              heading: tr('{name} has no token on this {computer}', { name, computer }),
              why: tr('Sessions can’t run under it until its token is added here.'),
            };
          case 'authFailed':
            return {
              heading: tr('{name} can’t sign in', { name }),
              why:
                a.kind === 'token'
                  ? tr('Its token was refused: get a new one')
                  : tr('A session could not sign in: sign in again'),
            };
          default:
            return {
              heading: tr('{name} is signed out', { name }),
              why: tr('Its sessions can’t run until it signs in again.'),
            };
        }
      }
      case 'skew': {
        const p = i.peer;
        const mine = this.machines.view()?.self.versions;
        if (i.what === 'app') {
          return i.order === 'older'
            ? {
                heading: tr('{name} needs an update', { name: p.name }),
                why: tr('{name} runs an older Mother Claude ({v}; this {computer}: {w}).', {
                  name: p.name,
                  v: p.versions?.app ?? '',
                  w: mine?.app ?? '',
                  computer,
                }),
              }
            : {
                heading: capitalize(tr('This {computer} needs an update', { computer })),
                why: tr('This {computer} runs an older Mother Claude than {name} ({v}).', {
                  name: p.name,
                  v: p.versions?.app ?? '',
                  computer,
                }),
              };
        }
        return i.order === 'older'
          ? {
              heading: tr('{name} needs an update', { name: p.name }),
              why: tr('{name} runs an older Claude Code ({v}; here: {w}).', {
                name: p.name,
                v: p.versions?.claude ?? '',
                w: mine?.claude ?? '',
              }),
            }
          : {
              heading: capitalize(tr('This {computer} needs an update', { computer })),
              why: tr('Claude Code here is older than on {name} ({v}).', {
                name: p.name,
                v: p.versions?.claude ?? '',
              }),
            };
      }
      case 'sync':
        return {
          heading: tr('{project} changed on both computers', { project: i.pair.name }),
          why: tr('Nothing syncs until you choose which side to keep; the other is saved first.'),
        };
      case 'copy':
        return {
          heading: tr('The copy of {session} on {name} is paused', {
            session: copyTitle(i.copy),
            name: i.copy.machineName,
          }),
          why: i.copy.status ? sentence(copyReasonText(i.copy.status)) : '',
        };
      case 'job': {
        const what = jobTitle(
          i.job,
          (id) => this.accounts.byId(id)?.name,
          (id) => this.storage.labelOf(id),
        );
        return {
          heading: tr('{what} failed', { what }),
          why: sentence(i.job.error) || tr('It failed.'),
        };
      }
      case 'ask':
        return { heading: askTitle(i.ask), why: askStateText(i.ask) };
      case 'revoked':
        return {
          heading: tr('{name} is not revoked everywhere yet', { name: i.revocation.name }),
          why: `${revocationText(i.revocation)} ${tr('Each one revokes it when it next connects: until then it lets it in.')}`,
        };
      default:
        return {
          heading: tr('Mother Claude {version} is ready', { version: i.version }),
          why: tr('Install it from Software updates: the app restarts by itself.'),
        };
    }
  }

  protected openSettings(): void {
    const i = this.item();
    if (i.kind === 'permission') void this.permissions.openSettings(i.permission.id);
  }

  /** This computer's check's fix (as Computers runs it). */
  protected async fix(): Promise<void> {
    const i = this.item();
    if (i.kind !== 'check') return;
    const id = i.check.id;
    // *Update Mother Claude*: the Software updates page installs it.
    if (id === 'app') {
      void this.router.navigate(['/settings'], { fragment: 'updates' });
      return;
    }
    await this.run(async () => {
      this.machines.setup.set(await this.api.fixMachineCheck(id));
      await this.machines.load();
      // An account's sign-in goes on in Accounts (its link, its code).
      if (id.startsWith('account:')) void this.router.navigate(['/accounts']);
    });
  }

  /** Another computer's check: asked there, done when someone there says. */
  protected async askFix(): Promise<void> {
    const i = this.item();
    if (i.kind !== 'check' || !i.machine) return;
    await this.run(async () => {
      await this.api.askMachineFix(i.machine!, i.check.id);
      this.notices.success(
        tr('Asked {name}: it shows the fix to whoever is at it.', { name: i.machineName ?? '' }),
      );
    });
  }

  protected async askUpdate(): Promise<void> {
    const i = this.item();
    if (i.kind !== 'skew') return;
    await this.run(async () => {
      await this.api.askMachineFix(i.peer.id, i.what === 'app' ? 'app' : 'claudeCode');
      this.notices.success(
        tr('Asked {name}: it shows the update to whoever is at it.', { name: i.peer.name }),
      );
    });
  }

  /** This computer is the older one: Mother Claude on Software updates,
   * Claude Code by its own update (this computer's check's fix). */
  protected async updateHere(): Promise<void> {
    const i = this.item();
    if (i.kind !== 'skew') return;
    if (i.what === 'app') {
      void this.router.navigate(['/settings'], { fragment: 'updates' });
      return;
    }
    await this.run(async () => {
      this.machines.setup.set(await this.api.fixMachineCheck('claudeCode'));
      await this.machines.load();
    });
  }

  protected async keepIdentity(): Promise<void> {
    const i = this.item();
    if (i.kind !== 'signIn') return;
    await this.run(async () => {
      await this.api.acceptIdentity(i.accountView.id);
      await this.accounts.refresh();
    });
  }

  /** The backup again; its job is followed, so the failure goes once it
   * runs through. */
  protected async retry(): Promise<void> {
    const i = this.item();
    if (i.kind !== 'job' || !i.job.account) return;
    await this.run(async () => {
      const res = await this.api.pushBackup(i.job.account);
      for (const id of new Set((res?.jobs ?? []).map(jobIdOf).filter(Boolean))) {
        void this.jobs.follow(id, () => undefined).catch(() => undefined);
      }
    });
  }

  protected dismiss(): void {
    this.attention.dismiss(this.item().id);
  }

  private async run(fn: () => Promise<void>): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    try {
      await fn();
    } catch (e) {
      this.notices.error(e);
    } finally {
      this.busy.set(false);
    }
  }
}
