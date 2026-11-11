import {
  ChangeDetectionStrategy,
  Component,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService, describeError } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachinesSettings, RelayMode } from '../../core/machines';

/** A relay setting in words. */
export function relayText(s: MachinesSettings['relay'] | null | undefined): string {
  switch (s?.mode) {
    case 'custom':
      return tr('Your own relay: {url}', { url: s.url ?? '' });
    case 'none':
      return tr('No relay');
    default:
      return tr('Public relays (default)');
  }
}

/**
 * The options of D10 that Machines uses now (docs/PLAN-MACHINES.md §5.3,
 * §5.14): how computers meet across networks (n0's public relays, your own
 * relay, or none) and keeping this computer awake while another uses it.
 * A changed relay is offered to the other computers, each of which takes
 * it only when its own desktop accepts.
 */
@Component({
  selector: 'mc-machines-options',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, TPipe],
  template: `
    <form class="stack" (ngSubmit)="save()">
      <fieldset class="relay">
        <legend class="field-label">{{ 'Relay' | t }}</legend>
        <label class="opt">
          <input
            type="radio"
            name="relay"
            value="public"
            [ngModel]="mode()"
            (ngModelChange)="mode.set($event)"
          />
          <span>
            <strong>{{ 'Public relays (default)' | t }}</strong>
            <span class="muted small">{{
              'Works at once, anywhere. The relay only ever carries encrypted traffic.' | t
            }}</span>
          </span>
        </label>
        <label class="opt">
          <input
            type="radio"
            name="relay"
            value="custom"
            [ngModel]="mode()"
            (ngModelChange)="mode.set($event)"
          />
          <span>
            <strong>{{ 'Your own relay' | t }}</strong>
            <span class="muted small">{{
              'An iroh-relay you run, on your NAS or a small server: no third party in the path.'
                | t
            }}</span>
          </span>
        </label>
        @if (mode() === 'custom') {
          <label class="field url">
            <span class="field-label">{{ "Your relay's address" | t }}</span>
            <input
              name="url"
              type="url"
              placeholder="https://relay.example.com"
              [ngModel]="url()"
              (ngModelChange)="url.set($event)"
            />
          </label>
        }
        <label class="opt">
          <input
            type="radio"
            name="relay"
            value="none"
            [ngModel]="mode()"
            (ngModelChange)="mode.set($event)"
          />
          <span>
            <strong>{{ 'No relay' | t }}</strong>
            <span class="muted small">{{
              'Your computers meet on the same network only.' | t
            }}</span>
          </span>
        </label>
      </fieldset>
      <label class="switch">
        <input
          type="checkbox"
          name="keepAwake"
          [checked]="keepAwake()"
          (change)="keepAwake.set($any($event.target).checked)"
        />
        <span class="switch-text">
          <span>{{
            'Keep this {computer} awake while another computer uses it'
              | t: { computer: computer() }
          }}</span>
          <span class="muted small">{{
            'Mother Claude holds it awake only while a paired computer is connected; no system setting changes.'
              | t
          }}</span>
        </span>
      </label>
      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      <div class="row">
        <button type="submit" class="btn primary" [disabled]="busy() || readonly()">
          {{ busy() ? ('Saving…' | t) : ('Save' | t) }}
        </button>
        @if (cancellable()) {
          <button type="button" class="btn ghost" [disabled]="busy()" (click)="cancelled.emit()">
            {{ 'Cancel' | t }}
          </button>
        }
      </div>
    </form>
  `,
  styles: [
    `
      fieldset.relay {
        border: 0;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
      }
      .opt {
        display: flex;
        gap: 0.5rem;
        align-items: flex-start;
        cursor: pointer;
      }
      .opt > span {
        display: flex;
        flex-direction: column;
      }
      .url {
        margin-left: 1.6rem;
      }
    `,
  ],
})
export class OptionsFormComponent {
  readonly settings = input.required<MachinesSettings>();
  readonly readonly = input(false);
  readonly cancellable = input(false);
  readonly saved = output<MachinesSettings>();
  readonly cancelled = output<void>();

  private readonly api = inject(ApiService);
  protected readonly computer = computerNoun;
  protected readonly mode = signal<RelayMode>('public');
  protected readonly url = signal('');
  protected readonly keepAwake = signal(true);
  protected readonly busy = signal(false);
  protected readonly error = signal('');

  constructor() {
    effect(() => {
      const s = this.settings();
      untracked(() => {
        this.mode.set(s.relay.mode);
        this.url.set(s.relay.url ?? '');
        this.keepAwake.set(s.keepAwake);
      });
    });
  }

  protected async save(): Promise<void> {
    const url = this.url().trim();
    if (
      this.mode() === 'custom' &&
      !/^https:\/\/\S+$/i.test(url) &&
      !/^http:\/\/localhost/i.test(url)
    ) {
      this.error.set(tr("Your relay's address must start with https://."));
      return;
    }
    this.busy.set(true);
    this.error.set('');
    try {
      const next = await this.api.saveMachinesSettings({
        relay: { mode: this.mode(), url: this.mode() === 'custom' ? url : null },
        keepAwake: this.keepAwake(),
      });
      this.saved.emit(next);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }
}
