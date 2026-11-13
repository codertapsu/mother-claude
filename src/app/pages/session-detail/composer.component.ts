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
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachineContext } from '../../core/machine-context';
import { beforeLanded } from '../../core/moves';
import { MovesService } from '../../core/moves.service';
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
/** Text before the caret that is a `/command` being typed. */
const SLASH_QUERY = /^\/[^\s]*$/;

/** What a send took from the composer. */
interface Sent {
  session: string;
  text: string;
  attachments: ReadonlySet<number>;
}

/**
 * What stays in the composer once `sent` went out: text typed after it while
 * the request was in flight. If the text was edited in a way that no longer
 * starts with what was sent, it is kept as it is.
 */
export function remainderAfterSend(now: string, sent: string): string {
  if (!now.startsWith(sent)) return now;
  return now.slice(sent.length).trimStart();
}

/**
 * The files a paste or a drop carries: a screenshot or a copied image, files
 * copied in Finder or File Explorer. Some browsers fill only `items`.
 */
export function transferFiles(data: DataTransfer | null | undefined): File[] {
  if (!data) return [];
  const files = Array.from(data.files ?? []);
  if (files.length) return files;
  return Array.from(data.items ?? [])
    .filter((i) => i.kind === 'file')
    .map((i) => i.getAsFile())
    .filter((f): f is File => !!f);
}

/**
 * Whether a paste that carries files and text means the text: cells copied
 * from a spreadsheet carry a picture of themselves too (the text is meant),
 * while a file copied in Finder carries only its own name as text (the file
 * is).
 */
export function pasteMeansText(text: string, files: readonly File[]): boolean {
  const lines = text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean);
  if (!lines.length) return false;
  const names = new Set(files.map((f) => f.name));
  return !lines.every((l) => names.has(l) || names.has(l.split(/[\\/]/).pop() ?? ''));
}

/** The names a clipboard gives what it holds (`image.png`, or none). */
const GENERIC_NAME = /^(image|blob)?(\.[a-z0-9]{1,5})?$/i;

/**
 * A pasted screenshot comes as `image.png` or with no name at all: give it
 * one that says what it is and when — Claude reads the file by that name.
 * A real file name is kept.
 */
export function pastedName(file: File, now = new Date()): string {
  if (!GENERIC_NAME.test(file.name)) return file.name;
  const image = file.type.startsWith('image/');
  const sub = (file.type.split('/')[1] ?? '').split('+')[0].replace(/[^a-z0-9]/gi, '');
  const ext = sub === 'jpeg' ? 'jpg' : sub === 'plain' ? 'txt' : sub || (image ? 'png' : 'bin');
  const p = (n: number) => String(n).padStart(2, '0');
  const day = `${now.getFullYear()}${p(now.getMonth() + 1)}${p(now.getDate())}`;
  const time = `${p(now.getHours())}${p(now.getMinutes())}${p(now.getSeconds())}`;
  return `pasted-${image ? 'image' : 'file'}-${day}-${time}.${ext}`;
}

const carriesFiles = (e: DragEvent): boolean => !!e.dataTransfer?.types?.includes('Files');
const RELAY_EXPLAINED_KEY = 'mc_relay_explained';
/** Commands the header handles with its own pickers. */
const PICKER_COMMANDS: Record<string, PickerKind> = {
  model: 'model',
  effort: 'effort',
  mode: 'mode',
  permissions: 'mode',
  rename: 'rename',
};
/** Commands that ask first, and what they do (in the language shown). */
function confirmText(name: string): string {
  switch (name) {
    case 'clear':
      return tr(
        'This starts a fresh conversation in this session; the current one is kept in its history.',
      );
    case 'compact':
      return tr('Claude summarises the conversation so far to free up context.');
    default:
      return '';
  }
}
/** Always offered (they map onto the header pickers), described in the
 * language shown. */
function builtinCommands(): CommandInfo[] {
  return [
    { name: 'model', description: tr('Switch the model') },
    { name: 'effort', description: tr('Change the effort level') },
    { name: 'mode', description: tr('Change the permission mode') },
    { name: 'rename', description: tr('Rename this session') },
  ];
}

/**
 * The message box, pinned to the bottom of the session page.
 *
 * - Enter: on touch devices it adds a newline (use Send); with a mouse and
 *   keyboard it sends and Shift+Enter adds a newline. Cmd/Ctrl+Enter always
 *   sends. Never while an IME is composing.
 * - The draft is kept per session (localStorage `mc_draft_<id>`) and cleared
 *   only after a send succeeded; a failed send keeps the text ("Not sent ·
 *   Retry").
 * - Busy session: a session Mother Claude runs takes the message into the
 *   running turn at its next step (Claude Code folds it in, CLI 2.1.319),
 *   with "Send now" beside Send to make the turn take it at once; another
 *   offers "Interrupt & send". With exactly one pending permission: "Deny &
 *   send as feedback".
 * - Ended session: sending resumes the conversation here (POST /continue).
 * - `/` opens the command palette (model / effort / mode / rename go to the
 *   header's pickers; /clear and /compact ask first); `@` completes paths.
 * - Attachments: photos are downscaled to ≤ 2048 px and sent inline where
 *   the session takes images; other files (or every file, for text-only
 *   channels) are uploaded and referenced as `@<path>`. The buttons, a
 *   paste (a screenshot, a copied image or file) and a drop on the box all
 *   attach the same way; a paste that carries text pastes the text.
 * - Waiting messages are listed with ✕ while they can still be taken back,
 *   and Send now where the session takes it.
 * - A session that can't take messages explains why, with its fixes; another
 *   computer's says when that computer is offline or lets this one watch
 *   only.
 */
@Component({
  selector: 'mc-composer',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, CapLimitsComponent, IconComponent, SheetComponent, TPipe],
  templateUrl: './composer.component.html',
  styleUrl: './composer.component.scss',
  host: {
    '(dragover)': 'onDragOver($event)',
    '(dragleave)': 'onDragLeave($event)',
    '(drop)': 'onDrop($event)',
  },
})
export class ComposerComponent {
  protected readonly ctx = inject(SessionContext);
  private host = inject<ElementRef<HTMLElement>>(ElementRef);
  /** Another computer's session (none: this computer's). */
  private machine = inject(MachineContext, { optional: true });
  private moves = inject(MovesService);
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
  /** Files are dragged over the box: whether it takes them. */
  protected readonly dropping = signal<'yes' | 'no' | null>(null);

  protected readonly palette = signal<'slash' | 'files' | null>(null);
  protected readonly paletteItems = signal<PaletteItem[]>([]);
  protected readonly active = signal(0);
  private fileQueryTimer?: ReturnType<typeof setTimeout>;
  private fileQuerySeq = 0;
  /** Bumped on every palette change: a late `/commands` answer checks it. */
  private slashSeq = 0;
  private draftTimer?: ReturnType<typeof setTimeout>;
  private nextId = 1;

  protected readonly session = this.ctx.session;
  protected readonly caps = this.ctx.caps;
  protected readonly coarse = this.viewport.coarse;
  /** Why another computer's session takes nothing from here now (offline,
   * or it lets this computer watch only); '' otherwise. */
  protected readonly machineBlock = computed(() => this.machine?.blocked() ?? '');
  /** Why it takes nothing while it lives on another computer, or moves to
   * one (docs/PLAN-MACHINES.md §5.7); '' otherwise. */
  protected readonly moveBlock = computed(() => {
    const s = this.session();
    if (!s) return '';
    // A move of it under way first: it says where it goes.
    const e = this.moves.current(s.id);
    if (e && e.move.role === 'out' && !e.move.asCopy && beforeLanded(e.move)) {
      const words = this.moves.words();
      return words.isHere(e.move.to)
        ? tr('Coming back to this {computer}: it takes messages once it is here.', {
            computer: computerNoun(),
          })
        : tr('Moving to {name}: it takes messages again once it is there.', {
            name: words.name(e.move.to),
          });
    }
    const lives = s.livesOn;
    if (lives) {
      if (lives.machine === this.moves.selfId()) {
        return tr('This session lives on this {computer} now: open it here to send.', {
          computer: computerNoun(),
        });
      }
      const peer = this.moves.peers().find((p) => p.id === lives.machine);
      const name = peer?.name ?? lives.name;
      if (this.machine) return tr('This session lives on {name} now: open it there.', { name });
      return peer && !this.moves.online(peer.id)
        ? tr('This session lives on {name}, which is offline: take it over to go on here.', {
            name,
          })
        : tr('This session lives on {name} now: open it there, or bring it back here.', { name });
    }
    return '';
  });
  protected readonly mode = computed<'message' | 'resume' | 'blocked'>(() => {
    const s = this.session();
    const c = this.caps();
    if (!s || !c || this.machineBlock() || this.moveBlock()) return 'blocked';
    if (s.live && c.message !== 'none') return 'message';
    if (!s.live && (c.continueOwned || c.fork)) return 'resume';
    return 'blocked';
  });
  protected readonly attach = computed(() =>
    this.mode() === 'message' ? capAttach(this.caps()) : 'none',
  );
  protected readonly relay = computed(() => this.caps()?.message === 'relay');
  protected readonly surfaceName = computed(() => tr(surfaceLabel(this.session()?.surface)));
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
    if (!this.ctx.canAct()) return tr('Reconnecting…');
    if (this.mode() === 'resume') return tr('Resume');
    if (this.onePermission() && this.hasContent()) return tr('Queue after this');
    return tr('Send');
  });
  /** A session Mother Claude runs: a message sent while it works is taken
   * into the running turn at its next step, and Send now hurries it. */
  protected readonly steers = computed(() => this.caps()?.channel === 'owned');
  protected readonly interruptLabel = computed(() =>
    this.caps()?.interrupt === 'next-tool' ? tr('Interrupt at next tool') : tr('Interrupt'),
  );
  protected readonly placeholder = computed(() =>
    this.mode() === 'resume' ? tr('Message to resume it here…') : tr('Message Claude…'),
  );
  protected readonly waitHint = computed(() => {
    const c = this.caps();
    if (!c || this.mode() !== 'message' || c.message !== 'relay') return '';
    if (c.channel === 'tmux') return tr('Typed into its tmux pane.');
    return capWake(c)
      ? tr("Delivered at Claude's next stopping point.")
      : tr('Waiting for the next step: delivered when Claude next stops or runs a tool.');
  });
  /** How to type here, under the box: with a keyboard the keys too; on a
   * touch screen the shortcuts only, where there are commands. */
  protected readonly keysHint = computed(() => {
    const slash = !!this.caps()?.slashCommands;
    if (this.coarse()) {
      return slash ? [tr('/ for commands'), tr('@ for files')].join(' · ') : '';
    }
    return [
      tr('Enter to send'),
      tr('Shift+Enter for a new line'),
      ...(slash ? [tr('/ for commands')] : []),
      tr('@ for files'),
      ...(this.attach() !== 'none' ? [tr('Paste or drop files to attach')] : []),
    ].join(' · ');
  });
  /** Why a pasted or dropped file can't go with a message here. */
  protected readonly noAttachWhy = computed(() =>
    this.mode() === 'resume'
      ? tr(
          'Send a message first to resume this session here: then you can attach images and files.',
        )
      : tr("This session can't take images or files from here."),
  );
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
    const seq = ++this.slashSeq;
    const before = this.beforeCaret(el);
    if (this.caps()?.slashCommands && SLASH_QUERY.test(before)) {
      void this.showCommands(before.slice(1).toLowerCase(), seq);
      return;
    }
    const at = /(?:^|\s)@([^\s@]*)$/.exec(before);
    if (at) {
      this.queryFiles(at[1]);
      return;
    }
    this.closePalette();
  }

  private beforeCaret(el: HTMLTextAreaElement): string {
    const value = el.value;
    return value.slice(0, el.selectionStart ?? value.length);
  }

  private async showCommands(q: string, seq: number): Promise<void> {
    const server = await this.ctx.loadCommands();
    // The first `/commands` can take a while over a tunnel: if the text
    // moved on meanwhile (the `/` deleted, a newer query), this answer is
    // stale and must not reopen the palette (Enter would then pick from it).
    const el = this.ta()?.nativeElement;
    if (seq !== this.slashSeq || !el || !SLASH_QUERY.test(this.beforeCaret(el))) return;
    const seen = new Set<string>();
    const all = [...builtinCommands(), ...server].filter((c) => {
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
    this.slashSeq++;
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
    const ask = confirmText(name);
    if (ask) {
      this.setText('');
      this.confirm.set({ name, text: ask });
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
    if (res) this.notices.success(tr('/{name} sent.', { name: c.name }));
  }

  // ─────────────────────────────────────────────────────── Attachments

  protected choose(kind: 'photo' | 'file'): void {
    (kind === 'photo' ? this.photoInput() : this.fileInput())?.nativeElement.click();
  }

  protected async onFiles(e: Event): Promise<void> {
    const input = e.target as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    await this.takeFiles(files);
  }

  /** A paste that carries files (a screenshot, a copied image or file)
   * attaches them; one that carries text pastes the text, as always. */
  protected onPaste(e: ClipboardEvent): void {
    const data = e.clipboardData;
    const files = transferFiles(data);
    if (!files.length || pasteMeansText(data?.getData('text/plain') ?? '', files)) return;
    e.preventDefault();
    const now = new Date();
    const named = files.map((f) => {
      const name = pastedName(f, now);
      return name === f.name
        ? f
        : new File([f], name, { type: f.type, lastModified: f.lastModified });
    });
    void this.dropIn(named, true);
  }

  protected onDragOver(e: DragEvent): void {
    if (!carriesFiles(e) || this.mode() === 'blocked') return;
    e.preventDefault();
    const takes = this.attach() !== 'none';
    if (e.dataTransfer) e.dataTransfer.dropEffect = takes ? 'copy' : 'none';
    this.dropping.set(takes ? 'yes' : 'no');
  }

  protected onDragLeave(e: DragEvent): void {
    const to = e.relatedTarget as Node | null;
    if (!to || !this.host.nativeElement.contains(to)) this.dropping.set(null);
  }

  protected onDrop(e: DragEvent): void {
    this.dropping.set(null);
    if (!carriesFiles(e) || this.mode() === 'blocked') return;
    e.preventDefault();
    void this.dropIn(transferFiles(e.dataTransfer), false);
  }

  /** Pasted or dropped files: attached as the buttons attach them, or why
   * not. */
  private async dropIn(files: readonly File[], atCaret: boolean): Promise<void> {
    if (!files.length) return;
    if (this.attach() === 'none') {
      this.notices.info(this.noAttachWhy());
      return;
    }
    await this.takeFiles(files, atCaret);
  }

  /** Images inline where the session takes them; anything else uploaded and
   * referenced as `@<path>` — where the caret is when `atCaret` (a paste),
   * else at the end. */
  private async takeFiles(files: readonly File[], atCaret = false): Promise<void> {
    for (const file of files) {
      if (this.attach() === 'inline' && file.type.startsWith('image/')) {
        await this.addImage(file);
      } else {
        await this.upload(file, atCaret);
      }
    }
  }

  private async addImage(file: File): Promise<void> {
    if (this.attachments().length >= MAX_IMAGES) {
      this.notices.warn(tr('At most {n} images per message.', { n: MAX_IMAGES }));
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

  private async upload(file: File, atCaret = false): Promise<void> {
    if (file.size > 50 * 1024 * 1024) {
      this.notices.warn(tr('{name} is over 50 MB.', { name: file.name }));
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
      if (!res?.path) throw new Error(tr('The server did not say where it stored the file.'));
      this.insertRef(`@${res.path} `, atCaret);
    } catch (err) {
      this.notices.error(err, undefined, tr('Could not attach {name}', { name: file.name }));
    } finally {
      this.uploading.update((n) => n - 1);
    }
  }

  /** Put a file's reference into the text: at the caret while the box has
   * the focus and `atCaret`, else at the end — spaced from its neighbours. */
  private insertRef(ref: string, atCaret: boolean): void {
    const cur = this.text();
    const el = this.ta()?.nativeElement;
    const here = atCaret && !!el && document.activeElement === el;
    const start = here ? el.selectionStart : cur.length;
    const end = here ? el.selectionEnd : cur.length;
    const before = cur.slice(0, start);
    const after = cur.slice(end);
    const sep = before && !/\s$/.test(before) ? ' ' : '';
    const put = `${before}${sep}${ref}`;
    this.setText(put + (/^\s/.test(after) ? after.slice(1) : after));
    if (here) el.setSelectionRange(put.length, put.length);
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

  protected async submit(how: 'send' | 'interrupt' | 'now' | 'deny'): Promise<void> {
    if (!this.canSend()) return;
    // What goes out; anything typed or attached while it is in flight stays.
    const sent: Sent = {
      session: this.ctx.id(),
      text: this.text(),
      attachments: new Set(this.attachments().map((a) => a.id)),
    };
    const text = sent.text.trim();
    const images = this.attachments().map(({ mediaType, data }) => ({ mediaType, data }));
    this.sending.set(true);
    this.failed.set(false);
    this.closePalette();
    try {
      if (this.mode() === 'resume') {
        if (await this.ctx.resumeWith(text)) this.cleared(sent);
        else this.failed.set(true);
        return;
      }
      if (how === 'deny') {
        const p = this.onePermission();
        if (!p) return;
        const res = await this.pendings
          .respond(p, { decision: 'deny', message: text }, tr('Denied with your note'))
          .catch((err: unknown) => {
            this.notices.error(err, undefined, tr('Not sent'));
            return undefined;
          });
        if (res) {
          this.notices.success(
            res.alreadyDecided ? res.text : tr('Denied, with your note to Claude.'),
          );
          if (!res.alreadyDecided) this.cleared(sent);
        } else if (res === undefined) this.failed.set(true);
        return;
      }
      if (how === 'interrupt' && !(await this.ctx.interrupt())) return;
      const res = await this.ctx.send(text, images);
      if (!res) {
        this.failed.set(true);
        return;
      }
      this.cleared(sent);
      if (res.delivery === 'queued' && how === 'now') {
        await this.ctx.sendNow(res.messageId);
      } else if (res.delivery === 'queued') {
        this.notices.info(
          this.steers()
            ? tr('Sent: Claude reads it after its current step.')
            : tr('Queued: Claude reads it when the current step ends.'),
        );
      } else if (res.delivery === 'relayed') {
        if (this.waitHint()) this.notices.info(this.waitHint());
        if (this.caps()?.channel === 'hooks' && storageGet(RELAY_EXPLAINED_KEY) !== '1') {
          this.explainer.set(true);
        }
      } else {
        this.notices.announce(tr('Message sent.'));
      }
    } finally {
      this.sending.set(false);
    }
  }

  protected dismissExplainer(): void {
    storageSet(RELAY_EXPLAINED_KEY, '1');
    this.explainer.set(false);
  }

  /** The message went out: take it out of the composer, keeping whatever
   * was typed or attached while the request was in flight (a slow tunnel). */
  private cleared(sent: Sent): void {
    if (this.ctx.id() !== sent.session) {
      // Another session's composer now: only its sent draft goes.
      storageRemove(DRAFT_PREFIX + sent.session);
      return;
    }
    this.setText(remainderAfterSend(this.text(), sent.text));
    if (!this.text().trim()) storageRemove(DRAFT_PREFIX + sent.session);
    for (const a of this.attachments()) if (sent.attachments.has(a.id)) URL.revokeObjectURL(a.url);
    this.attachments.update((prev) => prev.filter((a) => !sent.attachments.has(a.id)));
    this.failed.set(false);
  }
}
