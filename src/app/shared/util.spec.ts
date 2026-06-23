import { formatTokens, relativeTime, stateLabel, surfaceLabel } from './util';

describe('shared/util', () => {
  it('formats token counts', () => {
    expect(formatTokens(0)).toBe('0');
    expect(formatTokens(950)).toBe('950');
    expect(formatTokens(1500)).toBe('1.5k');
    expect(formatTokens(2_500_000)).toBe('2.50M');
  });

  it('formats relative time', () => {
    expect(relativeTime(undefined)).toBe('—');
    expect(relativeTime(Date.now() - 5_000)).toMatch(/s ago$/);
    expect(relativeTime(Date.now() - 3 * 60_000)).toMatch(/m ago$/);
  });

  it('labels states and surfaces', () => {
    expect(stateLabel('needs-input')).toBe('needs input');
    expect(surfaceLabel('vs-code')).toBe('VS Code');
    expect(surfaceLabel('cli')).toBe('CLI');
    expect(surfaceLabel('unknown')).toBe('Unknown');
  });
});
