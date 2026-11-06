// The pages the e2e suite drives, served on ephemeral loopback ports:
// http://127.0.0.1:<p> and http://localhost:<p> (two origins, so one page can
// embed the other cross-origin) and http://[::1]:<q> (an origin the e2e build
// has no host permission for).

import { readFile } from 'node:fs/promises';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { dirname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const dir = join(dirname(fileURLToPath(import.meta.url)), 'fixtures');

/** A page like a busy issue list: the "typical page" of the snapshot-size target. */
export function typicalPage(): string {
  const labels = [
    'bug',
    'enhancement',
    'documentation',
    'good first issue',
    'help wanted',
    'question',
  ];
  const people = ['alice', 'bob', 'carol', 'dave', 'erin', 'frank'];
  const issues = Array.from({ length: 25 }, (_, i) => {
    const n = 1200 - i;
    const who = people[i % people.length] ?? 'alice';
    const ls = labels.slice(i % 4, (i % 4) + 1 + (i % 3));
    return `
      <div class="issue" role="row">
        <input type="checkbox" aria-label="Select issue #${n}">
        <svg class="octicon" width="16" height="16" aria-hidden="true"><circle cx="8" cy="8" r="6"/></svg>
        <div class="main">
          <a class="title" href="/issues/${n}">Snapshot of issue ${n}: ${['Crash when opening settings', 'Support dark mode in the side panel', 'Typo in the README', 'Pairing code expires too fast', 'Allow per-site screenshot policy'][i % 5]}</a>
          ${ls.map((l) => `<a class="label" href="/labels/${encodeURIComponent(l)}">${l}</a>`).join(' ')}
          <div class="meta">#${n} opened ${i + 1} days ago by <a href="/users/${who}">${who}</a></div>
        </div>
        <a class="assignee" href="/users/${who}" aria-label="Assigned to ${who}"><img src="/avatar.png" alt=""></a>
        <a class="comments" href="/issues/${n}#comments" aria-label="${(i * 3) % 17} comments">${(i * 3) % 17}</a>
      </div>`;
  }).join('');
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Issues · mother-claude/example</title>
  <style>body{font:14px system-ui;margin:0} header,nav,footer{padding:8px} .issue{display:flex;gap:8px;padding:8px;border-top:1px solid #ddd}</style></head>
  <body>
    <header>
      <a href="/" aria-label="Homepage">Home</a>
      <input type="search" aria-label="Search or jump to…" placeholder="Search or jump to…">
      <nav aria-label="Global"><a href="/pulls">Pull requests</a> <a href="/issues">Issues</a> <a href="/marketplace">Marketplace</a> <a href="/explore">Explore</a></nav>
      <button aria-label="Notifications">🔔</button>
      <button aria-label="Create new…">+</button>
      <button aria-label="Open user account menu"><img src="/avatar.png" alt=""></button>
    </header>
    <div class="repohead">
      <a href="/mother-claude">mother-claude</a> / <a href="/mother-claude/example"><strong>example</strong></a>
      <button>Watch 12</button> <button>Fork 3</button> <button>Star 210</button>
    </div>
    <nav aria-label="Repository">
      <a href="/code">Code</a> <a href="/issues" aria-current="page">Issues 25</a> <a href="/pulls">Pull requests 3</a>
      <a href="/actions">Actions</a> <a href="/projects">Projects</a> <a href="/wiki">Wiki</a> <a href="/security">Security</a> <a href="/pulse">Insights</a> <a href="/settings">Settings</a>
    </nav>
    <main>
      <div class="filters">
        <button aria-haspopup="true" aria-expanded="false">Filters</button>
        <input aria-label="Search all issues" value="is:issue is:open">
        <a href="/labels">Labels 9</a> <a href="/milestones">Milestones 2</a>
        <a href="/issues/new">New issue</a>
      </div>
      <div role="table" aria-label="Issues">
        <div class="head" role="row">
          <a href="?q=is:open">25 Open</a> <a href="?q=is:closed">120 Closed</a>
          <button aria-haspopup="true">Author</button><button aria-haspopup="true">Label</button>
          <button aria-haspopup="true">Projects</button><button aria-haspopup="true">Milestones</button>
          <button aria-haspopup="true">Assignee</button><button aria-haspopup="true">Sort</button>
        </div>
        ${issues}
      </div>
      <nav aria-label="Pagination"><a href="?page=0" aria-disabled="true">Previous</a> <a href="?page=1" aria-current="page">1</a> <a href="?page=2">2</a> <a href="?page=3">3</a> <a href="?page=2">Next</a></nav>
      <p>ProTip! Add no:assignee to see everything that’s not assigned.</p>
    </main>
    <footer>
      ${['Terms', 'Privacy', 'Security', 'Status', 'Docs', 'Contact', 'Manage cookies', 'Do not share my personal information'].map((t) => `<a href="/${t.toLowerCase().replace(/\W+/g, '-')}">${t}</a>`).join(' ')}
      <p>© 2026 Fixture, Inc.</p>
    </footer>
  </body></html>`;
}

function longPage(rows: number): string {
  const body = Array.from(
    { length: rows },
    (_, i) =>
      `<li><a href="/row/${i}">Row ${i}</a> <span>detail ${i}</span> <button>Act ${i}</button></li>`,
  ).join('');
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Long list</title></head><body><h1>Long list</h1><ul>${body}</ul></body></html>`;
}

const TYPES: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.png': 'image/png',
  '.txt': 'text/plain; charset=utf-8',
};

async function handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
  const url = new URL(req.url ?? '/', 'http://fixture');
  const send = (
    status: number,
    type: string,
    body: string | Buffer,
    extra: Record<string, string> = {},
  ): void => {
    res.writeHead(status, { 'content-type': type, 'cache-control': 'no-store', ...extra });
    res.end(body);
  };
  if (url.pathname === '/typical.html') return send(200, TYPES['.html'] ?? '', typicalPage());
  if (url.pathname === '/long.html')
    return send(200, TYPES['.html'] ?? '', longPage(Number(url.searchParams.get('rows') ?? 3000)));
  if (url.pathname === '/api/slow') {
    const ms = Math.min(5000, Number(url.searchParams.get('ms') ?? 500));
    await new Promise((r) => setTimeout(r, ms));
    return send(200, 'application/json', JSON.stringify({ ok: `waited ${ms}` }));
  }
  if (url.pathname === '/download/report.txt') {
    return send(200, 'text/plain', 'quarterly report\n', {
      'content-disposition': 'attachment; filename="report.txt"',
    });
  }
  if (url.pathname === '/avatar.png' || url.pathname === '/favicon.ico')
    return send(204, 'image/png', '');
  if (url.pathname.startsWith('/spa/')) {
    return send(200, TYPES['.html'] ?? '', await readFile(join(dir, 'spa.html')));
  }
  const file = normalize(join(dir, url.pathname === '/' ? 'form.html' : url.pathname));
  if (!file.startsWith(dir)) return send(403, 'text/plain', 'no');
  try {
    const body = await readFile(file);
    const ext = file.slice(file.lastIndexOf('.'));
    return send(200, TYPES[ext] ?? 'application/octet-stream', body);
  } catch {
    return send(404, 'text/plain', 'not found');
  }
}

function listen(host: string, port = 0): Promise<{ server: Server; port: number }> {
  return new Promise((resolve, reject) => {
    const server = createServer((req, res) => {
      handle(req, res).catch(() => {
        res.writeHead(500);
        res.end();
      });
    });
    server.once('error', reject);
    server.listen(port, host, () =>
      resolve({ server, port: (server.address() as AddressInfo).port }),
    );
  });
}

export class Fixtures {
  private servers: Server[] = [];
  port = 0;
  v6Port = 0;

  get base(): string {
    return `http://127.0.0.1:${this.port}`;
  }
  get other(): string {
    return `http://localhost:${this.port}`;
  }
  get v6(): string {
    return `http://[::1]:${this.v6Port}`;
  }

  async start(): Promise<void> {
    const a = await listen('127.0.0.1');
    this.port = a.port;
    this.servers.push(a.server);
    // "localhost" may resolve to ::1 first: answer there on the same port too.
    try {
      const same = await listen('::1', this.port);
      this.servers.push(same.server);
    } catch {
      // IPv6 loopback taken or missing: Chrome falls back to 127.0.0.1
    }
    const v6 = await listen('::1');
    this.v6Port = v6.port;
    this.servers.push(v6.server);
  }

  async stop(): Promise<void> {
    await Promise.all(this.servers.map((s) => new Promise<void>((r) => s.close(() => r()))));
    this.servers = [];
  }
}
