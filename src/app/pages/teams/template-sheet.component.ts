import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { TeamTemplate, TemplateEntry, templateJson } from '../../core/teams';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';
import { TemplateCanvasComponent } from './template-canvas.component';
import { EditorIssue, TemplateEditor } from './template-editor';
import { selectionOf, titleOf } from './template-graph';
import { TemplatePanelComponent } from './template-panel.component';

export type TemplateSheetMode = 'view' | 'edit' | 'new';

type View = 'visual' | 'json';

/** A skeleton for a new template. */
const NEW_TEMPLATE: TeamTemplate = {
  id: 'my-team',
  name: 'My team',
  description: '',
  roles: [
    {
      id: 'lead',
      title: 'Lead',
      lead: true,
      charter: 'You plan the work, write one task per role and review what comes back.',
      model: 'opus',
      effort: 'high',
      permissionMode: 'default',
      paths: ['docs/**', '*.md'],
      autostart: true,
    },
    {
      id: 'dev',
      title: 'Developer',
      charter: 'You implement the tasks you are given, with tests.',
      model: 'sonnet',
      effort: 'medium',
      permissionMode: 'acceptEdits',
      worktree: true,
      paths: ['src/**'],
    },
  ],
  integration: { branch: 'main', mergeBy: 'lead', gate: 'npm test', review: true },
  idleStopMinutes: 30,
};

let nextId = 0;

/**
 * View a template, or edit one of the user's (docs/TEAMS.md "Templates"):
 * **Visual** — its roles as nodes on a canvas, the handoffs between them,
 * who merges, with a panel for what is selected — or **JSON**, the same
 * template as text, kept in step. Check asks the server what is wrong;
 * every problem shows on its node or edge and in the list below.
 */
@Component({
  selector: 'mc-template-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, SheetComponent, TemplateCanvasComponent, TemplatePanelComponent, TPipe],
  providers: [TemplateEditor],
  template: `
    <mc-sheet [open]="open()" [title]="title()" size="editor" (closed)="closed.emit()">
      <div class="shell">
        <div class="bar">
          <div class="tabs" role="tablist" [attr.aria-label]="'Template views' | t">
            @for (v of views; track v; let i = $index) {
              <button
                type="button"
                role="tab"
                class="tab"
                [id]="tabId(v)"
                [attr.aria-selected]="view() === v"
                [attr.aria-controls]="panelId"
                [attr.tabindex]="view() === v ? 0 : -1"
                (click)="view.set(v)"
                (keydown)="tabKey($event, i)"
              >
                {{ v === 'visual' ? ('Visual' | t) : 'JSON' }}
              </button>
            }
          </div>
          @if (readOnly()) {
            <span class="muted small">
              {{ 'A built-in template cannot be changed: duplicate it to make your own.' | t }}
            </span>
          }
        </div>
        <div class="view" role="tabpanel" [id]="panelId" [attr.aria-labelledby]="tabId(view())">
          @if (view() === 'visual') {
            @if (ed.parseError()) {
              <div class="notice warn small stale-json">
                <span>{{
                  'The JSON has a mistake, so this shows the last version that worked. Fix it on the JSON tab, or go back to this version.'
                    | t
                }}</span>
                <button class="btn small" type="button" (click)="ed.restoreText()">
                  {{ 'Use this version' | t }}
                </button>
              </div>
            }
            <div class="editor">
              <mc-template-canvas />
              <mc-template-panel class="side" />
            </div>
          } @else {
            @if (!readOnly()) {
              <p class="muted small">
                {{
                  'Roles: id, title, lead, charter, model, effort, permissionMode, worktree, paths (globs relative to the project, gitignore style), readOnly, budgetUsd, autostart, addDirs, tools, classes, machine (the computer a role runs on: a paired computer’s name, or os:windows, os:macos). Exactly one role is the lead, and it runs here.'
                    | t
                }}
                {{
                  'The flow: {"from": "writer", "to": "qa", "note": "the draft"} — where a role’s finished work goes next.'
                    | t
                }}
              </p>
            }
            <textarea
              class="mono json"
              name="json"
              spellcheck="false"
              [attr.aria-label]="'Template (JSON)' | t"
              [readOnly]="readOnly()"
              [value]="ed.text()"
              (input)="ed.setText(value($event))"
            ></textarea>
            @if (ed.parseError()) {
              <p class="notice error small" role="alert">{{ ed.parseError() }}</p>
            }
          }
        </div>
        @if (ed.issues().length) {
          <ul class="issues" [attr.aria-label]="'Problems' | t">
            @for (i of ed.issues(); track $index) {
              <li>
                <button
                  type="button"
                  class="issue"
                  [class.warn]="i.level !== 'error'"
                  [class.stale]="i.stale"
                  (click)="goTo(i)"
                >
                  <mc-icon [name]="i.level === 'error' ? 'alert' : 'info'" [size]="14" />
                  <span>
                    @if (where(i); as w) {
                      <strong>{{ w }}:</strong>
                    }
                    {{ i.message }}
                    @if (i.stale) {
                      <span class="faint">{{ '(at the last check)' | t }}</span>
                    }
                  </span>
                </button>
              </li>
            }
          </ul>
        }
        @if (valid()) {
          <p class="notice ok small">{{ 'The template is valid.' | t }}</p>
        }
        @if (error()) {
          <p class="notice error small" role="alert">{{ error() }}</p>
        }
        <div class="row foot">
          <span class="spacer"></span>
          @if (readOnly()) {
            <button class="btn" type="button" [disabled]="busy()" (click)="duplicate()">
              {{ 'Duplicate to edit' | t }}
            </button>
          } @else {
            <button class="btn" type="button" [disabled]="busy()" (click)="check()">
              {{ 'Check' | t }}
            </button>
            <button class="btn primary" type="button" [disabled]="busy()" (click)="save()">
              {{ busy() ? ('Saving…' | t) : ('Save' | t) }}
            </button>
          }
        </div>
      </div>
    </mc-sheet>
  `,
  styles: [
    `
      .shell {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }
      .shell p {
        margin: 0;
      }
      .bar {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.4rem 1rem;
      }
      .tabs {
        display: flex;
        gap: 0.15rem;
        border-bottom: 1px solid var(--border);
      }
      .tab {
        appearance: none;
        background: none;
        border: 0;
        border-bottom: 2px solid transparent;
        color: var(--muted);
        font: inherit;
        font-size: 0.9rem;
        font-weight: 500;
        padding: 0.35rem 0.85rem;
        min-height: var(--btn-h);
        cursor: pointer;
      }
      .tab[aria-selected='true'] {
        color: var(--text);
        border-bottom-color: var(--accent);
      }
      .view,
      .editor {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        min-height: 0;
      }
      .stale-json {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.5rem;
      }
      .stale-json > span {
        flex: 1 1 16rem;
      }
      .json {
        min-height: 50dvh;
        font-size: 0.78rem;
        line-height: 1.45;
        white-space: pre;
        overflow: auto;
      }
      .issues {
        flex: none;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        max-height: 7.5rem;
        overflow: auto;
        margin: 0;
        padding: 0;
        list-style: none;
      }
      .issue {
        display: flex;
        align-items: flex-start;
        gap: 0.45rem;
        width: 100%;
        padding: 0.3rem 0.55rem;
        border: 1px solid var(--red-border);
        border-radius: var(--radius-sm);
        background: var(--red-soft);
        color: var(--text);
        font: inherit;
        font-size: 0.82rem;
        line-height: 1.4;
        text-align: left;
        cursor: pointer;
      }
      .issue > mc-icon {
        margin-top: 0.1rem;
        color: var(--red);
      }
      .issue.warn {
        border-color: var(--amber-border);
        background: var(--amber-soft);
      }
      .issue.warn > mc-icon {
        color: var(--amber);
      }
      .issue.stale {
        opacity: 0.75;
      }
      .foot {
        flex: none;
      }
      @media (min-width: 900px) {
        .shell {
          height: 100%;
        }
        .view {
          flex: 1;
        }
        .editor {
          flex: 1;
          display: grid;
          grid-template-columns: minmax(0, 1fr) 21rem;
          gap: 1rem;
        }
        .side {
          min-height: 0;
          overflow: auto;
          padding: 0.1rem 0.3rem 0.5rem 0.1rem;
        }
        .json {
          flex: 1;
          min-height: 0;
        }
      }
    `,
  ],
})
export class TemplateSheetComponent {
  private api = inject(ApiService);
  protected readonly ed = inject(TemplateEditor);

  readonly open = input(false);
  readonly entry = input<TemplateEntry | null>(null);
  readonly mode = input<TemplateSheetMode>('view');
  readonly closed = output<void>();
  readonly saved = output<void>();
  readonly duplicated = output<void>();

  private readonly uid = nextId++;
  protected readonly panelId = `mc-template-view-${this.uid}`;
  protected readonly views: readonly View[] = ['visual', 'json'];
  protected readonly view = signal<View>('visual');
  protected readonly error = signal('');
  protected readonly busy = signal(false);
  /** The edit the last check was made at (−1: none). */
  private readonly checkedAt = signal(-1);

  protected readonly readOnly = computed(() => this.mode() === 'view' || !!this.entry()?.builtin);
  protected readonly title = computed(() => {
    const e = this.entry();
    if (this.mode() === 'new' || !e) return tr('New template');
    return this.readOnly() ? e.name : tr('Edit {name}', { name: e.name });
  });
  /** Checked, nothing changed since, and no error. */
  protected readonly valid = computed(
    () =>
      this.checkedAt() === this.ed.revision() &&
      !this.ed.parseError() &&
      !this.ed.issues().some((i) => i.level === 'error'),
  );

  constructor() {
    effect(() => {
      if (!this.open()) return;
      const e = this.entry();
      const mode = this.mode();
      untracked(() => {
        const fresh = mode === 'new' || !e;
        const t: TeamTemplate = fresh
          ? structuredClone(NEW_TEMPLATE)
          : (JSON.parse(templateJson(e)) as TeamTemplate);
        this.ed.load(t, { readOnly: this.readOnly(), isNew: fresh });
        if (!fresh) this.ed.setReport(e.report ?? null, t);
        this.view.set('visual');
        this.error.set('');
        this.checkedAt.set(-1);
        if (!this.readOnly()) void this.loadPeers();
      });
    });
  }

  private async loadPeers(): Promise<void> {
    try {
      this.ed.peers.set((await this.api.getMachines()).peers ?? []);
    } catch {
      // Machines unavailable (an older server, a phone without it): the
      // computer choice offers this one and any of an OS.
      this.ed.peers.set([]);
    }
  }

  protected value(e: Event): string {
    return (e.target as HTMLTextAreaElement).value;
  }

  protected tabId(v: View): string {
    return `mc-template-tab-${this.uid}-${v}`;
  }

  protected tabKey(e: KeyboardEvent, i: number): void {
    const n = this.views.length;
    let next = -1;
    if (e.key === 'ArrowRight') next = (i + 1) % n;
    else if (e.key === 'ArrowLeft') next = (i - 1 + n) % n;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = n - 1;
    if (next < 0) return;
    e.preventDefault();
    const v = this.views[next];
    this.view.set(v);
    queueMicrotask(() => document.getElementById(this.tabId(v))?.focus());
  }

  /** Where a problem is, in the words of the tab shown. */
  protected where(i: EditorIssue): string {
    if (this.view() === 'json') return i.path;
    const t = this.ed.template();
    const g = i.target;
    switch (g.kind) {
      case 'role':
        return titleOf(t, g.id);
      case 'handoff':
        return `${titleOf(t, g.from)} → ${titleOf(t, g.to)}`;
      case 'merge':
        return tr('Merge');
      default:
        return i.path;
    }
  }

  /** Show what a problem is about. */
  protected goTo(i: EditorIssue): void {
    this.view.set('visual');
    this.ed.select(selectionOf(i.target));
  }

  /** Validate on the server (what saving would say). */
  protected async check(): Promise<boolean> {
    this.error.set('');
    if (this.ed.parseError()) {
      this.error.set(this.ed.parseError());
      return false;
    }
    const t = this.ed.template();
    const at = this.ed.revision();
    try {
      const r = await this.api.validateTeamTemplate(t);
      this.ed.setReport(r, t, at);
      this.checkedAt.set(at);
      return r.ok;
    } catch (e) {
      this.error.set(describeError(e).text);
      return false;
    }
  }

  protected async save(): Promise<void> {
    this.busy.set(true);
    try {
      const t = this.ed.template();
      if (!(await this.check())) return;
      const e = this.entry();
      if (this.mode() === 'new' || !e) {
        await this.api.createTeamTemplate(t);
      } else {
        await this.api.saveTeamTemplate(e.id, t);
      }
      this.saved.emit();
      this.closed.emit();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }

  protected async duplicate(): Promise<void> {
    const e = this.entry();
    if (!e) return;
    this.busy.set(true);
    try {
      await this.api.duplicateTeamTemplate(e.id);
      this.duplicated.emit();
      this.closed.emit();
    } catch (err) {
      this.error.set(describeError(err).text);
    } finally {
      this.busy.set(false);
    }
  }
}
