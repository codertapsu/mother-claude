import assert from 'node:assert/strict';
import { afterEach, describe, it } from 'node:test';
import { RefRegistry } from '../../src/content/refs.ts';
import { SelectorNotFound, snapshot } from '../../src/content/snapshot.ts';
import { page, settle, type TestPage } from './dom.ts';

let open: TestPage[] = [];
function load(html: string, url?: string): TestPage {
  const p = page(html, url);
  open.push(p);
  return p;
}
afterEach(async () => {
  await Promise.all(open.map((p) => p.close()));
  open = [];
});

describe('snapshot format', () => {
  it('renders the protocol example exactly', () => {
    const p = load(`
      <header>
        <a href="/">Mother Claude</a>
        <nav aria-label="Main"><a href="/issues">Issues</a></nav>
      </header>
      <main>
        <h1>New issue</h1>
        <label for="t">Title</label><input id="t">
        <label for="p">Password</label><input id="p" type="password" value="hunter2">
        <button disabled>Submit new issue</button>
        <iframe title="Captcha" src="https://captcha.example.net/"></iframe>
      </main>`);
    const r = snapshot(p.document, new RefRegistry());
    assert.equal(
      r.snapshot,
      [
        '- banner',
        '  - link "Mother Claude" [ref=e1]',
        '  - navigation "Main"',
        '    - link "Issues" [ref=e2]',
        '- main',
        '  - heading "New issue" [level=1]',
        '  - textbox "Title" [ref=e3] value=""',
        '  - textbox "Password" [ref=e4] value=«redacted»',
        '  - button "Submit new issue" [ref=e5] [disabled]',
        '  - iframe "Captcha" (cross-origin, not read)',
      ].join('\n'),
    );
    assert.equal(r.truncated, false);
    assert.equal(r.nodes, 10);
    assert.ok(!r.snapshot.includes('hunter2'));
  });

  it('shows text that no name already tells, and folds a paragraph', () => {
    const p = load(
      `<main><p>Hello <b>world</b></p><div>Loose text</div><ul><li>One</li><li>Two</li></ul></main>`,
    );
    const r = snapshot(p.document, new RefRegistry());
    assert.equal(
      r.snapshot,
      [
        '- main',
        '  - paragraph "Hello world"',
        '  - text "Loose text"',
        '  - list',
        '    - listitem "One"',
        '    - listitem "Two"',
      ].join('\n'),
    );
  });

  it('collapses a nameless list item around a single link', () => {
    const p = load(
      `<nav aria-label="Main"><ul><li><a href="/a">A</a></li><li><a href="/b">B</a> (new)</li></ul></nav>`,
    );
    const r = snapshot(p.document, new RefRegistry());
    assert.equal(
      r.snapshot,
      [
        '- navigation "Main"',
        '  - list',
        '    - link "A" [ref=e1]',
        '    - listitem',
        '      - link "B" [ref=e2]',
        '      - text "(new)"',
      ].join('\n'),
    );
  });

  it('truncates names to 120 characters and escapes quotes', () => {
    const long = 'x'.repeat(200);
    const p = load(`<button>${long}</button><a href="/q">Say "hi"</a>`);
    const r = snapshot(p.document, new RefRegistry());
    const [first, second] = r.snapshot.split('\n');
    assert.equal(first, `- button "${'x'.repeat(119)}…" [ref=e1]`);
    assert.equal(second, '- link "Say \\"hi\\"" [ref=e2]');
  });

  it('carries states: checked, expanded, selected, required, readonly, invalid, pressed', () => {
    const p = load(`
      <input type="checkbox" id="c" checked><label for="c">Agree</label>
      <button aria-expanded="false">Menu</button>
      <button aria-pressed="true">Bold</button>
      <div role="tablist"><div role="tab" aria-selected="true" tabindex="0">One</div></div>
      <label>Email <input type="email" required readonly aria-invalid="true" value="a@b.c"></label>
      <select aria-label="Size"><option>S</option><option selected>M</option></select>`);
    const lines = snapshot(p.document, new RefRegistry()).snapshot.split('\n');
    assert.deepEqual(lines, [
      '- checkbox "Agree" [ref=e1] [checked]',
      '- button "Menu" [ref=e2] [expanded=false]',
      '- button "Bold" [ref=e3] [pressed]',
      '- tablist',
      '  - tab "One" [ref=e4] [selected]',
      '- textbox "Email" [ref=e5] [required] [readonly] [invalid] value="a@b.c"',
      '- combobox "Size" [ref=e6] value="M"',
      '  - option "S"',
      '  - option "M" [selected]',
    ]);
  });

  it('skips what is not rendered: display none, [hidden], aria-hidden, closed details', () => {
    const p = load(`
      <style>.gone { display: none }</style>
      <button class="gone">A</button>
      <button hidden>B</button>
      <div aria-hidden="true"><button>C</button></div>
      <details><summary>More</summary><button>D</button></details>
      <button>E</button>`);
    const r = snapshot(p.document, new RefRegistry());
    assert.equal(
      r.snapshot,
      ['- button "More" [ref=e1] [expanded=false]', '- button "E" [ref=e2]'].join('\n'),
    );
  });
});

describe('refs', () => {
  it('keeps the same ref for the same element and never reuses one', () => {
    const p = load(`<button id="a">A</button><button id="b">B</button>`);
    const refs = new RefRegistry();
    const first = snapshot(p.document, refs).snapshot;
    assert.match(first, /"A" \[ref=e1\]/);
    p.document.getElementById('a')?.remove();
    const added = p.document.createElement('button');
    added.textContent = 'C';
    p.document.body.append(added);
    const second = snapshot(p.document, refs).snapshot;
    assert.equal(second, ['- button "B" [ref=e2]', '- button "C" [ref=e3]'].join('\n'));
    const gone = refs.resolve('e1');
    assert.equal(gone.ok, false);
    assert.equal(gone.ok === false && gone.code, 'stale_ref');
    const b = refs.resolve('e2');
    assert.equal(b.ok && b.el.id, 'b');
  });

  it('continues the tab numbering in a new document', () => {
    const p = load(`<button>A</button>`);
    const refs = new RefRegistry();
    refs.continueFrom(40, 3);
    assert.equal(snapshot(p.document, refs).snapshot, '- button "A" [ref=e40]');
    assert.deepEqual(refs.counters, { nextRef: 41, nextFrame: 3 });
    const unknown = refs.resolve('e12');
    assert.equal(unknown.ok === false && unknown.code, 'stale_ref');
    const bad = refs.resolve('button');
    assert.equal(bad.ok === false && bad.code, 'not_found');
  });
});

describe('redaction', () => {
  it('redacts secret-looking fields by type, name, id, autocomplete and label', () => {
    const p = load(`
      <input aria-label="One" type="password" value="p1">
      <input aria-label="Two" name="api_key" value="k1">
      <input aria-label="Three" id="userToken" value="t1">
      <input aria-label="Four" autocomplete="cc-number" value="4242">
      <input aria-label="Five" autocomplete="one-time-code" value="123456">
      <label>SSN <input value="078-05-1120"></label>
      <input aria-label="Six" name="shipping" value="kept">
      <div data-mc-redact><input aria-label="Seven" value="s1"><span>secret text</span></div>`);
    const r = snapshot(p.document, new RefRegistry()).snapshot;
    for (const secret of ['p1', 'k1', 't1', '4242', '123456', '078-05-1120', 's1', 'secret text']) {
      assert.ok(!r.includes(secret), `${secret} leaked:\n${r}`);
    }
    assert.match(r, /textbox "Six" \[ref=e7\] value="kept"/);
    assert.match(r, /textbox «redacted» \[ref=e8\] value=«redacted»/);
    assert.match(r, /- text «redacted»/);
  });
});

describe('shadow DOM and frames', () => {
  it('includes open shadow roots and slotted content', () => {
    const p = load(`<div id="host"><span slot="label">Slotted</span></div>`);
    const host = p.document.getElementById('host');
    const root = host?.attachShadow({ mode: 'open' });
    if (root)
      root.innerHTML = `<button><slot name="label"></slot></button><input aria-label="Inner">`;
    const r = snapshot(p.document, new RefRegistry()).snapshot;
    assert.equal(
      r,
      ['- button "Slotted" [ref=e1]', '- textbox "Inner" [ref=e2] value=""'].join('\n'),
    );
  });

  it('gives same-origin frames their own ref prefix', async () => {
    const p = load(
      `<button>Top</button><iframe title="Inner" srcdoc="<button>Framed</button>"></iframe>`,
    );
    await settle(p);
    const refs = new RefRegistry();
    const r = snapshot(p.document, refs).snapshot;
    assert.equal(
      r,
      ['- button "Top" [ref=e1]', '- iframe "Inner"', '  - button "Framed" [ref=f1e2]'].join('\n'),
    );
    const framed = refs.resolve('f1e2');
    assert.equal(framed.ok && framed.el.textContent, 'Framed');
  });
});

describe('options', () => {
  const FORM = `
    <header><a href="/">Home</a></header>
    <main>
      <h1>Title</h1><p>Some prose that is not interactive.</p>
      <form aria-label="Search"><input aria-label="Query"><button>Go</button></form>
      <ul><li>Plain</li><li><a href="/x">X</a></li></ul>
    </main>`;

  it('interactiveOnly keeps interactive nodes and the containers that place them', () => {
    const p = load(FORM);
    const r = snapshot(p.document, new RefRegistry(), { interactiveOnly: true });
    assert.equal(
      r.snapshot,
      [
        '- banner',
        '  - link "Home" [ref=e1]',
        '- main',
        '  - form "Search"',
        '    - textbox "Query" [ref=e2] value=""',
        '    - button "Go" [ref=e3]',
        '  - link "X" [ref=e4]',
      ].join('\n'),
    );
  });

  it('maxNodes cuts the tree and says so', () => {
    const items = Array.from(
      { length: 50 },
      (_, i) => `<li><a href="/${i}">Item ${i}</a></li>`,
    ).join('');
    const p = load(`<ul>${items}</ul>`);
    const r = snapshot(p.document, new RefRegistry(), { maxNodes: 10 });
    assert.equal(r.truncated, true);
    assert.equal(r.nodes, 10);
    assert.equal(r.snapshot.split('\n').length, 10);
  });

  it('scopes to a selector, and reports a selector that matches nothing', () => {
    const p = load(FORM);
    const r = snapshot(p.document, new RefRegistry(), { selector: 'form' });
    assert.equal(r.snapshot.split('\n')[0], '- form "Search"');
    assert.throws(
      () => snapshot(p.document, new RefRegistry(), { selector: '#nope' }),
      SelectorNotFound,
    );
  });
});
