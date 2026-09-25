import { IconName } from '../../shared/icon.component';

const TOOL_ICONS: Record<string, IconName> = {
  Bash: 'terminal',
  Read: 'file',
  Write: 'file',
  Edit: 'edit',
  MultiEdit: 'edit',
  NotebookEdit: 'edit',
  Grep: 'search',
  Glob: 'search',
  WebFetch: 'globe',
  WebSearch: 'globe',
  Agent: 'agents',
  Task: 'agents',
  Workflow: 'workflow',
  TodoWrite: 'list',
};

export function toolIcon(name: string | undefined): IconName {
  if (!name) return 'terminal';
  if (name.startsWith('mcp__')) return 'plug';
  return TOOL_ICONS[name] ?? 'terminal';
}
