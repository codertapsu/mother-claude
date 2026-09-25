import {
  Component,
  DestroyRef,
  NgZone,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DomSanitizer, SafeHtml } from '@angular/platform-browser';
import { RouterLink } from '@angular/router';

import { ApiError, ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import {
  DeviceInfo,
  ForeignControlStatus,
  PairCode,
  PairTarget,
  PendingPair,
  RemoteInfo,
} from '../../core/models';
import { NowService } from '../../core/now.service';
import { RealtimeService } from '../../core/realtime.service';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { IconComponent } from '../../shared/icon.component';
import { PairApprovalDialogComponent } from '../../shared/pair-approval-dialog.component';
import { RelativeTimePipe } from '../../shared/time.pipes';

const POLL_MS = 10_000;

const INGRESS_LABELS: Record<string, string> = {
  loopback: 'this computer',
  lan: 'local network',
  tunnel: 'internet',
  'bridge-loopback': 'this computer (API)',
};

const PROVIDER_LABELS: Record<string, string> = {
  ngrok: 'ngrok',
  cloudflared: 'Cloudflare Tunnel',
  tailscale: 'Tailscale Funnel',
};

/**
 * Desktop: everything about reaching Mother Claude from other devices —
 * pairing QR codes (LAN or internet), pending pair approvals, internet access
 * through a tunnel (detection + copyable commands), paired devices, and the
 * foreign-session control plugin.
 */
@Component({
  selector: 'mc-remote',
  imports: [
    FormsModule,
    RouterLink,
    CopyButtonComponent,
    IconComponent,
    PairApprovalDialogComponent,
    RelativeTimePipe,
  ],
  templateUrl: './remote.component.html',
  styleUrl: './remote.component.scss',
})
export class RemoteComponent {
  private api = inject(ApiService);
  private auth = inject(AuthService);
  private realtime = inject(RealtimeService);
  private sanitizer = inject(DomSanitizer);
  private clock = inject(NowService);
  private zone = inject(NgZone);

  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly remote = signal<RemoteInfo | null>(null);
  protected readonly devices = signal<DeviceInfo[]>([]);
  protected readonly pendingPairs = signal<PendingPair[]>([]);
  /** The pairing request whose typed-number dialog is open. */
  protected readonly approving = signal<PendingPair | null>(null);
  protected readonly foreign = signal<ForeignControlStatus | null>(null);
  protected readonly error = signal('');
  protected readonly notice = signal('');

  // Pairing QR
  protected readonly target = signal<PairTarget>('lan');
  protected readonly lanUrl = signal('');
  protected readonly code = signal<PairCode | null>(null);
  protected readonly qr = signal<SafeHtml | null>(null);
  protected readonly creating = signal(false);
  protected readonly codeError = signal('');

  // Internet
  protected readonly manualUrl = signal('');
  protected readonly savingRemote = signal(false);

  // Devices
  protected readonly busy = signal<ReadonlySet<string>>(new Set());
  protected readonly renaming = signal<string | null>(null);
  protected readonly renameText = signal('');
  protected readonly confirming = signal<string | null>(null);
  private confirmTimer?: ReturnType<typeof setTimeout>;

  // Foreign control
  protected readonly foreignBusy = signal(false);
  protected readonly foreignError = signal('');

  protected readonly expiresIn = computed(() => {
    const c = this.code();
    if (!c) return 0;
    return Math.max(0, Math.round((c.expiresAt - this.clock.now()) / 1000));
  });
  protected readonly expired = computed(() => !!this.code() && this.expiresIn() === 0);
  protected readonly countdown = computed(() => {
    const s = this.expiresIn();
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
  });
  protected readonly groupedCode = computed(() =>
    (this.code()?.code ?? '').replace(/(.{4})(?=.)/g, '$1-'),
  );
  protected readonly publicUrl = computed(() => {
    const i = this.remote()?.internet;
    return i?.publicUrl || i?.manualUrl || i?.detected[0]?.url || '';
  });
  protected readonly internetReady = computed(
    () => !!this.remote()?.internet.enabled && !!this.publicUrl(),
  );
  protected readonly activeDevices = computed(() => this.devices().filter((d) => !d.revoked));

  constructor() {
    const destroyRef = inject(DestroyRef);
    const subs = [
      this.realtime.devices$.subscribe(() => void this.loadDevices()),
      this.realtime.pairRequest$.subscribe((f) => {
        if (!f.pending?.id) return;
        this.pendingPairs.update((prev) =>
          prev.some((p) => p.id === f.pending.id) ? prev : [...prev, f.pending],
        );
      }),
      this.realtime.refetch$.subscribe(() => void this.loadAll()),
    ];
    // The server only probes for tunnels while this page (or internet
    // access) is on, so keep asking while it's open.
    let timer: ReturnType<typeof setInterval> | undefined;
    this.zone.runOutsideAngular(() => {
      timer = setInterval(() => {
        if (document.visibilityState === 'visible' && this.isDesktop()) {
          this.zone.run(() => void this.loadRemote());
        }
      }, POLL_MS);
    });
    destroyRef.onDestroy(() => {
      clearInterval(timer);
      clearTimeout(this.confirmTimer);
      subs.forEach((s) => s.unsubscribe());
    });
    // Who we are (`/me`) may arrive after this page opened: load once the
    // desktop principal is known.
    let loaded = false;
    effect(() => {
      if (!this.isDesktop() || loaded) return;
      loaded = true;
      untracked(() => void this.loadAll());
    });
  }

  // ───────────────────────────────────────────────────────────── Pairing

  protected setTarget(t: PairTarget): void {
    if (this.target() === t) return;
    this.target.set(t);
    this.code.set(null);
    this.qr.set(null);
    this.codeError.set('');
  }

  protected async createCode(): Promise<void> {
    this.creating.set(true);
    this.codeError.set('');
    try {
      const target = this.target();
      const baseUrl = target === 'lan' ? this.lanUrl() || undefined : this.publicUrl() || undefined;
      const code = await this.api.createPairCode({ target, ...(baseUrl ? { baseUrl } : {}) });
      this.code.set(code);
      this.qr.set(this.sanitizer.bypassSecurityTrustHtml(code.svg ?? ''));
    } catch (e) {
      this.code.set(null);
      this.qr.set(null);
      this.codeError.set(
        e instanceof ApiError && e.code === 'no_public_url'
          ? 'No public URL yet: start a tunnel (see Internet access) or enter its URL.'
          : errorMessage(e),
      );
    } finally {
      this.creating.set(false);
    }
  }

  protected onSettled(p: PendingPair, approved: boolean): void {
    this.pendingPairs.update((prev) => prev.filter((x) => x.id !== p.id));
    if (approved) void this.loadDevices();
  }

  protected async deny(p: PendingPair): Promise<void> {
    await this.withBusy(p.id, async () => {
      await this.api.denyPair(p.id);
      this.pendingPairs.update((prev) => prev.filter((x) => x.id !== p.id));
    });
  }

  protected pairName(p: PendingPair): string {
    return p.deviceName || p.name || 'A device';
  }

  // ──────────────────────────────────────────────────────────── Internet

  protected async setInternet(enabled: boolean): Promise<void> {
    await this.saveRemote({ internetEnabled: enabled });
  }

  protected async setRequireApproval(on: boolean): Promise<void> {
    await this.saveRemote({ requireApproval: on });
  }

  protected async saveManualUrl(): Promise<void> {
    const url = this.manualUrl().trim().replace(/\/+$/, '');
    if (url && !/^https:\/\//i.test(url)) {
      this.error.set('The public URL must start with https://');
      return;
    }
    await this.saveRemote({ manualUrl: url });
  }

  protected providerLabel(p: string): string {
    return PROVIDER_LABELS[p] ?? p;
  }

  // ───────────────────────────────────────────────────────────── Devices

  protected ingressLabel(i: string | undefined): string {
    return i ? (INGRESS_LABELS[i] ?? i) : '';
  }

  protected startRename(d: DeviceInfo): void {
    this.renaming.set(d.id);
    this.renameText.set(d.name);
  }

  protected async saveRename(d: DeviceInfo): Promise<void> {
    const name = this.renameText().trim();
    this.renaming.set(null);
    if (!name || name === d.name) return;
    await this.withBusy(d.id, async () => {
      await this.api.updateDevice(d.id, { name });
      this.devices.update((prev) => prev.map((x) => (x.id === d.id ? { ...x, name } : x)));
    });
  }

  protected async setDangerous(d: DeviceInfo, on: boolean): Promise<void> {
    await this.withBusy(d.id, async () => {
      await this.api.updateDevice(d.id, { allowDangerous: on });
      this.devices.update((prev) =>
        prev.map((x) => (x.id === d.id ? { ...x, allowDangerous: on } : x)),
      );
    });
  }

  /** Two-step confirm for destructive actions (no blocking dialogs). */
  protected armed(key: string): boolean {
    return this.confirming() === key;
  }

  protected arm(key: string): void {
    this.confirming.set(key);
    clearTimeout(this.confirmTimer);
    this.confirmTimer = setTimeout(() => this.confirming.set(null), 4000);
  }

  protected async revoke(d: DeviceInfo): Promise<void> {
    if (!this.armed(`revoke:${d.id}`)) {
      this.arm(`revoke:${d.id}`);
      return;
    }
    this.confirming.set(null);
    await this.withBusy(d.id, async () => {
      await this.api.revokeDevice(d.id);
      this.devices.update((prev) => prev.filter((x) => x.id !== d.id));
      this.notice.set(`Revoked ${d.name}. It was disconnected immediately.`);
    });
  }

  // ─────────────────────────────────────────────────────── Foreign control

  protected async installForeign(): Promise<void> {
    await this.foreignAction(
      () => this.api.installForeignControl(),
      'Foreign-session control installed. It takes effect for sessions started from now on.',
    );
  }

  protected async uninstallForeign(): Promise<void> {
    if (!this.armed('uninstall')) {
      this.arm('uninstall');
      return;
    }
    this.confirming.set(null);
    await this.foreignAction(
      () => this.api.uninstallForeignControl(),
      'Foreign-session control removed.',
    );
  }

  // ────────────────────────────────────────────────────────────── Loading

  private async foreignAction(
    fn: () => Promise<ForeignControlStatus | null>,
    ok: string,
  ): Promise<void> {
    this.foreignBusy.set(true);
    this.foreignError.set('');
    try {
      const status = await fn();
      if (status) this.foreign.set(status);
      else await this.loadForeign();
      this.notice.set(ok);
    } catch (e) {
      this.foreignError.set(errorMessage(e));
    } finally {
      this.foreignBusy.set(false);
    }
  }

  private async saveRemote(patch: Parameters<ApiService['updateRemote']>[0]): Promise<void> {
    this.savingRemote.set(true);
    this.error.set('');
    try {
      const info = await this.api.updateRemote(patch);
      if (info) this.applyRemote(info);
      else await this.loadRemote();
    } catch (e) {
      this.error.set(errorMessage(e));
    } finally {
      this.savingRemote.set(false);
    }
  }

  private async withBusy(id: string, fn: () => Promise<void>): Promise<void> {
    this.busy.update((prev) => new Set(prev).add(id));
    this.error.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(errorMessage(e));
    } finally {
      this.busy.update((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }

  private async loadAll(): Promise<void> {
    if (!this.isDesktop()) return;
    await Promise.all([this.loadRemote(), this.loadDevices(), this.loadForeign()]);
  }

  private async loadRemote(): Promise<void> {
    try {
      this.applyRemote(await this.api.getRemote());
    } catch (e) {
      this.error.set(`Remote settings: ${errorMessage(e)}`);
    }
  }

  private applyRemote(info: RemoteInfo): void {
    const first = !this.remote();
    this.remote.set(info);
    if (first) {
      this.manualUrl.set(info.internet.manualUrl ?? '');
      if (!this.lanUrl()) this.lanUrl.set(info.lan.addresses[0]?.url ?? '');
    }
  }

  private async loadDevices(): Promise<void> {
    try {
      const list = await this.api.listDevices();
      this.devices.set(list.devices ?? []);
      this.pendingPairs.set(list.pending ?? []);
    } catch (e) {
      this.error.set(`Devices: ${errorMessage(e)}`);
    }
  }

  private async loadForeign(): Promise<void> {
    try {
      this.foreign.set(await this.api.getForeignControl());
    } catch (e) {
      this.foreignError.set(errorMessage(e));
    }
  }
}
