import assert from 'node:assert/strict';
import { afterEach, describe, it } from 'node:test';
import { find } from '../../src/content/find.ts';
import { mainContent, read } from '../../src/content/read.ts';
import { RefRegistry } from '../../src/content/refs.ts';
import { page, type TestPage } from './dom.ts';

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

const ARTICLE = `
  <header><nav><a href="/">Home</a> <a href="/blog">Blog</a></nav></header>
  <aside class="sidebar"><p>Subscribe to our newsletter for weekly updates, offers and more.</p></aside>
  <div id="post" class="post-content">
    <h1>Shipping the browser bridge</h1>
    <p>We built a <strong>small</strong> extension, and it <em>talks</em> to the app over a loopback socket, which keeps everything local.</p>
    <h2>Steps</h2>
    <ol><li>Pair</li><li>Open a <a href="/lease">lease</a></li></ol>
    <ul><li>Snapshots<ul><li>refs</li></ul></li></ul>
    <pre><code class="language-ts">const x = 1;
console.log(x);</code></pre>
    <blockquote><p>Local only, always.</p></blockquote>
    <table><tr><th>Action</th><th>Mode</th></tr><tr><td>read</td><td>observe | look</td></tr></table>
    <p>Inline <code>code</code> and an image: <img src="/a.png" alt="Diagram"></p>
  </div>
  <footer><p>Copyright notice that is long enough to be a paragraph, really it is.</p></footer>`;

describe('read: markdown', () => {
  it('finds the main content and converts it', () => {
    const p = load(ARTICLE, 'https://blog.example.com/posts/1');
    p.document.title = 'Bridge post';
    assert.equal(mainContent(p.document)?.id, 'post');
    const r = read(p.document, { mode: 'markdown' });
    // The content opens with its own H1, so the document title is not added.
    assert.equal(
      r.content,
      [
        '# Shipping the browser bridge',
        '',
        'We built a **small** extension, and it _talks_ to the app over a loopback socket, which keeps everything local.',
        '',
        '## Steps',
        '',
        '1. Pair',
        '2. Open a [lease](https://blog.example.com/lease)',
        '',
        '- Snapshots',
        '  - refs',
        '',
        '```ts',
        'const x = 1;',
        'console.log(x);',
        '```',
        '',
        '> Local only, always.',
        '',
        '| Action | Mode |',
        '| --- | --- |',
        '| read | observe \\| look |',
        '',
        'Inline `code` and an image: ![Diagram](https://blog.example.com/a.png)',
      ].join('\n'),
    );
    assert.ok(!r.content.includes('newsletter'));
    assert.ok(!r.content.includes('Copyright'));
  });

  it('starts with the document title when the content has no H1', () => {
    const p = load(`<article><p>${'Words, words. '.repeat(30)}</p></article>`);
    p.document.title = 'A page';
    assert.ok(
      read(p.document, { mode: 'markdown' }).content.startsWith('# A page\n\nWords, words.'),
    );
  });

  it('prefers <main> when it holds the content', () => {
    const body = 'x '.repeat(150);
    const p = load(`<nav>menu</nav><main id="m"><p>${body}</p></main>`);
    assert.equal(mainContent(p.document)?.id, 'm');
  });

  it('keeps page chrome when a selector is given', () => {
    const p = load(ARTICLE);
    const r = read(p.document, { mode: 'markdown', selector: 'header' });
    assert.equal(r.content, '[Home](https://example.com/) [Blog](https://example.com/blog)');
  });

  it('redacts [data-mc-redact] and truncates to maxChars', () => {
    const p = load(
      `<main><p>Balance: <span data-mc-redact>$12,345</span></p><p>${'y'.repeat(500)}</p></main>`,
    );
    const md = read(p.document, { mode: 'markdown', selector: 'main' });
    assert.ok(md.content.startsWith('Balance: «redacted»'));
    assert.ok(!md.content.includes('12,345'));
    const short = read(p.document, { mode: 'markdown', selector: 'main', maxChars: 50 });
    assert.equal(short.truncated, true);
    assert.equal(Array.from(short.content).length, 50);
  });
});

describe('read: text and html', () => {
  it('reads visible text with blocks on their own lines', () => {
    const p = load(
      `<h1>Title</h1><p>One <b>two</b></p><div style="display:none">hidden</div><div>Three<br>Four</div><span data-mc-redact>card 4242</span>`,
    );
    const r = read(p.document, { mode: 'text' });
    assert.equal(r.content, 'Title\n\nOne two\n\nThree\nFour\n«redacted»');
  });

  it('serializes HTML without scripts and with every secret value redacted', () => {
    const p = load(`
      <form id="f">
        <input name="user" value="ann">
        <input type="password" name="pw" value="hunter2">
        <input type="hidden" name="authenticity_token" value="tok123">
        <textarea name="notes">hello</textarea>
        <div data-mc-redact title="x">secret body</div>
        <script>window.SECRET = "s3"</script>
      </form>`);
    const html = read(p.document, { mode: 'html', selector: '#f' }).content;
    for (const leak of ['hunter2', 'tok123', 'secret body', 'SECRET', '<script']) {
      assert.ok(!html.includes(leak), `${leak} in ${html}`);
    }
    assert.match(html, /value="ann"/);
    assert.match(html, /«redacted»/);
    assert.match(html, /<textarea name="notes">hello<\/textarea>/);
  });
});

describe('find', () => {
  const FORM = `
    <main>
      <h2>Account</h2>
      <button><span>Sign in</span></button>
      <a href="/help">Trouble? Sign in help</a>
      <label>Email <input name="email"></label>
      <p>Plain text mentioning Sign in once more</p>
    </main>`;

  it('finds by text and lifts to the clickable ancestor', () => {
    const p = load(FORM);
    const refs = new RefRegistry();
    const m = find(p.document, refs, { text: 'sign in' });
    assert.deepEqual(
      m.map((x) => [x.role, x.name]),
      [
        ['button', 'Sign in'],
        ['link', 'Trouble? Sign in help'],
        ['paragraph', ''],
      ],
    );
    const first = m[0];
    assert.ok(first);
    const resolved = refs.resolve(first.ref);
    assert.equal(resolved.ok && resolved.el.localName, 'button');
  });

  it('finds by role and name, and by selector', () => {
    const p = load(FORM);
    const refs = new RefRegistry();
    const byRole = find(p.document, refs, { role: 'textbox', name: 'email' });
    assert.equal(byRole.length, 1);
    assert.equal(byRole[0]?.name, 'Email');
    const bySelector = find(p.document, refs, { selector: 'a[href]' });
    assert.equal(bySelector[0]?.role, 'link');
    assert.equal(find(p.document, refs, { role: 'heading', limit: 1 }).length, 1);
  });
});
