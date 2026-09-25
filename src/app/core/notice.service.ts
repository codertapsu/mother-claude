import { Injectable, NgZone, inject, signal } from '@angular/core';
import { Router } from '@angular/router';

import { describeError } from './api.service';

export type ToastLevel = 'info' | 'success' | 'warning' | 'error';

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  level: ToastLevel;
  text: string;
  action?: ToastAction;
}

export interface ToastOptions {
  action?: ToastAction;
  /** Milliseconds before it goes away by itself (0 = stays until closed).
   * Default: 4 s, 8 s for errors, 12 s when there is an action. */
  timeout?: number;
}

const MAX_TOASTS = 4;

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

  private readonly toastsS = signal<readonly Toast[]>([]);
  readonly toasts = this.toastsS.asReadonly();

  private readonly announcementS = signal('');
  /** Text for the visually hidden polite live region. */
  readonly announcement = this.announcementS.asReadonly();

  show(level: ToastLevel, text: string, opts: ToastOptions = {}): number {
    const id = this.nextId++;
    const toast: Toast = { id, level, text, action: opts.action };
    this.toastsS.update((prev) => {
      // The same message twice in a row just stays (no stacking duplicates).
      const rest = prev.filter((t) => t.text !== text);
      return [...rest, toast].slice(-MAX_TOASTS);
    });
    const timeout = opts.timeout ?? (opts.action ? 12_000 : level === 'error' ? 8_000 : 4_000);
    if (timeout > 0) {
      this.zone.runOutsideAngular(() => {
        this.timers.set(
          id,
          setTimeout(() => this.zone.run(() => this.dismiss(id)), timeout),
        );
      });
    }
    return id;
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
      ? { label: 'Open API page', run: () => void this.router.navigate(['/api']) }
      : retry && d.retry
        ? { label: 'Retry', run: retry }
        : undefined;
    return this.show('error', text, { action });
  }

  dismiss(id: number): void {
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    this.toastsS.update((prev) => prev.filter((t) => t.id !== id));
  }

  /** Say something to screen readers only. Repeating the same text still
   * announces it (the region is cleared first). */
  announce(text: string): void {
    this.announcementS.set('');
    queueMicrotask(() => this.announcementS.set(text));
  }
}
