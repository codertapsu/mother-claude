import assert from 'node:assert/strict';
import { afterEach, describe, it } from 'node:test';
import { accessibleName } from '../../src/content/accname.ts';
import { implicitRole, isInteractive, roleOf } from '../../src/content/roles.ts';
import { page, type TestPage } from './dom.ts';

let open: TestPage[] = [];
function name(html: string, selector = '#t'): string {
  const p = page(html);
  open.push(p);
  const el = p.document.querySelector(selector);
  assert.ok(el, `no ${selector}`);
  return accessibleName(el);
}
afterEach(async () => {
  await Promise.all(open.map((p) => p.close()));
  open = [];
});

describe('accessible name', () => {
  it('prefers aria-labelledby, in order, over everything else', () => {
    assert.equal(
      name(
        `<span id="a">Delete</span><span id="b">draft</span><button id="t" aria-labelledby="a b" aria-label="x">y</button>`,
      ),
      'Delete draft',
    );
  });

  it('reads aria-labelledby targets even when they are hidden', () => {
    assert.equal(
      name(`<span id="h" hidden>Secret label</span><input id="t" aria-labelledby="h">`),
      'Secret label',
    );
  });

  it('uses aria-label next', () => {
    assert.equal(name(`<button id="t" aria-label="Close dialog">×</button>`), 'Close dialog');
  });

  it('uses <label for> and a wrapping label', () => {
    assert.equal(name(`<label for="t">First name</label><input id="t">`), 'First name');
    assert.equal(name(`<label>Remember me <input id="t" type="checkbox"></label>`), 'Remember me');
  });

  it('does not put a field’s own value into its name', () => {
    assert.equal(name(`<label>City <input id="t" value="Paris"></label>`), 'City');
  });

  it('uses alt for images and image buttons', () => {
    assert.equal(name(`<img id="t" src="a.png" alt="A cat">`), 'A cat');
    assert.equal(name(`<input id="t" type="image" alt="Search">`), 'Search');
  });

  it('uses the text content of buttons, links and headings, skipping hidden parts', () => {
    assert.equal(
      name(
        `<a id="t" href="/">Open <span style="display:none">never</span><img alt="settings" src="x.png"></a>`,
      ),
      'Open settings',
    );
    assert.equal(name(`<h2 id="t">Recent <em>activity</em></h2>`), 'Recent activity');
  });

  it('falls back to title, then placeholder', () => {
    assert.equal(
      name(`<input id="t" title="Search the site" placeholder="Search…">`),
      'Search the site',
    );
    assert.equal(name(`<input id="t" placeholder="Search…">`), 'Search…');
  });

  it('names fieldsets, figures and tables from legend, figcaption and caption', () => {
    assert.equal(name(`<fieldset id="t"><legend>Shipping</legend><input></fieldset>`), 'Shipping');
    assert.equal(
      name(`<figure id="t"><img alt=""><figcaption>Chart 1</figcaption></figure>`),
      'Chart 1',
    );
    assert.equal(
      name(`<table id="t"><caption>Prices</caption><tr><td>1</td></tr></table>`),
      'Prices',
    );
  });

  it('gives submit and reset inputs their default names', () => {
    assert.equal(name(`<input id="t" type="submit">`), 'Submit');
    assert.equal(name(`<input id="t" type="reset" value="Clear form">`), 'Clear form');
  });

  it('includes an embedded control’s value when naming from content', () => {
    assert.equal(
      name(
        `<div id="t" role="button">Rows per page <select><option>10</option><option selected>25</option></select></div>`,
      ),
      'Rows per page 25',
    );
  });

  it('never puts a password into a name', () => {
    assert.equal(
      name(`<div id="t" role="button">PIN <input type="password" value="1234"></div>`),
      'PIN',
    );
  });
});

describe('roles', () => {
  it('maps HTML to implicit roles', () => {
    const p = page(`
      <a id="a" href="/">x</a><a id="a2">x</a><input id="s" type="search"><input id="r" type="range">
      <select id="sel"></select><select id="multi" multiple></select><header id="h"></header>
      <article><header id="h2"></header></article><section id="sec"></section><section id="sec2" aria-label="x"></section>
      <img id="deco" alt=""><img id="pic" alt="x"><div id="ce" contenteditable="true"></div>`);
    open.push(p);
    const role = (id: string): string | null => {
      const el = p.document.getElementById(id);
      assert.ok(el);
      return roleOf(el);
    };
    assert.equal(role('a'), 'link');
    assert.equal(role('a2'), null);
    assert.equal(role('s'), 'searchbox');
    assert.equal(role('r'), 'slider');
    assert.equal(role('sel'), 'combobox');
    assert.equal(role('multi'), 'listbox');
    assert.equal(role('h'), 'banner');
    assert.equal(role('h2'), null);
    assert.equal(role('sec'), null);
    assert.equal(role('sec2'), 'region');
    assert.equal(role('deco'), null);
    assert.equal(role('pic'), 'img');
    assert.equal(role('ce'), 'textbox');
  });

  it('honours explicit roles, but keeps a focusable element’s own role', () => {
    const p = page(
      `<div id="d" role="button">x</div><button id="b" role="presentation">x</button><span id="g" role="none">x</span>`,
    );
    open.push(p);
    const d = p.document.getElementById('d');
    const b = p.document.getElementById('b');
    const g = p.document.getElementById('g');
    assert.ok(d && b && g);
    assert.equal(roleOf(d), 'button');
    // role=presentation on something focusable is ignored (ARIA conflict resolution).
    assert.equal(roleOf(b), 'button');
    assert.equal(roleOf(g), null);
    assert.equal(implicitRole(b), 'button');
  });

  it('treats tabindex, onclick and contenteditable as interactive', () => {
    const p = page(
      `<div id="t" tabindex="0">x</div><div id="o" onclick="go()">x</div><div id="n">x</div>`,
    );
    open.push(p);
    const el = (id: string): Element => {
      const e = p.document.getElementById(id);
      assert.ok(e);
      return e;
    };
    assert.equal(isInteractive(el('t'), null), true);
    assert.equal(isInteractive(el('o'), null), true);
    assert.equal(isInteractive(el('n'), null), false);
  });
});
