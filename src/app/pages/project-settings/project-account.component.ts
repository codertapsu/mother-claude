import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { accountForCwd } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { samePath, tildePath, trimSeparators } from '../../core/paths';

let nextId = 0;

/**
 * A project's account (its settings sheet, docs/UI-REDESIGN.md §9): the
 * account new sessions in its folder run under, and how that is decided —
 * its own rule, a rule for a folder above it, or the default for new
 * sessions. Picking one gives it its own rule; *The default* drops it. The
 * rules are Accounts' *Defaults › Per project* (`AccountsService.
 * setDefaults`), which the desktop and a paired phone change alike.
 */
@Component({
  selector: 'mc-project-account',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, TPipe],
  template: `
    @if (!accounts.multi()) {
      <p class="muted small">
        {{
          'Its sessions run under your Claude account. With another account, this project could have one of its own.'
            | t
        }}
        <a routerLink="/accounts">{{ 'Open Accounts' | t }}</a>
      </p>
    } @else {
      <div class="pick">
        <label class="grow" [for]="id">
          <strong>{{ 'New sessions here run under' | t }}</strong>
          <span class="muted small">{{ hint() }}</span>
        </label>
        <select [id]="id" [disabled]="saving()" (change)="pick($event)">
          <option value="" [selected]="!own()">
            {{ 'The default: {value}' | t: { value: inheritedName() } }}
          </option>
          @for (a of accounts.list(); track a.id) {
            <option [value]="a.id" [selected]="a.id === own()">{{ a.name }}</option>
          }
        </select>
      </div>
      @if (error()) {
        <p class="error-text small" role="alert">{{ error() }}</p>
      }
    }
  `,
  styles: [
    `
      .pick {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.5rem 0.75rem;
      }
      .grow {
        flex: 1 1 14rem;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
      }
      select {
        width: auto;
        flex: 0 1 14rem;
        min-width: 10rem;
      }
      p {
        margin: 0;
      }
    `,
  ],
})
export class ProjectAccountComponent {
  /** The project's folder. */
  readonly cwd = input.required<string>();

  protected readonly accounts = inject(AccountsService);
  protected readonly id = `mc-project-account-${nextId++}`;
  protected readonly saving = signal(false);
  protected readonly error = signal('');

  /** Its own rule's folder, as the rules write it. */
  private readonly ownKey = computed(() => {
    const rules = this.accounts.view()?.defaults.byProject ?? {};
    return Object.keys(rules).find((dir) => samePath(dir, this.cwd())) ?? null;
  });
  /** The account its own rule names ('' without one). */
  protected readonly own = computed(() => {
    const key = this.ownKey();
    return key ? (this.accounts.view()?.defaults.byProject[key] ?? '') : '';
  });
  /** What it runs under without a rule of its own. */
  private readonly inherited = computed(() => {
    const view = this.accounts.view();
    const key = this.ownKey();
    if (!view || !key) return accountForCwd(view, this.cwd());
    const byProject = { ...view.defaults.byProject };
    delete byProject[key];
    return accountForCwd({ ...view, defaults: { ...view.defaults, byProject } }, this.cwd());
  });
  protected readonly inheritedName = computed(() => this.nameOf(this.inherited().id));
  protected readonly hint = computed(() => {
    const own = this.own();
    if (own) {
      return tr(
        'Its own: sessions started in its folder and the folders inside it run under {name}, unless you pick another when you start one.',
        { name: this.nameOf(own) },
      );
    }
    const via = this.inherited();
    return via.via === 'project' && via.dir
      ? tr('From the rule for {dir}. Pick an account to give this project its own.', {
          dir: tildePath(via.dir),
        })
      : tr('The default for new sessions. Pick an account to give this project its own.');
  });

  constructor() {
    void this.accounts.ensureLoaded();
  }

  private nameOf(id: string): string {
    return this.accounts.byId(id)?.name ?? tr('A removed account');
  }

  /** Its own rule set to an account, or dropped (''). */
  protected async pick(e: Event): Promise<void> {
    const select = e.target as HTMLSelectElement;
    const defaults = this.accounts.view()?.defaults;
    if (!defaults || select.value === this.own()) return;
    const byProject = { ...defaults.byProject };
    const key = this.ownKey();
    if (key) delete byProject[key];
    if (select.value) byProject[trimSeparators(this.cwd().trim())] = select.value;
    this.saving.set(true);
    this.error.set('');
    try {
      await this.accounts.setDefaults({ ...defaults, byProject });
    } catch (err) {
      this.error.set(
        tr('Could not save the defaults: {error}', { error: describeError(err).text }),
      );
      select.value = this.own();
    } finally {
      this.saving.set(false);
    }
  }
}
