import { SessionState, Surface } from '../core/models';

export function relativeTime(ms?: number): string {
  if (!ms) return '—';
  const diff = Date.now() - ms;
  if (diff < 0) return 'just now';
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export function formatTokens(n?: number): string {
  if (!n) return '0';
  if (n < 1000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

export function stateLabel(state: SessionState): string {
  return state.replace('-', ' ');
}

export function surfaceLabel(surface: Surface): string {
  switch (surface) {
    case 'cli':
      return 'CLI';
    case 'vs-code':
      return 'VS Code';
    case 'jet-brains':
      return 'JetBrains';
    case 'desktop':
      return 'Desktop';
    default:
      return 'Unknown';
  }
}
