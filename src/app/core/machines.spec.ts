import { defaultGrants, groupDigits, healthAlarm, pairingLive, setGrant } from './machines';
import { machinesView, pairing } from '../pages/machines/machines.testing';

describe('machines helpers', () => {
  it('keeps what a grant needs: on switches its bases on, off its dependents off', () => {
    const none = setGrant(
      setGrant(defaultGrants(), 'view', 'lan', false),
      'view',
      'internet',
      false,
    );
    expect(Object.values(none).every((p) => !p.lan && !p.internet)).toBe(true);

    // Dangerous on the same network needs driving, which needs seeing.
    const danger = setGrant(none, 'dangerous', 'lan', true);
    expect(danger.dangerous).toEqual({ lan: true, internet: false });
    expect(danger.control).toEqual({ lan: true, internet: false });
    expect(danger.view).toEqual({ lan: true, internet: false });
    expect(danger.move).toEqual({ lan: false, internet: false });

    // No driving over the internet: nothing that needs it either.
    const noDrive = setGrant(defaultGrants(), 'control', 'internet', false);
    expect(noDrive.view.internet).toBe(true);
    expect(noDrive.move.internet).toBe(false);
    expect(noDrive.move.lan).toBe(true);
  });

  it('raises the health dot only while Machines is on or being set up', () => {
    const blocked = { state: 'blocker' as const, blockers: 1, warnings: 0, at: 1 };
    expect(healthAlarm(machinesView({ health: blocked }))).toBe(false);
    expect(healthAlarm(machinesView({ health: blocked, setupStep: 2 }))).toBe(true);
    expect(healthAlarm(machinesView({ health: blocked, enabled: true }))).toBe(true);
    expect(
      healthAlarm(
        machinesView({
          enabled: true,
          health: { state: 'warning', blockers: 0, warnings: 2, at: 1 },
        }),
      ),
    ).toBe(false);
  });

  it('knows a pairing that has not ended, and groups its digits', () => {
    expect(pairingLive(pairing())).toBe(true);
    expect(pairingLive(pairing({ state: 'paired' }))).toBe(false);
    expect(pairingLive(pairing({ state: 'failed' }))).toBe(false);
    expect(pairingLive(null)).toBe(false);
    expect(groupDigits('481270')).toBe('481 270');
  });
});
