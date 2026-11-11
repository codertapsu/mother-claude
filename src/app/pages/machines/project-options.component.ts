import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, inject, input, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  CodeApply,
  CodeSettings,
  CodeTransfer,
  ProjectInput,
  ProjectOptions,
  RemoteCheckView,
  defaultCode,
} from '../../core/machines';
import { ProjectInfo } from '../../core/models';
import { applyText, serverText, transferText } from './machines-text';

/** A project's form: `''` follows the defaults. */
interface Draft {
  project: string;
  transfer: '' | CodeTransfer;
  apply: '' | CodeApply;
  keep: '' | 'on' | 'off';
  macos: string;
  windows: string;
  linux: string;
  carry: string;
}

function lines(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
}

/**
 * Each project's own Machines options (docs/PLAN-MACHINES.md §5.10, "Where
 * each choice is made"): how its code travels and where it lands — else the
 * defaults — and its recipe: what to run after a move on each system (asked
 * of Claude in the move note, never run by Mother Claude), and the files
 * git ignores that travel with it. Adding a file to carry needs the
 * dangerous right: such files usually hold secrets. *Check the remote*
 * tells whether the project's remote answers without a prompt, here and on
 * the other computers (on demand only).
 */
@Component({
  selector: 'mc-machines-project-options',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, NgTemplateOutlet, TPipe],
  template: `
    @if (error()) {
      <p class="notice error small" role="alert">{{ error() }}</p>
    }
    @if (notice()) {
      <p class="notice ok small" role="status">{{ notice() }}</p>
    }
    <ul class="list">
      @for (p of projects(); track p.key) {
        <li>
          <div class="row">
            <strong>{{ nameOf(p) }}</strong>
            <span class="muted small mono">{{ p.project ?? p.key }}</span>
            <span class="spacer"></span>
            @if (p.project) {
              <button
                type="button"
                class="btn ghost small"
                [disabled]="busy() === p.key"
                (click)="checkRemote(p.project, p.key)"
              >
                {{ 'Check the remote' | t }}
              </button>
              <button type="button" class="btn ghost small" (click)="edit(p)">
                {{ 'Change' | t }}
              </button>
            }
            <button
              type="button"
              class="btn ghost small"
              [disabled]="busy() === p.key"
              (click)="remove(p)"
            >
              {{ 'Use the defaults' | t }}
            </button>
          </div>
          <span class="small">{{ summary(p) }}</span>
          @if (remote()?.key === p.key) {
            <ul class="remote small" role="status">
              @for (r of remote()!.view.results; track r.machine) {
                <li [attr.data-state]="r.state">
                  <span aria-hidden="true">{{
                    r.state === 'ok' ? '✓' : r.state === 'failed' ? '✕' : '–'
                  }}</span>
                  <strong>{{ r.name }}:</strong> {{ text(r.detail) }}
                </li>
              }
            </ul>
          }
          @if (editing() === p.key) {
            <ng-container [ngTemplateOutlet]="form" />
          }
        </li>
      } @empty {
        <li class="muted small">
          {{ 'No project has its own options: every one follows the defaults.' | t }}
        </li>
      }
    </ul>

    @if (editing() === 'new') {
      <ng-container [ngTemplateOutlet]="form" />
    } @else if (!readonly()) {
      <button type="button" class="btn small" (click)="add()">
        {{ 'Set a project’s own options…' | t }}
      </button>
    }

    <ng-template #form>
      @if (draft(); as d) {
        <form class="stack editor" (ngSubmit)="save()">
          @if (editing() === 'new') {
            <label class="field">
              <span class="field-label">{{ 'Project' | t }}</span>
              <select
                name="project"
                [ngModel]="d.project"
                (ngModelChange)="patch({ project: $event })"
              >
                <option value="">{{ 'Choose a project' | t }}</option>
                @for (c of candidates(); track c.path) {
                  <option [value]="c.path">{{ c.name }} — {{ c.path }}</option>
                }
              </select>
            </label>
          }
          <label class="field">
            <span class="field-label">{{ 'How code travels' | t }}</span>
            <select
              name="transfer"
              [ngModel]="d.transfer"
              (ngModelChange)="patch({ transfer: $event })"
            >
              <option value="">
                {{ 'The default: {value}' | t: { value: transferLabel(defaults().transfer) } }}
              </option>
              <option value="bundle">{{ 'Machine to machine' | t }}</option>
              <option value="remote">{{ 'Through your git remote' | t }}</option>
            </select>
          </label>
          <label class="field">
            <span class="field-label">{{ 'Where the code lands' | t }}</span>
            <select name="apply" [ngModel]="d.apply" (ngModelChange)="patch({ apply: $event })">
              <option value="">
                {{ 'The default: {value}' | t: { value: applyLabel(defaults().apply) } }}
              </option>
              <option value="checkout">{{ 'In the project folder' | t }}</option>
              <option value="worktree">{{ 'Always a separate folder (worktree)' | t }}</option>
            </select>
          </label>
          <label class="field">
            <span class="field-label">{{ 'Hidden refs on the remote' | t }}</span>
            <select name="keep" [ngModel]="d.keep" (ngModelChange)="patch({ keep: $event })">
              <option value="">
                {{ 'The default: {value}' | t: { value: keepLabel(defaults().keepRemoteRefs) } }}
              </option>
              <option value="on">{{ 'Keep them' | t }}</option>
              <option value="off">{{ 'Delete them after the move' | t }}</option>
            </select>
          </label>
          <fieldset class="recipe">
            <legend class="field-label">{{ 'After a move, ask Claude to run' | t }}</legend>
            <p class="muted small">
              {{
                'One command a line. They join the note the session gets, so they run through its own approvals.'
                  | t
              }}
            </p>
            <label class="field">
              <span class="field-label">macOS</span>
              <textarea
                name="macos"
                rows="2"
                [ngModel]="d.macos"
                (ngModelChange)="patch({ macos: $event })"
              ></textarea>
            </label>
            <label class="field">
              <span class="field-label">Windows</span>
              <textarea
                name="windows"
                rows="2"
                [ngModel]="d.windows"
                (ngModelChange)="patch({ windows: $event })"
              ></textarea>
            </label>
            <label class="field">
              <span class="field-label">Linux</span>
              <textarea
                name="linux"
                rows="2"
                [ngModel]="d.linux"
                (ngModelChange)="patch({ linux: $event })"
              ></textarea>
            </label>
          </fieldset>
          <label class="field">
            <span class="field-label">
              {{ 'Files git ignores that travel with it' | t }}
              <span class="badge danger">{{ 'risky' | t }}</span>
            </span>
            <textarea
              name="carry"
              rows="2"
              placeholder=".env.local"
              [disabled]="!canDangerous() && !hasCarried(d)"
              [ngModel]="d.carry"
              (ngModelChange)="patch({ carry: $event })"
            ></textarea>
            <span class="muted small">
              {{
                'One path in the project a line. They usually hold secrets: adding one needs the dangerous right.'
                  | t
              }}
            </span>
          </label>
          <div class="row">
            <button
              type="submit"
              class="btn primary small"
              [disabled]="!d.project || busy() === 'save'"
            >
              {{ busy() === 'save' ? ('Saving…' | t) : ('Save' | t) }}
            </button>
            <button type="button" class="btn ghost small" (click)="editing.set(null)">
              {{ 'Cancel' | t }}
            </button>
          </div>
        </form>
      }
    </ng-template>
  `,
  styles: [
    `
      .list {
        list-style: none;
        margin: 0.3rem 0;
        padding: 0;
      }
      .list > li {
        padding: 0.35rem 0;
        border-bottom: 1px solid var(--border);
      }
      .list > li:last-child {
        border-bottom: 0;
      }
      .row {
        flex-wrap: wrap;
      }
      .editor {
        margin: 0.5rem 0;
      }
      fieldset.recipe {
        border: 0;
        margin: 0;
        padding: 0;
      }
      .remote {
        list-style: none;
        margin: 0.3rem 0 0;
        padding: 0;
      }
      .remote li[data-state='ok'] > span:first-child {
        color: var(--green);
      }
      .remote li[data-state='failed'] > span:first-child {
        color: var(--red);
      }
      textarea {
        font-family: var(--mono, monospace);
      }
    `,
  ],
})
export class ProjectOptionsComponent {
  /** The defaults a project follows. */
  readonly defaults = input<CodeSettings>(defaultCode());
  readonly readonly = input(false);

  private readonly api = inject(ApiService);
  private readonly auth = inject(AuthService);

  protected readonly canDangerous = this.auth.canDangerous;
  protected readonly projects = signal<ProjectOptions[]>([]);
  protected readonly candidates = signal<ProjectInfo[]>([]);
  /** The project being changed (its key), or `new`. */
  protected readonly editing = signal<string | null>(null);
  protected readonly draft = signal<Draft | null>(null);
  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');
  protected readonly notice = signal('');
  protected readonly remote = signal<{ key: string; view: RemoteCheckView } | null>(null);
  /** The files a project carried when its form opened. */
  private carriedBefore: string[] = [];

  constructor() {
    void this.load();
  }

  private async load(): Promise<void> {
    try {
      const s = await this.api.getMachinesSettings();
      this.projects.set(s.projects ?? []);
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  protected text(s: string): string {
    return serverText(s);
  }

  protected transferLabel(t: CodeTransfer): string {
    return transferText(t);
  }

  protected applyLabel(a: CodeApply): string {
    return applyText(a);
  }

  protected keepLabel(on: boolean): string {
    return on ? tr('Keep them') : tr('Delete them after the move');
  }

  protected nameOf(p: ProjectOptions): string {
    const path = p.project ?? p.key;
    const parts = path.split(/[\\/]/).filter((x) => x);
    return parts[parts.length - 1] ?? path;
  }

  protected summary(p: ProjectOptions): string {
    const parts: string[] = [];
    if (p.code.transfer) parts.push(transferText(p.code.transfer));
    if (p.code.apply) parts.push(applyText(p.code.apply));
    if (p.code.keepRemoteRefs !== undefined) {
      parts.push(p.code.keepRemoteRefs ? tr('hidden refs kept') : tr('hidden refs deleted'));
    }
    const after = p.recipe?.afterMove;
    const n =
      (after?.macos?.length ?? 0) + (after?.windows?.length ?? 0) + (after?.linux?.length ?? 0);
    if (n)
      parts.push(n === 1 ? tr('1 command after a move') : tr('{n} commands after a move', { n }));
    const carried = p.recipe?.carryIgnored?.length ?? 0;
    if (carried) {
      parts.push(
        carried === 1
          ? tr('carries 1 ignored file')
          : tr('carries {n} ignored files', { n: carried }),
      );
    }
    return parts.join(' · ') || tr('The defaults');
  }

  protected hasCarried(d: Draft): boolean {
    return lines(d.carry).length > 0 || this.carriedBefore.length > 0;
  }

  protected patch(p: Partial<Draft>): void {
    this.draft.update((d) => (d ? { ...d, ...p } : d));
  }

  protected async add(): Promise<void> {
    this.carriedBefore = [];
    this.draft.set({
      project: '',
      transfer: '',
      apply: '',
      keep: '',
      macos: '',
      windows: '',
      linux: '',
      carry: '',
    });
    this.editing.set('new');
    if (!this.candidates().length) {
      try {
        this.candidates.set((await this.api.getProjects()).projects);
      } catch (e) {
        this.error.set(describeError(e).text);
      }
    }
  }

  protected edit(p: ProjectOptions): void {
    if (!p.project) return;
    const after = p.recipe?.afterMove;
    this.carriedBefore = p.recipe?.carryIgnored ?? [];
    this.draft.set({
      project: p.project,
      transfer: p.code.transfer ?? '',
      apply: p.code.apply ?? '',
      keep: p.code.keepRemoteRefs === undefined ? '' : p.code.keepRemoteRefs ? 'on' : 'off',
      macos: (after?.macos ?? []).join('\n'),
      windows: (after?.windows ?? []).join('\n'),
      linux: (after?.linux ?? []).join('\n'),
      carry: this.carriedBefore.join('\n'),
    });
    this.editing.set(p.key);
  }

  protected async save(): Promise<void> {
    const d = this.draft();
    if (!d?.project) return;
    const input: ProjectInput = {
      project: d.project,
      code: {
        ...(d.transfer ? { transfer: d.transfer } : {}),
        ...(d.apply ? { apply: d.apply } : {}),
        ...(d.keep ? { keepRemoteRefs: d.keep === 'on' } : {}),
      },
      recipe: {
        afterMove: { macos: lines(d.macos), windows: lines(d.windows), linux: lines(d.linux) },
        carryIgnored: lines(d.carry),
      },
    };
    await this.withBusy('save', async () => {
      const s = await this.api.saveMachinesSettings({ projects: { upsert: [input] } });
      this.projects.set(s.projects ?? []);
      this.editing.set(null);
      this.notice.set(tr('Saved.'));
    });
  }

  protected async remove(p: ProjectOptions): Promise<void> {
    await this.withBusy(p.key, async () => {
      const s = await this.api.saveMachinesSettings({ projects: { remove: [p.key] } });
      this.projects.set(s.projects ?? []);
      this.notice.set(tr('{name} follows the defaults again.', { name: this.nameOf(p) }));
    });
  }

  protected async checkRemote(project: string, key: string): Promise<void> {
    await this.withBusy(key, async () => {
      this.remote.set({ key, view: await this.api.checkProjectRemote(project) });
    });
  }

  private async withBusy(key: string, fn: () => Promise<void>): Promise<void> {
    this.busy.set(key);
    this.error.set('');
    this.notice.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
