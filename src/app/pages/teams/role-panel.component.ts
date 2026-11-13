import { ChangeDetectionStrategy, Component, computed, inject, linkedSignal } from '@angular/core';

import { MODELS, taskClassLabel, validModel } from '../../core/capacity';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  DANGEROUS_MODES,
  HANDOFF_NOTE_MAX,
  ROLE_EFFORTS,
  ROLE_ID_MAX,
  ROLE_MODES,
  RoleDef,
  TASK_CLASSES,
  lines,
  machineWords,
} from '../../core/teams';
import { IconComponent } from '../../shared/icon.component';
import { PERMISSION_MODES, effortLabel, modeLabel } from '../../shared/util';
import { TemplateEditor } from './template-editor';
import {
  canRemove,
  connect,
  drawnFlow,
  flowOf,
  roleIdProblem,
  roleOf,
  rolesOf,
  setHandoffNote,
  titleOf,
  updateRole,
} from './template-graph';

const OSES = ['os:windows', 'os:macos', 'os:linux'];
const str = (v: unknown) => (typeof v === 'string' ? v : '');
const strs = (v: unknown) =>
  Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
const same = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((x, i) => x === b[i]);
const value = (e: Event) => (e.target as HTMLInputElement).value;
const checked = (e: Event) => (e.target as HTMLInputElement).checked;
let nextId = 0;

/**
 * The selected role in the template sheet's panel: everything a role has —
 * title, id (renaming it renames its handoffs, who merges and its place),
 * charter, model, effort, permission mode, where its finished work goes next
 * (the non-drag way to draw handoffs, with their notes), files, task classes,
 * budget, autostart and the computer it runs on — with Make lead, Duplicate
 * and Delete.
 */
@Component({
  selector: 'mc-role-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TPipe],
  template: `
    @if (role(); as r) {
      <div class="head">
        <h3 class="name">{{ title() }}</h3>
        @if (r.lead === true) {
          <span class="badge accent">{{ 'lead' | t }}</span>
        }
      </div>
      @if (ed.editable()) {
        <div class="row acts">
          @if (r.lead !== true) {
            <button
              class="btn small"
              type="button"
              [title]="'The lead plans the work and gives each role its tasks.' | t"
              (click)="ed.makeLead(r.id)"
            >
              {{ 'Make lead' | t }}
            </button>
          }
          <button
            class="btn small"
            type="button"
            [disabled]="ed.full()"
            (click)="ed.duplicateRole(r.id)"
          >
            <mc-icon name="copy" [size]="13" /> {{ 'Duplicate' | t }}
          </button>
          <button
            class="btn small danger"
            type="button"
            [disabled]="!removable()"
            [title]="removable() ? '' : ('Make another role the lead first.' | t)"
            (click)="ed.deleteRole(r.id)"
          >
            <mc-icon name="trash" [size]="13" /> {{ 'Delete' | t }}
          </button>
        </div>
      }
      @for (i of issues(); track $index) {
        <p
          class="notice small"
          [class.error]="i.level === 'error'"
          [class.warn]="i.level !== 'error'"
        >
          {{ i.message }}
        </p>
      }

      <section class="group">
        <label class="field">
          <span class="lbl">{{ 'Title' | t }}</span>
          <input
            name="role-title"
            maxlength="60"
            [value]="str(r.title)"
            [readOnly]="!ed.editable()"
            (input)="update({ title: value($event) })"
          />
        </label>
        <label class="field">
          <span class="lbl">{{ 'Id' | t }}</span>
          <input
            class="mono"
            name="role-id"
            spellcheck="false"
            autocapitalize="off"
            [attr.maxlength]="idMax"
            [value]="idDraft()"
            [readOnly]="!ed.editable()"
            [attr.aria-invalid]="!!idProblem()"
            (input)="typeId(value($event))"
            (change)="commitId(value($event))"
          />
          @if (idProblem()) {
            <span class="problem" role="alert">{{ idProblem() }}</span>
          } @else {
            <span class="field-hint">{{
              'It names the role’s session and worktree; renaming it keeps its arrows.' | t
            }}</span>
          }
        </label>
        <label class="field">
          <span class="lbl">{{ 'Description' | t }}</span>
          <input
            name="role-description"
            maxlength="300"
            [placeholder]="'One line: what it is for' | t"
            [value]="str(r.description)"
            [readOnly]="!ed.editable()"
            (input)="update({ description: value($event) || undefined })"
          />
        </label>
        <label class="field">
          <span class="lbl">{{ 'Charter' | t }}</span>
          <textarea
            name="role-charter"
            rows="5"
            [placeholder]="'Its standing instructions: what it does, and how.' | t"
            [value]="str(r.charter)"
            [readOnly]="!ed.editable()"
            (input)="update({ charter: value($event) || undefined })"
          ></textarea>
        </label>
      </section>

      <section class="group">
        <h4 class="sect">{{ 'Its finished work goes next to' | t }}</h4>
        @for (o of next(); track o.id) {
          <div class="next">
            <label class="check">
              <input
                type="checkbox"
                [name]="'next-' + o.id"
                [checked]="o.on"
                [disabled]="!ed.editable()"
                (change)="toggleNext(o.id, checked($event))"
              />
              <span>{{ o.title }}</span>
            </label>
            @if (o.on) {
              <input
                class="note"
                [name]="'note-' + o.id"
                [attr.maxlength]="noteMax"
                [value]="o.note"
                [readOnly]="!ed.editable()"
                [placeholder]="'What passes along (optional)' | t"
                [attr.aria-label]="'What passes along to {role}' | t: { role: o.title }"
                (input)="setNote(o.id, value($event))"
              />
            }
          </div>
        } @empty {
          <p class="muted small">{{ 'Add another role to hand work on to it.' | t }}</p>
        }
        @if (sources()) {
          <span class="field-hint">{{
            'It gets finished work from {roles}.' | t: { roles: sources() }
          }}</span>
        }
        <span class="field-hint">{{
          'The lead is told this order and writes the next role’s task.' | t
        }}</span>
      </section>

      <section class="group">
        <h4 class="sect">{{ 'Model' | t }}</h4>
        <div class="two">
          <label class="field">
            <span class="lbl">{{ 'Model' | t }}</span>
            <input
              name="role-model"
              spellcheck="false"
              autocapitalize="off"
              [attr.list]="listId"
              [placeholder]="'default' | t"
              [value]="str(r.model)"
              [readOnly]="!ed.editable()"
              (input)="update({ model: value($event).trim() || undefined })"
            />
          </label>
          <label class="field">
            <span class="lbl">{{ 'Effort' | t }}</span>
            <select
              name="role-effort"
              [disabled]="!ed.editable()"
              (change)="update({ effort: value($event) || undefined })"
            >
              <option value="" [selected]="!str(r.effort)">{{ 'default' | t }}</option>
              @for (e of efforts; track e) {
                <option [value]="e" [selected]="r.effort === e">{{ effortName(e) }}</option>
              }
              @if (str(r.effort) && !known(efforts, r.effort)) {
                <option [value]="r.effort" selected>{{ r.effort }}</option>
              }
            </select>
          </label>
        </div>
        <datalist [id]="listId">
          @for (m of models; track m) {
            <option [value]="m"></option>
          }
        </datalist>
        @if (modelProblem()) {
          <span class="problem">{{ modelProblem() }}</span>
        }
        <label class="field">
          <span class="lbl">{{ 'Permission mode' | t }}</span>
          <select
            name="role-mode"
            [disabled]="!ed.editable()"
            (change)="update({ permissionMode: value($event) })"
          >
            @for (m of modes; track m) {
              <option [value]="m" [selected]="mode() === m">{{ modeName(m) }}</option>
            }
            @if (!known(modes, mode())) {
              <option [value]="mode()" selected>{{ mode() }}</option>
            }
          </select>
          <span class="field-hint">{{ modeHint() }}</span>
        </label>
        @if (dangerous()) {
          <p class="notice warn small">
            {{
              'It runs without permission prompts: only your {computer}, or a device allowed dangerous actions, can start this team.'
                | t: { computer: computer() }
            }}
          </p>
        }
      </section>

      <section class="group">
        <h4 class="sect">{{ 'Files' | t }}</h4>
        <label class="check">
          <input
            type="checkbox"
            name="role-worktree"
            [checked]="r.worktree === true"
            [disabled]="!ed.editable()"
            (change)="update({ worktree: checked($event) || undefined })"
          />
          <span>{{ 'Works in a worktree of its own' | t }}</span>
        </label>
        <span class="field-hint">{{
          'Its edits never get in the other roles’ way; the lead merges its branch. The project must be a git repository.'
            | t
        }}</span>
        <label class="check">
          <input
            type="checkbox"
            name="role-read-only"
            [checked]="r.readOnly === true"
            [disabled]="!ed.editable()"
            (change)="update({ readOnly: checked($event) || undefined })"
          />
          <span>{{ 'Read-only' | t }}</span>
        </label>
        <span class="field-hint">{{ 'It changes no files: for scouts and reviewers.' | t }}</span>
        <label class="field">
          <span class="lbl">{{ 'Files it may change (one per line)' | t }}</span>
          <textarea
            class="mono"
            name="role-paths"
            rows="3"
            spellcheck="false"
            placeholder="src/app/**"
            [value]="pathsText()"
            [readOnly]="!ed.editable()"
            (input)="setPaths(value($event))"
          ></textarea>
          <span class="field-hint">{{
            'Globs relative to the project, like src/app/** or *.md. Empty: any file.' | t
          }}</span>
        </label>
        <label class="field">
          <span class="lbl">{{ 'Other folders it may use (one per line)' | t }}</span>
          <textarea
            class="mono"
            name="role-add-dirs"
            rows="2"
            spellcheck="false"
            [value]="dirsText()"
            [readOnly]="!ed.editable()"
            (input)="setDirs(value($event))"
          ></textarea>
          <span class="field-hint">{{ 'Folders relative to the project (--add-dir).' | t }}</span>
        </label>
      </section>

      <details class="group more">
        <summary class="sect">{{ 'More' | t }}</summary>
        <label class="check">
          <input
            type="checkbox"
            name="role-autostart"
            [checked]="r.autostart === true"
            [disabled]="!ed.editable()"
            (change)="update({ autostart: checked($event) || undefined })"
          />
          <span>{{ 'Start with the team' | t }}</span>
        </label>
        <span class="field-hint">{{ 'Otherwise it starts when it has work.' | t }}</span>
        <label class="field">
          <span class="lbl">{{ 'Budget per session (USD)' | t }}</span>
          <input
            type="number"
            name="role-budget"
            min="0"
            max="1000"
            step="0.5"
            [value]="r.budgetUsd ?? ''"
            [readOnly]="!ed.editable()"
            (input)="setBudget($event)"
          />
          <span class="field-hint">{{ 'Only for accounts that pay per token.' | t }}</span>
        </label>
        <fieldset class="classes">
          <legend class="lbl">{{ 'Tasks it usually takes' | t }}</legend>
          @for (c of classes; track c) {
            <label class="chip">
              <input
                type="checkbox"
                [checked]="hasClass(c)"
                [disabled]="!ed.editable()"
                (change)="toggleClass(c, checked($event))"
              />
              <span>{{ className(c) }}</span>
            </label>
          }
        </fieldset>
        @if (r.lead !== true) {
          <label class="field">
            <span class="lbl">{{ 'Computer' | t }}</span>
            <select
              name="role-machine"
              [disabled]="!ed.editable()"
              (change)="update({ machine: value($event) || undefined })"
            >
              <option value="" [selected]="!machine()">
                {{ 'This {computer}' | t: { computer: computer() } }}
              </option>
              @for (p of ed.peers(); track p.id) {
                <option [value]="p.name" [selected]="machine() === p.name">{{ p.name }}</option>
              }
              @for (os of oses; track os) {
                <option [value]="os" [selected]="machine() === os">{{ where(os) }}</option>
              }
              @if (unknownMachine()) {
                <option [value]="machine()" selected>{{ machine() }}</option>
              }
            </select>
            <span class="field-hint">{{
              'Another computer runs it once it gave this one its Run permission; its work comes back as a branch.'
                | t
            }}</span>
          </label>
        } @else {
          <span class="field-hint">{{
            'The lead runs on the computer that runs the team.' | t
          }}</span>
        }
      </details>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
      }
      .head {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-width: 0;
      }
      .name {
        margin: 0;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .acts {
        gap: 0.35rem;
      }
      .notice {
        margin: 0;
      }
      .group {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        padding-top: 0.7rem;
        border-top: 1px solid var(--border);
        min-width: 0;
      }
      .sect,
      .lbl {
        margin: 0;
        font-size: 0.72rem;
        font-weight: 600;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.04em;
      }
      .more > summary {
        cursor: pointer;
      }
      .two {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 0.5rem;
      }
      .check,
      .chip {
        display: inline-flex;
        align-items: center;
        gap: 0.45rem;
        font-size: 0.88rem;
        cursor: pointer;
      }
      .next {
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
      }
      .note {
        margin-left: 1.5rem;
        width: calc(100% - 1.5rem);
        font-size: 0.88rem;
        min-height: 0;
        padding: 0.3rem 0.5rem;
      }
      .problem {
        font-size: 0.8rem;
        color: var(--red);
      }
      .classes {
        display: flex;
        flex-wrap: wrap;
        gap: 0.3rem 0.5rem;
        margin: 0;
        padding: 0;
        border: 0;
        min-width: 0;
      }
      .classes legend {
        margin-bottom: 0.35rem;
      }
      .chip {
        padding: 0.15rem 0.55rem 0.15rem 0.4rem;
        border: 1px solid var(--border);
        border-radius: 1rem;
        font-size: 0.8rem;
      }
    `,
  ],
})
export class RolePanelComponent {
  protected readonly ed = inject(TemplateEditor);

  protected readonly listId = `mc-role-models-${nextId++}`;
  protected readonly idMax = ROLE_ID_MAX;
  protected readonly noteMax = HANDOFF_NOTE_MAX;
  protected readonly efforts: readonly string[] = ROLE_EFFORTS;
  protected readonly modes: readonly string[] = ROLE_MODES;
  protected readonly models = MODELS;
  protected readonly classes = TASK_CLASSES;
  protected readonly oses = OSES;
  protected readonly str = str;
  protected readonly value = value;
  protected readonly checked = checked;
  protected readonly computer = computerNoun;

  protected readonly role = computed(() => {
    const s = this.ed.selection();
    return s?.kind === 'role' ? (roleOf(this.ed.template(), s.id) ?? null) : null;
  });
  private readonly id = computed(() => this.role()?.id ?? '');
  protected readonly title = computed(() => titleOf(this.ed.template(), this.id()));
  protected readonly removable = computed(() => canRemove(this.ed.template(), this.id()));
  protected readonly issues = computed(() =>
    this.ed.issues().filter((i) => i.target.kind === 'role' && i.target.id === this.id()),
  );

  /** The id as typed: it renames the role when it is left (Enter, Tab). */
  protected readonly idDraft = linkedSignal(() => this.id());
  /** The role the id was typed for. */
  private draftOf = '';
  protected readonly idProblem = linkedSignal({ source: this.id, computation: () => '' });
  /** The lists as typed (a line being written stays). */
  protected readonly pathsText = linkedSignal<{ id: string; list: string[] }, string>({
    source: () => ({ id: this.id(), list: strs(this.role()?.paths) }),
    computation: (src, prev) =>
      prev && prev.source.id === src.id && same(lines(prev.value), src.list)
        ? prev.value
        : src.list.join('\n'),
  });
  protected readonly dirsText = linkedSignal<{ id: string; list: string[] }, string>({
    source: () => ({ id: this.id(), list: strs(this.role()?.addDirs) }),
    computation: (src, prev) =>
      prev && prev.source.id === src.id && same(lines(prev.value), src.list)
        ? prev.value
        : src.list.join('\n'),
  });

  /** Every other role, and whether this one's work goes on to it. */
  protected readonly next = computed(() => {
    const t = this.ed.template();
    const id = this.id();
    const flow = flowOf(t);
    return rolesOf(t)
      .filter((r) => r.id !== id)
      .map((r) => {
        const h = flow.find((x) => x.from === id && x.to === r.id);
        return { id: r.id, title: titleOf(t, r.id), on: !!h, note: str(h?.note) };
      });
  });
  /** The roles whose work comes to this one. */
  protected readonly sources = computed(() => {
    const t = this.ed.template();
    return drawnFlow(t)
      .filter((h) => h.to === this.id())
      .map((h) => titleOf(t, h.from))
      .join(', ');
  });

  protected readonly mode = computed(() => str(this.role()?.permissionMode) || 'default');
  protected readonly dangerous = computed(() => DANGEROUS_MODES.includes(this.mode()));
  protected readonly modeHint = computed(() => {
    const m = PERMISSION_MODES.find((x) => x.value === this.mode());
    if (m) return tr(m.hint);
    return this.mode() === 'dontAsk' ? tr('Never ask (dangerous)') : '';
  });
  protected readonly modelProblem = computed(() => {
    const m = str(this.role()?.model);
    return m && !validModel(m) ? tr('{model} is not a model.', { model: m }) : '';
  });
  protected readonly machine = computed(() => str(this.role()?.machine));
  protected readonly unknownMachine = computed(() => {
    const m = this.machine();
    return !!m && !OSES.includes(m) && !this.ed.peers().some((p) => p.name === m);
  });

  protected update(patch: Partial<RoleDef>): void {
    this.ed.apply(updateRole(this.ed.template(), this.id(), patch));
  }

  protected typeId(v: string): void {
    this.idDraft.set(v);
    this.draftOf = this.id();
    const to = v.trim();
    this.idProblem.set(to === this.id() ? '' : roleIdProblem(this.ed.template(), to, this.id()));
  }

  /** The id typed is left: rename the role it was typed for (a click on
   * another node selects that one before this field loses focus). */
  protected commitId(v: string): void {
    const from = this.draftOf || this.id();
    this.draftOf = '';
    const to = v.trim();
    if (to === from) {
      this.idDraft.set(to);
      this.idProblem.set('');
      return;
    }
    const problem = this.ed.renameRole(from, to);
    if (from === this.id()) this.idProblem.set(problem);
  }

  protected toggleNext(to: string, on: boolean): void {
    const t = this.ed.template();
    if (on) this.ed.apply(connect(t, this.id(), to));
    else this.ed.disconnect(this.id(), to);
  }

  protected setNote(to: string, note: string): void {
    this.ed.apply(setHandoffNote(this.ed.template(), this.id(), to, note));
  }

  protected setPaths(text: string): void {
    this.pathsText.set(text);
    const list = lines(text);
    this.update({ paths: list.length ? list : undefined });
  }

  protected setDirs(text: string): void {
    this.dirsText.set(text);
    const list = lines(text);
    this.update({ addDirs: list.length ? list : undefined });
  }

  /** A budget as it is typed: a number, or none once the field is empty
   * (a number half typed, like "1.", changes nothing yet). */
  protected setBudget(e: Event): void {
    const input = e.target as HTMLInputElement;
    if (input.validity?.badInput) return;
    const n = input.value.trim() === '' ? NaN : Number(input.value);
    this.update({ budgetUsd: Number.isFinite(n) ? n : undefined });
  }

  protected hasClass(c: string): boolean {
    return strs(this.role()?.classes).includes(c);
  }

  protected toggleClass(c: string, on: boolean): void {
    const now = strs(this.role()?.classes);
    const list = on ? [...now.filter((x) => x !== c), c] : now.filter((x) => x !== c);
    this.update({ classes: list.length ? list : undefined });
  }

  protected known(list: readonly string[], v: unknown): boolean {
    return typeof v === 'string' && list.includes(v);
  }

  protected effortName(e: string): string {
    return effortLabel(e);
  }

  protected modeName(m: string): string {
    return modeLabel(m);
  }

  protected className(c: string): string {
    return taskClassLabel(c);
  }

  protected where(os: string): string {
    return machineWords(os);
  }
}
