import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { NgTemplateOutlet } from '@angular/common';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  AccountCell,
  AccountRow,
  AccountsTable,
  FoldersView,
  LoneRepo,
  MachineAsk,
  PeerFolders,
  RootEntry,
  RootMachine,
  RootProposal,
  RootsView,
  askOpen,
} from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { RealtimeService } from '../../core/realtime.service';
import { IconComponent } from '../../shared/icon.component';
import { askText, serverText, thisComputerTitle } from './machines-text';

/** Frames after which step 5 reads again. */
const REFRESH_ON = new Set(['setup', 'asks', 'roots', 'folders', 'peers', 'accounts']);

/**
 * Step 5 of the setup, *Accounts and folders* (docs/PLAN-MACHINES.md §5.14,
 * spikes/machines/UX.md §3.5): each account matched by name on every
 * computer with its sign-in there (*Sign in on DESKTOP-PC…*, *Add it on
 * DESKTOP-PC…* — asked there, done when someone there confirms); the roots
 * (one folder per computer), with the ones the computers' git remotes
 * propose; and the repositories one computer lacks (*Clone it on …*,
 * *Clone it here*).
 */
@Component({
  selector: 'mc-machines-accounts-folders',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, NgTemplateOutlet, RouterLink, IconComponent, TPipe],
  template: `
    @if (error()) {
      <p class="notice error" role="alert">{{ error() }}</p>
    }
    @if (notice()) {
      <p class="notice ok" role="status">{{ notice() }}</p>
    }

    <!-- ─────────────── Accounts ─────────────── -->
    <section class="block" aria-labelledby="mc-accounts-h">
      <div class="row">
        <h3 id="mc-accounts-h">{{ 'Accounts' | t }}</h3>
        <span class="spacer"></span>
        <button type="button" class="btn small" [disabled]="loading()" (click)="load(true)">
          <mc-icon name="refresh" [size]="14" />
          {{ loading() ? ('Checking…' | t) : ('Re-check' | t) }}
        </button>
      </div>
      <p class="muted small">
        {{ 'A session moves to the account of the same name on the other computer.' | t }}
      </p>
      @if (accounts(); as a) {
        <table class="grid accounts">
          <caption class="visually-hidden">
            {{
              'Accounts on each computer' | t
            }}
          </caption>
          <thead>
            <tr>
              <th scope="col">{{ 'Account' | t }}</th>
              @for (m of a.machines; track m.id) {
                <th scope="col">{{ m.here ? thisTitle() : m.name }}</th>
              }
            </tr>
          </thead>
          <tbody>
            @for (row of a.accounts; track row.name) {
              <tr>
                <th scope="row">{{ row.name }}</th>
                @for (m of a.machines; track m.id) {
                  @let cell = cellOf(row, m.id);
                  <td [attr.data-state]="cell?.state ?? 'unknown'">
                    <span class="mark" aria-hidden="true">{{ mark(cell) }}</span>
                    <span>{{ cellText(cell) }}</span>
                    @if (cell?.name) {
                      <span class="muted small">{{ '(as {name})' | t: { name: cell?.name } }}</span>
                    }
                    @if (m.here) {
                      @if (needsSignIn(cell)) {
                        <a class="btn small" routerLink="/accounts">{{ 'Sign in…' | t }}</a>
                      }
                    } @else {
                      @let asked = askOf(row, m.id);
                      @if (open(asked)) {
                        <span class="muted small asked">{{ askedText(asked, m.name) }}</span>
                      } @else if (needsSignIn(cell)) {
                        <button
                          type="button"
                          class="btn small"
                          [disabled]="busy() === m.id + ':' + row.name"
                          (click)="askSignIn(m.id, m.name, row)"
                        >
                          {{ 'Sign in on {name}…' | t: { name: m.name } }}
                        </button>
                      } @else if (cell?.state === 'missing') {
                        <button
                          type="button"
                          class="btn small"
                          [disabled]="busy() === m.id + ':' + row.name"
                          (click)="askAdd(m.id, m.name, row)"
                        >
                          {{ 'Add it on {name}…' | t: { name: m.name } }}
                        </button>
                      }
                      @if (asked && !open(asked)) {
                        <span class="muted small">{{ askedText(asked, m.name) }}</span>
                      }
                    }
                  </td>
                }
              </tr>
            }
          </tbody>
        </table>
        @for (m of a.machines; track m.id) {
          @if (m.error) {
            <p class="notice warn small">{{ text(m.error) }}</p>
          }
        }
        @if (signInsAsked()) {
          <p class="muted small">
            {{
              'A sign-in asked of another computer opens its browser there: someone there finishes it.'
                | t
            }}
          </p>
        }
      } @else if (loading()) {
        <p class="muted small" role="status">{{ 'Looking at every computer’s accounts…' | t }}</p>
      }
    </section>

    <!-- ─────────────── Roots ─────────────── -->
    <section class="block" aria-labelledby="mc-roots-h">
      <h3 id="mc-roots-h">{{ 'Folders' | t }}</h3>
      <p class="muted small">
        {{
          'A shared folder is one folder on each computer: a project under it on one is the same project, at the same place, on the other.'
            | t
        }}
      </p>
      @if (roots(); as r) {
        <ul class="roots">
          @for (root of r.roots; track root.id) {
            <li class="root">
              @if (editing() === root.id) {
                <ng-container *ngTemplateOutlet="editor" />
              } @else {
                <div class="row">
                  <strong>{{ root.name }}</strong>
                  <span class="spacer"></span>
                  <button type="button" class="btn ghost small" (click)="edit(root)">
                    {{ 'Edit' | t }}
                  </button>
                  <button
                    type="button"
                    class="btn ghost small"
                    [class.danger]="confirming() === root.id"
                    [disabled]="busy() === root.id"
                    (click)="removeRoot(root)"
                  >
                    {{ confirming() === root.id ? ('Tap again to remove' | t) : ('Remove' | t) }}
                  </button>
                </div>
                <dl class="paths">
                  @for (m of r.machines; track m.id) {
                    @if (root.paths[m.id]) {
                      <dt>{{ machineLabel(m) }}</dt>
                      <dd class="mono">{{ root.paths[m.id] }}</dd>
                    }
                  }
                </dl>
              }
            </li>
          } @empty {
            <li class="muted small">{{ 'No folder is shared yet.' | t }}</li>
          }
        </ul>
        @if (editing() === 'new') {
          <ng-container *ngTemplateOutlet="editor" />
        } @else {
          <button type="button" class="btn small" (click)="add()">
            <mc-icon name="plus" [size]="14" /> {{ 'Add a folder' | t }}
          </button>
        }
      }
      <ng-template #editor>
        <form class="editor stack" (ngSubmit)="saveRoot()">
          <label class="field">
            <span class="field-label">{{ 'Name' | t }}</span>
            <input
              name="rootName"
              maxlength="40"
              [ngModel]="draftName()"
              (ngModelChange)="draftName.set($event)"
            />
          </label>
          @for (m of editable(); track m.id) {
            <label class="field">
              <span class="field-label">{{
                'Its folder on {name}' | t: { name: machineLabel(m) }
              }}</span>
              <input
                class="mono"
                [name]="'path-' + m.id"
                [placeholder]="placeholder(m)"
                [ngModel]="draftPaths()[m.id] ?? ''"
                (ngModelChange)="setPath(m.id, $event)"
              />
            </label>
          }
          <div class="row">
            <button type="submit" class="btn primary small" [disabled]="busy() === 'root'">
              {{ 'Save' | t }}
            </button>
            <button type="button" class="btn ghost small" (click)="editing.set(null)">
              {{ 'Cancel' | t }}
            </button>
          </div>
        </form>
      </ng-template>
    </section>

    <!-- ─────────────── Matched by their git remote ─────────────── -->
    <section class="block" aria-labelledby="mc-found-h">
      <div class="row">
        <h3 id="mc-found-h">{{ 'Projects on each computer' | t }}</h3>
        <span class="spacer"></span>
        <button
          type="button"
          class="btn small"
          [disabled]="loadingFolders()"
          (click)="loadFolders(true)"
        >
          <mc-icon name="refresh" [size]="14" />
          {{ loadingFolders() ? ('Looking…' | t) : ('Look again' | t) }}
        </button>
      </div>
      @if (folders(); as f) {
        @for (p of f.peers; track p.machine) {
          <div class="peer">
            <h4>{{ 'With {name}' | t: { name: p.name } }}</h4>
            @if (p.error) {
              <p class="notice warn small">{{ text(p.error) }}</p>
            }
            @if (p.proposals.length) {
              <p class="small">{{ 'Found on both computers, by their git remote:' | t }}</p>
              <ul class="list">
                @for (pr of p.proposals; track pr.id) {
                  <li>
                    <div class="row">
                      <strong>{{ pr.name }}</strong>
                      <span class="mono small">{{ pr.here }}</span>
                      <span aria-hidden="true">↔</span>
                      <span class="mono small">{{ pr.there }}</span>
                      <span class="spacer"></span>
                      <button
                        type="button"
                        class="btn primary small"
                        [disabled]="busy() === pr.id"
                        (click)="useProposal(p, pr)"
                      >
                        {{ pr.root ? ('Add to {name}' | t: { name: pr.name }) : ('Use' | t) }}
                      </button>
                    </div>
                    <span class="muted small">{{ reposText(pr) }}</span>
                  </li>
                }
              </ul>
            } @else if (!p.error && !p.matched.length) {
              <p class="muted small">{{ 'No project is on both yet.' | t }}</p>
            }
            @if (p.matched.length) {
              <p class="small">
                {{
                  p.matched.length === 1
                    ? ('1 project on both maps already.' | t)
                    : ('{n} projects on both map already.' | t: { n: p.matched.length })
                }}
              </p>
            }
            @if (p.mismatched.length) {
              <p class="notice warn small">
                {{
                  'On both, but your folders map them elsewhere: {names}.'
                    | t: { names: names(p.mismatched) }
                }}
              </p>
            }
            @if (p.onlyHere.length) {
              <h5>{{ 'Only on {name}' | t: { name: thisTitle() } }}</h5>
              <ul class="list">
                @for (l of p.onlyHere; track l.path) {
                  <li class="row">
                    <strong>{{ l.name }}</strong>
                    <span class="muted small mono">{{ l.origin }}</span>
                    <span class="spacer"></span>
                    @let asked = cloneAsk(p.machine, l);
                    @if (asked) {
                      <span class="muted small">{{ askedText(asked, p.name) }}</span>
                    }
                    @if (!open(asked)) {
                      @if (l.root && l.rel && l.origin) {
                        <button
                          type="button"
                          class="btn small"
                          [disabled]="busy() === l.path"
                          (click)="askClone(p, l)"
                        >
                          {{ 'Clone it on {name}' | t: { name: p.name } }}
                        </button>
                      } @else {
                        <span class="muted small">{{
                          'Share a folder that holds it first.' | t
                        }}</span>
                      }
                    }
                  </li>
                }
              </ul>
            }
            @if (p.onlyThere.length) {
              <h5>{{ 'Only on {name}' | t: { name: p.name } }}</h5>
              <ul class="list">
                @for (l of p.onlyThere; track l.path) {
                  <li class="row">
                    <strong>{{ l.name }}</strong>
                    <span class="muted small mono">{{ l.origin }}</span>
                    <span class="spacer"></span>
                    @if (l.root && l.rel && l.origin) {
                      <button
                        type="button"
                        class="btn small"
                        [disabled]="busy() === l.path"
                        (click)="cloneHere(l)"
                      >
                        {{ busy() === l.path ? ('Cloning…' | t) : ('Clone it here' | t) }}
                      </button>
                    } @else {
                      <span class="muted small">{{
                        'Share a folder that holds it first.' | t
                      }}</span>
                    }
                  </li>
                }
              </ul>
            }
          </div>
        } @empty {
          <p class="muted small">{{ 'Pair a computer first (step 2).' | t }}</p>
        }
        @if (f.noOrigin.length) {
          <p class="muted small">
            {{
              'No git remote, so they can’t move with their code: {names}.'
                | t: { names: originless(f) }
            }}
          </p>
        }
        <p class="muted small">
          {{ 'Your folder pairs from Backup & sync are kept as they are.' | t }}
        </p>
      } @else if (loadingFolders()) {
        <p class="muted small" role="status">{{ 'Looking for projects on every computer…' | t }}</p>
      }
    </section>
  `,
  styles: [
    `
      .block {
        padding: 0.4rem 0 0.8rem;
        border-bottom: 1px solid var(--border);
      }
      .block:last-child {
        border-bottom: 0;
      }
      h3 {
        margin: 0.3rem 0;
        font-size: 0.95rem;
      }
      h4 {
        margin: 0.6rem 0 0.2rem;
        font-size: 0.88rem;
      }
      h5 {
        margin: 0.5rem 0 0.15rem;
        font-size: 0.82rem;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.03em;
      }
      .grid {
        width: 100%;
        border-collapse: collapse;
        margin: 0.4rem 0;
      }
      .grid th,
      .grid td {
        text-align: left;
        padding: 0.35rem 0.4rem;
        border-bottom: 1px solid var(--border);
        vertical-align: top;
      }
      .grid td > * {
        margin-right: 0.35rem;
      }
      .grid td[data-state='signedIn'] .mark {
        color: var(--green);
      }
      .grid td[data-state='signedOut'] .mark,
      .grid td[data-state='authFailed'] .mark,
      .grid td[data-state='tokenMissing'] .mark,
      .grid td[data-state='missing'] .mark {
        color: var(--red);
      }
      .asked {
        display: block;
      }
      .roots,
      .list {
        list-style: none;
        margin: 0.3rem 0;
        padding: 0;
      }
      .roots > li,
      .list > li {
        padding: 0.35rem 0;
        border-bottom: 1px solid var(--border);
      }
      .roots > li:last-child,
      .list > li:last-child {
        border-bottom: 0;
      }
      .paths {
        display: grid;
        grid-template-columns: max-content 1fr;
        gap: 0.1rem 0.6rem;
        margin: 0.2rem 0 0;
        font-size: 0.82rem;
      }
      .paths dt {
        color: var(--muted);
      }
      .paths dd {
        margin: 0;
        overflow-wrap: anywhere;
      }
      .editor {
        margin: 0.4rem 0;
      }
      .row {
        flex-wrap: wrap;
      }
    `,
  ],
})
export class AccountsFoldersComponent {
  private readonly api = inject(ApiService);
  private readonly machines = inject(MachinesService);

  protected readonly thisTitle = thisComputerTitle;
  protected readonly accounts = signal<AccountsTable | null>(null);
  protected readonly folders = signal<FoldersView | null>(null);
  protected readonly roots = signal<RootsView | null>(null);
  protected readonly loading = signal(false);
  protected readonly loadingFolders = signal(false);
  protected readonly error = signal('');
  protected readonly notice = signal('');
  protected readonly busy = signal<string | null>(null);
  protected readonly confirming = signal<string | null>(null);
  /** The root being edited (`new` for one being added). */
  protected readonly editing = signal<string | null>(null);
  protected readonly draftName = signal('');
  protected readonly draftPaths = signal<Record<string, string>>({});

  /** The computers a root may name here: this one and the paired ones. */
  protected readonly editable = computed<RootMachine[]>(() =>
    (this.roots()?.machines ?? []).filter((m) => m.here || m.paired),
  );
  protected readonly signInsAsked = computed(() =>
    (this.accounts()?.accounts ?? []).some((r) =>
      Object.values(r.asks ?? {}).some((a) => a.kind === 'signIn' && askOpen(a)),
    ),
  );

  constructor() {
    const destroyRef = inject(DestroyRef);
    const sub = inject(RealtimeService).machines$.subscribe((f) => {
      if (!REFRESH_ON.has(f.what)) return;
      if (f.what === 'roots') void this.loadRoots();
      else void this.loadAccounts();
    });
    destroyRef.onDestroy(() => {
      sub.unsubscribe();
      clearTimeout(this.confirmTimer);
    });
    void this.load(false);
  }

  private confirmTimer?: ReturnType<typeof setTimeout>;

  // ───────────────────────────────────────────────────────────── Loading

  async load(fresh: boolean): Promise<void> {
    this.loading.set(true);
    this.error.set('');
    try {
      await Promise.all([this.loadAccounts(), this.loadRoots(), this.loadFolders(fresh)]);
    } finally {
      this.loading.set(false);
    }
  }

  private async loadAccounts(): Promise<void> {
    try {
      this.accounts.set(await this.api.getMachineAccounts());
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  private async loadRoots(): Promise<void> {
    try {
      this.roots.set(await this.api.getRoots());
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  async loadFolders(fresh: boolean): Promise<void> {
    this.loadingFolders.set(true);
    try {
      this.folders.set(await this.api.getMachineFolders(fresh));
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.loadingFolders.set(false);
    }
  }

  // ───────────────────────────────────────────────────────────── Words

  protected text(s: string | null | undefined): string {
    return serverText(s);
  }

  protected open(a: MachineAsk | null | undefined): boolean {
    return askOpen(a);
  }

  protected askedText(a: MachineAsk | null, name: string): string {
    return a ? askText(a, name) : '';
  }

  /** An account's cell on a computer (none from a computer that did not
   * answer). */
  protected cellOf(row: AccountRow, machine: string): AccountCell | undefined {
    return row.cells[machine];
  }

  protected askOf(row: AccountRow, machine: string): MachineAsk | null {
    return row.asks?.[machine] ?? null;
  }

  protected placeholder(m: RootMachine): string {
    return m.os === 'windows' ? 'D:\\development\\projects' : '/Users/you/development/projects';
  }

  protected mark(c: AccountCell | undefined): string {
    switch (c?.state) {
      case 'signedIn':
        return '✓';
      case 'unknown':
      case 'checkFailed':
      case undefined:
        return '?';
      default:
        return '✕';
    }
  }

  protected cellText(c: AccountCell | undefined): string {
    switch (c?.state) {
      case 'signedIn':
        return tr('signed in');
      case 'signedOut':
        return tr('not signed in');
      case 'authFailed':
        return tr('its sign-in failed');
      case 'tokenMissing':
        return tr('its token is not there');
      case 'checkFailed':
        return tr('could not be checked');
      case 'missing':
        return tr('not there');
      default:
        return tr('not known now');
    }
  }

  protected needsSignIn(c: AccountCell | undefined): boolean {
    return c?.state === 'signedOut' || c?.state === 'authFailed' || c?.state === 'tokenMissing';
  }

  protected machineLabel(m: RootMachine): string {
    return m.here ? thisComputerTitle() : m.name || m.id.slice(0, 8);
  }

  protected reposText(pr: RootProposal): string {
    const names = pr.repos.map((r) => r.name);
    if (names.length <= 3) return names.join(', ');
    return tr('{names} and {n} more', { names: names.slice(0, 3).join(', '), n: names.length - 3 });
  }

  protected names(list: { name: string }[]): string {
    return list.map((r) => r.name).join(', ');
  }

  protected originless(f: FoldersView): string {
    return f.noOrigin.map((r) => r.name).join(', ');
  }

  /** The clone this computer asked `machine` for `l`, if any. */
  protected cloneAsk(machine: string, l: LoneRepo): MachineAsk | null {
    const asks = this.machines.setup()?.peers.find((p) => p.machine === machine)?.asks ?? [];
    return [...asks].reverse().find((a) => a.kind === 'clone' && a.origin === l.origin) ?? null;
  }

  // ───────────────────────────────────────────────────────────── Asking

  protected async askSignIn(machine: string, name: string, row: AccountRow): Promise<void> {
    const account = row.cells[machine]?.name ?? row.name;
    await this.withBusy(`${machine}:${row.name}`, async () => {
      await this.api.askMachine(machine, { check: 'signIn', account });
      this.notice.set(
        tr('Asked {name}: someone there signs {account} in, in its browser.', { name, account }),
      );
      await Promise.all([this.loadAccounts(), this.machines.loadSetup()]);
    });
  }

  protected async askAdd(machine: string, name: string, row: AccountRow): Promise<void> {
    await this.withBusy(`${machine}:${row.name}`, async () => {
      await this.api.askMachine(machine, { check: 'addAccount', account: row.name });
      this.notice.set(
        tr('Asked {name}: someone there adds {account}.', { name, account: row.name }),
      );
      await Promise.all([this.loadAccounts(), this.machines.loadSetup()]);
    });
  }

  protected async askClone(p: PeerFolders, l: LoneRepo): Promise<void> {
    if (!l.origin || !l.root || !l.rel) return;
    await this.withBusy(l.path, async () => {
      await this.api.askMachine(p.machine, {
        check: 'clone',
        origin: l.origin,
        root: l.root,
        rel: l.rel,
      });
      this.notice.set(
        tr('Asked {name}: it clones {repo} once someone there confirms.', {
          name: p.name,
          repo: l.name,
        }),
      );
      await this.machines.loadSetup();
    });
  }

  protected async cloneHere(l: LoneRepo): Promise<void> {
    if (!l.origin || !l.root || !l.rel) return;
    await this.withBusy(l.path, async () => {
      const r = await this.api.cloneHere({ origin: l.origin!, root: l.root!, rel: l.rel! });
      this.notice.set(tr('Cloned into {folder}.', { folder: r.folder }));
      await this.loadFolders(true);
    });
  }

  // ───────────────────────────────────────────────────────────── Roots

  protected async useProposal(p: PeerFolders, pr: RootProposal): Promise<void> {
    const here = this.folders()?.machine ?? this.roots()?.machine;
    if (!here) return;
    const existing = pr.root ? this.roots()?.roots.find((r) => r.id === pr.root) : undefined;
    const upsert = existing
      ? {
          id: existing.id,
          name: existing.name,
          paths: { ...existing.paths, [p.machine]: pr.there },
        }
      : { name: pr.name, paths: { [here]: pr.here, [p.machine]: pr.there } };
    await this.withBusy(pr.id, async () => {
      this.roots.set(await this.api.saveRoots({ upsert: [upsert] }));
      this.notice.set(tr('Saved the folder {name}.', { name: upsert.name }));
      await this.loadFolders(true);
    });
  }

  protected add(): void {
    this.editing.set('new');
    this.draftName.set('');
    this.draftPaths.set({});
  }

  protected edit(root: RootEntry): void {
    this.editing.set(root.id);
    this.draftName.set(root.name);
    this.draftPaths.set({ ...root.paths });
  }

  protected setPath(machine: string, path: string): void {
    this.draftPaths.update((d) => ({ ...d, [machine]: path }));
  }

  protected async saveRoot(): Promise<void> {
    const id = this.editing();
    if (!id) return;
    const paths: Record<string, string> = {};
    for (const [m, p] of Object.entries(this.draftPaths())) {
      if (p.trim()) paths[m] = p.trim();
    }
    const upsert = { ...(id === 'new' ? {} : { id }), name: this.draftName().trim(), paths };
    await this.withBusy('root', async () => {
      this.roots.set(await this.api.saveRoots({ upsert: [upsert] }));
      this.editing.set(null);
      this.notice.set(tr('Saved the folder {name}.', { name: upsert.name }));
      await this.loadFolders(false);
    });
  }

  protected async removeRoot(root: RootEntry): Promise<void> {
    if (this.confirming() !== root.id) {
      this.confirming.set(root.id);
      clearTimeout(this.confirmTimer);
      this.confirmTimer = setTimeout(() => this.confirming.set(null), 4000);
      return;
    }
    this.confirming.set(null);
    await this.withBusy(root.id, async () => {
      this.roots.set(await this.api.saveRoots({ remove: [root.id] }));
      this.notice.set(tr('Removed the folder {name}.', { name: root.name }));
    });
  }

  private async withBusy(key: string, fn: () => Promise<void>): Promise<void> {
    this.busy.set(key);
    this.error.set('');
    this.notice.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
