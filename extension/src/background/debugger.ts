// chrome.debugger (an optional permission): trusted input, screenshots,
// console capture, eval and dialogs. Attached per tab on first use, kept while
// the tab is in its lease, detached when the lease closes. Chrome shows its
// "started debugging this browser" bar while anything is attached; the side
// panel explains it. If the user cancels that bar, we do not attach again in
// that tab until its lease ends.

import { CommandError, messageOf, permissionNeeded } from '../shared/errors.ts';
import { dialogKind, type Dialogs } from './dialogs.ts';
import { hasPermission } from './env.ts';

export interface ConsoleMessage {
  level: 'debug' | 'log' | 'info' | 'warning' | 'error';
  text: string;
  at: number;
  url?: string;
}

const CONSOLE_LIMIT = 1000;
const TEXT_LIMIT = 2000;

type Params = Record<string, unknown>;

function consoleLevel(type: string): ConsoleMessage['level'] {
  switch (type) {
    case 'warning':
    case 'warn':
      return 'warning';
    case 'error':
    case 'assert':
      return 'error';
    case 'debug':
    case 'verbose':
      return 'debug';
    case 'info':
      return 'info';
    default:
      return 'log';
  }
}

interface RemoteObject {
  type?: string;
  value?: unknown;
  description?: string;
  unserializableValue?: string;
}

function remoteText(o: RemoteObject): string {
  if (o.type === 'string') return String(o.value);
  if (o.value !== undefined) {
    try {
      return JSON.stringify(o.value);
    } catch {
      return String(o.value);
    }
  }
  return o.unserializableValue ?? o.description ?? o.type ?? '';
}

export class DebuggerManager {
  private readonly attached = new Set<number>();
  private readonly cancelled = new Set<number>();
  private readonly consoleBuffers = new Map<number, ConsoleMessage[]>();
  private readonly networks = new Map<
    number,
    { inflight: Map<string, { at: number; url: string }>; last: number; mainFrame: string }
  >();
  private listening = false;
  private readonly dialogs: Dialogs;
  /** Called when the user cancels the debugging bar (the side panel says why). */
  onCancelled: () => void = () => undefined;

  constructor(dialogs: Dialogs) {
    this.dialogs = dialogs;
  }

  isAttached(tabId: number): boolean {
    return this.attached.has(tabId);
  }

  get attachedCount(): number {
    return this.attached.size;
  }

  async available(): Promise<boolean> {
    return (await hasPermission('debugger')) && typeof chrome.debugger !== 'undefined';
  }

  /** Attach to the tab (once), or explain why it cannot be done. */
  async ensure(tabId: number, why: string): Promise<void> {
    if (!(await hasPermission('debugger'))) {
      throw permissionNeeded(
        'debugger',
        `${why} needs the "debugger" permission: grant it in the Mother Claude side panel`,
      );
    }
    if (typeof chrome.debugger === 'undefined') {
      throw new CommandError('debugger_unavailable', 'this browser has no debugger API');
    }
    this.listen();
    if (this.attached.has(tabId)) return;
    if (this.cancelled.has(tabId)) {
      throw new CommandError(
        'debugger_unavailable',
        'the user cancelled debugging in this tab: it stays off until the session’s lease ends',
      );
    }
    try {
      await chrome.debugger.attach({ tabId }, '1.3');
    } catch (e) {
      const msg = messageOf(e);
      if (/already attached/i.test(msg)) {
        // Ours from before a worker restart? Detaching succeeds only then.
        try {
          await chrome.debugger.detach({ tabId });
          await chrome.debugger.attach({ tabId }, '1.3');
        } catch {
          throw new CommandError(
            'debugger_unavailable',
            'another debugger (DevTools or an extension) is attached to this tab',
          );
        }
      } else {
        throw new CommandError('debugger_unavailable', `cannot debug this tab: ${msg}`);
      }
    }
    this.attached.add(tabId);
    this.networks.set(tabId, { inflight: new Map(), last: Date.now(), mainFrame: '' });
    await Promise.all([
      this.send(tabId, 'Page.enable'),
      this.send(tabId, 'Runtime.enable'),
      this.send(tabId, 'Log.enable'),
      // Only to count requests (network idle): no bodies kept.
      this.send(tabId, 'Network.enable', { maxTotalBufferSize: 0, maxResourceBufferSize: 0 }).catch(
        () => this.send(tabId, 'Network.enable'),
      ),
    ]).catch(() => undefined);
    const tree = await this.send<{ frameTree?: { frame?: { id?: string } } }>(
      tabId,
      'Page.getFrameTree',
    ).catch(() => ({}) as { frameTree?: { frame?: { id?: string } } });
    const net = this.networks.get(tabId);
    if (net) net.mainFrame = tree.frameTree?.frame?.id ?? '';
  }

  /**
   * Requests open in the tab and when the last one started or ended, or null
   * when the debugger is not attached. A request open for more than 10 s (a
   * long poll, a stream) does not count as keeping the network busy.
   */
  network(tabId: number): { inflight: number; last: number } | null {
    const net = this.networks.get(tabId);
    if (!net || !this.attached.has(tabId)) return null;
    const now = Date.now();
    let inflight = 0;
    for (const r of net.inflight.values()) if (now - r.at < 10_000) inflight++;
    return { inflight, last: net.last };
  }

  async send<T = Params>(tabId: number, method: string, params?: Params): Promise<T> {
    try {
      return (await chrome.debugger.sendCommand({ tabId }, method, params)) as T;
    } catch (e) {
      const msg = messageOf(e);
      if (/not attached|detached/i.test(msg)) this.attached.delete(tabId);
      throw new CommandError('debugger_unavailable', `${method}: ${msg}`);
    }
  }

  async detach(tabId: number): Promise<void> {
    this.cancelled.delete(tabId);
    this.consoleBuffers.delete(tabId);
    this.networks.delete(tabId);
    if (!this.attached.delete(tabId)) return;
    try {
      await chrome.debugger.detach({ tabId });
    } catch {
      // already gone
    }
  }

  consoleMessages(tabId: number): ConsoleMessage[] {
    return this.consoleBuffers.get(tabId) ?? [];
  }

  private push(tabId: number, m: ConsoleMessage): void {
    const buf = this.consoleBuffers.get(tabId) ?? [];
    buf.push({
      ...m,
      text: m.text.length > TEXT_LIMIT ? `${m.text.slice(0, TEXT_LIMIT)}…` : m.text,
    });
    if (buf.length > CONSOLE_LIMIT) buf.splice(0, buf.length - CONSOLE_LIMIT);
    this.consoleBuffers.set(tabId, buf);
  }

  /** Listeners exist only once the permission does (it is optional). */
  listen(): void {
    if (this.listening || typeof chrome.debugger === 'undefined') return;
    this.listening = true;
    chrome.debugger.onEvent.addListener((source, method, params) => {
      const tabId = source.tabId;
      if (tabId === undefined) return;
      this.onEvent(tabId, method, (params ?? {}) as Params);
    });
    chrome.debugger.onDetach.addListener((source, reason) => {
      const tabId = source.tabId;
      if (tabId === undefined) return;
      this.attached.delete(tabId);
      if (reason === 'canceled_by_user') {
        this.cancelled.add(tabId);
        this.onCancelled();
      }
    });
  }

  private onEvent(tabId: number, method: string, p: Params): void {
    switch (method) {
      case 'Page.javascriptDialogOpening':
        this.dialogs.opened(tabId, dialogKind(p['type']), String(p['message'] ?? ''));
        break;
      case 'Page.javascriptDialogClosed':
        this.dialogs.closed(tabId);
        break;
      case 'Network.requestWillBeSent':
      case 'Network.responseReceived':
      case 'Network.loadingFinished':
      case 'Network.loadingFailed': {
        const net = this.networks.get(tabId);
        const id = String(p['requestId'] ?? '');
        if (!net || !id) break;
        if (method === 'Network.requestWillBeSent') {
          // A new document in the main frame: the old page's requests are gone.
          if (p['type'] === 'Document' && net.mainFrame && p['frameId'] === net.mainFrame) {
            net.inflight.clear();
          }
          const url = String((p['request'] as { url?: unknown } | undefined)?.url ?? '');
          // A redirect reuses the request id: still the same open request.
          if (!net.inflight.has(id)) net.inflight.set(id, { at: Date.now(), url });
        } else if (method === 'Network.responseReceived') {
          // A response that becomes a download never reports finishing to the page.
          const headers = ((p['response'] as { headers?: Record<string, string> } | undefined)
            ?.headers ?? {}) as Record<string, string>;
          const disposition = Object.entries(headers).find(
            ([k]) => k.toLowerCase() === 'content-disposition',
          )?.[1];
          if (disposition && /attachment/i.test(disposition)) net.inflight.delete(id);
          break;
        } else net.inflight.delete(id);
        net.last = Date.now();
        break;
      }
      case 'Page.downloadWillBegin': {
        const net = this.networks.get(tabId);
        const url = String(p['url'] ?? '');
        if (net) for (const [id, r] of net.inflight) if (r.url === url) net.inflight.delete(id);
        break;
      }
      case 'Runtime.consoleAPICalled': {
        const args = Array.isArray(p['args']) ? (p['args'] as RemoteObject[]) : [];
        this.push(tabId, {
          level: consoleLevel(String(p['type'] ?? 'log')),
          text: args.map(remoteText).join(' '),
          at: typeof p['timestamp'] === 'number' ? Math.round(p['timestamp']) : Date.now(),
        });
        break;
      }
      case 'Runtime.exceptionThrown': {
        const d = (p['exceptionDetails'] ?? {}) as {
          text?: string;
          exception?: RemoteObject;
          url?: string;
        };
        this.push(tabId, {
          level: 'error',
          text: d.exception?.description ?? d.text ?? 'Uncaught exception',
          at: Date.now(),
          ...(d.url ? { url: d.url } : {}),
        });
        break;
      }
      case 'Log.entryAdded': {
        const e = (p['entry'] ?? {}) as {
          level?: string;
          text?: string;
          url?: string;
          timestamp?: number;
        };
        this.push(tabId, {
          level: consoleLevel(e.level ?? 'info'),
          text: e.text ?? '',
          at: typeof e.timestamp === 'number' ? Math.round(e.timestamp) : Date.now(),
          ...(e.url ? { url: e.url } : {}),
        });
        break;
      }
      default:
        break;
    }
  }
}
