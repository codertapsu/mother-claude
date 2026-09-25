import { Component, computed, inject, signal, viewChild } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { ImageAttachment, ProjectInfo } from '../../core/models';
import { IconComponent } from '../../shared/icon.component';
import { MAX_IMAGES, prepareImage } from '../../shared/images';
import { LaunchOptionsComponent } from '../../shared/launch-options.component';
import { RelativeTimePipe } from '../../shared/time.pipes';
import { shortPath } from '../../shared/util';

interface Attachment extends ImageAttachment {
  name: string;
  /** data: URL for the thumbnail. */
  preview: string;
}

/**
 * Start an owned session: pick a project (recent projects from the server;
 * the desktop may also type any path), model / effort / permission mode, a
 * first prompt and optional images.
 */
@Component({
  selector: 'mc-new-session',
  imports: [FormsModule, IconComponent, LaunchOptionsComponent, RelativeTimePipe],
  template: `
    <div class="page narrow">
      <div class="page-head"><h1>New session</h1></div>
      <p class="lede">
        Sessions started here are <strong>owned</strong> by Mother Claude: full control from this
        device and your phone — approvals, questions, interrupts and model switches.
      </p>

      <section class="card">
        <h2 class="first">Project</h2>
        @if (projects().length > 6) {
          <input
            type="search"
            class="filter"
            placeholder="Filter projects"
            aria-label="Filter projects"
            [ngModel]="projectQuery()"
            (ngModelChange)="projectQuery.set($event)"
          />
        }
        <div class="projects" role="radiogroup" aria-label="Project">
          @for (p of shownProjects(); track p.path) {
            <button
              type="button"
              role="radio"
              class="project"
              [class.on]="cwd() === p.path"
              [attr.aria-checked]="cwd() === p.path"
              (click)="cwd.set(p.path)"
            >
              <mc-icon name="folder" [size]="16" />
              <span class="pmain">
                <span class="pname">{{ p.name }}</span>
                <span class="ppath">{{ short(p.path) }}</span>
              </span>
              @if (p.lastActivityAt) {
                <span class="pwhen">{{ p.lastActivityAt | relTime }}</span>
              }
            </button>
          } @empty {
            <p class="muted small">
              {{ loadingProjects() ? 'Loading projects…' : 'No recent projects found.' }}
            </p>
          }
        </div>
        @if (isDesktop()) {
          <label class="field path">
            <span class="field-label">Or any folder on this computer</span>
            <input
              [ngModel]="cwd()"
              (ngModelChange)="cwd.set($event)"
              placeholder="/Users/you/project"
              spellcheck="false"
              autocapitalize="off"
            />
          </label>
        }
      </section>

      <section class="card">
        <h2 class="first">Options</h2>
        <mc-launch-options />
      </section>

      <section class="card">
        <label class="field">
          <span class="field-label">First prompt</span>
          <textarea
            rows="5"
            [ngModel]="prompt()"
            (ngModelChange)="prompt.set($event)"
            placeholder="Describe what Claude should do… (optional)"
          ></textarea>
        </label>

        <div class="attach">
          <label class="btn">
            <mc-icon name="image" [size]="16" /> Add images
            <input
              type="file"
              accept="image/*"
              multiple
              class="visually-hidden"
              (change)="onFiles($event)"
            />
          </label>
          <span class="muted small">PNG, JPEG, GIF or WebP · up to 10 · 5 MB each</span>
        </div>
        @if (images().length) {
          <div class="thumbs">
            @for (img of images(); track $index) {
              <figure class="thumb">
                <img [src]="img.preview" [alt]="img.name" />
                <button
                  type="button"
                  class="remove"
                  [attr.aria-label]="'Remove ' + img.name"
                  (click)="removeImage($index)"
                >
                  <mc-icon name="x" [size]="14" />
                </button>
              </figure>
            }
          </div>
        }
      </section>

      @if (error()) {
        <p class="notice error" role="alert">{{ error() }}</p>
      }

      <div class="actions">
        <button
          class="btn primary"
          type="button"
          [disabled]="starting() || !cwd().trim()"
          (click)="start()"
        >
          <mc-icon name="send" [size]="16" /> {{ starting() ? 'Starting…' : 'Start session' }}
        </button>
        @if (!cwd().trim()) {
          <span class="muted small">Pick a project first.</span>
        }
      </div>
    </div>
  `,
  styles: [
    `
      section.card {
        margin-bottom: 0.75rem;
      }
      h2.first {
        margin-top: 0;
      }
      .filter {
        margin-bottom: 0.5rem;
      }
      .projects {
        display: grid;
        gap: 0.4rem;
        max-height: 22rem;
        overflow-y: auto;
      }
      .project {
        appearance: none;
        display: flex;
        align-items: center;
        gap: 0.65rem;
        width: 100%;
        min-height: var(--tap);
        padding: 0.5rem 0.7rem;
        text-align: left;
        font: inherit;
        color: var(--text);
        background: var(--panel-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        cursor: pointer;
      }
      .project mc-icon {
        color: var(--faint);
      }
      .project.on {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .project.on mc-icon {
        color: var(--accent-text);
      }
      .pmain {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
      }
      .pname {
        font-weight: 500;
      }
      .ppath {
        font-family: var(--mono);
        font-size: 0.75rem;
        color: var(--muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .pwhen {
        font-size: 0.75rem;
        color: var(--faint);
        white-space: nowrap;
      }
      .path {
        margin-top: 0.75rem;
      }
      .attach {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        flex-wrap: wrap;
        margin-top: 0.75rem;
      }
      .thumbs {
        display: flex;
        gap: 0.5rem;
        flex-wrap: wrap;
        margin-top: 0.75rem;
      }
      .thumb {
        position: relative;
        margin: 0;
        width: 4.5rem;
        height: 4.5rem;
        border-radius: var(--radius-sm);
        overflow: hidden;
        border: 1px solid var(--border);
      }
      .thumb img {
        width: 100%;
        height: 100%;
        object-fit: cover;
        display: block;
      }
      .remove {
        position: absolute;
        top: 0.2rem;
        right: 0.2rem;
        width: 1.6rem;
        height: 1.6rem;
        border-radius: 50%;
        border: 0;
        background: var(--scrim);
        color: #fff;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
      }
      .actions {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        flex-wrap: wrap;
        margin: 1rem 0 2rem;
      }
      @media (max-width: 767px) {
        .actions .btn {
          width: 100%;
        }
      }
    `,
  ],
})
export class NewSessionComponent {
  private api = inject(ApiService);
  private auth = inject(AuthService);
  private router = inject(Router);
  private readonly options = viewChild.required(LaunchOptionsComponent);

  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly projects = signal<ProjectInfo[]>([]);
  protected readonly loadingProjects = signal(true);
  protected readonly projectQuery = signal('');
  protected readonly cwd = signal('');
  protected readonly prompt = signal('');
  protected readonly images = signal<Attachment[]>([]);
  protected readonly starting = signal(false);
  protected readonly error = signal('');

  protected readonly shownProjects = computed(() => {
    const q = this.projectQuery().trim().toLowerCase();
    const list = this.projects();
    return q
      ? list.filter((p) => p.name.toLowerCase().includes(q) || p.path.toLowerCase().includes(q))
      : list;
  });

  protected readonly short = shortPath;

  constructor() {
    void this.loadProjects();
  }

  protected async onFiles(event: Event): Promise<void> {
    const input = event.target as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    this.error.set('');
    for (const file of files) {
      if (this.images().length >= MAX_IMAGES) {
        this.error.set(`At most ${MAX_IMAGES} images.`);
        break;
      }
      try {
        // Same as the composer: phone photos are downscaled (≤ 2048 px) and
        // re-encoded as needed, instead of refused or sent full size.
        const img = await prepareImage(file);
        this.images.update((prev) => [
          ...prev,
          {
            name: file.name,
            mediaType: img.mediaType,
            data: img.data,
            preview: `data:${img.mediaType};base64,${img.data}`,
          },
        ]);
      } catch (e) {
        this.error.set(`${file.name}: ${errorMessage(e)}`);
      }
    }
  }

  protected removeImage(i: number): void {
    this.images.update((prev) => prev.filter((_, j) => j !== i));
  }

  protected async start(): Promise<void> {
    const cwd = this.cwd().trim();
    if (!cwd || this.starting()) return;
    const opts = this.options();
    if (opts.invalid()) {
      this.error.set('Enter a custom model id, or pick a model from the list.');
      return;
    }
    this.starting.set(true);
    this.error.set('');
    try {
      const prompt = this.prompt().trim();
      const images = this.images().map(({ mediaType, data }) => ({ mediaType, data }));
      const res = await this.api.spawnSession({
        cwd,
        ...(prompt ? { prompt } : {}),
        ...opts.value(),
        ...(images.length ? { images } : {}),
      });
      if (res?.id) void this.router.navigate(['/session', res.id]);
      else void this.router.navigate(['/']);
    } catch (e) {
      this.error.set(`Could not start the session: ${errorMessage(e)}`);
    } finally {
      this.starting.set(false);
    }
  }

  private async loadProjects(): Promise<void> {
    try {
      const { projects } = await this.api.getProjects();
      const sorted = [...(projects ?? [])].sort(
        (a, b) => (b.lastActivityAt ?? 0) - (a.lastActivityAt ?? 0),
      );
      this.projects.set(sorted);
      if (!this.cwd() && sorted.length === 1) this.cwd.set(sorted[0].path);
    } catch (e) {
      this.error.set(`Could not load projects: ${errorMessage(e)}`);
    } finally {
      this.loadingProjects.set(false);
    }
  }
}
