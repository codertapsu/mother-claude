import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  linkedSignal,
  output,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiError, ApiService, describeError } from '../core/api.service';
import { PendingPair } from '../core/models';
import { NoticeService } from '../core/notice.service';
import { IconComponent } from './icon.component';
import { SheetComponent } from './sheet.component';
import { TimeAgoComponent } from './time-ago.component';

const INGRESS: Record<string, string> = {
  loopback: 'this computer',
  lan: 'your local network',
  tunnel: 'the internet (through your tunnel)',
};

/**
 * Desktop: approve a device that asked to pair. The device shows a 4-digit
 * number; typing it here proves the request comes from the device in the
 * user's hand, not from someone else who got hold of the code.
 */
@Component({
  selector: 'mc-pair-approval-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, IconComponent, SheetComponent, TimeAgoComponent],
  template: `
    <mc-sheet [open]="!!pending()" title="Approve a new device?" (closed)="closed.emit()">
      @if (pending(); as p) {
        <div class="who">
          <mc-icon name="phone" [size]="22" />
          <div>
            <strong>{{ p.deviceName || p.name || 'A device' }}</strong>
            <p class="muted small">
              {{ p.platform || 'unknown platform' }} · from {{ where(p)
              }}{{ p.ip ? ' (' + p.ip + ')' : '' }}
              @if (p.createdAt) {
                · <mc-time [ms]="p.createdAt" />
              }
            </p>
          </div>
        </div>
        <form class="form" (submit)="approve($event, p)">
          <label class="field">
            <span class="field-label">Number shown on the device</span>
            <input
              name="digits"
              class="digits"
              inputmode="numeric"
              autocomplete="off"
              maxlength="4"
              pattern="[0-9]*"
              [ngModel]="digits()"
              (ngModelChange)="digits.set(clean($event)); error.set('')"
              placeholder="••••"
              aria-describedby="pair-help"
            />
          </label>
          <p id="pair-help" class="muted small">
            Only approve a device you are holding. It gets access to all your Claude sessions until
            you revoke it in Remote › Devices.
          </p>
          @if (error()) {
            <p class="error-text small" role="alert">{{ error() }}</p>
          }
          <div class="row">
            <button class="btn primary" type="submit" [disabled]="!valid() || busy()">
              <mc-icon name="check" [size]="15" /> Approve
            </button>
            <button class="btn danger" type="button" [disabled]="busy()" (click)="deny(p)">
              Deny
            </button>
          </div>
        </form>
      }
    </mc-sheet>
  `,
  styles: [
    `
      .who {
        display: flex;
        gap: 0.75rem;
        align-items: flex-start;
        margin-bottom: 0.75rem;
      }
      .who mc-icon {
        color: var(--accent-text);
        margin-top: 0.15rem;
      }
      .who p {
        margin: 0.15rem 0 0;
      }
      .form {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }
      .digits {
        font-family: var(--mono);
        font-size: 1.8rem;
        letter-spacing: 0.5em;
        text-align: center;
        max-width: 12rem;
      }
    `,
  ],
})
export class PairApprovalDialogComponent {
  private api = inject(ApiService);
  private notices = inject(NoticeService);

  readonly pending = input<PendingPair | null>(null);
  readonly closed = output<void>();
  /** Emitted after the server accepted the approval or denial. */
  readonly settled = output<{ pending: PendingPair; approved: boolean }>();

  protected readonly digits = linkedSignal({ source: this.pending, computation: () => '' });
  protected readonly error = linkedSignal({ source: this.pending, computation: () => '' });
  protected readonly busy = signal(false);
  protected readonly valid = computed(() => /^\d{4}$/.test(this.digits()));

  protected clean(v: string): string {
    return String(v ?? '')
      .replace(/\D/g, '')
      .slice(0, 4);
  }

  protected where(p: PendingPair): string {
    return INGRESS[p.ingress ?? ''] ?? p.ingress ?? 'an unknown network';
  }

  protected async approve(e: Event, p: PendingPair): Promise<void> {
    e.preventDefault();
    if (!this.valid() || this.busy()) return;
    this.busy.set(true);
    try {
      await this.api.approvePair(p.id, this.digits());
      this.notices.success(`Approved ${p.deviceName || p.name || 'the device'}.`);
      this.settled.emit({ pending: p, approved: true });
      this.closed.emit();
    } catch (err) {
      this.error.set(
        err instanceof ApiError && [400, 403, 409, 422].includes(err.status)
          ? "Those digits don't match. Check the number on the device and try again."
          : describeError(err).text,
      );
    } finally {
      this.busy.set(false);
    }
  }

  protected async deny(p: PendingPair): Promise<void> {
    this.busy.set(true);
    try {
      await this.api.denyPair(p.id);
      this.settled.emit({ pending: p, approved: false });
      this.closed.emit();
    } catch (err) {
      this.error.set(describeError(err).text);
    } finally {
      this.busy.set(false);
    }
  }
}
