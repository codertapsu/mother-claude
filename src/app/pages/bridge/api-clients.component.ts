import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import {
  ApiClientRoot,
  ApiClientView,
  ApiConversationFull,
  ApiConversationRow,
  clientKind,
  conversationState,
  messageText,
  tokens,
} from '../../core/claude-api';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { sessionLink } from '../../core/session-links';
import { IconComponent } from '../../shared/icon.component';
import { relativeTime, shortPath } from '../../shared/util';

/** A client, and the folder it is kept in. */
interface Opened {
  root: string;
  client: ApiClientView;
  conversations: ApiConversationRow[];
}

/**
 * The Claude API's clients and their conversations (docs/CLAUDE-API.md
 * "Managing clients"): each folder an API server keeps clients in, each
 * client with its conversations — read, deleted one at a time or several
 * at once, or all of a client's data. With *their transcripts too* (the
 * dangerous right), Claude Code's transcripts of them go as well, kept
 * seven days for undo, as any delete of conversations.
 */
@Component({
  selector: 'mc-api-clients',
  imports: [FormsModule, IconComponent, RouterLink, TPipe],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="card clients">
      <div class="row head">
        <span class="muted small">{{ summary() }}</span>
        <span class="spacer"></span>
        <button class="btn small" type="button" [disabled]="loading()" (click)="load()">
          <mc-icon name="refresh" [size]="14" /> {{ 'Refresh' | t }}
        </button>
      </div>

      @if (error(); as e) {
        <p class="err small" role="alert">{{ e }}</p>
      }
      @if (said(); as s) {
        <p class="ok small" role="status">{{ s }}</p>
      }

      @for (r of withClients(); track r.path) {
        <div class="root">
          <div class="root-head small">
            <span class="lbl">{{ 'Folder' | t }}</span>
            <span class="mono" [title]="r.path">{{ short(r.path) }}</span>
            @if (r.default) {
              <span class="muted">{{ '(the default)' | t }}</span>
            }
          </div>
          <ul class="list">
            @for (c of r.clients; track c.id) {
              <li [class.open]="isOpen(r.path, c.id)">
                <button
                  class="client"
                  type="button"
                  [attr.aria-expanded]="isOpen(r.path, c.id)"
                  (click)="toggle(r.path, c)"
                >
                  <mc-icon
                    [name]="isOpen(r.path, c.id) ? 'chevronDown' : 'chevronRight'"
                    [size]="14"
                  />
                  <span class="name">{{ c.label }}</span>
                  <span class="muted small">{{ kind(c.kind) }}</span>
                  <span class="spacer"></span>
                  <span class="muted small">
                    {{ convCount(c.conversations ?? 0) }} ·
                    {{ 'last seen {when}' | t: { when: rel(c.lastSeen) } }}
                  </span>
                </button>

                @if (opened(); as o) {
                  @if (o.root === r.path && o.client.id === c.id) {
                    <div class="convs">
                      <p class="muted small">
                        {{ 'Its folder:' | t }} <span class="mono">{{ c.folder }}</span> ·
                        {{
                          '{n} requests, {in} tokens in, {out} out'
                            | t
                              : {
                                  n: c.requests,
                                  in: tok(c.usage?.inputTokens),
                                  out: tok(c.usage?.outputTokens),
                                }
                        }}
                      </p>
                      @if (o.conversations.length) {
                        <label class="check small">
                          <input
                            type="checkbox"
                            [checked]="allPicked()"
                            (change)="pickAll($any($event.target).checked)"
                          />
                          <span>{{ 'All' | t }}</span>
                        </label>
                        <ul class="rows">
                          @for (v of o.conversations; track v.id) {
                            <li>
                              <label class="check">
                                <input
                                  type="checkbox"
                                  [checked]="picked().has(v.id)"
                                  [disabled]="v.busy"
                                  (change)="pick(v.id, $any($event.target).checked)"
                                  [attr.aria-label]="'Pick {title}' | t: { title: titleOf(v) }"
                                />
                              </label>
                              <div class="conv">
                                <div class="line">
                                  <span class="title">{{ titleOf(v) }}</span>
                                  <span class="badge small">{{ state(v) }}</span>
                                </div>
                                <div class="muted small">
                                  {{ v.model }} · {{ msgCount(v.messages) }} ·
                                  {{ 'updated {when}' | t: { when: rel(v.updatedAt) } }}
                                  @if (v.forkedFrom) {
                                    · {{ 'forked' | t }}
                                  }
                                </div>
                                <div class="acts">
                                  <button class="btn ghost small" type="button" (click)="show(v)">
                                    {{
                                      shown()?.id === v.id
                                        ? ('Hide messages' | t)
                                        : ('Messages' | t)
                                    }}
                                  </button>
                                  <a class="btn ghost small" [routerLink]="link(v.id)">{{
                                    'Open the session' | t
                                  }}</a>
                                </div>
                                @if (shown(); as m) {
                                  @if (m.id === v.id) {
                                    <ol class="messages">
                                      @for (msg of m.messageList; track $index) {
                                        <li [class.assistant]="msg.role === 'assistant'">
                                          <span class="who small">{{
                                            msg.role === 'assistant'
                                              ? ('Claude' | t)
                                              : ('Client' | t)
                                          }}</span>
                                          <span class="text">{{ preview(msg) }}</span>
                                        </li>
                                      }
                                    </ol>
                                  }
                                }
                              </div>
                            </li>
                          }
                        </ul>
                      } @else {
                        <p class="muted small">{{ 'No conversations.' | t }}</p>
                      }

                      <label class="check small">
                        <input
                          type="checkbox"
                          [ngModel]="transcripts()"
                          (ngModelChange)="transcripts.set($event)"
                          [disabled]="!canDangerous()"
                        />
                        <span>
                          {{
                            'Their Claude Code transcripts too — kept seven days, so the delete can be undone on the Data page'
                              | t
                          }}
                          @if (!canDangerous()) {
                            <em>{{ '(the desktop app)' | t }}</em>
                          }
                        </span>
                      </label>

                      @if (confirming(); as what) {
                        <div class="confirm" role="alert">
                          <span>{{ confirmText(what) }}</span>
                          <button
                            class="btn small danger"
                            type="button"
                            [disabled]="busy()"
                            (click)="confirm(what)"
                          >
                            {{ 'Delete' | t }}
                          </button>
                          <button class="btn small" type="button" (click)="confirming.set(null)">
                            {{ 'Cancel' | t }}
                          </button>
                        </div>
                      } @else {
                        <div class="acts">
                          <button
                            class="btn small"
                            type="button"
                            [disabled]="busy() || !picked().size"
                            (click)="confirming.set('picked')"
                          >
                            {{ 'Delete picked ({n})' | t: { n: picked().size } }}
                          </button>
                          <button
                            class="btn small"
                            type="button"
                            [disabled]="busy()"
                            (click)="confirming.set('client')"
                          >
                            {{ "Delete all of this client's data…" | t }}
                          </button>
                        </div>
                      }
                    </div>
                  }
                }
              </li>
            }
          </ul>
        </div>
      } @empty {
        @if (!loading()) {
          <p class="muted small">
            {{ 'No client has called the Claude API yet.' | t }}
          </p>
        }
      }
    </div>
  `,
  styles: [
    `
      .clients {
        display: flex;
        flex-direction: column;
        gap: 12px;
      }
      .row.head {
        display: flex;
        align-items: center;
        gap: 8px;
      }
      .spacer {
        flex: 1;
      }
      .root-head {
        display: flex;
        gap: 8px;
        align-items: baseline;
        flex-wrap: wrap;
        margin-bottom: 6px;
      }
      .root-head .mono {
        overflow-wrap: anywhere;
      }
      .lbl {
        font-size: 0.75rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        opacity: 0.6;
      }
      .list,
      .rows,
      .messages {
        list-style: none;
        margin: 0;
        padding: 0;
      }
      .list > li {
        border-top: 1px solid rgba(127, 127, 127, 0.18);
      }
      .client {
        all: unset;
        box-sizing: border-box;
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        padding: 8px 2px;
        cursor: pointer;
        flex-wrap: wrap;
      }
      .client:focus-visible {
        outline: 2px solid var(--accent, #6b5bd2);
        border-radius: 6px;
      }
      .name,
      .title {
        font-weight: 600;
        overflow-wrap: anywhere;
      }
      .convs {
        display: flex;
        flex-direction: column;
        gap: 8px;
        padding: 0 0 12px 22px;
      }
      .convs .mono {
        overflow-wrap: anywhere;
      }
      .rows {
        display: flex;
        flex-direction: column;
        gap: 10px;
      }
      .rows > li {
        display: flex;
        gap: 8px;
        align-items: flex-start;
      }
      .conv {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
        flex: 1;
      }
      .line {
        display: flex;
        gap: 8px;
        align-items: baseline;
        flex-wrap: wrap;
      }
      .acts {
        display: flex;
        gap: 6px;
        flex-wrap: wrap;
      }
      .acts .btn.ghost {
        margin-left: -0.5rem;
      }
      label.check {
        display: flex;
        gap: 8px;
        align-items: flex-start;
      }
      label.check input {
        margin-top: 3px;
      }
      .messages {
        display: flex;
        flex-direction: column;
        gap: 6px;
        margin-top: 6px;
        padding: 8px 10px;
        border-radius: 8px;
        background: rgba(127, 127, 127, 0.08);
      }
      .messages li {
        display: flex;
        gap: 8px;
      }
      .messages .who {
        flex: none;
        width: 3.5rem;
        opacity: 0.6;
      }
      .messages .text {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
      }
      .confirm {
        display: flex;
        gap: 8px;
        align-items: center;
        flex-wrap: wrap;
      }
      .err {
        color: var(--error, #a63232);
        margin: 0;
      }
      .ok {
        color: var(--ok, #2f7d4f);
        margin: 0;
      }
    `,
  ],
})
export class ApiClientsComponent {
  private api = inject(ApiService);
  readonly canDangerous = inject(AuthService).canDangerous;

  readonly roots = signal<ApiClientRoot[]>([]);
  readonly loading = signal(false);
  readonly error = signal('');
  readonly said = signal('');
  readonly opened = signal<Opened | null>(null);
  readonly picked = signal<ReadonlySet<string>>(new Set());
  readonly shown = signal<ApiConversationFull | null>(null);
  readonly transcripts = signal(false);
  readonly confirming = signal<'picked' | 'client' | null>(null);
  readonly busy = signal(false);

  /** The folders that have clients. */
  readonly withClients = computed(() => this.roots().filter((r) => r.clients.length));
  readonly summary = computed(() => {
    const clients = this.roots().reduce((n, r) => n + r.clients.length, 0);
    const convs = this.roots().reduce(
      (n, r) => n + r.clients.reduce((m, c) => m + (c.conversations ?? 0), 0),
      0,
    );
    return tr('{clients} · {convs}', {
      clients: clients === 1 ? tr('1 client') : tr('{n} clients', { n: clients }),
      convs: this.convCount(convs),
    });
  });
  readonly allPicked = computed(() => {
    const o = this.opened();
    const free = o?.conversations.filter((c) => !c.busy) ?? [];
    return free.length > 0 && free.every((c) => this.picked().has(c.id));
  });

  constructor() {
    void this.load();
  }

  async load(): Promise<void> {
    this.loading.set(true);
    this.error.set('');
    try {
      this.roots.set((await this.api.listApiClients()).roots ?? []);
      const o = this.opened();
      if (o) await this.open(o.root, o.client, true);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.loading.set(false);
    }
  }

  isOpen(root: string, id: string): boolean {
    const o = this.opened();
    return !!o && o.root === root && o.client.id === id;
  }

  async toggle(root: string, c: ApiClientView): Promise<void> {
    if (this.isOpen(root, c.id)) {
      this.opened.set(null);
      return;
    }
    await this.open(root, c);
  }

  /** Opens a client to its conversations; `again`: the same one, read
   * anew after a change (what was just said stays). */
  private async open(root: string, c: ApiClientView, again = false): Promise<void> {
    if (!again) this.said.set('');
    this.confirming.set(null);
    try {
      const v = await this.api.getApiClient(c.id, root);
      this.opened.set({ root, client: { ...c, ...v.client }, conversations: v.conversations });
      const ids = new Set(v.conversations.map((x) => x.id));
      this.picked.update((p) => new Set([...p].filter((id) => ids.has(id))));
    } catch (e) {
      this.opened.set(null);
      this.error.set(describeError(e).text);
    }
  }

  pick(id: string, on: boolean): void {
    this.picked.update((p) => {
      const next = new Set(p);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }

  pickAll(on: boolean): void {
    const free = this.opened()?.conversations.filter((c) => !c.busy) ?? [];
    this.picked.set(on ? new Set(free.map((c) => c.id)) : new Set());
  }

  async show(v: ApiConversationRow): Promise<void> {
    if (this.shown()?.id === v.id) {
      this.shown.set(null);
      return;
    }
    try {
      this.shown.set(await this.api.getApiConversation(v.id));
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  confirmText(what: 'picked' | 'client'): string {
    const also = this.transcripts() ? ' ' + tr('Their transcripts go too.') : '';
    if (what === 'picked') {
      return tr('Delete {convs}?', { convs: this.convCount(this.picked().size) }) + also;
    }
    const o = this.opened();
    return (
      tr('Delete everything of {client}: {convs} and its folder?', {
        client: o?.client.label ?? '',
        convs: this.convCount(o?.conversations.length ?? 0),
      }) + also
    );
  }

  async confirm(what: 'picked' | 'client'): Promise<void> {
    const o = this.opened();
    if (!o) return;
    this.busy.set(true);
    this.error.set('');
    try {
      const transcripts = this.transcripts() && this.canDangerous();
      if (what === 'picked') {
        const r = await this.api.deleteApiConversations([...this.picked()], transcripts);
        const busy = r.busy?.length
          ? ' ' + tr('{n} answering a request now were left.', { n: r.busy.length })
          : '';
        this.said.set(tr('Deleted {convs}.', { convs: this.convCount(r.deleted.length) }) + busy);
        this.picked.set(new Set());
        await this.load();
      } else {
        const r = await this.api.deleteApiClient(o.client.id, o.root, transcripts);
        this.said.set(
          tr('Deleted {client} and {convs}.', {
            client: o.client.label,
            convs: this.convCount(r.deleted.length),
          }),
        );
        this.opened.set(null);
        this.picked.set(new Set());
        await this.load();
      }
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.confirming.set(null);
      this.busy.set(false);
    }
  }

  titleOf(v: ApiConversationRow): string {
    return v.title?.trim() || v.id.slice(0, 8);
  }

  state(v: ApiConversationRow): string {
    return conversationState(v);
  }

  kind(k: string): string {
    return clientKind(k);
  }

  preview(m: ApiConversationFull['messageList'][number]): string {
    const t = messageText(m);
    return t.length > 600 ? t.slice(0, 599) + '…' : t;
  }

  convCount(n: number): string {
    return n === 1 ? tr('1 conversation') : tr('{n} conversations', { n });
  }

  msgCount(n: number): string {
    return n === 1 ? tr('1 message') : tr('{n} messages', { n });
  }

  tok(n: number | undefined): string {
    return tokens(n);
  }

  link(id: string): string[] {
    return sessionLink(id, null);
  }

  rel(at?: number): string {
    return relativeTime(at);
  }

  short(path: string): string {
    return shortPath(path);
  }
}
