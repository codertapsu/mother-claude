import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  computed,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';

import { RouterLink } from '@angular/router';

import { ApiService } from '../../core/api.service';
import { CommandInfo, PendingRef } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import { PendingService } from '../../core/pending.service';
import { storageGet, storageRemove, storageSet } from '../../core/platform';
import { ViewportService } from '../../core/viewport.service';
import { CapLimitsComponent } from '../../shared/cap-limits.component';
import { capAttach, capWake, limitFor } from '../../shared/caps';
import { IconComponent } from '../../shared/icon.component';
import { MAX_IMAGES, blobToBase64, prepareImage } from '../../shared/images';
import { SheetComponent } from '../../shared/sheet.component';
import { surfaceLabel } from '../../shared/util';
import { PickerKind, SessionContext } from './session-context';

interface Attachment {
  id: number;
  name: string;
  url: string;
  mediaType: string;
  data: string;
}

interface PaletteItem {
  key: string;
  label: string;
  hint?: string;
  value: string;
}

const DRAFT_PREFIX = 'mc_draft_';
const RELAY_EXPLAINED_KEY = 'mc_relay_explained';
/** Commands the header handles with its own pickers. */
const PICKER_COMMANDS: Record<string, PickerKind> = {
  model: 'model',
  effort: 'effort',
  mode: 'mode',
  permissions: 'mode',
  rename: 'rename',
};
const CONFIRM_COMMANDS: Record<string, string> = {
  clear:
    'This starts a fresh conversation in this session; the current one is kept in its history.',
  compact: 'Claude summarises the conversation so far to free up context.',
};
/** Always offered (they map onto the header pickers). */
const BUILTIN: CommandInfo[] = [
  { name: 'model', description: 'Switch the model' },
  { name: 'effort', description: 'Change the effort level' },
  { name: 'mode', description: 'Change the permission mode' },
  { name: 'rename', description: 'Rename this session' },
];

/**
 * The message box, pinned to the bottom of the session page.
 *
 * - Enter: on touch devices it adds a newline (use Send); with a mouse and
 *   keyboard it sends and Shift+Enter adds a newline. Cmd/Ctrl+Enter always
 *   sends. Never while an IME is composing.
 * - The draft is kept per session (localStorage `mc_draft_<id>`) and cleared
 *   only after a send succeeded; a failed send keeps the text ("Not sent ·
 *   Retry").
 * - Busy session: Send becomes "Queue", with "Interrupt & send" next to it.
 *   With exactly one pending permission: "Deny & send as feedback".
 * - Ended session: sending resumes the conversation here (POST /continue).
 * - `/` opens the command palette (model / effort / mode / rename go to the
 *   header's pickers; /clear and /compact ask first); `@` completes paths.
 * - Attachments: photos are downscaled to ≤ 2048 px and sent inline where
 *   the session takes images; other files (or every file, for text-only
 *   channels) are uploaded and referenced as `@<path>`.
 * - Queued messages are listed with ✕ while they can still be taken back.
 * - A session that can't take messages explains why, with its fixes.
 */
@Component({
  selector: 'mc-composer',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, CapLimitsComponent, IconComponent, SheetComponent],
  templateUrl: './composer.component.html',
  styleUrl: './composer.component.scss',
})
export class ComposerComponent {
  protected readonly ctx = inject(SessionContext);
  private api = inject(ApiService);
  private notices = inject(NoticeService);
  private pendings = inject(PendingService);
  private viewport = inject(ViewportService);

  private readonly ta = viewChild<ElementRef<HTMLTextAreaElement>>('ta');
  private readonly photoInput = viewChild<ElementRef<HTMLInputElement>>('photo');
  private readonly fileInput = viewChild<ElementRef<HTMLInputElement>>('file');

  protected readonly text = signal('');
  protected readonly attachments = signal<Attachment[]>([]);
  protected readonly uploading = signal(0);
  protected readonly sending = signal(false);
  protected readonly failed = signal(false);
  protected readonly explainer = signal(false);
  protected readonly confirm = signal<{ name: string; text: string } | null>(null);

  protected readonly palette = signal<'slash' | 'files' | null>(null);
  protected readonly paletteItems = signal<PaletteItem[]>([]);
  protected readonly active = signal(0);
  private fileQueryTimer?: ReturnType<typeof setTimeout>;
  private fileQuerySeq = 0;
  private draftTimer?: ReturnType<typeof setTimeout>;
  private nextId = 1;

  protected readonly session = this.ctx.session;
  protected readonly caps = this.ctx.caps;
  protected readonly coarse = this.viewport.coarse;
  protected readonly mode = computed<'message' | 'resume' | 'blocked'>(() => {
    const s = this.session();
    const c = this.caps();
    if (!s || !c) return 'blocked';
    if (s.live && c.message !== 'none') return 'message';
    if (!s.live && (c.continueOwned || c.fork)) return 'resume';
    return 'blocked';
  });
  protected readonly attach = computed(() =>
    this.mode() === 'message' ? capAttach(this.caps()) : 'none',
  );
  protected readonly relay = computed(() => this.caps()?.message === 'relay');
  protected readonly surfaceName = computed(() => surfaceLabel(this.session()?.surface));
  /** Desktop, a live foreign session nobody can reach, and the server's
   * limits don't already offer the plugin. */
  protected readonly installMissing = computed(() => {
    const s = this.session();
    return (
      !!s &&
      this.ctx.isDesktop() &&
      s.live &&
      !s.owned &&
      s.control.channel === 'none' &&
      !limitFor(s.control, 'install-foreign-control')
    );
  });
  /** A limit already offers Continue/Fork: don't add a second button. */
  protected readonly continueOffered = computed(
    () => !!limitFor(this.caps(), 'fork', 'continue-owned'),
  );
  protected readonly onePermission = computed<PendingRef | null>(() => {
    const perms = this.ctx
      .pending()
      .filter((p) => p.kind === 'permission' && p.answerable && !p.dangerous);
    return perms.length === 1 && this.ctx.pending().length === 1 ? perms[0] : null;
  });
  protected readonly hasContent = computed(
    () => !!this.text().trim() || this.attachments().length > 0,
  );
  protected readonly canSend = computed(
    () =>
      this.hasContent() &&
      !this.sending() &&
      this.uploading() === 0 &&
      this.ctx.canAct() &&
      this.mode() !== 'blocked',
  );
  protected readonly sendLabel = computed(() => {
    if (!this.ctx.canAct()) return 'Reconnecting…';
    if (this.mode() === 'resume') return 'Resume';
    if (this.onePermission() && this.hasContent()) return 'Queue after this';
    return this.ctx.busy() ? 'Queue' : 'Send';
  });
  protected readonly interruptLabel = computed(() =>
    this.caps()?.interrupt === 'next-tool' ? 'Interrupt at next tool' : 'Interrupt',
  );
  protected readonly placeholder = computed(() =>
    this.mode() === 'resume' ? 'Message to resume it here…' : 'Message Claude…',
  );
  protected readonly waitHint = computed(() => {
    const c = this.caps();
    if (!c || this.mode() !== 'message' || c.message !== 'relay') return '';
    if (c.channel === 'tmux') return 'Typed into its tmux pane.';
    return capWake(c)
      ? "Delivered at Claude's next stopping point."
      : 'Waiting for the next step: delivered when Claude next stops or runs a tool.';
  });
  protected readonly activeId = computed(() => (this.palette() ? `mc-pal-${this.active()}` : null));

  constructor() {
    // Per-session draft.
    effect(() => {
      const id = this.ctx.id();
      untracked(() => {
        this.text.set(id ? (storageGet(DRAFT_PREFIX + id) ?? '') : '');
        this.clearAttachments();
        this.failed.set(false);
        this.closePalette();
        queueMicrotask(() => this.autosize());
      });
    });
    // Text handed over by another part (e.g. unsent pending feedback).
    effect(() => {
      const ins = this.ctx.composerInsert();
      if (!ins) return;
      untracked(() => {
        const cur = this.text().trim();
        this.setText(cur ? `${cur}\n${ins.text}` : ins.text);
        this.ta()?.nativeElement.focus();
      });
    });
    inject(DestroyRef).onDestroy(() => {
      clearTimeout(this.draftTimer);
      clearTimeout(this.fileQueryTimer);
      this.ctx.composerFocused.set(false);
      for (const a of this.attachments()) URL.revokeObjectURL(a.url);
    });
  }

  // ─────────────────────────────────────────────────────────── Typing

  protected onInput(e: Event): void {
    const el = e.target as HTMLTextAreaElement;
    this.setText(el.value, false);
    this.failed.set(false);
    this.updatePalette(el);
  }

  protected onKeydown(e: KeyboardEvent): void {
    if (e.isComposing || e.keyCode === 229) return;
    if (this.palette() && this.paletteKey(e)) return;
    if (e.key !== 'Enter') return;
    if (e.metaKey || e.ctrlKey) {
      e.preventDefault();
      void this.submit('send');
      return;
    }
    if (!this.coarse() && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      void this.submit('send');
    }
  }

  protected onFocus(): void {
    this.ctx.composerFocused.set(true);
  }

  protected onBlur(): void {
    // Let a tap on the palette land before it closes.
    setTimeout(() => {
      if (document.activeElement !== this.ta()?.nativeElement) {
        this.ctx.composerFocused.set(false);
        this.closePalette();
      }
    }, 150);
  }

  private setText(value: string, sync = true): void {
    this.text.set(value);
    if (sync) {
      const el = this.ta()?.nativeElement;
      if (el && el.value !== value) el.value = value;
    }
    this.autosize();
    clearTimeout(this.draftTimer);
    const id = this.ctx.id();
    this.draftTimer = setTimeout(() => {
      if (!id) return;
      if (value.trim()) storageSet(DRAFT_PREFIX + id, value);
      else storageRemove(DRAFT_PREFIX + id);
    }, 300);
  }

  private autosize(): void {
    const el = this.ta()?.nativeElement;
    if (!el) return;
    el.style.height = 'auto';
    const max = Math.round((window.visualViewport?.height ?? window.innerHeight) * 0.35);
    el.style.height = `${Math.min(el.scrollHeight + 2, Math.max(max, 120))}px`;
  }

  // ─────────────────────────────────────────────────────────── Palette

  private updatePalette(el: HTMLTextAreaElement): void {
    const value = el.value;
    const caret = el.selectionStart ?? value.length;
    const before = value.slice(0, caret);
    if (this.caps()?.slashCommands && /^\/[^\s]*$/.test(before)) {
      void this.showCommands(before.slice(1).toLowerCase());
      return;
    }
    const at = /(?:^|\s)@([^\s@]*)$/.exec(before);
    if (at) {
      this.queryFiles(at[1]);
      return;
    }
    this.closePalette();
  }

  private async showCommands(q: string): Promise<void> {
    const server = await this.ctx.loadCommands();
    const seen = new Set<string>();
    const all = [...BUILTIN, ...server].filter((c) => {
      const name = c.name.replace(/^\//, '');
      if (seen.has(name)) return false;
      seen.add(name);
      return true;
    });
    const items = all
      .map((c) => ({ ...c, name: c.name.replace(/^\//, '') }))
      .filter((c) => !q || c.name.toLowerCase().startsWith(q) || c.name.toLowerCase().includes(q))
      .sort((a, b) => Number(!a.name.startsWith(q)) - Number(!b.name.startsWith(q)))
      .slice(0, 12)
      .map((c) => ({
        key: c.name,
        label: `/${c.name}${c.argumentHint ? ` ${c.argumentHint}` : ''}`,
        hint: c.description,
        value: c.name,
      }));
    this.palette.set(items.length ? 'slash' : null);
    this.paletteItems.set(items);
    this.active.set(0);
  }

  private queryFiles(q: string): void {
    clearTimeout(this.fileQueryTimer);
    const seq = ++this.fileQuerySeq;
    this.fileQueryTimer = setTimeout(async () => {
      try {
        const res = await this.api.getFiles(this.ctx.id(), q);
        if (seq !== this.fileQuerySeq) return;
        const items = (res.files ?? []).slice(0, 12).map((f) => ({ key: f, label: f, value: f }));
        this.palette.set(items.length ? 'files' : null);
        this.paletteItems.set(items);
        this.active.set(0);
      } catch {
        if (seq === this.fileQuerySeq) this.closePalette();
      }
    }, 150);
  }

  protected closePalette(): void {
    this.palette.set(null);
    this.paletteItems.set([]);
    this.fileQuerySeq++;
  }

  /** Arrow keys / Enter / Tab / Escape while the palette is open. */
  private paletteKey(e: KeyboardEvent): boolean {
    const n = this.paletteItems().length;
    switch (e.key) {
      case 'ArrowDown':
        this.active.set((this.active() + 1) % n);
        break;
      case 'ArrowUp':
        this.active.set((this.active() - 1 + n) % n);
        break;
      case 'Enter':
      case 'Tab': {
        const item = this.paletteItems()[this.active()];
        if (!item) return false;
        this.pick(item);
        break;
      }
      case 'Escape':
        this.closePalette();
        break;
      default:
        return false;
    }
    e.preventDefault();
    return true;
  }

  protected pick(item: PaletteItem): void {
    const kind = this.palette();
    this.closePalette();
    if (kind === 'files') {
      const el = this.ta()?.nativeElement;
      const value = this.text();
      const caret = el?.selectionStart ?? value.length;
      const before = value.slice(0, caret).replace(/@([^\s@]*)$/, `@${item.value} `);
      this.setText(before + value.slice(caret));
      queueMicrotask(() => {
        el?.focus();
        el?.setSelectionRange(before.length, before.length);
      });
      return;
    }
    const name = item.value;
    const picker = PICKER_COMMANDS[name];
    if (picker) {
      this.setText('');
      this.ctx.openPicker.set(picker);
      return;
    }
    if (CONFIRM_COMMANDS[name]) {
      this.setText('');
      this.confirm.set({ name, text: CONFIRM_COMMANDS[name] });
      return;
    }
    this.setText(`/${name} `);
    queueMicrotask(() => this.ta()?.nativeElement.focus());
  }

  protected async runConfirmed(): Promise<void> {
    const c = this.confirm();
    this.confirm.set(null);
    if (!c) return;
    const res = await this.ctx.send(`/${c.name}`);
    if (res) this.notices.success(`/${c.name} sent.`);
  }

  // ─────────────────────────────────────────────────────── Attachments

  protected choose(kind: 'photo' | 'file'): void {
    (kind === 'photo' ? this.photoInput() : this.fileInput())?.nativeElement.click();
  }

  protected async onFiles(e: Event): Promise<void> {
    const input = e.target as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    for (const file of files) {
      if (this.attach() === 'inline' && file.type.startsWith('image/')) {
        await this.addImage(file);
      } else {
        await this.upload(file);
      }
    }
  }

  private async addImage(file: File): Promise<void> {
    if (this.attachments().length >= MAX_IMAGES) {
      this.notices.warn(`At most ${MAX_IMAGES} images per message.`);
      return;
    }
    this.uploading.update((n) => n + 1);
    try {
      const img = await prepareImage(file);
      const url = URL.createObjectURL(img.blob);
      this.attachments.update((prev) => [
        ...prev,
        { id: this.nextId++, name: file.name, url, mediaType: img.mediaType, data: img.data },
      ]);
    } catch (err) {
      this.notices.error(err, undefined, file.name);
    } finally {
      this.uploading.update((n) => n - 1);
    }
  }

  private async upload(file: File): Promise<void> {
    if (file.size > 50 * 1024 * 1024) {
      this.notices.warn(`${file.name} is over 50 MB.`);
      return;
    }
    this.uploading.update((n) => n + 1);
    try {
      const data = await blobToBase64(file);
      const res = await this.api.uploadFile(this.ctx.id(), {
        name: file.name,
        mediaType: file.type || 'application/octet-stream',
        data,
      });
      if (!res?.path) throw new Error('The server did not say where it stored the file.');
      const cur = this.text();
      const sep = cur && !/\s$/.test(cur) ? ' ' : '';
      this.setText(`${cur}${sep}@${res.path} `);
    } catch (err) {
      this.notices.error(err, undefined, `Could not attach ${file.name}`);
    } finally {
      this.uploading.update((n) => n - 1);
    }
  }

  protected remove(a: Attachment): void {
    URL.revokeObjectURL(a.url);
    this.attachments.update((prev) => prev.filter((x) => x.id !== a.id));
  }

  private clearAttachments(): void {
    for (const a of this.attachments()) URL.revokeObjectURL(a.url);
    this.attachments.set([]);
  }

  // ─────────────────────────────────────────────────────────── Sending

  protected async submit(how: 'send' | 'interrupt' | 'deny'): Promise<void> {
    if (!this.canSend()) return;
    const text = this.text().trim();
    const images = this.attachments().map(({ mediaType, data }) => ({ mediaType, data }));
    this.sending.set(true);
    this.failed.set(false);
    this.closePalette();
    try {
      if (this.mode() === 'resume') {
        if (await this.ctx.resumeWith(text)) this.cleared();
        else this.failed.set(true);
        return;
      }
      if (how === 'deny') {
        const p = this.onePermission();
        if (!p) return;
        const res = await this.pendings
          .respond(p, { decision: 'deny', message: text }, 'Denied with your note')
          .catch((err: unknown) => {
            this.notices.error(err, undefined, 'Not sent');
            return undefined;
          });
        if (res) {
          this.notices.success(res.alreadyDecided ? res.text : 'Denied, with your note to Claude.');
          if (!res.alreadyDecided) this.cleared();
        } else if (res === undefined) this.failed.set(true);
        return;
      }
      if (how === 'interrupt' && !(await this.ctx.interrupt())) return;
      const res = await this.ctx.send(text, images);
      if (!res) {
        this.failed.set(true);
        return;
      }
      this.cleared();
      if (res.delivery === 'queued') {
        this.notices.info('Queued: Claude reads it when the current step ends.');
      } else if (res.delivery === 'relayed') {
        if (this.waitHint()) this.notices.info(this.waitHint());
        if (this.caps()?.channel === 'hooks' && storageGet(RELAY_EXPLAINED_KEY) !== '1') {
          this.explainer.set(true);
        }
      } else {
        this.notices.announce('Message sent.');
      }
    } finally {
      this.sending.set(false);
    }
  }

  protected dismissExplainer(): void {
    storageSet(RELAY_EXPLAINED_KEY, '1');
    this.explainer.set(false);
  }

  private cleared(): void {
    this.setText('');
    storageRemove(DRAFT_PREFIX + this.ctx.id());
    this.clearAttachments();
    this.failed.set(false);
  }
}
