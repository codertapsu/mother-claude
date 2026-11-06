// Key names for `press` (DOM `KeyboardEvent.key` names, plus the usual
// aliases) and what a keyboard event needs to look real: code and keyCode.

export type Modifier = 'Alt' | 'Control' | 'Meta' | 'Shift';

export interface KeyDef {
  key: string;
  code: string;
  keyCode: number;
  /** Text the key types, if any. */
  text?: string;
}

const NAMED: Record<string, KeyDef> = {
  Enter: { key: 'Enter', code: 'Enter', keyCode: 13, text: '\r' },
  Tab: { key: 'Tab', code: 'Tab', keyCode: 9 },
  Escape: { key: 'Escape', code: 'Escape', keyCode: 27 },
  Backspace: { key: 'Backspace', code: 'Backspace', keyCode: 8 },
  Delete: { key: 'Delete', code: 'Delete', keyCode: 46 },
  Insert: { key: 'Insert', code: 'Insert', keyCode: 45 },
  ArrowUp: { key: 'ArrowUp', code: 'ArrowUp', keyCode: 38 },
  ArrowDown: { key: 'ArrowDown', code: 'ArrowDown', keyCode: 40 },
  ArrowLeft: { key: 'ArrowLeft', code: 'ArrowLeft', keyCode: 37 },
  ArrowRight: { key: 'ArrowRight', code: 'ArrowRight', keyCode: 39 },
  Home: { key: 'Home', code: 'Home', keyCode: 36 },
  End: { key: 'End', code: 'End', keyCode: 35 },
  PageUp: { key: 'PageUp', code: 'PageUp', keyCode: 33 },
  PageDown: { key: 'PageDown', code: 'PageDown', keyCode: 34 },
  Space: { key: ' ', code: 'Space', keyCode: 32, text: ' ' },
  Shift: { key: 'Shift', code: 'ShiftLeft', keyCode: 16 },
  Control: { key: 'Control', code: 'ControlLeft', keyCode: 17 },
  Alt: { key: 'Alt', code: 'AltLeft', keyCode: 18 },
  Meta: { key: 'Meta', code: 'MetaLeft', keyCode: 91 },
  CapsLock: { key: 'CapsLock', code: 'CapsLock', keyCode: 20 },
  ContextMenu: { key: 'ContextMenu', code: 'ContextMenu', keyCode: 93 },
};
for (let i = 1; i <= 12; i++) NAMED[`F${i}`] = { key: `F${i}`, code: `F${i}`, keyCode: 111 + i };

const ALIASES: Record<string, string> = {
  esc: 'Escape',
  escape: 'Escape',
  return: 'Enter',
  enter: 'Enter',
  tab: 'Tab',
  up: 'ArrowUp',
  down: 'ArrowDown',
  left: 'ArrowLeft',
  right: 'ArrowRight',
  arrowup: 'ArrowUp',
  arrowdown: 'ArrowDown',
  arrowleft: 'ArrowLeft',
  arrowright: 'ArrowRight',
  del: 'Delete',
  delete: 'Delete',
  backspace: 'Backspace',
  space: 'Space',
  spacebar: 'Space',
  home: 'Home',
  end: 'End',
  pageup: 'PageUp',
  pagedown: 'PageDown',
  insert: 'Insert',
};

const MODIFIER_ALIASES: Record<string, Modifier | 'ControlOrMeta'> = {
  alt: 'Alt',
  option: 'Alt',
  opt: 'Alt',
  control: 'Control',
  ctrl: 'Control',
  meta: 'Meta',
  cmd: 'Meta',
  command: 'Meta',
  super: 'Meta',
  win: 'Meta',
  shift: 'Shift',
  controlormeta: 'ControlOrMeta',
  mod: 'ControlOrMeta',
};

const SYMBOL_CODES: Record<string, [string, number]> = {
  '-': ['Minus', 189],
  '=': ['Equal', 187],
  ',': ['Comma', 188],
  '.': ['Period', 190],
  '/': ['Slash', 191],
  ';': ['Semicolon', 186],
  "'": ['Quote', 222],
  '[': ['BracketLeft', 219],
  ']': ['BracketRight', 221],
  '\\': ['Backslash', 220],
  '`': ['Backquote', 192],
};

/** The definition of one printable character. */
export function charKey(ch: string): KeyDef {
  if (/^[a-z]$/i.test(ch)) {
    const upper = ch.toUpperCase();
    return { key: ch, code: `Key${upper}`, keyCode: upper.charCodeAt(0), text: ch };
  }
  if (/^[0-9]$/.test(ch))
    return { key: ch, code: `Digit${ch}`, keyCode: ch.charCodeAt(0), text: ch };
  if (ch === ' ') return { key: ' ', code: 'Space', keyCode: 32, text: ' ' };
  if (ch === '\n' || ch === '\r') return { key: 'Enter', code: 'Enter', keyCode: 13, text: '\r' };
  const sym = SYMBOL_CODES[ch];
  if (sym) return { key: ch, code: sym[0], keyCode: sym[1], text: ch };
  return { key: ch, code: '', keyCode: 0, text: ch };
}

export interface KeySpec {
  def: KeyDef;
  modifiers: Modifier[];
}

/**
 * "Enter", "a", "Shift+Tab", "ControlOrMeta+A" with optional extra modifiers.
 * Returns null for a key nobody knows.
 */
export function parseKey(
  key: string,
  extra: readonly string[] = [],
  isMac = false,
): KeySpec | null {
  const mods = new Set<Modifier>();
  const addMod = (raw: string): boolean => {
    const m = MODIFIER_ALIASES[raw.trim().toLowerCase()];
    if (!m) return false;
    mods.add(m === 'ControlOrMeta' ? (isMac ? 'Meta' : 'Control') : m);
    return true;
  };
  for (const m of extra) addMod(m);
  let name = key;
  if (key.length > 1 && key.includes('+')) {
    const parts = key.split('+');
    const last = parts.pop() ?? '';
    name = last === '' ? '+' : last;
    for (const p of parts) if (p && !addMod(p)) return null;
  }
  if (name.length === 1) {
    const def = charKey(name);
    if (mods.has('Shift') && /^[a-z]$/.test(name)) {
      return {
        def: { ...def, key: name.toUpperCase(), text: name.toUpperCase() },
        modifiers: [...mods],
      };
    }
    return { def, modifiers: [...mods] };
  }
  const named = NAMED[name] ?? NAMED[ALIASES[name.toLowerCase()] ?? ''];
  if (named) return { def: named, modifiers: [...mods] };
  const modAsKey = MODIFIER_ALIASES[name.toLowerCase()];
  if (modAsKey && modAsKey !== 'ControlOrMeta') {
    const def = NAMED[modAsKey];
    if (def) return { def, modifiers: [...mods] };
  }
  return null;
}

/** CDP's modifier bit mask: Alt 1, Control 2, Meta 4, Shift 8. */
export function modifierMask(mods: readonly Modifier[]): number {
  let m = 0;
  if (mods.includes('Alt')) m |= 1;
  if (mods.includes('Control')) m |= 2;
  if (mods.includes('Meta')) m |= 4;
  if (mods.includes('Shift')) m |= 8;
  return m;
}
