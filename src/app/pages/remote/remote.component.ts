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

import { MAIN_SERVER, RemoteServer, remoteServers, tunnelServer } from '../../core/api-servers';
import { ApiError, ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  DetectedTunnel,
  DeviceInfo,
  PairCode,
  PairTarget,
  PairedDeviceView,
  RemoteInfo,
} from '../../core/models';
import { NowService } from '../../core/now.service';
import { RealtimeService } from '../../core/realtime.service';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { IconComponent } from '../../shared/icon.component';
import { PageHeadComponent } from '../../shared/page-head.component';
import { RelativeTimePipe } from '../../shared/time.pipes';
import { capitalize } from '../../shared/util';

const POLL_MS = 10_000;

/** The network a device came from (English: translated where shown). */
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
  'tailscale-funnel': 'Tailscale Funnel',
  'tailscale-serve': 'Tailscale Serve',
  tailscaleServe: 'Tailscale Serve',
  cloudflare: 'Cloudflare',
};

/**
 * *Settings › Phone & devices* (docs/UI-REDESIGN.md §6.7): on the desktop,
 * everything about reaching Mother Claude from other devices — pairing QR
 * codes (LAN or internet), internet access through a tunnel (detection +
 * copyable commands), the paired devices with what each may do. A device
 * that asks to pair waits in the Inbox, and the shell's dialog asks for its
 * number on any page. A phone sees itself and the devices paired, read
 * only (no dead end). The plugin for sessions started elsewhere is a page
 * of its own.
 */
@Component({
  selector: 'mc-remote',
  imports: [
    FormsModule,
    RouterLink,
    CopyButtonComponent,
    IconComponent,
    PageHeadComponent,
    RelativeTimePipe,
    TPipe,
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
  /** The desktop's computer, in sentences. */
  protected readonly computer = computerNoun;
  protected readonly remote = signal<RemoteInfo | null>(null);
  protected readonly devices = signal<DeviceInfo[]>([]);
  /** A phone's read of the devices paired (`GET /api/devices`). */
  protected readonly phoneDevices = signal<PairedDeviceView[] | null>(null);
  protected readonly me = this.auth.me;
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
  protected readonly idleDays = signal<number | null>(null);

  /** The API servers (main first), while one besides the main one exists:
   * a tunnel to one of their ports names it (docs/ACCOUNTS.md §13.5). */
  protected readonly apiServers = computed(() => remoteServers(this.remote()));
  /** Servers besides the main one that a tunnel points at. */
  protected readonly exposedServers = computed(() =>
    this.apiServers().filter((s) => s.exposed && s.id !== MAIN_SERVER),
  );
  /** A tunnel points at the main API port (its own flag while it is the
   * only server, its entry's otherwise). */
  protected readonly mainExposed = computed(() => {
    const main = this.apiServers().find((s) => s.id === MAIN_SERVER);
    return main ? main.exposed : !!this.remote()?.bridge.exposed;
  });
  /** "Main: running, port 5612 · Work: stopped, port 5613". */
  protected readonly serversLine = computed(() =>
    this.apiServers()
      .map((s) => {
        const state = s.running ? tr('running') : tr('stopped');
        return s.port
          ? tr('{name}: {state}, port {port}', { name: s.name, state, port: s.port })
          : tr('{name}: {state}', { name: s.name, state });
      })
      .join(' · '),
  );

  constructor() {
    const destroyRef = inject(DestroyRef);
    const subs = [
      this.realtime.devices$.subscribe(() => void this.loadDevices()),
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
    // A phone reads the list once it is known to be one.
    let phoneLoaded = false;
    effect(() => {
      if (!this.me() || this.isDesktop() || phoneLoaded) return;
      phoneLoaded = true;
      untracked(() => void this.loadPhoneList());
    });
  }

  // ───────────────────────────────────────────────────────── A phone's view

  /** The devices paired as a phone reads them, this one first. */
  protected readonly phoneList = computed(() =>
    [...(this.phoneDevices() ?? [])].sort((a, b) => Number(b.current) - Number(a.current)),
  );

  /** The network this device reaches the desktop through. */
  protected readonly meIngress = computed(() => {
    const m = this.me();
    const i = m?.effectiveIngress ?? m?.ingress;
    return i ? (INGRESS_LABELS[i] ? capitalize(tr(INGRESS_LABELS[i])) : i) : '—';
  });

  /** What a device may do, in words: dangerous actions here and there. */
  protected dangerText(d: PairedDeviceView): string {
    if (d.allowDangerous && d.allowDangerousInternet) return tr('Dangerous actions allowed');
    if (d.allowDangerous) return tr('Dangerous actions on the local network');
    if (d.allowDangerousInternet) return tr('Dangerous actions over the internet');
    return '';
  }

  private async loadPhoneList(): Promise<void> {
    try {
      const list = await this.api.listDevices();
      // A phone's answer is the read-only view.
      this.phoneDevices.set((list.devices ?? []) as unknown as PairedDeviceView[]);
    } catch (e) {
      this.phoneDevices.set([]);
      this.error.set(tr('Devices: {error}', { error: errorMessage(e) }));
    }
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
          ? tr('No public URL yet: start a tunnel (see Internet access) or enter its URL.')
          : errorMessage(e),
      );
    } finally {
      this.creating.set(false);
    }
  }

  // ──────────────────────────────────────────────────────────── Internet

  protected async setInternet(enabled: boolean): Promise<void> {
    await this.saveRemote({ internetEnabled: enabled });
  }

  protected async setBindTailscale(on: boolean): Promise<void> {
    await this.saveRemote({ bindTailscale: on });
  }

  protected async saveManualUrl(): Promise<void> {
    const url = this.manualUrl().trim().replace(/\/+$/, '');
    if (url && !/^https:\/\//i.test(url)) {
      this.error.set(tr('The public URL must start with https://'));
      return;
    }
    await this.saveRemote({ manualUrl: url || null });
  }

  protected providerLabel(p: string): string {
    return PROVIDER_LABELS[p] ?? p;
  }

  /** The API server a detected tunnel points at, if any. */
  protected serverOf(t: DetectedTunnel): RemoteServer | null {
    return tunnelServer(t, this.apiServers());
  }

  // ───────────────────────────────────────────────────────────── Devices

  protected ingressLabel(i: string | undefined): string {
    if (!i) return '';
    return INGRESS_LABELS[i] ? tr(INGRESS_LABELS[i]) : i;
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

  protected async setDangerousInternet(d: DeviceInfo, on: boolean): Promise<void> {
    await this.withBusy(d.id, async () => {
      await this.api.updateDevice(d.id, { allowDangerousInternet: on });
      this.devices.update((prev) =>
        prev.map((x) => (x.id === d.id ? { ...x, allowDangerousInternet: on } : x)),
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
      this.notice.set(tr('Revoked {name}. It was disconnected immediately.', { name: d.name }));
    });
  }

  // ────────────────────────────────────────────────────────────── Loading

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
    await Promise.all([this.loadRemote(), this.loadDevices()]);
  }

  private async loadRemote(): Promise<void> {
    try {
      this.applyRemote(await this.api.getRemote());
    } catch (e) {
      this.error.set(tr('Phone & devices: {error}', { error: errorMessage(e) }));
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
      this.idleDays.set(list.idleDays ?? null);
    } catch (e) {
      this.error.set(tr('Devices: {error}', { error: errorMessage(e) }));
    }
  }
}
