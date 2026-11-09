import {
  BudgetState,
  RateLimitNote,
  budgetLabel,
  budgetNote,
  bytesLabel,
  choiceLabel,
  csvName,
  effortRank,
  effortIdLabel,
  estimateNoteText,
  normalizeCapacity,
  parseUsd,
  pctLabel,
  policyProblems,
  projectName,
  rank,
  rateLimitText,
  ratioLabel,
  schedulingWindows,
  taskClassLabel,
  tier,
  validModel,
} from './capacity';
import { useLanguage } from './i18n/i18n';
import { MockCapacity, defaultPolicy } from './mock/mock-capacity';

describe('capacity helpers', () => {
  it('labels sizes, shares and choices as the server does', () => {
    expect(bytesLabel(1.1 * 1024 ** 3)).toBe('1.1 GB');
    expect(bytesLabel(600 * 1024 ** 2)).toBe('600 MB');
    expect(bytesLabel(undefined)).toBe('—');
    expect(pctLabel(91)).toBe('91 %');
    expect(pctLabel(95.5)).toBe('95.5 %');
    expect(ratioLabel(0.833)).toBe('83 %');
    expect(ratioLabel(undefined)).toBe('—');
    expect(choiceLabel({ model: 'opus', effort: 'high' })).toBe('Opus · high');
    expect(choiceLabel({ model: 'sonnet' })).toBe('Sonnet');
    expect(choiceLabel({ model: 'claude-opus-4-8', effort: 'xhigh' })).toBe(
      'claude-opus-4-8 · xhigh',
    );
    expect(projectName('/Users/me/dev/mother-claude/')).toBe('mother-claude');
    expect(projectName('C:\\dev\\app')).toBe('app');
    expect(csvName(7, 'groups')).toBe('mother-claude-routing-7d.csv');
    expect(csvName(30, 'outcomes')).toBe('mother-claude-task-outcomes-30d.csv');
  });

  it('ranks models by tier, then effort', () => {
    expect(tier('haiku')).toBe(0);
    expect(tier('claude-sonnet-5-5')).toBe(1);
    expect(tier('opus')).toBe(2);
    expect(tier('claude-mythos-5-1')).toBe(3);
    expect(effortRank(undefined)).toBe(1);
    expect(effortRank('xhigh')).toBe(3);
    expect(rank({ model: 'sonnet', effort: 'high' })).toBeLessThan(
      rank({ model: 'opus', effort: 'low' }),
    );
    expect(validModel('opus')).toBe(true);
    expect(validModel('claude-opus-4-8[1m]')).toBe(true);
    expect(validModel('gpt-5')).toBe(false);
    expect(validModel('claude-')).toBe(false);
  });

  it('finds what the server would refuse in a policy, by field', () => {
    expect(policyProblems(defaultPolicy())).toEqual({});
    const p = defaultPolicy();
    p.ladder = [
      { model: 'opus', effort: 'high' },
      { model: 'sonnet', effort: 'high' },
    ];
    p.classes['plan'] = { model: 'gpt-5', effort: 'high' };
    p.classes['fix'] = { model: 'sonnet', effort: 'ultra' };
    p.gateChoice = { model: 'opus', effort: 'low' };
    p.gatesPerStep = 0;
    p.cleanToStepDown = 11;
    expect(Object.keys(policyProblems(p)).sort()).toEqual([
      'classes.fix',
      'classes.plan',
      'cleanToStepDown',
      'gateChoice',
      'gatesPerStep',
      'ladder',
    ]);
    expect(policyProblems({ ...defaultPolicy(), ladder: [{ model: 'haiku' }] })['ladder']).toBe(
      'The ladder has 2 to 12 rungs.',
    );
  });

  it('reads dollars the way people type them', () => {
    expect(parseUsd('')).toBeNull();
    expect(parseUsd('  ')).toBeNull();
    expect(parseUsd('$12.50')).toBe(12.5);
    expect(parseUsd('1,200')).toBe(1200);
    expect(parseUsd('abc')).toBeUndefined();
    expect(parseUsd('0')).toBeUndefined();
    expect(parseUsd('12.345')).toBeUndefined();
    expect(parseUsd('2000000')).toBeUndefined();
  });

  it('says what a rate limit report means', () => {
    const n = (status: string, limit?: string): RateLimitNote => ({
      at: 1,
      account: 'default',
      status,
      source: 'stream',
      ...(limit ? { limit } : {}),
    });
    expect(rateLimitText(n('rejected', 'five_hour'))).toBe(
      'Claude refused a request on the 5-hour limit',
    );
    expect(rateLimitText(n('allowed_warning', 'seven_day_opus'))).toBe(
      'Claude warned that the Opus weekly limit is near',
    );
    expect(rateLimitText(n('allowed'))).toBe('Claude reported a usage limit');
  });

  it('keeps the scheduling windows, 5-hour first', () => {
    const w = (key: string) => ({ key, label: key, pct: 1 });
    expect(
      schedulingWindows([w('seven_day_opus'), w('overage'), w('five_hour'), w('seven_day')]).map(
        (x) => x.key,
      ),
    ).toEqual(['five_hour', 'seven_day', 'seven_day_opus']);
  });

  it('reads a capacity view tolerantly', () => {
    expect(normalizeCapacity(null)).toBeNull();
    expect(normalizeCapacity({ caps: {} })).toBeNull();
    const full = new MockCapacity({ desktop: true }).view();
    const v = normalizeCapacity({ ...full, queue: 'nope', routes: undefined });
    expect(v).not.toBeNull();
    expect(v?.queue).toEqual([]);
    expect(v?.routes).toEqual([]);
    expect(v?.running.length).toBe(full.running.length);
    expect(v?.canEditPolicy).toBe(true);
  });

  it('names budgets and their notes in the language shown', async () => {
    const base = { spentUsd: 1, limitUsd: 2, pct: 50, level: 'ok' } as const;
    const project = {
      ...base,
      scope: 'project',
      key: '/dev/site',
      label: 'site today',
      project: '/dev/site',
    };
    const session = {
      ...base,
      scope: 'session',
      key: 's1',
      label: 'writer in site',
      project: '/dev/site',
      role: 'writer',
      note: 'a plan account: not passed to Claude Code, shown only',
    };
    const task = {
      ...base,
      scope: 'task',
      key: '12',
      label: 'task 12 in site',
      project: '/dev/site',
    };
    const old = { ...base, scope: 'task', key: '7', label: 'from an older server' };
    const states = [project, session, task, old] as unknown as BudgetState[];
    const note =
      'Estimates: token usage from the sessions\u2019 transcripts priced at first-party API rates. On plan accounts (Pro, Max, Team) the work is paid in usage windows, so dollars are what it would have cost through the API.';
    expect(states.map(budgetLabel)).toEqual([
      'site today',
      'writer in site',
      'task 12 in site',
      'from an older server',
    ]);
    expect(budgetNote(session.note)).toBe(session.note);
    try {
      await useLanguage('vi');
      expect(states.map(budgetLabel)).toEqual([
        'site hôm nay',
        'writer trong site',
        'tác vụ 12 trong site',
        'from an older server',
      ]);
      expect(budgetNote(session.note)).toBe(
        'tài khoản theo gói: không chuyển cho Claude Code, chỉ để xem',
      );
      expect(budgetNote('something new')).toBe('something new');
      expect(estimateNoteText(note)).toMatch(/^Số liệu ước tính/);
      expect(estimateNoteText('Another note.')).toBe('Another note.');
    } finally {
      await useLanguage('en');
    }
  });

  it('names task classes and efforts in the language shown', async () => {
    expect(['plan', 'planning', 'ops', 'review', 'mystery'].map(taskClassLabel)).toEqual([
      'planning',
      'planning',
      'operations',
      'review',
      'mystery',
    ]);
    expect(effortIdLabel('xhigh')).toBe('xhigh');
    try {
      await useLanguage('vi');
      expect(['plan', 'implementation', 'summarize', 'mystery'].map(taskClassLabel)).toEqual([
        'lập kế hoạch',
        'triển khai',
        'tóm tắt',
        'mystery',
      ]);
      expect(choiceLabel({ model: 'opus', effort: 'high' })).toBe('Opus · cao');
      expect(effortIdLabel('turbo')).toBe('turbo');
    } finally {
      await useLanguage('en');
    }
  });
});
