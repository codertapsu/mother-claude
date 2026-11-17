import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { TPipe } from '../core/i18n/t.pipe';
import { runsUnapproved, sessionTalkInput } from '../core/session-talk';

/**
 * What a session asks to do with another one (the pending card's body for
 * a `session` card, docs/MCP.md "Sessions working together"): which session
 * — its project, folder and computer, whether it runs — and what it would
 * read, send or ask, or where a new session would start with what.
 */
@Component({
  selector: 'mc-session-talk-action',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `
    @if (view(); as v) {
      <dl class="kv">
        @if (v.action !== 'start') {
          <dt>{{ 'Session' | t }}</dt>
          <dd>
            {{ v.sessionName }}
            <span class="muted small">{{ v.live ? ('running' | t) : ('not running' | t) }}</span>
            @if (v.session) {
              <span class="mono muted small block">{{ v.session }}</span>
            }
          </dd>
        }
        @if (v.sessionProject) {
          <dt>{{ 'Project' | t }}</dt>
          <dd>{{ v.sessionProject }}</dd>
        }
        @if (v.folder) {
          <dt>{{ 'Folder' | t }}</dt>
          <dd class="mono">{{ v.folder }}</dd>
        }
        <dt>{{ 'Computer' | t }}</dt>
        <dd>{{ v.here ? ('This one' | t) : v.computer }}</dd>
        @switch (v.action) {
          @case ('read') {
            <dt>{{ 'Reads' | t }}</dt>
            <dd>
              {{ 'Its last {n} messages' | t: { n: v.last ?? 10 } }}
              <span class="muted small block">{{ 'Nothing is sent to it.' | t }}</span>
            </dd>
          }
          @case ('start') {
            <dt>{{ 'First message' | t }}</dt>
            <dd>
              <pre class="msg">{{ v.message }}</pre>
              <span class="muted small block">{{
                'A new session Mother Claude runs, in the folder above.' | t
              }}</span>
            </dd>
          }
          @default {
            <dt>{{ v.action === 'ask' ? ('Question' | t) : ('Message' | t) }}</dt>
            <dd>
              <pre class="msg">{{ v.message }}</pre>
              <span class="muted small block">{{
                v.action === 'ask'
                  ? ('It reaches the session saying which session asks; this session waits for the answer.'
                    | t)
                  : ('It reaches the session saying which session sent it and how to answer.' | t)
              }}</span>
              @if (v.live === false) {
                <span class="muted small block">{{
                  'It is not running: the message continues it as a session Mother Claude runs.' | t
                }}</span>
              }
            </dd>
          }
        }
        @if (v.permissionMode) {
          <dt>{{ 'Permission mode' | t }}</dt>
          <dd [class.warn]="unapproved()">
            <span class="mono">{{ v.permissionMode }}</span>
          </dd>
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
      .msg {
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
export class SessionTalkActionComponent {
  readonly input = input<unknown>(undefined);

  protected readonly view = computed(() => sessionTalkInput(this.input()));
  protected readonly unapproved = computed(() => runsUnapproved(this.view()?.permissionMode ?? ''));
}
