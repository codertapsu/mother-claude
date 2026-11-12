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
import {
  CodeApply,
  CodeTransfer,
  MachinesSettings,
  RelayMode,
  defaultCode,
  normalizePin,
} from '../../core/machines';

/** A relay setting in words. */
export function relayText(s: MachinesSettings['relay'] | null | undefined): string {
  switch (s?.mode) {
    case 'custom': {
      const pin = normalizePin(s.certSha256);
      return pin
        ? tr('Your own relay: {url}, its certificate pinned ({sha}…)', {
            url: s.url ?? '',
            sha: pin.slice(0, 12),
          })
        : tr('Your own relay: {url}', { url: s.url ?? '' });
    }
    case 'none':
      return tr('No relay');
    default:
      return tr('Public relays (default)');
  }
}

/**
 * The options of D10 (docs/PLAN-MACHINES.md §5.3, §5.10, §5.14): how
 * computers meet across networks (n0's public relays, your own relay, or
 * none), how a moving session's code travels (machine to machine, or
 * through your git remote) and where it lands (the project folder when it
 * has no changes of its own, or always a worktree), how much this computer
 * keeps of each other computer's copies (§5.9), and keeping this computer
 * awake while another uses it. A changed relay or code option is
 * offered to the other computers, each of which takes it only when its own
 * desktop accepts.
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
          <label class="field url">
            <span class="field-label">{{ "Its certificate's SHA-256 (optional)" | t }}</span>
            <input
              name="certSha256"
              type="text"
              autocomplete="off"
              spellcheck="false"
              placeholder="AB:CD:…"
              [ngModel]="pin()"
              (ngModelChange)="pin.set($event)"
            />
            <span class="muted small">{{
              'Pinned, your relay is taken only with exactly this certificate, whoever signed it — a self-signed one too. A renewed certificate needs its new SHA-256.'
                | t
            }}</span>
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
      <fieldset class="relay">
        <legend class="field-label">{{ 'How code travels when a session moves' | t }}</legend>
        <label class="opt">
          <input
            type="radio"
            name="transfer"
            value="bundle"
            [ngModel]="transfer()"
            (ngModelChange)="transfer.set($event)"
          />
          <span>
            <strong>{{ 'Computer to computer (default)' | t }}</strong>
            <span class="muted small">{{
              'Straight to the other computer: no remote needed, nothing pushed anywhere.' | t
            }}</span>
          </span>
        </label>
        <label class="opt">
          <input
            type="radio"
            name="transfer"
            value="remote"
            [ngModel]="transfer()"
            (ngModelChange)="transfer.set($event)"
          />
          <span>
            <strong>{{ 'Through your git remote' | t }}</strong>
            <span class="muted small">{{
              'GitHub or another: both computers need access to it without a password prompt.' | t
            }}</span>
          </span>
        </label>
        @if (transfer() === 'remote') {
          <label class="opt sub">
            <input
              type="checkbox"
              name="keepRemoteRefs"
              [checked]="keepRemoteRefs()"
              (change)="keepRemoteRefs.set($any($event.target).checked)"
            />
            <span>
              <span>{{ 'Keep the hidden refs on the remote after a move' | t }}</span>
              <span class="muted small">{{
                'Off: they are deleted there once the other computer has them (kept locally for undo).'
                  | t
              }}</span>
            </span>
          </label>
        }
      </fieldset>
      <fieldset class="relay">
        <legend class="field-label">{{ 'Where the code lands' | t }}</legend>
        <label class="opt">
          <input
            type="radio"
            name="apply"
            value="checkout"
            [ngModel]="apply()"
            (ngModelChange)="apply.set($event)"
          />
          <span>
            <strong>{{
              'In the project folder, when it has no changes of its own (default)' | t
            }}</strong>
          </span>
        </label>
        <label class="opt">
          <input
            type="radio"
            name="apply"
            value="worktree"
            [ngModel]="apply()"
            (ngModelChange)="apply.set($event)"
          />
          <span>
            <strong>{{
              'Always in a separate folder (a worktree), never the project folder' | t
            }}</strong>
          </span>
        </label>
      </fieldset>
      <label class="field cap">
        <span class="field-label">{{ 'Copies of other computers’ sessions' | t }}</span>
        <span class="cap-row">
          <input
            name="capGb"
            type="number"
            min="0.1"
            max="4000"
            step="0.5"
            [ngModel]="capGb()"
            (ngModelChange)="capGb.set($event)"
          />
          <span>{{ 'GB at most for each computer' | t }}</span>
        </span>
        <span class="muted small">{{
          'When a computer’s copies here grow past it, the copy refreshed longest ago goes first.'
            | t
        }}</span>
      </label>
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
      <p class="muted small">
        {{
          'Each project can change these, and each move too. The other computers take a change when someone there accepts it.'
            | t
        }}
      </p>
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
      .sub {
        margin-left: 1.6rem;
      }
      .cap-row {
        display: flex;
        align-items: center;
        gap: 0.5rem;
      }
      .cap-row input {
        width: 6rem;
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
  /** Your relay's certificate's SHA-256, as typed (colons allowed). */
  protected readonly pin = signal('');
  protected readonly keepAwake = signal(true);
  protected readonly transfer = signal<CodeTransfer>('bundle');
  protected readonly apply = signal<CodeApply>('checkout');
  protected readonly keepRemoteRefs = signal(false);
  /** The copies' cap, in GB (1000³ bytes). */
  protected readonly capGb = signal(2.1);
  /** …as the settings had it: sent only when changed. */
  private capGbBefore = 2.1;
  protected readonly busy = signal(false);
  protected readonly error = signal('');

  constructor() {
    effect(() => {
      const s = this.settings();
      untracked(() => {
        const code = s.code ?? defaultCode();
        this.mode.set(s.relay.mode);
        this.url.set(s.relay.url ?? '');
        this.pin.set(s.relay.certSha256 ?? '');
        this.keepAwake.set(s.keepAwake);
        this.transfer.set(code.transfer);
        this.apply.set(code.apply);
        this.keepRemoteRefs.set(code.keepRemoteRefs);
        this.capGb.set(Math.round(((s.copiesCapBytes ?? 2 * 1024 ** 3) / 1e9) * 10) / 10);
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
    const typed = this.mode() === 'custom' ? this.pin().trim() : '';
    const pin = normalizePin(typed);
    if (typed && !pin) {
      this.error.set(tr("A certificate's SHA-256 is 64 hexadecimal digits (colons allowed)."));
      return;
    }
    if (pin && !/^https:\/\//i.test(url)) {
      this.error.set(tr('Pinning a certificate needs an https:// address.'));
      return;
    }
    const capChanged = Number(this.capGb()) !== this.capGbBefore;
    const cap = Math.round(Number(this.capGb()) * 1e9);
    if (capChanged && (!Number.isFinite(cap) || cap < 64 * 1024 ** 2 || cap > 4 * 1024 ** 4)) {
      this.error.set(tr('Keep between 0.1 and 4000 GB of each computer’s copies.'));
      return;
    }
    this.busy.set(true);
    this.error.set('');
    try {
      const next = await this.api.saveMachinesSettings({
        // The whole setting: no pin sent is none kept.
        relay: {
          mode: this.mode(),
          url: this.mode() === 'custom' ? url : null,
          ...(pin ? { certSha256: pin } : {}),
        },
        keepAwake: this.keepAwake(),
        code: {
          transfer: this.transfer(),
          apply: this.apply(),
          keepRemoteRefs: this.keepRemoteRefs(),
        },
        ...(capChanged ? { copiesCapBytes: cap } : {}),
      });
      this.saved.emit({
        relay: next.relay,
        keepAwake: next.keepAwake,
        code: next.code,
        copiesCapBytes: next.copiesCapBytes,
      });
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }
}
