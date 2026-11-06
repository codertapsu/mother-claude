// The placeholder that holds an empty lease's tab group open.

import { byId, request } from './common.ts';

async function show(): Promise<void> {
  const lease = decodeURIComponent(location.hash.slice(1));
  if (!lease) return;
  const res = await request({ op: 'leaseInfo', lease });
  if (!res.ok) return;
  const title =
    typeof res['title'] === 'string' && res['title'] ? res['title'] : 'A Claude Code session';
  byId('title').textContent = title;
  document.title = `${title} — waiting for its first page`;
  byId('stopped').hidden = res['stopped'] !== true;
}

void show();
