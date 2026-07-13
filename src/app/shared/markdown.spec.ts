import { renderMarkdown } from './markdown';

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
    expect(html).toContain('<blockquote>note</blockquote>');
    expect(html).toContain('<hr>');
  });
});
