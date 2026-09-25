import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  inject,
  input,
  signal,
} from '@angular/core';

import { ApiService } from '../core/api.service';

/**
 * An image from a transcript (`UiEvent.image.ref`). Media needs the bearer
 * token, so it's fetched with auth when scrolled into view and shown from a
 * blob: URL (revoked on destroy).
 */
@Component({
  selector: 'mc-media-image',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (src(); as s) {
      <img [src]="s" [alt]="alt()" loading="lazy" />
    } @else if (failed()) {
      <span class="muted small">Image unavailable</span>
    } @else {
      <span class="ph" aria-hidden="true"></span>
    }
  `,
  styles: [
    `
      :host {
        display: inline-block;
        max-width: 100%;
      }
      img {
        display: block;
        max-width: min(100%, 22rem);
        max-height: 18rem;
        border-radius: var(--radius-sm);
        border: 1px solid var(--border);
      }
      .ph {
        display: block;
        width: 8rem;
        height: 5rem;
        border-radius: var(--radius-sm);
        background: var(--panel-2);
      }
    `,
  ],
})
export class MediaImageComponent {
  private api = inject(ApiService);
  readonly ref = input.required<string>();
  readonly alt = input('Image from the conversation');
  protected readonly src = signal<string | null>(null);
  protected readonly failed = signal(false);
  private url: string | null = null;
  private destroyed = false;
  private readonly abort = new AbortController();

  constructor() {
    const host = inject(ElementRef<HTMLElement>).nativeElement as HTMLElement;
    const load = () => void this.load();
    let io: IntersectionObserver | null = null;
    if ('IntersectionObserver' in window) {
      io = new IntersectionObserver((entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          io?.disconnect();
          load();
        }
      });
      io.observe(host);
    } else {
      queueMicrotask(load);
    }
    inject(DestroyRef).onDestroy(() => {
      this.destroyed = true;
      io?.disconnect();
      this.abort.abort();
      if (this.url) URL.revokeObjectURL(this.url);
    });
  }

  private async load(): Promise<void> {
    try {
      const blob = await this.api.fetchMedia(this.ref(), this.abort.signal);
      // Gone while it downloaded: a URL made now would never be revoked.
      if (this.destroyed) return;
      this.url = URL.createObjectURL(blob);
      this.src.set(this.url);
    } catch {
      if (!this.destroyed) this.failed.set(true);
    }
  }
}
