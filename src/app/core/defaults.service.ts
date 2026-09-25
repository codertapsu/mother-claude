import { Injectable, inject, signal } from '@angular/core';

import { ApiService } from './api.service';
import { LaunchDefaults } from './models';

/** Effort levels the CLI accepts; the server's list overrides this. */
export const FALLBACK_EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'];

/**
 * `GET /defaults` (models, efforts and the user's own defaults), fetched once
 * and shared by the launch forms and the session header's pickers.
 */
@Injectable({ providedIn: 'root' })
export class DefaultsService {
  private api = inject(ApiService);
  readonly defaults = signal<LaunchDefaults | null>(null);
  private loading?: Promise<LaunchDefaults | null>;

  load(): Promise<LaunchDefaults | null> {
    if (this.defaults()) return Promise.resolve(this.defaults());
    this.loading ??= this.api
      .getDefaults()
      .then((d) => {
        this.defaults.set(d);
        return d;
      })
      .catch(() => {
        this.loading = undefined;
        return null;
      });
    return this.loading;
  }
}
