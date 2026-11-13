import { Component } from '@angular/core';
import { RouterLink } from '@angular/router';

import { TPipe } from '../../core/i18n/t.pipe';
import { IconComponent } from '../../shared/icon.component';
import { PageHeadComponent } from '../../shared/page-head.component';
import { thisComputerTitle } from '../machines/machines-text';
import { McpPanelComponent } from '../services/mcp-panel.component';

/**
 * *Settings › Claude Code tools* (docs/UI-REDESIGN.md §6.7, docs/MCP.md):
 * Mother Claude's own tools for every Claude Code session on this
 * computer — the switch (the browser extension and the team tools need
 * it), the tool count, the sessions connected and the last calls. What
 * Claude Code itself runs besides sessions is Computers › This Mac ›
 * Details.
 */
@Component({
  selector: 'mc-settings-tools',
  imports: [IconComponent, McpPanelComponent, PageHeadComponent, RouterLink, TPipe],
  template: `
    <div class="page narrow">
      <a class="btn ghost small back" routerLink="/settings">
        <mc-icon name="back" [size]="15" /> {{ 'Settings' | t }}
      </a>
      <mc-page-head
        [heading]="'Claude Code tools' | t"
        [lede]="
          'Mother Claude’s own tools for the Claude Code sessions on this computer: the browser extension and the team tools need them.'
            | t
        "
        help="claude-code-tools"
      />
      <mc-mcp-panel />
      <p class="muted small more">
        {{ 'Claude Code’s background service, its MCP servers and background jobs:' | t }}
        <a routerLink="/computers">{{
          'Computers › {computer} › Details' | t: { computer: thisTitle() }
        }}</a>
      </p>
    </div>
  `,
  styles: [
    `
      .back {
        margin-bottom: 0.4rem;
      }
      .more {
        margin-top: 0.75rem;
      }
    `,
  ],
})
export class ToolsPageComponent {
  protected readonly thisTitle = thisComputerTitle;
}
