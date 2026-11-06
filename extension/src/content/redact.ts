// What the extension never sends: the value of a password field, of a field
// whose name, id, autocomplete or label says it holds a secret, and anything
// inside a [data-mc-redact] subtree (docs/BROWSER-PROTOCOL.md, "Snapshot
// format"). The desktop redacts again before anything reaches a model.

import { inputType, isElement, isInput, parentElementOrHost, tag } from './dom.ts';

export const REDACTED = '«redacted»';

/** Single words that mark a secret (matched as whole words after splitting). */
const SECRET_WORDS = new Set([
  'password',
  'passwd',
  'passcode',
  'passphrase',
  'pwd',
  'pin',
  'ssn',
  'cvc',
  'cvv',
  'cvv2',
  'csc',
  'otp',
  'totp',
  'token',
  'secret',
  'apikey',
  'ccnumber',
  'cardnumber',
  'ccnum',
  'cccsc',
  'onetimecode',
  'mfa',
]);

/** Phrases (normalized to single spaces) that mark a secret. */
const SECRET_PHRASES = [
  'cc number',
  'cc num',
  'card number',
  'credit card',
  'cc csc',
  'security code',
  'one time code',
  'one time password',
  'api key',
  'private key',
  'access key',
  'social security',
  'auth code',
  'verification code',
];

/** Autocomplete tokens that are secrets in themselves. */
const SECRET_AUTOCOMPLETE = new Set([
  'current-password',
  'new-password',
  'one-time-code',
  'cc-number',
  'cc-csc',
]);

/** "apiKey_value" → "api key value". */
export function words(s: string): string {
  return s
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim();
}

/** Does this attribute value (name, id, autocomplete, label) say "secret"? */
export function soundsSecret(value: string | null | undefined): boolean {
  if (!value) return false;
  const w = words(value);
  if (!w) return false;
  const list = w.split(' ');
  for (const token of list) if (SECRET_WORDS.has(token)) return true;
  const joined = list.join('');
  if (SECRET_WORDS.has(joined)) return true;
  const spaced = ` ${w} `;
  return SECRET_PHRASES.some((p) => spaced.includes(` ${p} `));
}

/** Is the element inside a [data-mc-redact] subtree (through shadow roots)? */
export function inRedactedSubtree(el: Element): boolean {
  for (let e: Element | null = el; e; e = parentElementOrHost(e)) {
    if (e.hasAttribute('data-mc-redact')) return true;
  }
  return false;
}

/** A form control whose value must never leave the page. */
export function isSecretField(el: Element): boolean {
  if (!isElement(el)) return false;
  const t = tag(el);
  if (isInput(el)) {
    const type = inputType(el);
    if (type === 'password') return true;
    if (type === 'checkbox' || type === 'radio' || type === 'button' || type === 'submit')
      return inRedactedSubtree(el);
  } else if (t !== 'textarea' && t !== 'select' && !el.hasAttribute('contenteditable')) {
    const role = el.getAttribute('role');
    if (role !== 'textbox' && role !== 'searchbox' && role !== 'combobox') return false;
  }
  const autocomplete = el.getAttribute('autocomplete');
  if (autocomplete) {
    for (const token of autocomplete.toLowerCase().split(/\s+/)) {
      if (SECRET_AUTOCOMPLETE.has(token)) return true;
    }
  }
  if (soundsSecret(el.getAttribute('name')) || soundsSecret(el.getAttribute('id'))) return true;
  if (soundsSecret(autocomplete)) return true;
  if (soundsSecret(el.getAttribute('aria-label'))) return true;
  const labels = (el as HTMLInputElement).labels;
  if (labels) {
    for (const label of Array.from(labels)) if (soundsSecret(label.textContent)) return true;
  }
  return inRedactedSubtree(el);
}
