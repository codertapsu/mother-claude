import { Component, OnInit, computed, inject } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';

import { ConfigService } from './core/config.service';
import { RealtimeService } from './core/realtime.service';

@Component({
  selector: 'mc-root',
  imports: [RouterOutlet, RouterLink, RouterLinkActive],
  templateUrl: './app.component.html',
  styleUrl: './app.component.scss',
})
export class AppComponent implements OnInit {
  private realtime = inject(RealtimeService);
  private config = inject(ConfigService);

  protected readonly connection = this.realtime.connection;
  protected readonly needsToken = computed(() => !this.config.config()?.token);

  async ngOnInit(): Promise<void> {
    await this.config.ensure();
    await this.realtime.start();
  }
}
