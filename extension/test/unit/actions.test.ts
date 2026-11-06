import assert from 'node:assert/strict';
import { afterEach, describe, it } from 'node:test';
import {
  click,
  fillField,
  press,
  selectOptions,
  setChecked,
  type as typeText,
  upload,
} from '../../src/content/actions.ts';
import { CommandError } from '../../src/shared/errors.ts';
import { parseKey } from '../../src/shared/keys.ts';
import { page, type TestPage } from './dom.ts';

let open: TestPage[] = [];
function load(html: string): TestPage {
  const p = page(html);
  open.push(p);
  return p;
}
afterEach(async () => {
  await Promise.all(open.map((p) => p.close()));
  open = [];
});

function get<T extends Element = HTMLInputElement>(p: TestPage, sel: string): T {
  const el = p.document.querySelector(sel);
  assert.ok(el, sel);
  return el as unknown as T;
}

/**
 * React's trick, reproduced: an own `value` property on the element that
 * records what the framework last saw. A plain `el.value = x` goes through
 * it (and React then ignores the next input event); the prototype setter
 * does not.
 */
function reactLike(input: HTMLInputElement): { seen: string[]; tracker: () => string } {
  const proto = Object.getPrototypeOf(input) as object;
  const desc =
    Object.getOwnPropertyDescriptor(proto, 'value') ??
    Object.getOwnPropertyDescriptor(Object.getPrototypeOf(proto) as object, 'value');
  assert.ok(desc?.get && desc.set);
  let tracked = input.value;
  Object.defineProperty(input, 'value', {
    configurable: true,
    get() {
      return desc.get?.call(this) as string;
    },
    set(v: string) {
      tracked = v;
      desc.set?.call(this, v);
    },
  });
  const seen: string[] = [];
  input.addEventListener('input', () => {
    const now = desc.get?.call(input) as string;
    if (now !== tracked) {
      tracked = now;
      seen.push(now);
    }
  });
  return { seen, tracker: () => tracked };
}

describe('filling and typing', () => {
  it('fill sets the value so a React-style tracker sees the change', () => {
    const p = load(`<input id="q">`);
    const input = get(p, '#q');
    const react = reactLike(input);
    let changes = 0;
    input.addEventListener('change', () => changes++);
    fillField(input, 'hello', 'e1');
    assert.equal(input.value, 'hello');
    assert.deepEqual(react.seen, ['hello']);
    assert.equal(changes, 1);
  });

  it('type sends key events around each character', () => {
    const p = load(`<input id="q" value="ab">`);
    const input = get(p, '#q');
    const keys: string[] = [];
    input.addEventListener('keydown', (e) => keys.push((e as KeyboardEvent).key));
    typeText(input, 'cd', { clear: false, what: 'e1' });
    assert.equal(input.value, 'abcd');
    assert.deepEqual(keys, ['c', 'd']);
    typeText(input, 'x', { clear: true, what: 'e1' });
    assert.equal(input.value, 'x');
  });

  it('refuses what is not a text field, or is read-only', () => {
    const p = load(`<button id="b">x</button><input id="r" readonly>`);
    assert.throws(() => typeText(get(p, '#b'), 'x', { what: 'e1' }), CommandError);
    assert.throws(() => typeText(get(p, '#r'), 'x', { what: 'e2' }), /read-only/);
  });

  it('a page that cancels keydown gets no text, as with a real keyboard', () => {
    const p = load(`<input id="q">`);
    const input = get(p, '#q');
    input.addEventListener('keydown', (e) => {
      if ((e as KeyboardEvent).key === 'x') e.preventDefault();
    });
    typeText(input, 'axb', { what: 'e1' });
    assert.equal(input.value, 'ab');
  });
});

describe('select and check', () => {
  it('selects by value, then by visible label', () => {
    const p = load(
      `<select id="s"><option value="fr">France</option><option value="de">Germany</option></select>`,
    );
    const select = get<HTMLSelectElement>(p, '#s');
    let changes = 0;
    select.addEventListener('change', () => changes++);
    assert.deepEqual(selectOptions(select, ['Germany'], 'e1'), ['de']);
    assert.equal(select.value, 'de');
    assert.deepEqual(selectOptions(select, ['fr'], 'e1'), ['fr']);
    assert.equal(changes, 2);
    assert.throws(
      () => selectOptions(select, ['Spain'], 'e1'),
      /no option "Spain" in e1; it has "France", "Germany"/,
    );
  });

  it('checks a checkbox by clicking it, and will not uncheck a radio', () => {
    const p = load(`<input id="c" type="checkbox"><input id="r" type="radio" name="g" checked>`);
    const box = get(p, '#c');
    let changed = 0;
    box.addEventListener('change', () => changed++);
    setChecked(box, true, 'e1');
    assert.equal(box.checked, true);
    assert.equal(changed, 1);
    setChecked(box, true, 'e1');
    assert.equal(changed, 1);
    assert.throws(() => setChecked(get(p, '#r'), false, 'e2'), /radio/);
  });

  it('reports a page that ignores the synthetic click', () => {
    const p = load(`<input id="c" type="checkbox">`);
    const box = get(p, '#c');
    box.addEventListener('click', (e) => {
      if (!e.isTrusted) e.preventDefault();
    });
    assert.throws(() => setChecked(box, true, 'e1'), /trusted/);
  });
});

describe('clicks, keys and uploads', () => {
  it('a click focuses, and fires the mouse sequence in order', () => {
    const p = load(`<button id="b">Go</button>`);
    const b = get<HTMLButtonElement>(p, '#b');
    const seen: string[] = [];
    for (const t of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {
      b.addEventListener(t, () => seen.push(t));
    }
    click(b);
    assert.deepEqual(seen, ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']);
    assert.equal(p.document.activeElement, b);
  });

  it('a link to a new tab is reported instead of being blocked as a popup', () => {
    const p = load(`<a id="a" href="https://example.com/next" target="_blank">Next</a>`);
    assert.deepEqual(click(get(p, '#a')), { openUrl: 'https://example.com/next' });
  });

  it('Enter in a field submits its form through the default button', () => {
    const p = load(`<form id="f"><input id="q"><button id="go">Go</button></form>`);
    const form = get<HTMLFormElement>(p, '#f');
    let submitted = 0;
    let clicked = 0;
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      submitted++;
    });
    get(p, '#go').addEventListener('click', () => clicked++);
    get(p, '#q').focus();
    const enter = parseKey('Enter');
    assert.ok(enter);
    press(p.document, enter);
    assert.equal(clicked, 1);
    assert.equal(submitted, 1);
  });

  it('Tab moves the focus, Shift+Tab moves it back', () => {
    const p = load(`<input id="a"><input id="b">`);
    get(p, '#a').focus();
    const tab = parseKey('Tab');
    const back = parseKey('Shift+Tab');
    assert.ok(tab && back);
    press(p.document, tab);
    assert.equal(p.document.activeElement?.id, 'b');
    press(p.document, back);
    assert.equal(p.document.activeElement?.id, 'a');
  });

  it('upload puts the bytes the desktop sent into the file input', () => {
    const p = load(`<input id="f" type="file">`);
    const input = get(p, '#f');
    let changed = 0;
    input.addEventListener('change', () => changed++);
    const n = upload(
      input,
      [{ name: 'a.txt', mimeType: 'text/plain', data: Buffer.from('hello').toString('base64') }],
      'e1',
    );
    assert.equal(n, 1);
    assert.equal(input.files?.length, 1);
    assert.equal(input.files?.[0]?.name, 'a.txt');
    assert.equal(changed, 1);
    assert.throws(
      () =>
        upload(
          input,
          [
            { name: 'a', data: 'YQ==' },
            { name: 'b', data: 'Yg==' },
          ],
          'e1',
        ),
      /takes one file/,
    );
  });
});
