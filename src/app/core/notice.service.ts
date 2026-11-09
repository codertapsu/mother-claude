import { Injectable, NgZone, inject, signal } from '@angular/core';
import { Router } from '@angular/router';

import { describeError } from './api.service';
import { tr } from './i18n/i18n';

export type ToastLevel = 'info' | 'success' | 'warning' | 'error';

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  level: ToastLevel;
  /** A bold first line (translated); else the level's own ("Done", …). */
  title?: string;
  text: string;
  action?: ToastAction;
  /** How long it shows by itself (0: until closed), for its countdown bar. */
  timeout: number;
}

export interface ToastOptions {
  action?: ToastAction;
  /** A bold first line, translated. */
  title?: string;
  /** Milliseconds before it goes away by itself (0 = stays until closed).
   * Default: 6 s, 5 s for success, 10 s for warnings, 12 s for errors, 15 s
   * when there is an action. The time stops while the pointer or the focus
   * is on it. */
  timeout?: number;
}

const MAX_TOASTS = 4;

/** How long each level shows by itself. */
const TIMEOUTS: Record<ToastLevel, number> = {
  info: 6_000,
  success: 5_000,
  warning: 10_000,
  error: 12_000,
};

/**
 * App-wide feedback: short toasts (the shell renders them in an
 * `aria-live=polite` region) and screen-reader announcements for things that
 * have no visible text of their own (a new approval request, the connection
 * dropping). Every page reports action results and errors through here, so
 * failures read the same everywhere (see `describeError`).
 */
@Injectable({ providedIn: 'root' })
export class NoticeService {
  private zone = inject(NgZone);
  private router = inject(Router);
  private nextId = 1;
  private readonly timers = new Map<number, ReturnType<typeof setTimeout>>();
  /** What is left of each toast's time, and since when it runs (0: held). */
  private readonly clocks = new Map<number, { left: number; since: number }>();

  private readonly toastsS = signal<readonly Toast[]>([]);
  readonly toasts = this.toastsS.asReadonly();

  private readonly announcementS = signal('');
  /** Text for the visually hidden polite live region. */
  readonly announcement = this.announcementS.asReadonly();

  show(level: ToastLevel, text: string, opts: ToastOptions = {}): number {
    const id = this.nextId++;
    const timeout = opts.timeout ?? (opts.action ? 15_000 : TIMEOUTS[level]);
    const toast: Toast = {
      id,
      level,
      text,
      timeout,
      ...(opts.title ? { title: opts.title } : {}),
      ...(opts.action ? { action: opts.action } : {}),
    };
    // The same message twice in a row just stays (no stacking duplicates);
    // past the most there may be, the oldest go.
    const all = [...this.toastsS().filter((t) => t.text !== text), toast];
    const kept = all.slice(-MAX_TOASTS);
    for (const t of this.toastsS()) if (!kept.includes(t)) this.stopClock(t.id);
    this.toastsS.set(kept);
    if (timeout > 0) this.run(id, timeout);
    return id;
  }

  /** Stop a toast's time while it is pointed at or focused. */
  hold(id: number): void {
    const c = this.clocks.get(id);
    if (!c || !c.since) return;
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    this.clocks.set(id, { left: Math.max(1_000, c.left - (Date.now() - c.since)), since: 0 });
  }

  /** Let a held toast's time run on. */
  release(id: number): void {
    const c = this.clocks.get(id);
    if (c && !c.since) this.run(id, c.left);
  }

  private run(id: number, ms: number): void {
    this.clocks.set(id, { left: ms, since: Date.now() });
    this.zone.runOutsideAngular(() => {
      this.timers.set(
        id,
        setTimeout(() => this.zone.run(() => this.dismiss(id)), ms),
      );
    });
  }

  private stopClock(id: number): void {
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    this.clocks.delete(id);
  }

  info(text: string, opts?: ToastOptions): number {
    return this.show('info', text, opts);
  }

  success(text: string, opts?: ToastOptions): number {
    return this.show('success', text, opts);
  }

  warn(text: string, opts?: ToastOptions): number {
    return this.show('warning', text, opts);
  }

  /** Report a failure through the shared mapper. `retry` adds a Retry button
   * when the failure is worth retrying. */
  error(e: unknown, retry?: () => void, prefix?: string): number {
    const d = describeError(e);
    const text = prefix ? `${prefix}: ${d.text}` : d.text;
    const action = d.bridge
      ? { label: tr('Open API page'), run: () => void this.router.navigate(['/api']) }
      : retry && d.retry
        ? { label: tr('Retry'), run: retry }
        : undefined;
    return this.show('error', text, { action });
  }

  dismiss(id: number): void {
    this.stopClock(id);
    this.toastsS.update((prev) => prev.filter((t) => t.id !== id));
  }

  /** Say something to screen readers only. Repeating the same text still
   * announces it (the region is cleared first). */
  announce(text: string): void {
    this.announcementS.set('');
    queueMicrotask(() => this.announcementS.set(text));
  }
}
