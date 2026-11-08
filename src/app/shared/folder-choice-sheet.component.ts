import { ChangeDetectionStrategy, Component, computed, inject, linkedSignal } from '@angular/core';

import { ApiService } from '../core/api.service';
import { AuthService } from '../core/auth.service';
import { osLabel } from '../core/backup';
import { FolderChoiceService } from '../core/folder-choice.service';
import { tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { PairMachine } from '../core/models';
import { NoticeService } from '../core/notice.service';
import {
  folderAbove,
  folderExample,
  folderPathProblem,
  lastSegment,
  machineNoun,
  pathDepth,
} from '../core/paths';
import { IconComponent } from './icon.component';
import { SheetComponent } from './sheet.component';

/** The folder pair offered with the answer: this machine's folder and the
 * other computer's. */
interface PairOffer {
  here: string;
  there: string;
}

/**
 * Where a conversation works on this machine, asked when continuing one
 * another computer started (restored or pulled from a backup) whose folder
 * no folder pair maps, or one whose folder is gone from here
 * (`409 folder_elsewhere`, docs/BACKUP.md). Offers the folders here that
 * look like it, or any folder typed in full; and, for another computer's
 * folder, pairs the folders (the ones above both projects when they have
 * the same name) so its other conversations continue without the question
 * and pulls put them there. One for the whole app, in the shell: every
 * continue asks through {@link FolderChoiceService}.
 */
@Component({
  selector: 'mc-folder-choice',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="!!question()" [title]="title()" (closed)="close()">
      @if (question(); as q) {
        <p class="lede small">{{ q.message }}</p>

        @if (q.suggestions.length) {
          <div class="opts" role="radiogroup" [attr.aria-label]="'Folder' | t">
            @for (f of q.suggestions; track f) {
              <button
                type="button"
                role="radio"
                class="opt"
                [class.on]="picked() === f"
                [attr.aria-checked]="picked() === f"
                (click)="pick(f)"
              >
                <span class="ot mono">{{ f }}</span>
                @if (picked() === f) {
                  <mc-icon name="check" [size]="16" />
                }
              </button>
            }
            <button
              type="button"
              role="radio"
              class="opt"
              [class.on]="picked() === ''"
              [attr.aria-checked]="picked() === ''"
              (click)="pick('')"
            >
              <span class="ot">{{ 'Another folder' | t }}</span>
              @if (picked() === '') {
                <mc-icon name="check" [size]="16" />
              }
            </button>
          </div>
        }
        @if (picked() === '') {
          <label class="field typed">
            <span class="field-label">{{
              'Folder on this {computer}' | t: { computer: noun() }
            }}</span>
            <input
              name="folder"
              spellcheck="false"
              autocapitalize="off"
              autocomplete="off"
              [placeholder]="placeholder()"
              [value]="typed()"
              (input)="type($event)"
              (keydown.enter)="go()"
            />
          </label>
        }

        @if (offer(); as p) {
          <label class="remember">
            <input type="checkbox" [checked]="remember()" (change)="toggle($event)" />
            <span>
              {{ 'Pair||folders' | t }} <span class="mono">{{ p.here }}</span>
              {{ 'on this {computer} with' | t: { computer: noun() } }}
              <span class="mono">{{ p.there }}</span>
              @if (q.machines.length === 1) {
                {{ 'on {machine}' | t: { machine: machineLabel(q.machines[0]) } }}
              }
              {{ '— its other conversations there continue here without asking.' | t }}
            </span>
          </label>
          @if (q.machines.length > 1 && remember()) {
            <label class="field machine">
              <span class="field-label">{{ 'The other computer' | t }}</span>
              <select name="machine" (change)="pickMachine($event)">
                @for (m of q.machines; track m.id) {
                  <option [value]="m.id" [selected]="m.id === machine()">
                    {{ machineLabel(m) }}
                  </option>
                }
              </select>
            </label>
          }
        }

        @if (error()) {
          <p class="notice error small" role="alert">{{ error() }}</p>
        }
        <div class="row actions">
          <button class="btn primary" type="button" [disabled]="!folder()" (click)="go()">
            <mc-icon name="play" [size]="14" /> {{ 'Continue here' | t }}
          </button>
          <button class="btn ghost" type="button" (click)="close()">{{ 'Cancel' | t }}</button>
        </div>
      }
    </mc-sheet>
  `,
  styles: [
    `
      .lede {
        margin: 0 0 0.75rem;
        color: var(--muted);
        line-height: 1.6;
        overflow-wrap: anywhere;
      }
      .opts {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
      }
      .opt {
        appearance: none;
        display: flex;
        align-items: center;
        gap: 0.65rem;
        width: 100%;
        min-height: var(--tap);
        padding: 0.5rem 0.7rem;
        text-align: left;
        font: inherit;
        color: var(--text);
        background: var(--panel-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        cursor: pointer;
      }
      .opt.on {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .opt mc-icon {
        color: var(--accent-text);
      }
      .ot {
        flex: 1;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .typed,
      .machine {
        display: flex;
        flex-direction: column;
        margin-top: 0.75rem;
      }
      .remember {
        display: flex;
        align-items: flex-start;
        gap: 0.55rem;
        margin-top: 0.9rem;
        line-height: 1.5;
        cursor: pointer;
      }
      .remember input {
        margin-top: 0.3rem;
        flex: 0 0 auto;
      }
      .remember .mono {
        overflow-wrap: anywhere;
      }
      .notice {
        margin: 0.75rem 0 0;
      }
      .actions {
        margin-top: 0.9rem;
        gap: 0.5rem;
      }
    `,
  ],
})
export class FolderChoiceSheetComponent {
  private choices = inject(FolderChoiceService);
  private api = inject(ApiService);
  private notices = inject(NoticeService);
  /** The desktop's system: folders here are written its way. */
  private readonly hostOs = inject(AuthService).hostOs;

  protected readonly question = this.choices.question;
  /** The suggestion picked; '' for a folder typed in full. */
  protected readonly picked = linkedSignal(() => this.question()?.suggestions[0] ?? '');
  // Each question starts afresh.
  protected readonly typed = linkedSignal({ source: this.question, computation: () => '' });
  protected readonly remember = linkedSignal({ source: this.question, computation: () => true });
  protected readonly machine = linkedSignal(() => this.question()?.machines[0]?.id ?? '');
  protected readonly error = linkedSignal({ source: this.question, computation: () => '' });

  /** The desktop's computer in a sentence ("Mac", "PC", "machine"), in the
   * language shown. */
  protected readonly noun = computed(() => tr(machineNoun(this.hostOs())));
  protected readonly title = computed(() =>
    tr('Choose its folder on this {computer}', { computer: this.noun() }),
  );
  protected readonly placeholder = computed(() => folderExample(this.hostOs(), 'project'));

  /** The folder the answer names ('' while none is). */
  protected readonly folder = computed(() => (this.picked() || this.typed()).trim());

  /** The pair offered for another computer's folder: the folders the two
   * projects are in when they have the same name (`…/projects` and
   * `D:\\…\\projects`), else the two project folders; never a root. */
  protected readonly offer = computed<PairOffer | null>(() => {
    const q = this.question();
    const folder = this.folder();
    if (!q?.otherMachine || !q.machines.length || !q.recorded) return null;
    if (!folder || folderPathProblem(folder, this.hostOs())) return null;
    const name = lastSegment(folder).toLowerCase();
    const above = { here: folderAbove(folder), there: folderAbove(q.recorded) };
    if (
      name &&
      name === lastSegment(q.recorded).toLowerCase() &&
      pathDepth(above.here) > 0 &&
      pathDepth(above.there) > 0
    ) {
      return above;
    }
    return { here: folder, there: q.recorded };
  });

  protected machineLabel(m: PairMachine): string {
    const os = osLabel(m.os);
    const name = m.name || tr('the other computer');
    return os ? `${name} (${os})` : name;
  }

  protected pick(folder: string): void {
    this.picked.set(folder);
    this.error.set('');
  }

  protected type(e: Event): void {
    this.typed.set((e.target as HTMLInputElement).value);
    this.error.set('');
  }

  protected toggle(e: Event): void {
    this.remember.set((e.target as HTMLInputElement).checked);
  }

  protected pickMachine(e: Event): void {
    this.machine.set((e.target as HTMLSelectElement).value);
  }

  protected close(): void {
    this.choices.answer(null);
  }

  /** Continue there; the folders are paired meanwhile when asked. */
  protected go(): void {
    const folder = this.folder();
    if (!folder) return;
    const problem = folderPathProblem(folder, this.hostOs());
    if (problem) {
      this.error.set(problem);
      return;
    }
    const offer = this.remember() ? this.offer() : null;
    const machine = this.question()?.machines.find((m) => m.id === this.machine());
    this.choices.answer(folder);
    if (offer && machine) void this.pair(offer, machine);
  }

  private async pair(offer: PairOffer, m: PairMachine): Promise<void> {
    try {
      await this.api.addFolderPair({ folder: offer.here, machine: m.id, otherFolder: offer.there });
      this.notices.info(
        tr('Paired {here} with {there} on {machine}.', {
          here: offer.here,
          there: offer.there,
          machine: m.name || tr('the other computer'),
        }),
      );
    } catch (e) {
      this.notices.error(e, () => void this.pair(offer, m), tr('The folders are not paired'));
    }
  }
}
