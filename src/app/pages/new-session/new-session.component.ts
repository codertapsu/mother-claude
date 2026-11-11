import {
  Component,
  computed,
  effect,
  inject,
  linkedSignal,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, Router } from '@angular/router';

import { accountCanRun } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiError, ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachineScopes } from '../../core/machine-scope';
import { grantedByPeer } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { AccountView, ImageAttachment, ProjectInfo } from '../../core/models';
import { HostOs, folderExample } from '../../core/paths';
import { sessionLink } from '../../core/session-links';
import { accountErrorText, cannotRunReason } from '../../shared/accounts';
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

/** Another computer a session can start on. */
interface Where {
  id: string;
  name: string;
  /** Its name, and why it can't be picked when it can't. */
  label: string;
  os: HostOs;
  /** Online, and it lets this computer drive its sessions. */
  ready: boolean;
}

/**
 * Start an owned session: pick a project (recent projects from the server;
 * the desktop may also type any path), the Claude account (while there are
 * several: the project's, else the default for new sessions, unless picked
 * here), model / effort / permission mode, a first prompt and optional
 * images.
 *
 * While other computers are paired (and Machines is on), *Where* picks the
 * computer it starts on (docs/PLAN-MACHINES.md §6.4; `?machine=<id>`
 * preselects one): its projects come from it, and the session starts there
 * under its own account for the folder — through this computer's proxy.
 */
@Component({
  selector: 'mc-new-session',
  imports: [FormsModule, IconComponent, LaunchOptionsComponent, RelativeTimePipe, TPipe],
  template: `
    <div class="page narrow">
      <div class="page-head">
        <h1>{{ 'New session' | t }}</h1>
      </div>
      <p class="lede">
        {{ 'Sessions started here are' | t }} <strong>{{ 'owned||by Mother Claude' | t }}</strong>
        {{
          'by Mother Claude: full control from this device and your phone — approvals, questions, interrupts and model switches.'
            | t
        }}
      </p>

      @if (wheres().length) {
        <section class="card">
          <h2 class="first">{{ 'Where' | t }}</h2>
          <div class="wheres" role="radiogroup" [attr.aria-label]="'Where' | t">
            <button
              type="button"
              role="radio"
              class="where"
              [class.on]="!where()"
              [attr.aria-checked]="!where()"
              (click)="pickWhere('')"
            >
              <mc-icon name="monitor" [size]="16" />
              <span>{{ 'This {computer}' | t: { computer: computer() } }}</span>
            </button>
            @for (w of wheres(); track w.id) {
              <button
                type="button"
                role="radio"
                class="where"
                [class.on]="where() === w.id"
                [attr.aria-checked]="where() === w.id"
                [disabled]="!w.ready"
                (click)="pickWhere(w.id)"
              >
                <mc-icon name="monitor" [size]="16" />
                <span>{{ w.label }}</span>
              </button>
            }
          </div>
          @if (target(); as t) {
            <p class="field-hint">
              {{
                'It starts on {name}, under its account for the folder, and you drive it from here.'
                  | t: { name: t.name }
              }}
            </p>
          }
        </section>
      }

      <section class="card">
        <h2 class="first">{{ 'Project' | t }}</h2>
        @if (projects().length > 6) {
          <input
            type="search"
            class="filter"
            [placeholder]="'Filter projects' | t"
            [attr.aria-label]="'Filter projects' | t"
            [ngModel]="projectQuery()"
            (ngModelChange)="projectQuery.set($event)"
          />
        }
        <div class="projects" role="radiogroup" [attr.aria-label]="'Project' | t">
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
              {{
                loadingProjects() ? ('Loading projects…' | t) : ('No recent projects found.' | t)
              }}
            </p>
          }
        </div>
        @if (isDesktop()) {
          <label class="field path">
            @if (target(); as t) {
              <span class="field-label">{{ 'Or any folder on {name}' | t: { name: t.name } }}</span>
            } @else {
              <span class="field-label">{{ 'Or any folder on this computer' | t }}</span>
            }
            <input
              [ngModel]="cwd()"
              (ngModelChange)="cwd.set($event)"
              [placeholder]="cwdPlaceholder()"
              spellcheck="false"
              autocapitalize="off"
            />
          </label>
        }
      </section>

      <!-- This computer's accounts: another computer runs it under its own. -->
      @if (accounts.multi() && !target()) {
        <section class="card">
          <h2 class="first">{{ 'Account' | t }}</h2>
          <label class="field">
            <span class="field-label">{{ 'Runs under' | t }}</span>
            <select [ngModel]="account()" (ngModelChange)="account.set($event)">
              @for (a of accounts.list(); track a.id) {
                <option [value]="a.id" [disabled]="!canRun(a)">{{ accountOption(a) }}</option>
              }
            </select>
          </label>
          <p class="field-hint">{{ accountHint() }}</p>
          @if (accountWarning(); as w) {
            <p class="notice warn small">{{ w }}</p>
          }
        </section>
      }

      <section class="card">
        <h2 class="first">{{ 'Options' | t }}</h2>
        <mc-launch-options />
      </section>

      <section class="card">
        <label class="field">
          <span class="field-label">{{ 'First prompt' | t }}</span>
          <textarea
            rows="5"
            [ngModel]="prompt()"
            (ngModelChange)="prompt.set($event)"
            [placeholder]="'Describe what Claude should do… (optional)' | t"
          ></textarea>
        </label>

        <div class="attach">
          <label class="btn">
            <mc-icon name="image" [size]="16" /> {{ 'Add images' | t }}
            <input
              type="file"
              accept="image/*"
              multiple
              class="visually-hidden"
              (change)="onFiles($event)"
            />
          </label>
          <span class="muted small">{{ 'PNG, JPEG, GIF or WebP · up to 10 · 5 MB each' | t }}</span>
        </div>
        @if (images().length) {
          <div class="thumbs">
            @for (img of images(); track $index) {
              <figure class="thumb">
                <img [src]="img.preview" [alt]="img.name" />
                <button
                  type="button"
                  class="remove"
                  [attr.aria-label]="'Remove {name}' | t: { name: img.name }"
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
          <mc-icon name="send" [size]="16" />
          @if (starting()) {
            {{ 'Starting…' | t }}
          } @else if (target(); as t) {
            {{ 'Start on {name}' | t: { name: t.name } }}
          } @else {
            {{ 'Start session' | t }}
          }
        </button>
        @if (!cwd().trim()) {
          <span class="muted small">{{ 'Pick a project first.' | t }}</span>
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
      .wheres {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
      }
      .where {
        appearance: none;
        display: inline-flex;
        align-items: center;
        gap: 0.45rem;
        min-height: var(--tap);
        padding: 0.4rem 0.75rem;
        font: inherit;
        color: var(--text);
        background: var(--panel-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        cursor: pointer;
      }
      .where mc-icon {
        color: var(--faint);
      }
      .where.on {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .where.on mc-icon {
        color: var(--accent-text);
      }
      .where:disabled {
        cursor: default;
        opacity: 0.6;
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
      /* A 44 px hit area from the thumbnail's corner (it clips overflow). */
      .remove::before {
        content: '';
        position: absolute;
        top: -0.2rem;
        right: -0.2rem;
        width: var(--tap);
        height: var(--tap);
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
  private machines = inject(MachinesService);
  private scopes = inject(MachineScopes);
  protected readonly accounts = inject(AccountsService);
  private readonly options = viewChild.required(LaunchOptionsComponent);

  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly computer = computerNoun;
  /** Other computers it can start on: the paired ones, while Machines is on. */
  protected readonly wheres = computed<Where[]>(() => {
    const v = this.machines.view();
    if (!v?.enabled) return [];
    return v.peers.map((p) => {
      const drive = grantedByPeer(p, 'control') !== false;
      const os: HostOs = p.os === 'macos' || p.os === 'windows' ? p.os : 'linux';
      return {
        id: p.id,
        name: p.name,
        label: !p.online
          ? tr('{name} (offline)', { name: p.name })
          : drive
            ? p.name
            : tr('{name} (watch only)', { name: p.name }),
        os,
        ready: p.online && drive,
      };
    });
  });
  /** The computer picked ('' = this one). */
  protected readonly where = signal(
    inject(ActivatedRoute).snapshot.queryParamMap.get('machine') ?? '',
  );
  /** The other computer it starts on, when one is picked and can. */
  protected readonly target = computed(
    () => this.wheres().find((w) => w.id === this.where() && w.ready) ?? null,
  );
  /** An example folder in the desktop's own form (the other computer's). */
  protected readonly cwdPlaceholder = computed(() =>
    folderExample(this.target()?.os ?? this.auth.hostOs(), 'project'),
  );
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

  /** Who the session runs under unless picked here: the project's
   * account, else the default for new sessions (as the server decides). */
  private readonly autoAccount = computed(() => this.accounts.accountFor(this.cwd()));
  private readonly autoAccountId = computed(() => this.autoAccount().id);
  /** The account picked; follows the project until one is chosen here (and
   * again once another project brings another account). */
  protected readonly account = linkedSignal(() => this.autoAccountId());
  protected readonly accountHint = computed(() => {
    const auto = this.autoAccount();
    const a = this.accounts.byId(this.account());
    const why =
      this.account() !== auto.id
        ? tr('Chosen here, for this session.')
        : auto.via === 'project'
          ? tr('The account of {folder}, set on the Accounts page.', {
              folder: shortPath(auto.dir),
            })
          : tr('The default for new sessions.');
    if (!a) return why;
    const computer = computerNoun();
    const what =
      a.kind === 'token'
        ? tr("It shares this {computer}'s projects and conversations.", { computer })
        : a.isDefault
          ? tr("It uses this {computer}'s own Claude Code setup.", { computer })
          : a.dir
            ? tr('It uses its own Claude Code setup in {dir}.', { dir: shortPath(a.dir) })
            : tr('It uses its own Claude Code setup.');
    return `${why} ${what}`;
  });
  protected readonly accountWarning = computed(() => {
    const a = this.accounts.byId(this.account());
    if (!a) return '';
    const reason = cannotRunReason(a);
    if (reason) return `${a.name}: ${reason}. ${this.fixWhere()}`;
    if (a.attention === 'authFailed') {
      return tr('A session of {name} could not sign in lately: it may need signing in again.', {
        name: a.name,
      });
    }
    return '';
  });

  constructor() {
    let shownFor = this.target()?.id ?? '';
    void this.loadProjects(shownFor);
    // Another computer picked (or this one again): its projects.
    effect(() => {
      const id = this.target()?.id ?? '';
      if (id === shownFor) return;
      shownFor = id;
      untracked(() => {
        this.cwd.set('');
        this.projectQuery.set('');
        this.projects.set([]);
        this.error.set('');
        void this.loadProjects(id);
      });
    });
    // A picker needs the accounts: with only the Default nothing sent them.
    void this.accounts.ensureLoaded();
  }

  protected pickWhere(id: string): void {
    this.where.set(id);
  }

  protected canRun(a: AccountView): boolean {
    return accountCanRun(a);
  }

  protected accountOption(a: AccountView): string {
    const reason = cannotRunReason(a);
    const who = a.email ? ` — ${a.email}` : '';
    return reason ? `${a.name} (${reason})` : `${a.name}${who}`;
  }

  private fixWhere(): string {
    return this.isDesktop()
      ? tr('Fix it on the Accounts page (Settings › Accounts).')
      : tr('Fix it on the Accounts page of the desktop app.');
  }

  protected async onFiles(event: Event): Promise<void> {
    const input = event.target as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    this.error.set('');
    for (const file of files) {
      if (this.images().length >= MAX_IMAGES) {
        this.error.set(tr('At most {n} images.', { n: MAX_IMAGES }));
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
      this.error.set(tr('Enter a custom model id, or pick a model from the list.'));
      return;
    }
    this.starting.set(true);
    this.error.set('');
    // Another computer: through its scope, under its own account.
    const machine = this.target()?.id ?? '';
    const api = machine ? this.scopes.api(machine) : this.api;
    // Only while several accounts exist: the request is unchanged otherwise.
    const account = this.accounts.multi() && !machine ? this.account() : '';
    try {
      const prompt = this.prompt().trim();
      const images = this.images().map(({ mediaType, data }) => ({ mediaType, data }));
      const res = await api.spawnSession({
        cwd,
        ...(prompt ? { prompt } : {}),
        ...opts.value(),
        ...(images.length ? { images } : {}),
        ...(account ? { account } : {}),
      });
      if (res?.id) void this.router.navigate(sessionLink(res.id, machine));
      else void this.router.navigate(['/']);
    } catch (e) {
      const code = e instanceof ApiError ? (e.code ?? '') : '';
      const accountRefused = [
        'account_token_missing',
        'unknown_account',
        'account_needs_copy',
      ].includes(code);
      if (code === 'unknown_account') void this.accounts.refresh();
      this.error.set(
        tr('Could not start the session: {error}', {
          error: accountRefused
            ? accountErrorText(e, this.accounts.byId(account)?.name)
            : errorMessage(e),
        }),
      );
    } finally {
      this.starting.set(false);
    }
  }

  /** The projects of this computer (`machine` ''), or another's. */
  private async loadProjects(machine: string): Promise<void> {
    const current = () => (this.target()?.id ?? '') === machine;
    this.loadingProjects.set(true);
    try {
      const api = machine ? this.scopes.api(machine) : this.api;
      const { projects } = await api.getProjects();
      if (!current()) return;
      const sorted = [...(projects ?? [])].sort(
        (a, b) => (b.lastActivityAt ?? 0) - (a.lastActivityAt ?? 0),
      );
      this.projects.set(sorted);
      if (!this.cwd() && sorted.length === 1) this.cwd.set(sorted[0].path);
    } catch (e) {
      if (current()) {
        this.error.set(tr('Could not load projects: {error}', { error: errorMessage(e) }));
      }
    } finally {
      if (current()) this.loadingProjects.set(false);
    }
  }
}
