import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { browserActionView, fileSize } from '../core/browser';
import { IconComponent } from './icon.component';

const VERBS: Record<string, string> = {
  navigate: 'Open a page',
  'tabs.open': 'Open a page in a new tab',
  click: 'Click',
  dblclick: 'Double-click',
  hover: 'Hover',
  type: 'Type text',
  fill: 'Fill a form',
  select: 'Choose an option',
  check: 'Check a box',
  press: 'Press a key',
  dialog: 'Answer a dialog',
  eval: 'Run a script in the page',
  upload: 'Upload files',
  scroll: 'Scroll',
};

/**
 * What a browser approval would do (the pending card's body for items whose
 * source is `browser`): the action, the page, the element, what would be
 * typed (never into a secret field: those are refused), the script, the
 * files. Read tolerantly from the item's `input`.
 */
@Component({
  selector: 'mc-browser-action',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  template: `
    @if (view(); as v) {
      <dl class="kv">
        <dt>Action</dt>
        <dd>{{ verb() }}</dd>
        @if (v.url || v.origin) {
          <dt>Page</dt>
          <dd class="mono">{{ v.url || v.origin }}</dd>
        }
        @if (v.element) {
          <dt>Element</dt>
          <dd>
            @if (v.element.name) {
              “{{ v.element.name }}”
            }
            <span class="muted small">{{ v.element.role }} · {{ v.element.ref }}</span>
          </dd>
        } @else if (v.selector) {
          <dt>Element</dt>
          <dd class="mono">{{ v.selector }}</dd>
        }
        @if (v.text !== undefined) {
          <dt>Text</dt>
          <dd class="mono">{{ v.text }}</dd>
        }
        @if (v.fields.length) {
          <dt>Fields</dt>
          <dd>
            <ul class="fields">
              @for (f of v.fields; track $index) {
                <li>
                  {{ f.name || f.ref }}: <span class="mono">{{ f.value }}</span>
                </li>
              }
            </ul>
          </dd>
        }
        @if (v.submit) {
          <dt>Then</dt>
          <dd>submits the form</dd>
        }
        @if (v.key) {
          <dt>Key</dt>
          <dd class="mono">{{ v.key }}</dd>
        }
        @if (v.files.length) {
          <dt>Files</dt>
          <dd>
            <ul class="fields">
              @for (f of v.files; track $index) {
                <li>
                  <span class="mono">{{ f.name }}</span>
                  <span class="muted small">{{ size(f.size) }}</span>
                </li>
              }
            </ul>
          </dd>
        }
        @if (v.browser) {
          <dt>Browser</dt>
          <dd>{{ v.browser }}</dd>
        }
      </dl>
      @if (v.expression) {
        <pre class="mono script">{{ v.expression }}</pre>
        @if (v.touchesStorage) {
          <p class="warn small">
            <mc-icon name="alert" [size]="13" /> It mentions cookies, storage or credentials.
          </p>
        }
      }
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
      .fields {
        margin: 0;
        padding-left: 1rem;
      }
      .script {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        max-height: 14rem;
        overflow: auto;
        margin: 0.5rem 0 0;
        font-size: 0.8rem;
      }
      .warn {
        color: var(--danger, #c0392b);
        display: flex;
        align-items: center;
        gap: 0.3rem;
        margin: 0.4rem 0 0;
      }
    `,
  ],
})
export class BrowserActionComponent {
  readonly input = input<unknown>(undefined);

  protected readonly view = computed(() => browserActionView(this.input()));
  protected readonly verb = computed(() => {
    const a = this.view()?.action ?? '';
    return VERBS[a] ?? a;
  });

  protected size(bytes: number | undefined): string {
    return fileSize(bytes);
  }
}
