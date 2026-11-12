import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  linkedSignal,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import {
  backupErrorText,
  normalizeFolderPairs,
  osLabel,
  otherFolderProblem,
  pathOsOf,
} from '../../core/backup';
import { FolderPairDraftService } from '../../core/folder-pair-draft.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { FolderPairsView, PairMachine, PairSide } from '../../core/models';
import { folderExample, folderPathProblem, machineNoun } from '../../core/paths';
import { IconComponent } from '../../shared/icon.component';

/**
 * Backups › Folder pairs (docs/BACKUP.md "Folder pairs"): a folder on
 * this machine and the folder that holds the same projects on another
 * machine of the backups (`/Users/marcus/dev` and `D:\dev`), so a session
 * pulled from there goes into the matching project here, and back. The pairs
 * are kept in the backups themselves (encrypted), so every machine uses
 * them and either may change them. Lists the pairs (this machine's folder
 * first) with Remove, and adds one: this machine's folder, another machine
 * the backups know and its folder, each checked the way that machine writes
 * paths. A machine shows once it has backed up or pulled; "Look again"
 * reads the backups anew. A report's "kept another machine's path" fills the
 * form ({@link FolderPairDraftService}). The desktop and paired phones
 * alike, as the backup settings.
 */
@Component({
  selector: 'mc-folder-pairs',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, IconComponent, TPipe],
  template: `
    <div class="shead" id="folder-pairs">
      <h2>{{ 'Folder pairs' | t }}</h2>
      <button
        class="btn small ghost"
        type="button"
        [disabled]="busy()"
        [attr.aria-label]="'Read the folder pairs from the backups again' | t"
        (click)="refresh()"
      >
        <mc-icon name="refresh" [size]="14" /> {{ 'Look again' | t }}
      </button>
    </div>
    <section class="card pairs">
      <p class="muted small lede">
        {{
          'Pair a folder on this {computer} with the folder that holds the same projects on another computer: a session pulled from there goes into the matching project here (and back), so Claude Code lists it where you work.'
            | t: { computer: computer() }
        }}
      </p>
      @if (view()?.pairs?.length) {
        <ul class="list">
          @for (p of view()!.pairs; track p.id) {
            <li class="prow">
              <div class="sides">
                @for (s of p.folders; track s.machine; let last = $last) {
                  <span class="side">
                    <span class="who small">{{ sideLabel(s) }}</span>
                    <span class="mono path">{{ s.folder }}</span>
                  </span>
                  @if (!last) {
                    <span class="arrow" aria-hidden="true">⇄</span>
                  }
                }
              </div>
              <button
                class="btn small ghost"
                type="button"
                [disabled]="busy()"
                [attr.aria-label]="
                  'Remove the pair of {folder}' | t: { folder: p.folders[0].folder }
                "
                (click)="remove(p.id)"
              >
                {{ 'Remove' | t }}
              </button>
            </li>
          }
        </ul>
      }
      @if (machines().length) {
        <form class="add" (submit)="add($event)">
          <label class="field">
            <span class="field-label">{{
              'Folder on this {computer}' | t: { computer: computer() }
            }}</span>
            <input
              name="here"
              spellcheck="false"
              autocapitalize="off"
              autocomplete="off"
              [placeholder]="herePlaceholder()"
              [ngModel]="folder()"
              (ngModelChange)="folder.set($event); error.set('')"
            />
          </label>
          <label class="field">
            <span class="field-label">{{ 'Other computer' | t }}</span>
            <select name="machine" [disabled]="busy()" (change)="pickMachine($event)">
              @for (m of machines(); track m.id) {
                <option [value]="m.id" [selected]="m.id === machine()">
                  {{ machineLabel(m) }}
                </option>
              }
            </select>
          </label>
          <label class="field">
            <span class="field-label">{{ 'Its folder' | t }}</span>
            <input
              name="there"
              spellcheck="false"
              autocapitalize="off"
              autocomplete="off"
              [placeholder]="therePlaceholder()"
              [ngModel]="otherFolder()"
              (ngModelChange)="otherFolder.set($event); error.set('')"
            />
          </label>
          <div class="addact">
            <button
              class="btn small primary"
              type="submit"
              [disabled]="busy() || !folder().trim() || !otherFolder().trim()"
            >
              {{ busy() ? ('Saving…' | t) : ('Add pair' | t) }}
            </button>
          </div>
        </form>
      } @else {
        <p class="muted small none">
          {{
            'Another computer shows here once it has backed up to or pulled from these backups.' | t
          }}
        </p>
      }
      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      @if (notice()) {
        <p class="notice ok small" role="status">{{ notice() }}</p>
      }
    </section>
  `,
  styles: [
    `
      .shead {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 0.5rem;
      }
      .lede {
        margin: 0 0 0.6rem;
        line-height: 1.5;
      }
      .list {
        margin: 0 0 0.6rem;
        padding: 0;
        list-style: none;
      }
      .prow {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.45rem 0;
        border-top: 1px solid var(--border);
      }
      .sides {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.25rem 0.6rem;
      }
      .side {
        display: flex;
        flex-direction: column;
        min-width: 0;
      }
      .who {
        color: var(--muted);
      }
      .path {
        overflow-wrap: anywhere;
      }
      .arrow {
        color: var(--muted);
      }
      .add {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
        gap: 0.6rem;
        align-items: end;
      }
      .addact {
        display: flex;
      }
      .none {
        margin: 0;
      }
      .notice {
        margin: 0.6rem 0 0;
      }
    `,
  ],
})
export class FolderPairsComponent {
  private api = inject(ApiService);
  private drafts = inject(FolderPairDraftService);
  /** The desktop's system: this machine's folder is typed its way. */
  private readonly hostOs = inject(AuthService).hostOs;

  /** What `GET /backup` said (`folderPairs`). */
  readonly pairs = input<FolderPairsView | null>(null);

  /** The pairs as they are now (an add or remove answers the new ones). */
  protected readonly view = linkedSignal(() => this.pairs());
  protected readonly machines = computed(() => this.view()?.machines ?? []);
  protected readonly folder = signal('');
  protected readonly machine = linkedSignal(() => this.machines()[0]?.id ?? '');
  protected readonly otherFolder = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly notice = signal('');

  protected readonly chosen = computed(() => this.machines().find((m) => m.id === this.machine()));
  /** The system the chosen machine writes paths for (once it said). */
  private readonly chosenOs = computed(() => pathOsOf(this.chosen()?.os));
  protected readonly hereLabel = computed(() => {
    switch (this.hostOs()) {
      case 'macos':
        return tr('This Mac');
      case 'windows':
        return tr('This PC');
      default:
        return tr('This computer');
    }
  });

  /** The desktop's computer inside a sentence ("this {computer}"): "Mac",
   * "PC" (Vietnamese "máy Mac", "máy PC"). */
  protected readonly computer = computed(() => tr(machineNoun(this.hostOs())));

  /** Each folder's example in its own machine's form. */
  protected readonly herePlaceholder = computed(() => folderExample(this.hostOs(), 'projects'));
  protected readonly therePlaceholder = computed(() =>
    folderExample(this.chosenOs() ?? this.hostOs(), 'projects'),
  );

  constructor() {
    // A report's "kept another machine's path" offers a pair.
    effect(() => {
      const draft = this.drafts.draft();
      const known = this.machines();
      if (!draft || !known.some((m) => m.id === draft.machine)) return;
      untracked(() => {
        this.drafts.take();
        this.machine.set(draft.machine);
        this.otherFolder.set(draft.otherFolder);
        this.error.set('');
        this.notice.set('');
      });
    });
  }

  protected machineLabel(m: PairMachine): string {
    const os = osLabel(m.os);
    const name = m.name || tr('A computer');
    return os ? `${name} (${os})` : name;
  }

  protected sideLabel(s: PairSide): string {
    if (s.here) return this.hereLabel();
    const os = osLabel(s.os);
    const name = s.name || tr('Another computer');
    return os ? `${name} (${os})` : name;
  }

  protected pickMachine(e: Event): void {
    this.machine.set((e.target as HTMLSelectElement).value);
    this.error.set('');
  }

  protected async refresh(): Promise<void> {
    await this.run(() => this.api.getFolderPairs(true), '');
  }

  protected async add(e: Event): Promise<void> {
    e.preventDefault();
    const folder = this.folder().trim();
    const other = this.otherFolder().trim();
    const m = this.chosen();
    if (!m) return;
    // Each the way its machine writes paths; the desktop has the last word.
    const here = folderPathProblem(folder, this.hostOs());
    if (here) {
      this.error.set(
        tr('On this {computer}: {problem}', { computer: this.computer(), problem: here }),
      );
      return;
    }
    const there = otherFolderProblem(other, this.chosenOs(), this.hostOs());
    if (there) {
      this.error.set(
        tr('On {name}: {problem}', { name: m.name || tr('the other computer'), problem: there }),
      );
      return;
    }
    const ok = await this.run(
      () => this.api.addFolderPair({ folder, machine: m.id, otherFolder: other }),
      tr('Paired. Sessions pulled from there now go into the matching project here.'),
    );
    if (ok) {
      this.folder.set('');
      this.otherFolder.set('');
    }
  }

  protected async remove(id: string): Promise<void> {
    await this.run(
      () => this.api.removeFolderPair(id),
      tr('The pair is removed from the backups.'),
    );
  }

  private async run(call: () => Promise<unknown>, ok: string): Promise<boolean> {
    if (this.busy()) return false;
    this.busy.set(true);
    this.error.set('');
    this.notice.set('');
    try {
      const next = normalizeFolderPairs(await call());
      if (next) this.view.set(next);
      if (ok) this.notice.set(ok);
      return true;
    } catch (e) {
      this.error.set(backupErrorText(e));
      return false;
    } finally {
      this.busy.set(false);
    }
  }
}
