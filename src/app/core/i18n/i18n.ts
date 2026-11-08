import { signal } from '@angular/core';

import { storageGet, storageSet } from '../platform';

/**
 * The languages of the interface (docs/I18N.md). English is the source: a
 * string's English text is its key, so English needs no dictionary and
 * anything not yet translated shows in English. Each device keeps its own
 * choice (local storage); the desktop app and a paired phone may differ.
 */
export type Lang = 'en' | 'vi';

export interface LangOption {
  id: Lang;
  /** Its name in English. */
  label: string;
  /** Its name in itself, as the switcher shows it. */
  native: string;
}

export const LANGS: readonly LangOption[] = [
  { id: 'en', label: 'English', native: 'English' },
  { id: 'vi', label: 'Vietnamese', native: 'Tiếng Việt' },
];

/** English text → its translation. */
export type Dict = Readonly<Record<string, string>>;

export type Params = Readonly<Record<string, unknown>>;

const KEY = 'mc_lang';

const langSig = signal<Lang>('en');
const dictSig = signal<Dict>({});

/** The language shown now. Reading it in a template or a `computed` makes
 * it follow a switch. */
export const language = langSig.asReadonly();

/**
 * `text` in the language shown, its `{name}` placeholders filled from
 * `params`. The key is the English text; `||` starts a note that tells two
 * same English strings apart (`'Restore||verb'`) and is never shown. Reads
 * the language signal, so a template or a `computed` that calls it updates
 * when the language changes.
 */
export function tr(text: string, params?: Params | null): string {
  const dict = dictSig();
  langSig();
  const found = dict[text];
  const shown = found ?? stripNote(text);
  return params ? fill(shown, params) : shown;
}

/** `{name}` placeholders replaced by `params[name]`; unknown ones stay. */
export function fill(text: string, params: Params): string {
  return text.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name] ?? '') : whole,
  );
}

function stripNote(text: string): string {
  const at = text.indexOf('||');
  return at >= 0 ? text.slice(0, at) : text;
}

/** The language this device chose, else English. */
export function storedLanguage(): Lang {
  const v = storageGet(KEY);
  return LANGS.some((l) => l.id === v) ? (v as Lang) : 'en';
}

/** Show `lang`: its dictionary loaded (once), `<html lang>` set, and the
 * choice kept on this device when `remember`. */
export async function useLanguage(lang: Lang, remember = false): Promise<void> {
  const dict = await dictionaryOf(lang);
  dictSig.set(dict);
  langSig.set(lang);
  try {
    document.documentElement.lang = lang;
  } catch {
    /* no document (tests on the server) */
  }
  if (remember) storageSet(KEY, lang);
}

const loaded = new Map<Lang, Dict>([['en', {}]]);

async function dictionaryOf(lang: Lang): Promise<Dict> {
  const have = loaded.get(lang);
  if (have) return have;
  // Each language is its own chunk: English never downloads Vietnamese.
  const dict: Dict = lang === 'vi' ? (await import('../../i18n/vi')).VI : {};
  loaded.set(lang, dict);
  return dict;
}

/** The locale dates and numbers are written in. */
export function locale(): string {
  return langSig() === 'vi' ? 'vi-VN' : 'en-US';
}

// ─────────────────────────────── The desktop's computer, in words

/** The desktop's system, as `GET /me` says it (AuthService keeps it here,
 * for code without injection). */
export const hostOsSignal = signal<'macos' | 'windows' | 'linux' | undefined>(undefined);

/**
 * The desktop's computer as a noun, translated: "Mac", "PC" or "computer"
 * (Vietnamese "máy Mac", "máy PC", "máy tính"). For sentences such as
 * `tr('Answer it on the {computer}.', { computer: computerNoun() })`.
 */
export function computerNoun(): string {
  const os = hostOsSignal();
  return tr(os === 'macos' ? 'Mac' : os === 'windows' ? 'PC' : 'computer');
}

/** Where its files are shown: "Finder", "File Explorer" or "the file
 * manager". */
export function fileManagerName(): string {
  const os = hostOsSignal();
  return os === 'macos'
    ? 'Finder'
    : os === 'windows'
      ? tr('File Explorer')
      : tr('the file manager');
}
