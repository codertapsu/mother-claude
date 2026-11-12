import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { osLabel } from '../core/machines';
import { TPipe } from '../core/i18n/t.pipe';
import { machineRunInput, shellName, timeLimitText } from '../core/runs';

/**
 * What a run approval would run (the pending card's body for items whose
 * source is `machine`, docs/PLAN-MACHINES.md Phase 4): the computer, the
 * folder there (its checkout of the project, and the folder inside it the
 * command runs in — in a work folder at the session's code), the command,
 * its shell, its time limit and the files it will list.
 */
@Component({
  selector: 'mc-machine-run-action',
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
          @if (v.cwd) {
            <span class="mono"> › {{ v.cwd }}</span>
          }
          <span class="muted small block">{{
            'In a work folder there, at this session’s code: that computer’s own copy is not touched.'
              | t
          }}</span>
        </dd>
        <dt>{{ 'Command' | t }}</dt>
        <dd>
          <pre class="mono cmd">{{ v.command }}</pre>
        </dd>
        @if (v.shellLabel || v.shell) {
          <dt>{{ 'Shell' | t }}</dt>
          <dd>{{ shell(v.shellLabel || v.shell) }}</dd>
        }
        @if (limit(v.timeoutSecs); as l) {
          <dt>{{ 'Time limit' | t }}</dt>
          <dd>{{ l }}</dd>
        }
        @if (v.artifacts) {
          <dt>{{ 'Files to list' | t }}</dt>
          <dd class="mono">{{ v.artifacts }}</dd>
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
      .cmd {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        max-height: 14rem;
        overflow: auto;
        margin: 0;
        font-size: 0.8rem;
      }
    `,
  ],
})
export class MachineRunActionComponent {
  readonly input = input<unknown>(undefined);

  protected readonly view = computed(() => machineRunInput(this.input()));

  protected os(os: string): string {
    return osLabel(os);
  }

  protected shell(s: string): string {
    return shellName(s);
  }

  protected limit(secs: number | null): string {
    return timeLimitText(secs);
  }
}
