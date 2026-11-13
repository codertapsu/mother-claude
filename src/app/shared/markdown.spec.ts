import { SecurityContext } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { DomSanitizer } from '@angular/platform-browser';

import { renderMarkdown } from './markdown';

/** The rendered HTML as an inert fragment (a template never loads or runs
 * anything), for questions about its structure. */
function fragment(html: string): DocumentFragment {
  const t = document.createElement('template');
  t.innerHTML = html;
  return t.content;
}

const texts = (html: string, selector: string): string[] =>
  Array.from(fragment(html).querySelectorAll(selector)).map((e) => e.textContent ?? '');

/** Everything markdown.ts may write, and nothing else. */
const ELEMENTS = new Set(
  'P BR H3 H4 H5 H6 HR STRONG EM DEL CODE PRE A SPAN DIV UL OL LI BLOCKQUOTE TABLE THEAD TBODY TR TH TD'.split(
    ' ',
  ),
);
const ATTRIBUTES = new Set(['class', 'role', 'tabindex', 'aria-label', 'title', 'start', 'href']);
const LINK_ATTRIBUTES = new Set(['class', 'href', 'target', 'rel']);

/** Only known elements and attributes, links only to http(s) and opened
 * apart (`noopener`). */
function expectInert(html: string): void {
  for (const el of Array.from(fragment(html).querySelectorAll('*'))) {
    expect(ELEMENTS.has(el.tagName), `<${el.tagName}>`).toBe(true);
    for (const a of Array.from(el.attributes)) {
      const allowed = el.tagName === 'A' ? LINK_ATTRIBUTES : ATTRIBUTES;
      expect(allowed.has(a.name), `${el.tagName} ${a.name}="${a.value}"`).toBe(true);
    }
    if (el.tagName === 'A') {
      expect(el.getAttribute('href')).toMatch(/^https?:\/\/\S+$/);
      expect(el.getAttribute('rel')).toBe('noopener noreferrer');
      expect(el.getAttribute('target')).toBe('_blank');
    }
  }
}

describe('renderMarkdown', () => {
  it('escapes HTML so transcript content cannot inject tags', () => {
    const html = renderMarkdown('<script>alert(1)</script> & <b>bold?</b>');
    expect(html).not.toContain('<script>');
    expect(html).toContain('&lt;script&gt;');
    expect(html).toContain('&amp;');
  });

  it('renders headings, bold, italics, and inline code', () => {
    const html = renderMarkdown('## Title\n\nUse **bold** and *soft* with `npm ci`.');
    expect(html).toContain('<h4>Title</h4>'); // headings shifted down two levels
    expect(html).toContain('<strong>bold</strong>');
    expect(html).toContain('<em>soft</em>');
    expect(html).toContain('<code>npm ci</code>');
  });

  it('keeps markdown inside inline code literal', () => {
    const html = renderMarkdown('run `a ** b` now');
    expect(html).toContain('<code>a ** b</code>');
    expect(html).not.toContain('<strong>');
  });

  it('does not mangle plain numbers between spaces', () => {
    expect(renderMarkdown('sum of 1 and 2 is 3')).toContain('sum of 1 and 2 is 3');
  });

  it('renders fenced code blocks verbatim, unclosed fences included', () => {
    const html = renderMarkdown('```bash\nls -la *.md\n```');
    expect(html).toContain('<pre class="md-code"><code>ls -la *.md</code></pre>');
    expect(renderMarkdown('```\nunclosed')).toContain('unclosed');
  });

  it('renders bullet and numbered lists', () => {
    const html = renderMarkdown('- one\n- two\n\n1. first\n2. second');
    expect(html).toContain('<ul>');
    expect(html).toContain('<li>one</li>');
    expect(html).toContain('<ol>');
    expect(html).toContain('<li>second</li>');
  });

  it('links only http(s) URLs', () => {
    const ok = renderMarkdown('[docs](https://example.com/a)');
    expect(ok).toContain('href="https://example.com/a"');
    const bad = renderMarkdown('[x](javascript:alert(1))');
    expect(bad).not.toContain('<a ');
  });

  it('renders blockquotes and horizontal rules', () => {
    const html = renderMarkdown('> note\n\n---');
    expect(html).toContain('<blockquote><p>note</p></blockquote>');
    expect(html).toContain('<hr>');
  });

  describe('blocks', () => {
    it('keeps a single newline inside a paragraph as a line break', () => {
      expect(renderMarkdown('line one\nline two\n\nnext')).toBe(
        '<p>line one<br>line two</p>\n<p>next</p>',
      );
    });

    it('shifts every heading level down two and drops closing hashes', () => {
      const html = renderMarkdown('# One\n## Two ##\n#### Four\n###### Six\n#hashtag');
      expect(html).toContain('<h3>One</h3>');
      expect(html).toContain('<h4>Two</h4>');
      expect(html).toContain('<h6>Four</h6>');
      expect(html).toContain('<h6>Six</h6>');
      expect(html).toContain('<p>#hashtag</p>');
    });

    it('makes rules of ---, *** and ___, spaced or not', () => {
      expect(renderMarkdown('above\n---\nbelow')).toBe('<p>above</p>\n<hr>\n<p>below</p>');
      expect(renderMarkdown('* * *')).toBe('<hr>');
      expect(renderMarkdown('___')).toBe('<hr>');
    });

    it('keeps consecutive quoted lines in one quote with its own paragraphs and lists', () => {
      const html = renderMarkdown('> first\n> still first\n>\n> - a point\n> - another\n\n> apart');
      const quotes = fragment(html).querySelectorAll('blockquote');
      expect(quotes.length).toBe(2);
      expect(quotes[0].innerHTML).toBe(
        '<p>first<br>still first</p>\n<ul><li>a point</li><li>another</li></ul>',
      );
      expect(quotes[1].innerHTML).toBe('<p>apart</p>');
    });

    it('nests quotes', () => {
      const html = renderMarkdown('> outer\n> > inner\n> outer again');
      expect(html).toBe(
        '<blockquote><p>outer</p>\n<blockquote><p>inner</p></blockquote>\n<p>outer again</p></blockquote>',
      );
    });
  });

  describe('tables', () => {
    it('renders the reported pipe table as a table, not lines of pipes', () => {
      const html = renderMarkdown(
        [
          'Who reads what:',
          '',
          "| Pages | Characters right | The Mac's reader |",
          '|---|---|---|',
          '| Help | 31 | yes |',
          '| Settings | 12 | no |',
        ].join('\n'),
      );
      expect(html).not.toContain('| Pages |');
      expect(html).not.toContain('<br>');
      expect(texts(html, 'th')).toEqual(['Pages', 'Characters right', "The Mac's reader"]);
      expect(texts(html, 'tbody tr').length).toBe(2);
      expect(texts(html, 'tbody tr:first-child td')).toEqual(['Help', '31', 'yes']);
      expect(html.startsWith('<p>Who reads what:</p>\n')).toBe(true);
    });

    it('puts a table in a labelled region that scrolls sideways', () => {
      const html = renderMarkdown('| a | b |\n| - | - |\n| 1 | 2 |');
      expect(html).toBe(
        '<div class="md-table" role="region" tabindex="0" aria-label="Table"><table>' +
          '<thead><tr><th>a</th><th>b</th></tr></thead>' +
          '<tbody><tr><td>1</td><td>2</td></tr></tbody></table></div>',
      );
    });

    it('aligns columns from the delimiter row', () => {
      const html = renderMarkdown('| l | c | r | n |\n|:---|:---:|---:|---|\n| 1 | 2 | 3 | 4 |');
      const classes = (sel: string) =>
        Array.from(fragment(html).querySelectorAll(sel)).map((e) => e.className);
      expect(classes('th')).toEqual(['al-left', 'al-center', 'al-right', '']);
      expect(classes('td')).toEqual(['al-left', 'al-center', 'al-right', '']);
    });

    it('needs no outer pipes', () => {
      const html = renderMarkdown('name | size\n--- | ---:\nlog | 12 KB');
      expect(texts(html, 'th')).toEqual(['name', 'size']);
      expect(texts(html, 'td')).toEqual(['log', '12 KB']);
    });

    it('keeps an escaped pipe in its cell', () => {
      const html = renderMarkdown('| expr | means |\n|---|---|\n| a \\| b | either |');
      expect(texts(html, 'td')).toEqual(['a | b', 'either']);
    });

    it('does not split cells at pipes inside code spans', () => {
      const html = renderMarkdown(
        '| op | example |\n|---|---|\n| or | `a || b` |\n| pipe | `` x | `y` `` |',
      );
      expect(texts(html, 'tr:first-child td')).toEqual(['or', 'a || b']);
      expect(fragment(html).querySelector('tr:first-child td:last-child')?.innerHTML).toBe(
        '<code>a || b</code>',
      );
      expect(texts(html, 'tbody tr:last-child td')).toEqual(['pipe', 'x | `y`']);
    });

    it("gives every row the header's cells: missing ones empty, extra ones dropped", () => {
      const html = renderMarkdown('| a | b | c |\n|---|---|---|\n| 1 |\n| 1 | 2 | 3 | 4 | 5 |');
      const rows = Array.from(fragment(html).querySelectorAll('tbody tr')).map((tr) =>
        Array.from(tr.querySelectorAll('td')).map((td) => td.textContent),
      );
      expect(rows).toEqual([
        ['1', '', ''],
        ['1', '2', '3'],
      ]);
      expect(html).not.toContain('4');
    });

    it('renders inline markup inside cells', () => {
      const html = renderMarkdown(
        '| what | where |\n|---|---|\n| **bold** _it_ ~~old~~ | [docs](https://example.com) `x` |',
      );
      expect(html).toContain(
        '<td><strong>bold</strong> <em>it</em> <del>old</del></td><td><a href="https://example.com"',
      );
      expect(html).toContain('<code>x</code></td>');
    });

    it('is not a table without a delimiter row of as many cells', () => {
      expect(renderMarkdown('| a | b |\n| --- |')).not.toContain('<table>');
      expect(renderMarkdown('| a | b |\n| c | d |')).not.toContain('<table>');
      expect(renderMarkdown('a | b')).toBe('<p>a | b</p>');
    });

    it('ends at a blank line or a line without a pipe, and may follow text directly', () => {
      const html = renderMarkdown('Before\n| a |\n|---|\n| 1 |\nAfter\n\n| x |');
      expect(html).toMatch(
        /^<p>Before<\/p>\n<div class="md-table"[^]*<\/table><\/div>\n<p>After<\/p>/,
      );
      expect(texts(html, 'td')).toEqual(['1']);
      expect(html.endsWith('<p>| x |</p>')).toBe(true);
    });

    it('may have a header and no rows', () => {
      const html = renderMarkdown('| only | header |\n|---|---|');
      expect(texts(html, 'th')).toEqual(['only', 'header']);
      expect(html).not.toContain('<tbody>');
    });
  });

  describe('lists', () => {
    it('nests lists by indentation: 2 or 4 spaces, or a tab', () => {
      const two = renderMarkdown('- a\n  - b\n    - c\n- d');
      expect(two).toBe('<ul><li>a\n<ul><li>b\n<ul><li>c</li></ul></li></ul></li><li>d</li></ul>');
      expect(renderMarkdown('- a\n    - b\n- c')).toBe(
        '<ul><li>a\n<ul><li>b</li></ul></li><li>c</li></ul>',
      );
      expect(renderMarkdown('- a\n\t- b\n\t\t- c')).toBe(
        '<ul><li>a\n<ul><li>b\n<ul><li>c</li></ul></li></ul></li></ul>',
      );
    });

    it('nests under a numbered item with two or three spaces', () => {
      const expected = '<ol><li>Step\n<ul><li>detail</li></ul></li><li>Next</li></ol>';
      expect(renderMarkdown('1. Step\n  - detail\n2. Next')).toBe(expected);
      expect(renderMarkdown('1. Step\n   - detail\n2. Next')).toBe(expected);
    });

    it('keeps continuation lines in their item, indented or lazy', () => {
      expect(renderMarkdown('- first line\n  goes on\n- second')).toBe(
        '<ul><li>first line<br>goes on</li><li>second</li></ul>',
      );
      expect(renderMarkdown('1. wrapped\nlazily\n2. next')).toBe(
        '<ol><li>wrapped<br>lazily</li><li>next</li></ol>',
      );
    });

    it('keeps an ordered list’s start number', () => {
      expect(renderMarkdown('3. three\n4. four')).toBe(
        '<ol start="3"><li>three</li><li>four</li></ol>',
      );
      expect(renderMarkdown('1) one\n2) two')).toBe('<ol><li>one</li><li>two</li></ol>');
      expect(renderMarkdown('0. zero')).toContain('<ol start="0">');
    });

    it('keeps counting when a code block between items splits the list', () => {
      const html = renderMarkdown('1. Install\n\n```\nnpm ci\n```\n\n2. Build');
      expect(html).toContain('<ol><li>Install</li></ol>');
      expect(html).toContain('<ol start="2"><li>Build</li></ol>');
    });

    it('wraps items in paragraphs when blank lines make the list loose', () => {
      expect(renderMarkdown('- a\n\n- b')).toBe('<ul><li><p>a</p></li><li><p>b</p></li></ul>');
      expect(renderMarkdown('1. **Title**\n\n   Explained here.\n2. Next')).toBe(
        '<ol><li><p><strong>Title</strong></p>\n<p>Explained here.</p></li><li><p>Next</p></li></ol>',
      );
    });

    it('starts a new list for another kind of marker', () => {
      expect(renderMarkdown('- a\n1. b')).toBe('<ul><li>a</li></ul>\n<ol><li>b</li></ol>');
    });

    it('draws task items with a check glyph, no input', () => {
      const html = renderMarkdown('- [ ] write tests\n- [x] fix tables\n- [X] ship\n- plain');
      expect(html).toBe(
        '<ul>' +
          '<li class="md-task"><span class="md-check" role="img" aria-label="To do"></span>write tests</li>' +
          '<li class="md-task done"><span class="md-check" role="img" aria-label="Done"></span>fix tables</li>' +
          '<li class="md-task done"><span class="md-check" role="img" aria-label="Done"></span>ship</li>' +
          '<li>plain</li>' +
          '</ul>',
      );
      expect(html).not.toContain('<input');
      expect(renderMarkdown('1. [x] numbered task')).toContain('<li class="md-task done">');
    });

    it('holds a fenced block indented under its item', () => {
      const html = renderMarkdown(
        '1. Run:\n   ```bash\n   npm ci\n     --prefer-offline\n   ```\n2. Then build',
      );
      expect(html).toMatch(/^<ol><li>Run:\n<div class="md-pre has-lang">/);
      expect(texts(html, 'code')).toEqual(['npm ci\n  --prefer-offline']);
      expect(texts(html, 'ol > li').length).toBe(2);
      expect(html).not.toContain('start=');
    });

    it('keeps a sloppily indented fence in its item, its code in shape', () => {
      const html = renderMarkdown('- Example:\n  ```python\ndef f():\n    return 1\n  ```\n- next');
      expect(texts(html, 'code')).toEqual(['def f():\n    return 1']);
      expect(texts(html, 'ul > li').length).toBe(2);
    });

    it('takes a fence on the marker line', () => {
      const html = renderMarkdown('- ```ts\n  const a = 1;\n  ```');
      expect(html).toMatch(/^<ul><li><div class="md-pre has-lang">/);
      expect(texts(html, 'code')).toEqual(['const a = 1;']);
    });

    it('is not a list without text after the marker, nor a rule', () => {
      expect(renderMarkdown('-')).toBe('<p>-</p>');
      expect(renderMarkdown('- - -')).toBe('<hr>');
      expect(renderMarkdown('**bold** start')).toBe('<p><strong>bold</strong> start</p>');
      expect(renderMarkdown('-1 is negative')).toBe('<p>-1 is negative</p>');
    });
  });

  describe('code blocks', () => {
    it('labels the language from the info string and offers Copy', () => {
      const html = renderMarkdown('~~~python title="x.py"\nprint(1)\n~~~');
      expect(html).toBe(
        '<div class="md-pre has-lang"><div class="md-pre-bar"><span class="md-lang">python</span>' +
          '<span class="md-copy" role="button" tabindex="0" aria-label="Copy code" title="Copy code">Copy</span></div>' +
          '<pre class="md-code"><code>print(1)</code></pre></div>',
      );
    });

    it('offers Copy without a language too', () => {
      const html = renderMarkdown('```\nplain\n```');
      expect(html).toContain('<div class="md-pre"><div class="md-pre-bar"><span class="md-copy"');
      expect(html).not.toContain('md-lang');
    });

    it('escapes the code and keeps it verbatim', () => {
      const html = renderMarkdown('```html\n<b class="x">**not bold**</b>\n\n\tindented\n```');
      expect(texts(html, 'code')).toEqual(['<b class="x">**not bold**</b>\n\n\tindented']);
      expect(html).not.toContain('<b ');
      expect(html).not.toContain('<strong>');
    });

    it('closes only on a fence of the same character, at least as long', () => {
      const html = renderMarkdown('````md\n```\ninner\n~~~\n````\nafter');
      expect(texts(html, 'code')).toEqual(['```\ninner\n~~~']);
      expect(html.endsWith('<p>after</p>')).toBe(true);
    });

    it('treats ```a``` on one line as inline code, not a fence', () => {
      expect(renderMarkdown('```a``` b')).toBe('<p><code>a</code> b</p>');
    });
  });

  describe('inline', () => {
    it('strikes through ~~text~~, but not ~5 or ~~~', () => {
      expect(renderMarkdown('~~gone~~')).toBe('<p><del>gone</del></p>');
      expect(renderMarkdown('about ~5 min, ~~~ or ~one~')).toBe(
        '<p>about ~5 min, ~~~ or ~one~</p>',
      );
    });

    it('renders ***bold italics*** and nested emphasis', () => {
      expect(renderMarkdown('***both***')).toBe('<p><em><strong>both</strong></em></p>');
      expect(renderMarkdown('**bold *in* bold** and *it **in** it*')).toBe(
        '<p><strong>bold <em>in</em> bold</strong> and <em>it <strong>in</strong> it</em></p>',
      );
    });

    it('takes __bold__ and _italics_ only at word boundaries', () => {
      expect(renderMarkdown('__bold__ and _italic_')).toBe(
        '<p><strong>bold</strong> and <em>italic</em></p>',
      );
      const names = 'snake_case_name, file_names_like_this.py and __dunder_ish_name';
      expect(renderMarkdown(names)).toBe(`<p>${names}</p>`);
    });

    it('leaves lone stars and globs alone', () => {
      expect(renderMarkdown('a * b * c and src/**/*.ts and *.md')).toBe(
        '<p>a * b * c and src/**/*.ts and *.md</p>',
      );
    });

    it('takes double-backtick code spans, with a backtick inside', () => {
      expect(renderMarkdown('``a ` b`` and `` `tick` ``')).toBe(
        '<p><code>a ` b</code> and <code>`tick`</code></p>',
      );
      expect(renderMarkdown('a lone ` backtick')).toBe('<p>a lone ` backtick</p>');
    });

    it('links bare http(s) URLs without their trailing punctuation', () => {
      const a = (url: string) =>
        `<a href="${url}" target="_blank" rel="noopener noreferrer">${url}</a>`;
      expect(renderMarkdown('See https://example.com/a.')).toBe(
        `<p>See ${a('https://example.com/a')}.</p>`,
      );
      expect(renderMarkdown('(https://example.com/b), then')).toBe(
        `<p>(${a('https://example.com/b')}), then</p>`,
      );
      expect(renderMarkdown('https://en.wikipedia.org/wiki/Foo_(bar)!')).toBe(
        `<p>${a('https://en.wikipedia.org/wiki/Foo_(bar)')}!</p>`,
      );
      expect(renderMarkdown('"https://example.com/q" and http://localhost:6725/x?')).toBe(
        `<p>&quot;${a('https://example.com/q')}&quot; and ${a('http://localhost:6725/x')}?</p>`,
      );
      expect(renderMarkdown('**https://example.com/bold**')).toBe(
        `<p><strong>${a('https://example.com/bold')}</strong></p>`,
      );
    });

    it('keeps a URL whole: no emphasis inside, & escaped once', () => {
      expect(renderMarkdown('https://example.com/a_b_c?x=1&y=*2*')).toBe(
        '<p><a href="https://example.com/a_b_c?x=1&amp;y=*2" target="_blank" rel="noopener noreferrer">' +
          'https://example.com/a_b_c?x=1&amp;y=*2</a>*</p>',
      );
    });

    it('links <https://…> and leaves URLs inside words and code alone', () => {
      expect(renderMarkdown('<https://example.com/x>')).toBe(
        '<p><a href="https://example.com/x" target="_blank" rel="noopener noreferrer">https://example.com/x</a></p>',
      );
      expect(renderMarkdown('xhttps://example.com')).toBe('<p>xhttps://example.com</p>');
      expect(renderMarkdown('`https://example.com`')).toBe(
        '<p><code>https://example.com</code></p>',
      );
      expect(renderMarkdown('https:// alone')).toBe('<p>https:// alone</p>');
    });

    it('renders links with markup in their text and a title', () => {
      expect(renderMarkdown('[the **docs**](https://example.com/d "Docs")')).toBe(
        '<p><a href="https://example.com/d" target="_blank" rel="noopener noreferrer">the <strong>docs</strong></a></p>',
      );
    });

    it('shows a non-web link as its text, a relative path as its tooltip', () => {
      expect(renderMarkdown('[auth.ts:42](src/auth.ts#L42)')).toBe(
        '<p><span class="md-link" title="src/auth.ts#L42">auth.ts:42</span></p>',
      );
      expect(renderMarkdown('[mail me](mailto:me@example.com)')).toBe(
        '<p><span class="md-link">mail me</span></p>',
      );
    });

    it('never loads an image: it becomes a link to it, showing its alt text', () => {
      expect(renderMarkdown('![build status](https://example.com/ci.svg)')).toBe(
        '<p><a class="md-img" href="https://example.com/ci.svg" target="_blank" rel="noopener noreferrer">build status</a></p>',
      );
      expect(renderMarkdown('[![CI](https://example.com/b.svg)](https://ci.example.com)')).toBe(
        '<p><a href="https://ci.example.com" target="_blank" rel="noopener noreferrer">CI</a></p>',
      );
      expect(renderMarkdown('![diagram](./diagram.png)')).toBe(
        '<p><span class="md-link" title="./diagram.png">diagram</span></p>',
      );
      expect(renderMarkdown('![x](https://example.com/i.png)')).not.toContain('<img');
    });

    it('honours backslash escapes, and leaves other backslashes be', () => {
      expect(renderMarkdown('\\*not italic\\* and \\_not\\_ and \\`tick\\`')).toBe(
        '<p>*not italic* and _not_ and `tick`</p>',
      );
      expect(renderMarkdown('C:\\Users\\me\\file_name.txt')).toBe(
        '<p>C:\\Users\\me\\file_name.txt</p>',
      );
      expect(renderMarkdown('\\# not a heading')).toBe('<p># not a heading</p>');
    });

    it('turns <br> into a line break, and nothing else of HTML', () => {
      expect(renderMarkdown('a<br>b<BR/>c<br />d')).toBe('<p>a<br>b<br>c<br>d</p>');
      expect(renderMarkdown('<br onclick="x">')).toBe('<p>&lt;br onclick=&quot;x&quot;&gt;</p>');
    });
  });

  describe('safety', () => {
    it('keeps <script> and event handlers inert text', () => {
      const html = renderMarkdown('<script>alert(1)</script>\n<img src=x onerror=alert(1)>');
      expect(html).toBe(
        '<p>&lt;script&gt;alert(1)&lt;/script&gt;<br>&lt;img src=x onerror=alert(1)&gt;</p>',
      );
      expectInert(html);
    });

    it('never links javascript:, data: or vbscript: targets', () => {
      for (const target of ['javascript:alert(1)', 'JaVaScRiPt:alert(1)', 'data:text/html,x']) {
        const html = renderMarkdown(`[click](${target}) <${target}> ${target}`);
        expect(html, target).not.toContain('<a');
        expect(html, target).not.toContain('href');
        expectInert(html);
      }
      expect(renderMarkdown('![x](javascript:alert(1))')).not.toContain('href');
    });

    it('keeps a URL from breaking out of its attribute', () => {
      const html = renderMarkdown(
        '[x](https://example.com/"onmouseover="alert(1)) https://example.com/\'onfocus=\'x',
      );
      expectInert(html);
      const links = fragment(html).querySelectorAll('a');
      expect(links.length).toBe(2);
      expect(links[0].getAttribute('href')).toBe('https://example.com/"onmouseover="alert(1)');
    });

    it('keeps HTML inside table cells inert', () => {
      const html = renderMarkdown(
        '| <b>bold</b> | <img src=x onerror=alert(1)> |\n|---|---|\n| <script>x</script> | [x](javascript:alert(1)) |',
      );
      expect(texts(html, 'th')).toEqual(['<b>bold</b>', '<img src=x onerror=alert(1)>']);
      expect(texts(html, 'td')).toEqual(['<script>x</script>', 'x']);
      expectInert(html);
    });

    it('keeps code blocks, languages and link titles inert', () => {
      const html = renderMarkdown(
        '```"><script>x</script>\n</code></pre><script>y</script>\n```\n\n[t](a"onclick="b)',
      );
      expectInert(html);
      expect(fragment(html).querySelectorAll('pre').length).toBe(1);
    });

    it("uses only what Angular's sanitizer keeps, so it strips nothing", () => {
      const html = renderMarkdown(
        [
          '# Title',
          '| a | b |',
          '|:--|--:|',
          '| `x \\| y` | **2** |',
          '',
          '3. three',
          '   - [x] done',
          '   - [ ] to do',
          '',
          '> quoted ~~old~~ _new_',
          '',
          '```ts',
          'const a = 1 < 2;',
          '```',
          '',
          '[link](https://example.com) ![img](https://example.com/i.png) [rel](src/a.ts) https://x.dev',
          '***',
        ].join('\n'),
      );
      const kept = TestBed.inject(DomSanitizer).sanitize(SecurityContext.HTML, html) ?? '';
      const normal = (h: string) => {
        const t = document.createElement('template');
        t.innerHTML = h;
        return t.innerHTML;
      };
      expect(normal(kept)).toBe(normal(html));
      expectInert(html);
    });
  });

  it('stays fast on input built to make a parser backtrack', () => {
    const nasty = [
      '[a](b'.repeat(20_000),
      '[a](u ('.repeat(20_000),
      '**a* '.repeat(20_000),
      'a_b _c '.repeat(20_000),
      Array.from({ length: 20_000 }, (_, i) => '`'.repeat((i % 7) + 1) + 'x').join(''),
      '<http'.repeat(20_000),
      'https://example.com/' + ')'.repeat(50_000),
      '>'.repeat(5_000) + ' deep',
      Array.from({ length: 300 }, (_, i) => ' '.repeat(i * 2) + '- item').join('\n'),
      '| a | b |\n|---|---|\n' + '| `x|y` | z |\n'.repeat(5_000),
      '- ```\n  x\n'.repeat(5_000),
    ];
    const start = performance.now();
    for (const s of nasty) renderMarkdown(s);
    expect(performance.now() - start).toBeLessThan(3_000);
  });
});
