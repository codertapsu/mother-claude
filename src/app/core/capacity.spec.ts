import {
  RateLimitNote,
  bytesLabel,
  choiceLabel,
  csvName,
  effortRank,
  normalizeCapacity,
  parseUsd,
  pctLabel,
  policyProblems,
  projectName,
  rank,
  rateLimitText,
  ratioLabel,
  schedulingWindows,
  tier,
  validModel,
} from './capacity';
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
    expect(validModel('opus')).toBeTrue();
    expect(validModel('claude-opus-4-8[1m]')).toBeTrue();
    expect(validModel('gpt-5')).toBeFalse();
    expect(validModel('claude-')).toBeFalse();
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
    expect(v?.canEditPolicy).toBeTrue();
  });
});
