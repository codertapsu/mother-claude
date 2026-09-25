import { Injectable, inject } from '@angular/core';

import { ApiService } from './api.service';
import { AuthService } from './auth.service';
import { PushService } from './push.service';

/**
 * This browser as a paired device. One place for "Forget this device", so
 * every page that offers it revokes the device on the Mac too, not only the
 * token in this browser.
 */
@Injectable({ providedIn: 'root' })
export class DeviceService {
  private api = inject(ApiService);
  private auth = inject(AuthService);
  private push = inject(PushService);

  /**
   * Unpair: drop this browser's push subscription (best effort), revoke the
   * device on the Mac (its token and server-side push subscriptions stop
   * working everywhere), then forget the token here, even when the Mac
   * can't be reached right now.
   */
  async forgetThisDevice(): Promise<void> {
    if (this.push.state() === 'on') await this.push.disable();
    await this.api.unpairSelf().catch(() => undefined);
    this.auth.forget();
  }

  /** Revoke this device on the Mac but keep going (re-pairing replaces it). */
  async releaseForRepair(): Promise<void> {
    await this.api.unpairSelf().catch(() => undefined);
  }
}
