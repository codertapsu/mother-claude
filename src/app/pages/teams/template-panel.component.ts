import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';

import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { HANDOFF_NOTE_MAX, Integration, TeamTemplate } from '../../core/teams';
import { IconComponent } from '../../shared/icon.component';
import { RolePanelComponent } from './role-panel.component';
import { TemplateEditor } from './template-editor';
import {
  drawnFlow,
  flowOf,
  leadOf,
  roleOf,
  rolesOf,
  setHandoffNote,
  setMergeBy,
  slugId,
  titleOf,
  updateIntegration,
  updateTemplate,
} from './template-graph';

const str = (v: unknown) => (typeof v === 'string' ? v : '');
const value = (e: Event) => (e.target as HTMLInputElement).value;
const checked = (e: Event) => (e.target as HTMLInputElement).checked;

/**
 * The template sheet's panel, beside the canvas (below it on a phone): what
 * is selected — a role, a handoff, Merge — or, with nothing selected, the
 * template itself. Every drag on the canvas has its way here too.
 */
@Component({
  selector: 'mc-template-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, RolePanelComponent, TPipe],
  template: `
    @switch (ed.selection()?.kind) {
      @case ('role') {
        <mc-role-panel />
      }
      @case ('merge') {
        <div class="head">
          <mc-icon class="merge-ico" name="branch" [size]="16" />
          <h3>{{ 'Merge' | t }}</h3>
        </div>
        <p class="intro">
          {{
            'How the roles’ work comes together: one role merges their branches, once the gate passes.'
              | t
          }}
        </p>
        @for (i of mergeIssues(); track $index) {
          <p
            class="notice small"
            [class.error]="i.level === 'error'"
            [class.warn]="i.level !== 'error'"
          >
            {{ i.message }}
          </p>
        }
        <label class="field">
          <span class="lbl">{{ 'Merged by' | t }}</span>
          <select name="merge-by" [disabled]="!ed.editable()" (change)="mergeBy(value($event))">
            <option value="" [selected]="!merger()">
              {{
                leadTitle()
                  ? ('{role}, the lead (default)' | t: { role: leadTitle() })
                  : ('The lead (default)' | t)
              }}
            </option>
            @for (r of roles(); track r.id) {
              <option [value]="r.id" [selected]="merger() === r.id">{{ r.title }}</option>
            }
            @if (merger() && !hasRole(merger())) {
              <option [value]="merger()" selected>{{ merger() }}</option>
            }
          </select>
          <span class="field-hint">{{ 'Or drag from a role’s dot to Merge.' | t }}</span>
        </label>
        <label class="field">
          <span class="lbl">{{ 'Branch' | t }}</span>
          <input
            class="mono"
            name="merge-branch"
            spellcheck="false"
            autocapitalize="off"
            placeholder="main"
            [value]="str(integ().branch)"
            [readOnly]="!ed.editable()"
            (input)="integrate({ branch: value($event).trim() || undefined })"
          />
          <span class="field-hint">{{ 'The branch the work is merged into.' | t }}</span>
        </label>
        <label class="field">
          <span class="lbl">{{ 'Gate' | t }}</span>
          <input
            class="mono"
            name="merge-gate"
            spellcheck="false"
            autocapitalize="off"
            placeholder="npm test"
            [value]="str(integ().gate)"
            [readOnly]="!ed.editable()"
            (input)="integrate({ gate: value($event) || undefined })"
          />
          <span class="field-hint">{{
            'A command that must pass before the work is merged.' | t
          }}</span>
        </label>
        <label class="check">
          <input
            type="checkbox"
            name="merge-review"
            [checked]="integ().review !== false"
            [disabled]="!ed.editable()"
            (change)="integrate({ review: checked($event) })"
          />
          <span>{{ 'The lead reviews each finished task' | t }}</span>
        </label>
        <span class="field-hint">{{ 'Off: a role’s finished task is done at once.' | t }}</span>
        <label class="field">
          <span class="lbl">{{ 'Stop an idle role after (minutes)' | t }}</span>
          <input
            type="number"
            name="idle-stop"
            min="0"
            max="1440"
            step="1"
            placeholder="30"
            [value]="idle()"
            [readOnly]="!ed.editable()"
            (change)="setIdle(value($event))"
          />
          <span class="field-hint">{{ '0 never stops it. Its conversation is kept.' | t }}</span>
        </label>
      }
      @case ('handoff') {
        @if (handoff(); as h) {
          <div class="head">
            <h3>{{ h.fromTitle }} → {{ h.toTitle }}</h3>
          </div>
          <p class="intro">
            {{
              'When a task of {from} is done, its result usually goes on to {to} next: the lead is told this order and writes the next task for {to}.'
                | t: { from: h.fromTitle, to: h.toTitle }
            }}
          </p>
          @for (i of handoffIssues(); track $index) {
            <p
              class="notice small"
              [class.error]="i.level === 'error'"
              [class.warn]="i.level !== 'error'"
            >
              {{ i.message }}
            </p>
          }
          <label class="field">
            <span class="lbl">{{ 'What passes along' | t }}</span>
            <input
              name="handoff-note"
              [attr.maxlength]="noteMax"
              [placeholder]="'the script, the test report…' | t"
              [value]="h.note"
              [readOnly]="!ed.editable()"
              (input)="setNote(value($event))"
            />
            <span class="field-hint">{{
              'Optional, at most {n} characters.' | t: { n: noteMax }
            }}</span>
          </label>
          <div class="row">
            <button
              class="btn small"
              type="button"
              (click)="ed.select({ kind: 'role', id: h.from })"
            >
              {{ 'Select {role}' | t: { role: h.fromTitle } }}
            </button>
            <button class="btn small" type="button" (click)="ed.select({ kind: 'role', id: h.to })">
              {{ 'Select {role}' | t: { role: h.toTitle } }}
            </button>
          </div>
          @if (ed.editable()) {
            <div class="row">
              <button class="btn small danger" type="button" (click)="ed.disconnect(h.from, h.to)">
                <mc-icon name="trash" [size]="13" /> {{ 'Remove this handoff' | t }}
              </button>
            </div>
          }
        }
      }
      @default {
        <div class="head">
          <h3>{{ 'Template' | t }}</h3>
        </div>
        <p class="intro">
          {{
            'The lead plans the work and gives each role its tasks (the dashed lines). An arrow from one role to another says where its finished work usually goes next, and one role merges the work at the end.'
              | t
          }}
        </p>
        <label class="field">
          <span class="lbl">{{ 'Name' | t }}</span>
          <input
            name="template-name"
            maxlength="80"
            [value]="str(t().name)"
            [readOnly]="!ed.editable()"
            (input)="setName(value($event))"
          />
        </label>
        <label class="field">
          <span class="lbl">{{ 'Id' | t }}</span>
          <input
            class="mono"
            name="template-id"
            maxlength="64"
            spellcheck="false"
            autocapitalize="off"
            [value]="str(t().id)"
            [readOnly]="!ed.editable() || !ed.isNew()"
            (input)="ed.apply(patch({ id: value($event).trim() }))"
          />
          <span class="field-hint">{{
            ed.isNew()
              ? ('Lower case letters, digits and -: it names the template’s file.' | t)
              : ('The id stays once saved: duplicate the template for another one.' | t)
          }}</span>
        </label>
        <label class="field">
          <span class="lbl">{{ 'Description' | t }}</span>
          <textarea
            name="template-description"
            rows="3"
            maxlength="2000"
            [value]="str(t().description)"
            [readOnly]="!ed.editable()"
            (input)="ed.apply(patch({ description: value($event) || undefined }))"
          ></textarea>
        </label>
        <p class="muted small">{{ summary() }}</p>
        <p class="muted small">{{ 'Select a role, an arrow or Merge to see it here.' | t }}</p>
      }
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.65rem;
        min-width: 0;
      }
      .head {
        display: flex;
        align-items: center;
        gap: 0.45rem;
      }
      h3 {
        margin: 0;
        overflow-wrap: anywhere;
      }
      .merge-ico {
        color: var(--green);
      }
      .intro,
      .notice {
        margin: 0;
      }
      .intro {
        font-size: 0.85rem;
        color: var(--muted);
        line-height: 1.45;
      }
      .lbl {
        font-size: 0.72rem;
        font-weight: 600;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.04em;
      }
      .check {
        display: inline-flex;
        align-items: center;
        gap: 0.45rem;
        font-size: 0.88rem;
        cursor: pointer;
      }
      p.muted {
        margin: 0;
      }
    `,
  ],
})
export class TemplatePanelComponent {
  protected readonly ed = inject(TemplateEditor);
  protected readonly noteMax = HANDOFF_NOTE_MAX;
  protected readonly str = str;
  protected readonly value = value;
  protected readonly checked = checked;

  protected readonly t = this.ed.template;
  protected readonly integ = computed<Integration>(() => {
    const i = this.t().integration;
    return i && typeof i === 'object' ? i : {};
  });
  protected readonly roles = computed(() =>
    rolesOf(this.t()).map((r) => ({ id: r.id, title: titleOf(this.t(), r.id) })),
  );
  protected readonly merger = computed(() => str(this.integ().mergeBy));
  protected readonly leadTitle = computed(() => {
    const lead = leadOf(this.t());
    return lead ? titleOf(this.t(), lead.id) : '';
  });
  protected readonly idle = computed(() => {
    const m = this.t().idleStopMinutes;
    return typeof m === 'number' ? String(m) : '';
  });
  protected readonly mergeIssues = computed(() =>
    this.ed.issues().filter((i) => i.target.kind === 'merge'),
  );
  protected readonly handoff = computed(() => {
    const s = this.ed.selection();
    if (s?.kind !== 'handoff') return null;
    const h = flowOf(this.t()).find((x) => x.from === s.from && x.to === s.to);
    return {
      from: s.from,
      to: s.to,
      fromTitle: titleOf(this.t(), s.from),
      toTitle: titleOf(this.t(), s.to),
      note: str(h?.note),
    };
  });
  protected readonly handoffIssues = computed(() => {
    const h = this.handoff();
    return h
      ? this.ed
          .issues()
          .filter(
            (i) => i.target.kind === 'handoff' && i.target.from === h.from && i.target.to === h.to,
          )
      : [];
  });
  /** "4 roles · 2 handoffs". */
  protected readonly summary = computed(() => {
    const roles = rolesOf(this.t()).length;
    const handoffs = drawnFlow(this.t()).length;
    return [
      roles === 1 ? tr('1 role') : tr('{n} roles', { n: roles }),
      handoffs === 1 ? tr('1 handoff') : tr('{n} handoffs', { n: handoffs }),
    ].join(' · ');
  });

  protected hasRole(id: string): boolean {
    return !!roleOf(this.t(), id);
  }

  protected patch(p: Partial<TeamTemplate>): TeamTemplate {
    return updateTemplate(this.t(), p);
  }

  /** A new template's id follows its name until it is typed itself. */
  protected setName(name: string): void {
    const t = this.t();
    const follows = this.ed.isNew() && str(t.id) === slugId(str(t.name));
    const id = follows ? slugId(name) : '';
    this.ed.apply(updateTemplate(t, id ? { name, id } : { name }));
  }

  protected mergeBy(id: string): void {
    this.ed.apply(setMergeBy(this.t(), id || undefined));
  }

  protected integrate(p: Partial<Integration>): void {
    this.ed.apply(updateIntegration(this.t(), p));
  }

  protected setIdle(v: string): void {
    const n = v.trim() === '' ? NaN : Number(v);
    this.ed.apply(
      updateTemplate(this.t(), { idleStopMinutes: Number.isFinite(n) ? Math.round(n) : undefined }),
    );
  }

  protected setNote(note: string): void {
    const h = this.handoff();
    if (h) this.ed.apply(setHandoffNote(this.t(), h.from, h.to, note));
  }
}
