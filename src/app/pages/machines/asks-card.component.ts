import { ChangeDetectionStrategy, Component, inject, input, output, signal } from '@angular/core';
import { Router } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachineAsk } from '../../core/machines';
import { serverText } from './machines-text';

/**
 * What your other computers asked this one to do (docs/PLAN-MACHINES.md
 * §5.14, spikes/machines/UX.md §4.1): a check's fix, signing an account
 * in, adding an account, cloning a repository, keeping a project in sync
 * (§5.15). Nothing runs because
 * another computer asked: each waits for someone here to confirm it (the
 * desktop), or to say *Not now*, which that computer is told.
 */
@Component({
  selector: 'mc-machine-asks',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `
    <section class="card asks" [attr.aria-label]="'Asked by your other computers' | t">
      <h2 class="first">{{ 'Asked by your other computers' | t }}</h2>
      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      <ul class="list">
        @for (a of asks(); track a.id) {
          <li [attr.data-state]="a.state">
            <div class="row">
              <strong>{{ title(a) }}</strong>
              <span class="muted small">{{ 'From {name}' | t: { name: a.fromName } }}</span>
            </div>
            <span class="small">{{ detail(a) }}</span>
            @if (a.state === 'asked') {
              @if (!readonly()) {
                <div class="row acts">
                  <button
                    type="button"
                    class="btn primary small"
                    [disabled]="busy() === a.id"
                    (click)="answer(a, true)"
                  >
                    {{ confirmLabel(a) }}
                  </button>
                  <button
                    type="button"
                    class="btn ghost small"
                    [disabled]="busy() === a.id"
                    (click)="answer(a, false)"
                  >
                    {{ 'Not now' | t }}
                  </button>
                </div>
              }
            } @else {
              <span class="muted small state">{{ stateText(a) }}</span>
            }
          </li>
        }
      </ul>
      <p class="muted small">
        {{ 'Nothing runs because another computer asked: each waits for someone here.' | t }}
      </p>
    </section>
  `,
  styles: [
    `
      .list {
        list-style: none;
        margin: 0.3rem 0;
        padding: 0;
      }
      .list > li {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        padding: 0.4rem 0;
        border-bottom: 1px solid var(--border);
      }
      .list > li:last-child {
        border-bottom: 0;
      }
      .acts {
        margin-top: 0.2rem;
      }
      li[data-state='failed'] .state {
        color: var(--red);
      }
    `,
  ],
})
export class AsksCardComponent {
  readonly asks = input.required<readonly MachineAsk[]>();
  /** A phone sees them, never answers. */
  readonly readonly = input(false);
  readonly answered = output<MachineAsk | null>();

  private readonly api = inject(ApiService);
  private readonly router = inject(Router);
  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');

  protected title(a: MachineAsk): string {
    switch (a.kind) {
      case 'signIn':
        return tr('Sign in to {account}', { account: a.account ?? '' });
      case 'addAccount':
        return tr('Add the account {account}', { account: a.account ?? '' });
      case 'clone':
        return tr('Clone {repo}', { repo: a.title ?? a.origin ?? '' });
      case 'sync':
        return tr('Keep {project} in sync', { project: a.title ?? '' });
      default:
        return serverText(a.title) || tr('A fix');
    }
  }

  protected detail(a: MachineAsk): string {
    switch (a.kind) {
      case 'signIn':
        return tr('Its sign-in opens in this computer’s browser.');
      case 'addAccount':
        return tr('Add it in Accounts, under the same name, so sessions can move to it.');
      case 'clone':
        return tr('Clone {origin} into {folder}?', {
          origin: a.origin ?? '',
          folder: a.folder ?? '',
        });
      case 'sync':
        return tr(
          'Keep {folder} the same as {name}’s copy: what changes on either computer goes to the other. When both change it, you choose which to keep; nothing is merged.',
          { folder: a.folder ?? '', name: a.fromName },
        );
      default:
        return tr('Fix it on this computer: {fix}.', { fix: serverText(a.fix) });
    }
  }

  protected confirmLabel(a: MachineAsk): string {
    switch (a.kind) {
      case 'signIn':
        return tr('Sign in…');
      case 'addAccount':
        return tr('Add it…');
      case 'clone':
        return tr('Clone it');
      case 'sync':
        return tr('Keep it in sync');
      default:
        return serverText(a.fix) || tr('Fix');
    }
  }

  protected stateText(a: MachineAsk): string {
    switch (a.state) {
      case 'running':
        return serverText(a.detail) || tr('Under way…');
      case 'done':
        return serverText(a.detail) || tr('Done.');
      case 'declined':
        return tr('Not now.');
      case 'failed':
        return serverText(a.detail) || tr('It failed.');
      default:
        return '';
    }
  }

  protected async answer(a: MachineAsk, confirm: boolean): Promise<void> {
    this.busy.set(a.id);
    this.error.set('');
    try {
      const r = await this.api.answerMachineAsk(a.id, confirm ? 'confirm' : 'decline');
      this.answered.emit(r.ask);
      if (confirm && r.next) void this.router.navigateByUrl(r.next);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
