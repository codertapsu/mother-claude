import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';

import { UnifiedRow, editToRows } from './diff';
import { DiffViewComponent } from './diff-view.component';
import { IconComponent } from './icon.component';
import { shortPath } from './util';

type Preview =
  | { kind: 'bash'; command: string; description?: string; background?: boolean; timeout?: number }
  | { kind: 'edit'; path: string; edits: UnifiedRow[][]; replaceAll?: boolean }
  | { kind: 'write'; path: string; head: string; lines: number; shown: number }
  | { kind: 'read'; path: string; range?: string }
  | { kind: 'fetch'; url: string; host: string; prompt?: string }
  | { kind: 'search'; query: string; domains?: string }
  | { kind: 'mcp'; server: string; tool: string; json: string }
  | { kind: 'notebook'; path: string; source: string }
  | { kind: 'pattern'; pattern: string; path?: string }
  | { kind: 'json'; json: string };

const WRITE_PREVIEW_LINES = 40;

function str(v: unknown): string {
  return typeof v === 'string' ? v : v === undefined || v === null ? '' : JSON.stringify(v);
}

function pretty(v: unknown): string {
  try {
    const text = JSON.stringify(v ?? {}, null, 2);
    return text === '{}' ? '' : text;
  } catch {
    return '';
  }
}

/** Parse a tool call's input into what a human needs to decide on it. */
export function buildPreview(name: string, raw: unknown): Preview {
  const input = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  if (name.startsWith('mcp__')) {
    const [, server = '', ...rest] = name.split('__');
    return { kind: 'mcp', server, tool: rest.join('__'), json: pretty(input) };
  }
  switch (name) {
    case 'Bash':
      return {
        kind: 'bash',
        command: str(input['command']),
        description: str(input['description']) || undefined,
        background: input['run_in_background'] === true,
        timeout: typeof input['timeout'] === 'number' ? input['timeout'] : undefined,
      };
    case 'Edit':
      return {
        kind: 'edit',
        path: str(input['file_path']),
        edits: [editToRows(str(input['old_string']), str(input['new_string']))],
        replaceAll: input['replace_all'] === true,
      };
    case 'MultiEdit': {
      const edits = Array.isArray(input['edits']) ? (input['edits'] as unknown[]) : [];
      return {
        kind: 'edit',
        path: str(input['file_path']),
        edits: edits.map((e) => {
          const r = (e ?? {}) as Record<string, unknown>;
          return editToRows(str(r['old_string']), str(r['new_string']));
        }),
      };
    }
    case 'Write': {
      const content = str(input['content']);
      const lines = content ? content.split('\n') : [];
      return {
        kind: 'write',
        path: str(input['file_path']),
        head: lines.slice(0, WRITE_PREVIEW_LINES).join('\n'),
        lines: lines.length,
        shown: Math.min(lines.length, WRITE_PREVIEW_LINES),
      };
    }
    case 'Read': {
      const offset = input['offset'];
      const limit = input['limit'];
      return {
        kind: 'read',
        path: str(input['file_path']),
        range:
          typeof offset === 'number'
            ? `lines ${offset}–${offset + (typeof limit === 'number' ? limit : 0)}`
            : undefined,
      };
    }
    case 'WebFetch': {
      const url = str(input['url']);
      let host = url;
      try {
        host = new URL(url).host;
      } catch {
        /* not a URL: show it as is */
      }
      return { kind: 'fetch', url, host, prompt: str(input['prompt']) || undefined };
    }
    case 'WebSearch': {
      const allowed = input['allowed_domains'];
      return {
        kind: 'search',
        query: str(input['query']),
        domains: Array.isArray(allowed) ? allowed.join(', ') : undefined,
      };
    }
    case 'NotebookEdit':
      return {
        kind: 'notebook',
        path: str(input['notebook_path']),
        source: str(input['new_source']),
      };
    case 'Grep':
    case 'Glob':
      return {
        kind: 'pattern',
        pattern: str(input['pattern']),
        path: str(input['path']) || undefined,
      };
    default:
      return { kind: 'json', json: pretty(input) };
  }
}

/**
 * What a tool call will actually do, before you approve it: the full Bash
 * command, an Edit as a diff, the head of a Write, the URL a fetch goes to,
 * an MCP server · tool with its arguments. Falls back to the input JSON.
 */
@Component({
  selector: 'mc-tool-preview',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DiffViewComponent, IconComponent],
  template: `
    @let p = preview();
    @switch (p.kind) {
      @case ('bash') {
        @if (p.description) {
          <p class="desc">{{ p.description }}</p>
        }
        <pre class="code cmd"><span class="prompt" aria-hidden="true">$ </span>{{ p.command }}</pre>
        <p class="meta">
          @if (cwd()) {
            <span><mc-icon name="folder" [size]="12" /> {{ short(cwd()) }}</span>
          }
          @if (p.background) {
            <span>runs in the background</span>
          }
          @if (p.timeout) {
            <span>timeout {{ p.timeout / 1000 }}s</span>
          }
        </p>
      }
      @case ('edit') {
        <p class="path mono"><mc-icon name="edit" [size]="13" /> {{ short(p.path) }}</p>
        @for (rows of p.edits; track $index) {
          <mc-diff-view class="diff" [rows]="rows" />
        }
        @if (p.replaceAll) {
          <p class="meta"><span>replaces every occurrence</span></p>
        }
      }
      @case ('write') {
        <p class="path mono"><mc-icon name="file" [size]="13" /> {{ short(p.path) }}</p>
        <pre class="code">{{ p.head }}</pre>
        <p class="meta">
          <span
            >{{ p.lines }} {{ p.lines === 1 ? 'line' : 'lines'
            }}{{ p.shown < p.lines ? ' · first ' + p.shown + ' shown' : '' }}</span
          >
        </p>
      }
      @case ('read') {
        <p class="path mono"><mc-icon name="file" [size]="13" /> {{ short(p.path) }}</p>
        @if (p.range) {
          <p class="meta">
            <span>{{ p.range }}</span>
          </p>
        }
      }
      @case ('fetch') {
        <p class="host"><mc-icon name="globe" [size]="15" /> {{ p.host }}</p>
        <p class="url mono">{{ p.url }}</p>
        @if (p.prompt) {
          <p class="desc">{{ p.prompt }}</p>
        }
      }
      @case ('search') {
        <p class="host"><mc-icon name="search" [size]="15" /> {{ p.query }}</p>
        @if (p.domains) {
          <p class="meta">
            <span>only {{ p.domains }}</span>
          </p>
        }
      }
      @case ('mcp') {
        <p class="host"><mc-icon name="plug" [size]="15" /> {{ p.server }} · {{ p.tool }}</p>
        @if (p.json) {
          <button class="btn ghost small" type="button" (click)="open.set(!open())">
            <mc-icon [name]="open() ? 'chevronDown' : 'chevronRight'" [size]="14" />
            Arguments
          </button>
          @if (open()) {
            <pre class="code">{{ p.json }}</pre>
          }
        }
      }
      @case ('notebook') {
        <p class="path mono"><mc-icon name="file" [size]="13" /> {{ short(p.path) }}</p>
        <pre class="code">{{ p.source }}</pre>
      }
      @case ('pattern') {
        <pre class="code">{{ p.pattern }}</pre>
        @if (p.path) {
          <p class="meta">
            <span>in {{ short(p.path) }}</span>
          </p>
        }
      }
      @default {
        @if (p.kind === 'json' && p.json) {
          <button class="btn ghost small" type="button" (click)="open.set(!open())">
            <mc-icon [name]="open() ? 'chevronDown' : 'chevronRight'" [size]="14" />
            {{ open() ? 'Hide' : 'Show' }} input
          </button>
          @if (open()) {
            <pre class="code">{{ p.json }}</pre>
          }
        }
      }
    }
    @if (truncated()) {
      <p class="meta"><span>Preview shortened; the full request is loading…</span></p>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
        min-width: 0;
      }
      p {
        margin: 0;
      }
      .desc {
        color: var(--muted);
        font-size: 0.85rem;
      }
      .code {
        margin: 0;
        padding: 0.5rem 0.65rem;
        background: var(--code-bg);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        font-size: 0.8rem;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        max-height: 16rem;
        overflow: auto;
      }
      .cmd {
        font-size: 0.85rem;
      }
      .prompt {
        color: var(--faint);
        user-select: none;
      }
      .path,
      .host {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        overflow-wrap: anywhere;
      }
      .host {
        font-weight: 600;
        font-size: 0.95rem;
      }
      .url {
        color: var(--muted);
        overflow-wrap: anywhere;
      }
      .meta {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
        font-size: 0.75rem;
        color: var(--faint);
      }
      .meta span {
        display: inline-flex;
        align-items: center;
        gap: 0.25rem;
      }
      .diff {
        max-height: 18rem;
      }
    `,
  ],
})
export class ToolPreviewComponent {
  readonly name = input.required<string>();
  readonly input = input<unknown>(undefined);
  /** The session's working directory (Bash runs there). */
  readonly cwd = input('');
  readonly truncated = input(false);

  protected readonly preview = computed(() => buildPreview(this.name(), this.input()));
  protected readonly open = signal(false);
  protected readonly short = shortPath;
}
