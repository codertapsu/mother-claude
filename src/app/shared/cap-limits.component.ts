import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';
import { RouterLink } from '@angular/router';

import { AuthService } from '../core/auth.service';
import { CapLimit, LimitAction, SessionSummary } from '../core/models';
import { SessionActionsService } from '../core/session-actions.service';
import { capLimits } from './caps';
import { CopyButtonComponent } from './copy-button.component';
import { IconComponent } from './icon.component';

/**
 * A session's limitations as a list, each with the button that fixes it
 * (design amendment: `ControlCaps.limits[].action`). Used in the channel
 * sheet, the header and composer "why not?" dialogs and the Info tab.
 *
 * Handles every action itself except `open-terminal`, which it emits when
 * a parent shows the terminal in place (else it navigates there).
 */
@Component({
  selector: 'mc-cap-limits',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, CopyButtonComponent, IconComponent],
  template: `
    <ul class="limits">
      @for (l of list(); track l.code) {
        <li>
          <mc-icon name="info" [size]="15" />
          <div class="ltext">
            <span>{{ l.text }}</span>
            @switch (l.action) {
              @case ('install-foreign-control') {
                @if (isDesktop()) {
                  <a class="btn small" routerLink="/remote" fragment="foreign" (click)="done.emit()"
                    ><mc-icon name="plug" [size]="14" /> Install foreign-session control</a
                  >
                } @else {
                  <span class="muted small">On the Mac: Remote › Foreign sessions › Install.</span>
                }
              }
              @case ('reload-plugins') {
                <span class="row">
                  <span class="muted small">Run this in that session once:</span>
                  <mc-copy-button text="/reload-plugins" label="Copy /reload-plugins" />
                </span>
              }
              @case ('continue-owned') {
                @if (session(); as s) {
                  <button
                    class="btn small primary"
                    type="button"
                    [disabled]="busy()"
                    (click)="continueHere(s, false)"
                  >
                    Continue here
                  </button>
                }
              }
              @case ('fork') {
                @if (session(); as s) {
                  <button
                    class="btn small primary"
                    type="button"
                    [disabled]="busy()"
                    (click)="continueHere(s, true)"
                  >
                    Continue here (fork)
                  </button>
                }
              }
              @case ('open-terminal') {
                <button class="btn small" type="button" (click)="terminal()">
                  <mc-icon name="terminal" [size]="14" /> Open terminal view
                </button>
              }
              @case ('open-on-desktop') {
                @if (vscodeUrl(); as url) {
                  <a class="btn small" [href]="url"
                    ><mc-icon name="external" [size]="14" /> Open in VS Code</a
                  >
                } @else {
                  <span class="muted small">Answer it on the Mac.</span>
                }
              }
            }
          </div>
        </li>
      } @empty {
        <li class="none"><mc-icon name="check" [size]="15" /> No limits: everything works.</li>
      }
    </ul>
  `,
  styles: [
    `
      .limits {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.55rem;
      }
      li {
        display: flex;
        gap: 0.5rem;
        align-items: flex-start;
        font-size: 0.875rem;
        line-height: 1.45;
      }
      li > mc-icon {
        margin-top: 0.15rem;
        color: var(--muted);
      }
      li.none mc-icon {
        color: var(--green);
      }
      .ltext {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 0.35rem;
      }
    `,
  ],
})
export class CapLimitsComponent {
  private auth = inject(AuthService);
  private actions = inject(SessionActionsService);

  readonly session = input<SessionSummary | undefined>(undefined);
  /** Show these instead of the session's own limits. */
  readonly limits = input<CapLimit[] | undefined>(undefined);
  /** Only limits whose action is one of these (e.g. just the fixes). */
  readonly only = input<LimitAction[] | undefined>(undefined);
  /** A parent shows the terminal in place. */
  readonly openTerminal = output<void>();
  /** An action ran (parents close their dialog). */
  readonly done = output<void>();

  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly list = computed(() => {
    const all = this.limits() ?? capLimits(this.session()?.control);
    const only = this.only();
    return only ? all.filter((l) => l.action && only.includes(l.action)) : all;
  });
  protected readonly busy = computed(() => {
    const s = this.session();
    return !!s && this.actions.busy().has(s.id);
  });
  protected readonly vscodeUrl = computed(() => {
    const s = this.session();
    return s?.surface === 'vscode' && this.isDesktop()
      ? `vscode://anthropic.claude-code/open?session=${encodeURIComponent(s.id)}`
      : '';
  });

  protected async continueHere(s: SessionSummary, fork: boolean): Promise<void> {
    this.done.emit();
    await this.actions.continueHere(s, fork ? { fork: true } : {});
  }

  protected terminal(): void {
    const s = this.session();
    this.done.emit();
    this.openTerminal.emit();
    if (s) this.actions.openTerminal(s.id);
  }
}
