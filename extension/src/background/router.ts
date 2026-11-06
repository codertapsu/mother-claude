// Every command gets exactly one `result`: the action's, an error, or
// `timeout` 500 ms before the desktop's deadline. Commands run one at a time
// per tab, in order (commands without a tabId first wait their turn in the
// lease, then in the tab they resolve to).

import { CommandError, messageOf } from '../shared/errors.ts';
import { errorResult, okResult, type CommandFrame, type ResultFrame } from '../shared/protocol.ts';
import {
  DeadlineError,
  KeyedQueue,
  abortReason,
  withDeadline,
  workBudgetMs,
} from '../shared/queue.ts';
import {
  ACTION_NEEDS,
  activityText,
  isKnownAction,
  modeFor,
  originOf,
  siteRefusal,
} from '../shared/site.ts';
import type { Cmd, Commands } from './commands.ts';
import type { DebuggerManager } from './debugger.ts';
import type { Dialogs } from './dialogs.ts';
import { hasPermission } from './env.ts';
import { isPlaceholderUrl, type Leases } from './leases.ts';
import type { LeaseRecord, Store } from './state.ts';

/** Commands that still make sense while a dialog blocks the page. */
const DIALOG_OK = new Set([
  'dialog',
  'tabs.list',
  'tabs.select',
  'tabs.close',
  'tabs.open',
  'console',
  'screenshot',
]);
/** Lease-level commands: no tab needed. */
const LEASE_LEVEL = new Set(['tabs.list', 'tabs.open']);
/** Commands whose target tab is in params.tabId. */
const PARAM_TAB = new Set(['tabs.select', 'tabs.close']);
/** Page commands that make no sense on the placeholder. */
const NEEDS_PAGE = (action: string): boolean =>
  !LEASE_LEVEL.has(action) && !PARAM_TAB.has(action) && action !== 'navigate';

interface InFlight {
  lease: string;
  controller: AbortController;
  /** The tab it runs in, once known (to stop the page's side of it). */
  tabId?: number;
  reply(frame: ResultFrame): void;
}

export interface Activity {
  lease: string;
  text: string;
  since: number;
}

export interface RouterDeps {
  store: Store;
  leases: Leases;
  commands: Commands;
  dialogs: Dialogs;
  dbg: DebuggerManager;
  send(frame: ResultFrame): void;
  changed(): void;
  /** Ask the tab's page script to stop what it is waiting for. */
  cancelPage(tabId: number): Promise<void>;
}

export class Router {
  private readonly d: RouterDeps;
  private readonly queue = new KeyedQueue();
  private readonly inflight = new Map<string, InFlight>();
  /** What each lease is doing now, and what it did last. */
  readonly activity = new Map<string, Activity>();
  readonly lastActivity = new Map<string, Activity>();

  constructor(deps: RouterDeps) {
    this.d = deps;
  }

  handle(frame: CommandFrame): void {
    const received = Date.now();
    const deadline = received + workBudgetMs(frame.timeoutMs);
    const controller = new AbortController();
    let replied = false;
    const entry: InFlight = {
      lease: frame.lease,
      controller,
      reply: (result) => {
        if (replied) return;
        replied = true;
        this.inflight.delete(frame.id);
        this.d.send(result);
      },
    };
    this.inflight.set(frame.id, entry);
    withDeadline(deadline, controller, () =>
      this.run(frame, controller.signal, deadline, entry),
    ).then(
      (data) => entry.reply(okResult(frame.id, data)),
      (e: unknown) => entry.reply(errorResult(frame.id, toWire(e, frame))),
    );
  }

  /**
   * Stop pressed (or the lease closed): every command of the lease ends now
   * with `stopped`, its tab is free for the next command, and the page stops
   * any wait it was doing for it.
   */
  abortLease(lease: string, message: string): void {
    const tabs = new Set<number>();
    for (const [id, f] of this.inflight) {
      if (f.lease !== lease) continue;
      const err = new CommandError('stopped', message);
      f.controller.abort(err);
      f.reply(errorResult(id, err.toWire()));
      if (f.tabId !== undefined) tabs.add(f.tabId);
    }
    for (const tabId of tabs) void this.d.cancelPage(tabId);
    this.activity.delete(lease);
  }

  /** The socket closed: nobody is waiting for these any more. */
  abortAll(): void {
    for (const f of this.inflight.values())
      f.controller.abort(new CommandError('stopped', 'disconnected'));
    this.inflight.clear();
    this.activity.clear();
  }

  private stoppedError(rec: LeaseRecord): CommandError {
    const secs = Math.max(1, Math.ceil((rec.stoppedUntil - Date.now()) / 1000));
    return new CommandError(
      'stopped',
      `the user pressed Stop for this session in the browser: commands are refused for ${secs} s more unless the user resumes`,
    );
  }

  private async run(
    frame: CommandFrame,
    signal: AbortSignal,
    deadline: number,
    entry: InFlight,
  ): Promise<unknown> {
    await this.d.leases.whenReady(frame.lease);
    const rec = this.d.leases.get(frame.lease);
    if (!rec) throw new CommandError('no_lease', `no lease ${frame.lease} in this browser`);
    if (this.d.leases.isStopped(rec)) throw this.stoppedError(rec);
    const action = frame.action;
    if (!isKnownAction(action)) throw new CommandError('unsupported', `unknown action "${action}"`);

    if (LEASE_LEVEL.has(action) && frame.tabId === undefined) {
      return this.queue.run(
        `lease:${rec.lease}`,
        () => this.exec(frame, rec, null, null, signal, deadline),
        signal,
      );
    }
    const explicit =
      frame.tabId ??
      (PARAM_TAB.has(action) && typeof frame.params['tabId'] === 'number'
        ? (frame.params['tabId'] as number)
        : undefined);
    if (PARAM_TAB.has(action) && explicit === undefined) {
      throw new CommandError('no_tab', `${action} needs a tabId`);
    }
    if (explicit !== undefined) {
      entry.tabId = explicit;
      return this.queue.run(
        `tab:${explicit}`,
        () => this.onTab(frame, rec, explicit, signal, deadline),
        signal,
      );
    }
    return this.queue.run(
      `lease:${rec.lease}`,
      async () => {
        if (signal.aborted) throw abortReason(signal);
        const tabId = await this.d.leases.activeTab(rec);
        if (tabId === null)
          throw new CommandError('no_tab', 'the session has no tab open: use tabs.open');
        entry.tabId = tabId;
        return this.queue.run(
          `tab:${tabId}`,
          () => this.onTab(frame, rec, tabId, signal, deadline),
          signal,
        );
      },
      signal,
    );
  }

  private async onTab(
    frame: CommandFrame,
    rec: LeaseRecord,
    tabId: number,
    signal: AbortSignal,
    deadline: number,
  ): Promise<unknown> {
    if (signal.aborted) throw abortReason(signal);
    if (this.d.leases.isStopped(rec)) throw this.stoppedError(rec);
    let tab: chrome.tabs.Tab;
    try {
      tab = await chrome.tabs.get(tabId);
    } catch {
      throw new CommandError('no_tab', `tab ${tabId} is gone`);
    }
    const inLease = rec.groupId !== null && tab.groupId === rec.groupId;
    const shared = this.d.store.session.shared;
    const isShared = !!shared && shared.tabId === tabId && shared.origin === originOf(tab.url);
    if (!inLease && !isShared) {
      throw new CommandError('no_tab', `tab ${tabId} is not in this session's tab group`);
    }
    const action = frame.action;
    const url = tab.url ?? tab.pendingUrl;
    if (NEEDS_PAGE(action) && isPlaceholderUrl(url)) {
      throw new CommandError(
        'no_tab',
        'the session has no page open yet: use tabs.open or navigate',
      );
    }
    const need = ACTION_NEEDS[action];
    const dialog = this.d.dialogs.get(tabId);
    if (need === 'observe' || need === 'act') {
      const { siteModes, settings } = this.d.store.session.desktop;
      const refusal = siteRefusal(
        action,
        modeFor(url, siteModes, settings),
        frame.params,
        dialog?.kind,
      );
      if (refusal) {
        const origin = originOf(url) ?? url ?? 'this page';
        throw new CommandError(
          refusal,
          refusal === 'site_off'
            ? `${origin} is off in Mother Claude: the user can allow it in the side panel`
            : `${origin} is read-only in Mother Claude: observing is allowed, acting is not`,
        );
      }
    }
    if (dialog && !DIALOG_OK.has(action)) {
      throw new CommandError(
        'dialog_open',
        `a ${dialog.kind} dialog blocks the page: answer it with dialog`,
        {
          kind: dialog.kind,
          message: dialog.message,
        },
      );
    }
    // With the debugger granted, a lease's tabs are attached from their first
    // command: dialogs, console and trusted input then work at once.
    if (inLease && !this.d.dbg.isAttached(tabId) && (await hasPermission('debugger'))) {
      await this.d.dbg.ensure(tabId, 'the session').catch(() => undefined);
    }
    const cmd: Cmd = { action, params: frame.params, rec, tabId, tab, deadline, signal };
    return this.exec(frame, rec, tabId, tab, signal, deadline, cmd);
  }

  private async exec(
    frame: CommandFrame,
    rec: LeaseRecord,
    tabId: number | null,
    tab: chrome.tabs.Tab | null,
    signal: AbortSignal,
    deadline: number,
    cmd?: Cmd,
  ): Promise<unknown> {
    if (signal.aborted) throw abortReason(signal);
    const handler = this.d.commands.handlers[frame.action];
    if (!handler) throw new CommandError('unsupported', `unknown action "${frame.action}"`);
    const act: Activity = {
      lease: rec.lease,
      text: activityText(
        frame.action,
        frame.action === 'navigate' || frame.action === 'tabs.open'
          ? typeof frame.params['url'] === 'string'
            ? frame.params['url']
            : undefined
          : (tab?.url ?? undefined),
      ),
      since: Date.now(),
    };
    this.activity.set(rec.lease, act);
    this.d.changed();
    try {
      return await handler(
        cmd ?? { action: frame.action, params: frame.params, rec, tabId, tab, deadline, signal },
      );
    } finally {
      if (this.activity.get(rec.lease) === act) this.activity.delete(rec.lease);
      this.lastActivity.set(rec.lease, { ...act, since: Date.now() });
      this.d.changed();
    }
  }
}

function toWire(
  e: unknown,
  frame: CommandFrame,
): { code: string; message: string; data?: unknown } {
  if (e instanceof DeadlineError) {
    return {
      code: 'timeout',
      message: `${frame.action} did not finish within ${frame.timeoutMs} ms`,
    };
  }
  if (e instanceof CommandError) return e.toWire();
  return { code: 'internal', message: messageOf(e) };
}
