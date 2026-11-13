import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  input,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { TPipe } from '../../core/i18n/t.pipe';
import { RootMachine, RootsView } from '../../core/machines';
import { projectPlaces } from '../../core/project-settings';
import { RealtimeService } from '../../core/realtime.service';
import { DesktopOnlyComponent } from '../../shared/desktop-only.component';
import { thisComputerTitle } from '../machines/machines-text';

/**
 * A project's folder on each of your computers (its settings sheet,
 * docs/UI-REDESIGN.md §9): through the deepest shared folder (root) that
 * holds it here, the same place under that folder on each paired computer
 * (docs/PLAN-MACHINES.md, roots) — or that none holds it, so its sessions
 * can't move yet. The shared folders are changed in *Computers › Options*,
 * from the desktop app.
 */
@Component({
  selector: 'mc-project-folders',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DesktopOnlyComponent, RouterLink, TPipe],
  template: `
    @if (error()) {
      <p class="error-text small" role="alert">{{ error() }}</p>
    } @else if (!roots()) {
      <p class="muted small" role="status">{{ 'Loading…' | t }}</p>
    } @else if (places(); as pl) {
      <dl class="kv">
        @for (f of pl.folders; track f.machine.id) {
          <dt>{{ label(f.machine) }}</dt>
          @if (f.path) {
            <dd class="mono">{{ f.path }}</dd>
          } @else {
            <dd class="muted">{{ 'Not there yet' | t }}</dd>
          }
        }
      </dl>
      <p class="muted small">
        {{ 'Through the shared folder {name}.' | t: { name: pl.root.name } }}
      </p>
    } @else {
      <p class="muted small">
        {{
          'No folder you share between your computers holds it yet: its sessions can’t move to another computer.'
            | t
        }}
      </p>
    }
    <mc-desktop-only>
      <a class="small" routerLink="/computers/options" fragment="accounts-folders">{{
        'Change the shared folders' | t
      }}</a>
    </mc-desktop-only>
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
        align-items: flex-start;
      }
      p {
        margin: 0;
      }
      .kv {
        width: 100%;
      }
    `,
  ],
})
export class ProjectFoldersComponent {
  /** The project's folder on this computer. */
  readonly cwd = input.required<string>();

  private readonly api = inject(ApiService);
  protected readonly roots = signal<RootsView | null>(null);
  protected readonly error = signal('');
  protected readonly places = computed(() => projectPlaces(this.roots(), this.cwd()));

  constructor() {
    void this.load();
    const sub = inject(RealtimeService).machines$.subscribe((f) => {
      if (f.what === 'roots' || f.what === 'peers') void this.load();
    });
    inject(DestroyRef).onDestroy(() => sub.unsubscribe());
  }

  protected label(m: RootMachine): string {
    return m.here ? thisComputerTitle() : m.name || m.id.slice(0, 8);
  }

  private async load(): Promise<void> {
    try {
      this.roots.set(await this.api.getRoots());
      this.error.set('');
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }
}
