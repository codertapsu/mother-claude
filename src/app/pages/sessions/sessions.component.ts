import { NgTemplateOutlet } from '@angular/common';
import { Component, computed, inject, signal, viewChild } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink } from '@angular/router';

import { ApiService } from '../../core/api.service';
import { RealtimeService } from '../../core/realtime.service';
import { LaunchOptionsComponent } from '../../shared/launch-options.component';
import { relativeTime, formatTokens, stateLabel, surfaceLabel } from '../../shared/util';

@Component({
  selector: 'mc-sessions',
  imports: [RouterLink, FormsModule, NgTemplateOutlet, LaunchOptionsComponent],
  templateUrl: './sessions.component.html',
  styleUrl: './sessions.component.scss',
})
export class SessionsComponent {
  private realtime = inject(RealtimeService);
  private api = inject(ApiService);
  private router = inject(Router);

  protected readonly sessions = this.realtime.sessions;

  protected readonly needsInput = computed(() =>
    this.sessions().filter((s) => s.state === 'needs-input'),
  );
  protected readonly working = computed(() => this.sessions().filter((s) => s.state === 'working'));
  protected readonly other = computed(() =>
    this.sessions().filter((s) => s.state !== 'needs-input' && s.state !== 'working'),
  );

  protected readonly showSpawn = signal(false);
  private readonly launchOpts = viewChild(LaunchOptionsComponent);
  protected readonly spawnCwd = signal('');
  protected readonly spawnPrompt = signal('');
  protected readonly spawning = signal(false);
  protected readonly error = signal('');

  /** Count of still-running background tasks (only meaningful while live). */
  runningTasks(s: { running: boolean; tasks?: { status: string }[] }): number {
    if (!s.running || !s.tasks) return 0;
    return s.tasks.filter((t) => t.status === 'running').length;
  }

  protected readonly rel = relativeTime;
  protected readonly tokens = formatTokens;
  protected readonly label = stateLabel;
  protected readonly surface = surfaceLabel;

  async spawn(): Promise<void> {
    this.error.set('');
    if (!this.spawnCwd().trim() || !this.spawnPrompt().trim()) {
      this.error.set('Working directory and prompt are required.');
      return;
    }
    if (this.launchOpts()?.invalid()) {
      this.error.set('Enter a custom model id, or pick a model from the list.');
      return;
    }
    this.spawning.set(true);
    try {
      const res = await this.api.spawnSession({
        cwd: this.spawnCwd().trim(),
        prompt: this.spawnPrompt().trim(),
        ...(this.launchOpts()?.value() ?? {}),
      });
      this.showSpawn.set(false);
      this.spawnPrompt.set('');
      if (res?.id) this.router.navigate(['/session', res.id]);
    } catch (e) {
      this.error.set(`Spawn failed: ${(e as Error).message}`);
    } finally {
      this.spawning.set(false);
    }
  }
}
