import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { osLabel } from '../core/machines';
import { TPipe } from '../core/i18n/t.pipe';
import { machineTaskInput } from '../core/tasks';

/**
 * What a task approval would hand over (the pending card's body for a
 * `machine_task`, docs/PLAN-MACHINES.md Phase 4 decision 6): the computer,
 * the folder there (the task gets its own worktree of it, at the session's
 * code), the task itself, and the session's permission mode, model, effort
 * and account there.
 */
@Component({
  selector: 'mc-machine-task-action',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `
    @if (view(); as v) {
      <dl class="kv">
        <dt>{{ 'Computer' | t }}</dt>
        <dd>
          {{ v.machineName }}
          @if (v.os) {
            <span class="muted small">{{ os(v.os) }}</span>
          }
        </dd>
        <dt>{{ 'Folder' | t }}</dt>
        <dd>
          <span class="mono">{{ v.checkout }}</span>
          <span class="muted small block">{{
            'In its own worktree there, at this session’s code: that computer’s own copy is not touched. Its work comes back here as a branch.'
              | t
          }}</span>
        </dd>
        <dt>{{ 'Task' | t }}</dt>
        <dd>
          <pre class="task">{{ v.task }}</pre>
        </dd>
        <dt>{{ 'Permission mode' | t }}</dt>
        <dd
          [class.warn]="v.permissionMode === 'bypassPermissions' || v.permissionMode === 'dontAsk'"
        >
          <span class="mono">{{ v.permissionMode }}</span>
        </dd>
        @if (v.model) {
          <dt>{{ 'Model' | t }}</dt>
          <dd class="mono">{{ v.model }}</dd>
        }
        @if (v.effort) {
          <dt>{{ 'Effort' | t }}</dt>
          <dd>{{ v.effort }}</dd>
        }
        @if (v.account) {
          <dt>{{ 'Account' | t }}</dt>
          <dd>{{ v.account }}</dd>
        }
      </dl>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .kv {
        margin: 0;
      }
      dd {
        overflow-wrap: anywhere;
      }
      .block {
        display: block;
      }
      .warn {
        color: var(--red);
      }
      .task {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        max-height: 16rem;
        overflow: auto;
        margin: 0;
        font-size: 0.8rem;
        font-family: inherit;
      }
    `,
  ],
})
export class MachineTaskActionComponent {
  readonly input = input<unknown>(undefined);

  protected readonly view = computed(() => machineTaskInput(this.input()));

  protected os(os: string): string {
    return osLabel(os);
  }
}
