import { Injectable, inject, signal } from '@angular/core';

import { ApiService, describeError } from './api.service';
import { PermissionRule, PermissionRulePatch, PermissionsView } from './permission-rules';
import { RealtimeService } from './realtime.service';

/**
 * Permissions — what Claude may do without asking you — for every screen
 * that shows them: `GET /api/permissions`, refetched on each
 * `{t:'permissions'}` frame (and after a reconnect) while a screen watches.
 * Its changes answer with the list (or the rule), which replaces the copy
 * here at once; the frame that follows refetches it again.
 */
@Injectable({ providedIn: 'root' })
export class PermissionRulesService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);

  readonly rules = signal<PermissionRule[]>([]);
  /** Mother Claude asks for everything; the rules are kept. */
  readonly paused = signal(false);
  /** The list was read at least once. */
  readonly loaded = signal(false);
  /** The last load's failure ('' when it worked). */
  readonly error = signal('');

  private watchers = 0;
  private loading: Promise<void> | null = null;
  private again = false;

  constructor() {
    this.realtime.permissions$.subscribe(() => {
      if (this.watchers > 0) void this.load();
    });
    this.realtime.refetch$.subscribe(() => {
      if (this.watchers > 0) void this.load();
    });
  }

  /** A screen shows them: loads them, and returns its release. */
  watch(): () => void {
    this.watchers += 1;
    void this.load();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.watchers -= 1;
    };
  }

  /** The list (one load at a time; a request meanwhile asks again). */
  load(): Promise<void> {
    if (this.loading) {
      this.again = true;
      return this.loading;
    }
    this.loading = this.fetch().finally(() => {
      this.loading = null;
      if (this.again) {
        this.again = false;
        void this.load();
      }
    });
    return this.loading;
  }

  /** Ask for everything (`true`), or use them again. */
  async setPaused(paused: boolean): Promise<void> {
    this.apply(await this.api.setPermissionsPaused(paused));
  }

  /** Change one's patterns or scope; answers it as saved. */
  async update(id: string, patch: PermissionRulePatch): Promise<PermissionRule> {
    const rule = await this.api.updatePermission(id, patch);
    this.rules.update((list) => list.map((r) => (r.id === rule.id ? rule : r)));
    return rule;
  }

  /** Remove one: what it allowed asks again. */
  async remove(id: string): Promise<void> {
    this.apply(await this.api.removePermission(id));
  }

  private apply(v: PermissionsView): void {
    this.rules.set(v.rules);
    this.paused.set(v.paused);
    this.loaded.set(true);
    this.error.set('');
  }

  private async fetch(): Promise<void> {
    try {
      this.apply(await this.api.getPermissions());
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }
}
