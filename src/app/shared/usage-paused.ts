import { tr } from '../core/i18n/i18n';
import { PausedInfo, windowText } from '../core/usage-guard';
import { formatCountdown, resetClock } from './util';

/**
 * How a paused session is spoken of (Settings › Usage limits;
 * `claude::usage_guard`): its marker in lists, and the sentence the session
 * page and the tooltips say.
 */

/** The marker: "Paused until 21:11", or "Paused" while the pause waits for
 * someone to resume it. */
export function pausedLabel(info: PausedInfo, now: number): string {
  if (info.pause.hold) return tr('Paused');
  return tr('Paused until {time}', { time: resetClock(info.pause.resumeAt, now) });
}

/** Why it is paused and when it goes on, in two short sentences. */
export function pausedDetail(info: PausedInfo, now: number): string {
  const p = info.pause;
  const words = { pct: Math.round(p.pct), window: windowText(p.window, p.label) };
  const at = { time: resetClock(p.resumeAt, now), left: formatCountdown(p.resumeAt - now) };
  if (info.kind === 'held') {
    const first = tr(
      'Its scheduled run waits: Mother Claude paused this account at {pct} % of {window}.',
      words,
    );
    const when = p.hold
      ? tr('It waits until you resume it.')
      : tr('The run waits until {time} (in {left}).', at);
    return `${first} ${when}`;
  }
  const first = tr('Mother Claude paused it at {pct} % of {window}.', words);
  const when = p.hold
    ? tr('It waits until you resume it.')
    : tr('It resumes at {time} (in {left}).', at);
  return `${first} ${when}`;
}

/** `pausedDetail` for the session page's line, beside `pausedWhat`: for a
 * held run, why and until when, without saying twice that a run waits. */
export function pausedWhy(info: PausedInfo, now: number): string {
  if (info.kind !== 'held') return pausedDetail(info, now);
  const p = info.pause;
  const first = tr('Mother Claude paused this account at {pct} % of {window}.', {
    pct: Math.round(p.pct),
    window: windowText(p.window, p.label),
  });
  const when = p.hold
    ? tr('It waits until you resume it.')
    : tr('The run waits until {time} (in {left}).', {
        time: resetClock(p.resumeAt, now),
        left: formatCountdown(p.resumeAt - now),
      });
  return `${first} ${when}`;
}

/** What stopped, or which run waits (a session's chip in the banner). */
export function pausedWhat(info: PausedInfo, now: number): string {
  const s = info.session;
  if (info.kind === 'held') {
    const time = s.runAt ? resetClock(s.runAt, now) : '';
    if (s.run && time) return tr('Its scheduled run “{run}” ({time}) waits.', { run: s.run, time });
    if (s.run) return tr('Its scheduled run “{run}” waits.', { run: s.run });
    return tr('Its scheduled run waits.');
  }
  return s.doing.length
    ? tr('Stopped: {what}.', { what: s.doing.map(doingText).join(', ') })
    : tr('Its work was stopped.');
}

/** One thing a pause stopped, as the server words it for the session
 * (English), in the page's language. */
export function doingText(d: string): string {
  if (d === 'the turn it was working on') return tr('the turn it was working on');
  if (/^the turn (its|a) scheduled run/.test(d)) return tr('the turn its scheduled run started');
  const named = /^(Agent|Workflow) “(.*)”$/.exec(d);
  if (named) {
    return named[1] === 'Agent'
      ? tr('agent “{name}”', { name: named[2] })
      : tr('workflow “{name}”', { name: named[2] });
  }
  return d;
}
