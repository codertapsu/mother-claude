import { SessionSummary } from '../core/models';
import {
  effortLabel,
  effortSourceText,
  formatTokens,
  relativeTime,
  switchPlace,
  thinkingModeLabel,
  thinkingShare,
  thinkingSourceText,
  ultracodeLabel,
} from './util';

/**
 * What the list card, the session header and the Info tab say about a
 * session's effort, ultracode, thinking and fast mode. Every value carries
 * how it is known: Claude Code reports these only partly (ultracode only
 * when a message is sent, thinking not at all), so the UI never states a
 * guess as a fact.
 */

type S = Pick<
  SessionSummary,
  | 'effort'
  | 'effortSource'
  | 'ultracode'
  | 'ultracodeSource'
  | 'ultracodeAt'
  | 'ultracodeUnknown'
  | 'ultracodeBefore'
  | 'ultracodeBeforeAt'
  | 'thinking'
  | 'lastThinkingAt'
  | 'fast'
  | 'usage'
  | 'live'
  | 'owned'
  | 'startedAt'
  | 'surface'
>;

/** Clock and rounding slack when dating a record against the process start
 * (the server uses the same). */
const START_SLACK_MS = 5_000;

/** Claude Code applies no effort level to this session: its model has none
 * (Haiku). An applied setting without a level, not an unknown one. */
export function noEffortLevels(s: Pick<SessionSummary, 'effort' | 'effortSource'>): boolean {
  return !s.effort && s.effortSource === 'applied';
}

/** "Max (as sent with the latest request)". */
export function effortText(s: S): string {
  if (!s.effort) return '';
  const how = effortSourceText(s.effortSource);
  return how ? `${effortLabel(s.effort)} (${how})` : effortLabel(s.effort);
}

/** How the ultracode value is known: "as of the last message, 5m ago". */
export function ultracodeWhen(s: S, now = Date.now()): string {
  switch (s.ultracodeSource) {
    case 'transcript':
      return s.ultracodeAt
        ? `as of the last message, ${relativeTime(s.ultracodeAt, now)}`
        : 'as of the last message';
    case 'launch':
      return 'set at launch';
    case 'applied':
      return 'as Claude Code applies it';
    case 'requested':
      return 'requested, not confirmed yet';
    default:
      return '';
  }
}

/** "On (as of the last message, 5m ago)" · "Unavailable (…)" · '' unknown. */
export function ultracodeText(s: S, now = Date.now()): string {
  if (!s.ultracode) return '';
  if (s.ultracode === 'unavailable') {
    return "Unavailable (requested, but this model or workspace can't run it)";
  }
  const when = ultracodeWhen(s, now);
  return when ? `${ultracodeLabel(s.ultracode)} (${when})` : ultracodeLabel(s.ultracode);
}

/** " in VS Code" (with its leading space), or '' where there is no one
 * place the session is driven from. */
function there(s: S): string {
  const place = switchPlace(s.surface);
  return place ? ` ${place}` : '';
}

/** Why a session's ultracode isn't known, as a short phrase after
 * "Unknown ·" ('' when it is known, or nothing more can be said). */
export function ultracodeUnknownText(s: S, now = Date.now()): string {
  if (s.ultracode) return '';
  if (s.owned && s.live) return "Claude Code hasn't reported it yet";
  switch (s.ultracodeUnknown) {
    case 'no-prompt':
      return s.live ? `shows after the first message${there(s)}` : 'no message was sent';
    case 'restarted': {
      if (!s.ultracodeBefore) return 'the session restarted since the last message';
      const when = s.ultracodeBeforeAt ? ` (${relativeTime(s.ultracodeBeforeAt, now)})` : '';
      return `was ${s.ultracodeBefore} at the last message${when}, before the restart`;
    }
    case 'reading':
      return "still reading this session's history";
    case 'compacted':
      return "can't tell since the conversation was compacted";
    case 'unsupported':
      return "this Claude Code version doesn't note it";
    default:
      return s.live ? `shows after the next message${there(s)}` : '';
  }
}

/** The title and accessible name of the header's "Ultracode ?" chip. */
export function ultracodeUnknownTitle(s: S, now = Date.now()): string {
  if (s.ultracodeUnknown === 'restarted' && s.ultracodeBefore) {
    return `Ultracode: not known since the session restarted (it was ${s.ultracodeBefore}); the next message${there(s)} shows it`;
  }
  const why = ultracodeUnknownText(s, now);
  return why ? `Ultracode: not known; ${why}` : 'Ultracode: not known';
}

/** "Adaptive (Claude Code's default)" · "On · 32k budget (at launch)". */
export function thinkingSettingText(s: S): string {
  const t = s.thinking;
  if (!t) return '';
  const mode = thinkingModeLabel(t.mode);
  const budget = t.mode === 'on' && t.budget ? ` · ${formatTokens(t.budget)} budget` : '';
  const how = thinkingSourceText(t.source);
  return `${mode}${budget}${how ? ` (${how})` : ''}`;
}

/** The main transcript has a thinking block from the session's current
 * process (any at all for a session that isn't running). A resumed session
 * carries the thinking of its earlier runs. */
export function thoughtThisRun(s: S): boolean {
  if (!s.lastThinkingAt) return false;
  if (!s.live || !s.startedAt) return true;
  return s.lastThinkingAt + START_SLACK_MS >= s.startedAt;
}

/** The setting says off, but only as launched or by default, and the
 * session has thought since: thinking was switched on in the session itself
 * (nothing reports that), so "off" would be a stale guess. What Mother
 * Claude set itself stands. */
export function thinkingOffContradicted(s: S): boolean {
  const t = s.thinking;
  return !!t && t.mode === 'off' && t.source !== 'control' && thoughtThisRun(s);
}

/** "12k tokens · 23% of output · last used 3m ago". */
export function thinkingEvidence(s: S, now = Date.now()): string {
  const tokens = s.usage?.thinking ?? 0;
  if (!tokens) return s.live ? 'No thinking tokens so far' : 'No thinking tokens';
  const share = thinkingShare(s.usage);
  return [
    `${formatTokens(tokens)} tokens`,
    share !== null ? `${share}% of output` : '',
    s.lastThinkingAt ? `last used ${relativeTime(s.lastThinkingAt, now)}` : '',
  ]
    .filter(Boolean)
    .join(' · ');
}

/** "9.8k tokens, 24% of the output" ('' when none were counted). */
function thinkingUsed(s: S): string {
  const tokens = s.usage?.thinking ?? 0;
  if (!tokens) return '';
  const share = thinkingShare(s.usage);
  return `${formatTokens(tokens)} tokens${share !== null ? `, ${share}% of the output` : ''}`;
}

/** The list card's thinking chip: "Thinking off", or the share of output
 * tokens spent thinking (`short`: the same without the word, for the
 * tightest layouts); null when there is nothing worth a chip. */
export function thinkingChip(s: S): { text: string; short: string; title: string } | null {
  const share = thinkingShare(s.usage);
  const setting = thinkingSettingText(s);
  const contradicted = thinkingOffContradicted(s);
  if (s.thinking?.mode === 'off' && !contradicted) {
    return { text: 'Thinking off', short: 'Off', title: `Thinking: ${setting}` };
  }
  if (share === null) return null;
  const note = contradicted ? ` · ${setting}, but used since` : setting ? ` · ${setting}` : '';
  return {
    text: `Thinking ${share}%`,
    short: `${share}%`,
    title: `Thinking: ${thinkingUsed(s)}${note}`,
  };
}

/** The session header's read-only thinking chip: the setting when it is
 * known (and not contradicted), else how much thinking was used; '' when
 * there is nothing to say. */
export function thinkingLabel(s: S): string {
  const t = s.thinking;
  if (t && !thinkingOffContradicted(s)) {
    return t.mode === 'off'
      ? 'Thinking off'
      : `Thinking ${thinkingModeLabel(t.mode).toLowerCase()}`;
  }
  const share = thinkingShare(s.usage);
  if (share !== null) return `Thinking ${share}%`;
  return t ? 'Thinking used' : '';
}

/** What that chip says about itself (its title and accessible name): the
 * setting and how it is known, else the tokens used. */
export function thinkingTitle(s: S): string {
  const setting = thinkingSettingText(s);
  const used = thinkingUsed(s);
  if (setting && thinkingOffContradicted(s)) {
    return `Thinking: ${setting}, but used since${used ? ` (${used})` : ''}`;
  }
  if (setting) return `Thinking: ${setting}`;
  return used
    ? `Thinking: ${used}; the setting isn't reported`
    : "Thinking: the setting isn't reported";
}

/** How fast mode is known. Outside owned sessions only the speed of the
 * latest request shows it; an owned session also reports the mode being
 * switched on (before any request ran in it). */
export function fastText(s: Pick<SessionSummary, 'owned' | 'live'>): string {
  return s.owned && s.live
    ? 'switched on, or the latest request ran in it'
    : 'the latest request ran in fast mode';
}

/** The sentences behind the header's ultracode chip. */
export function explainUltracode(s: S, now = Date.now()): string[] {
  const lines: string[] = [];
  switch (s.ultracode) {
    case 'on':
      lines.push(
        'Ultracode is on: Claude runs substantive tasks as multi-agent workflows, at any effort level. It lasts for this session only.',
      );
      break;
    case 'off':
      lines.push('Ultracode is off.');
      break;
    case 'unavailable':
      lines.push(
        "Ultracode was requested, but it can't run here: it needs a model that supports it and workflows switched on.",
        'The request stays in place: ultracode comes back on by itself as soon as it can run (after a switch to a model that supports it, say).',
      );
      break;
    default:
      lines.push(...explainUnknownUltracode(s, now));
  }
  if (s.ultracodeSource === 'transcript') {
    const at = s.ultracodeAt ? ` (${relativeTime(s.ultracodeAt, now)})` : '';
    lines.push(
      s.live
        ? `As of the last message${at}. Claude Code notes ultracode only when a message is sent, so a switch made${there(s)} since then shows here after the next message.`
        : `As of the last message${at}.`,
    );
  } else if (s.ultracodeSource === 'launch') {
    lines.push('Set when the session was started; a later switch shows after the next message.');
  } else if (s.ultracodeSource === 'requested') {
    lines.push(
      "Mother Claude asked for this, and Claude Code hasn't confirmed it yet (after a model change it may turn out unable to run).",
    );
  }
  return lines;
}

/** Why ultracode isn't known, and when it will be. */
function explainUnknownUltracode(s: S, now: number): string[] {
  if (s.owned && s.live) return ["Claude Code hasn't reported ultracode for this session yet."];
  const reading =
    "Not known yet: Mother Claude is still reading this session's history, where Claude Code notes ultracode.";
  const compacted =
    "The conversation was compacted since Claude Code last noted ultracode, and it notes it again only when it changes (or every ten or so messages while it is on), so the messages since then can't tell.";
  const unsupported =
    "This session's Claude Code version doesn't note ultracode in the conversation.";
  if (!s.live) {
    switch (s.ultracodeUnknown) {
      case 'reading':
        return [reading];
      case 'compacted':
        return [`Not recorded. ${compacted}`];
      case 'unsupported':
        return [`Not recorded. ${unsupported}`];
      default:
        return ['Not recorded for this session.'];
    }
  }
  const perSession =
    'Ultracode is set per session: switching it on in one session leaves the others as they are.';
  switch (s.ultracodeUnknown) {
    case 'no-prompt':
      return [
        `Not known yet. Claude Code notes ultracode in the conversation only when a message is sent, so it shows here after the first message${there(s)}.`,
        perSession,
      ];
    case 'restarted': {
      const when = s.ultracodeBeforeAt ? ` (${relativeTime(s.ultracodeBeforeAt, now)})` : '';
      return [
        s.ultracodeBefore
          ? `Not known since this session restarted. It was ${s.ultracodeBefore} at the last message before the restart${when}.`
          : 'Not known since this session restarted.',
        `A restart turns ultracode off until it is switched on again${there(s)}, and Claude Code notes it only when a message is sent: the next message${there(s)} shows whether it is on.`,
      ];
    }
    case 'reading':
      return [reading];
    case 'compacted':
      return [`Not known. ${compacted}`];
    case 'unsupported':
      return [unsupported];
    default:
      return [
        `Not known yet. Claude Code notes ultracode in the conversation only when a message is sent, so a switch made${there(s)} shows here after the next message.`,
        perSession,
      ];
  }
}

/** The sentences behind the header's thinking chip. */
export function explainThinking(s: S, now = Date.now()): string[] {
  const lines: string[] = [];
  const t = s.thinking;
  if (!t) {
    lines.push("Claude Code doesn't report whether thinking is on for this session.");
  } else if (thinkingOffContradicted(s)) {
    lines.push(
      t.source === 'launch'
        ? 'Thinking was off when the session was launched, but it has thought since: it was switched on in the session itself.'
        : 'Your Claude settings turn thinking off by default, but this session has thought since: it was switched on in the session itself.',
      "Mother Claude can't see such a change, so it shows the thinking used instead.",
    );
  } else {
    if (t.mode === 'adaptive') {
      lines.push('Adaptive thinking: Claude decides when, and how much, to think.');
    } else if (t.mode === 'on') {
      const budget = t.budget ? ` with a budget of ${formatTokens(t.budget)} tokens` : '';
      lines.push(`Thinking is on${budget}.`);
    } else {
      lines.push('Thinking is off.');
    }
    if (t.source === 'control') {
      lines.push('Set by Mother Claude.');
    } else if (t.source === 'launch') {
      lines.push(
        'As the session was launched: VS Code and the terminal can switch thinking later without Mother Claude seeing it.',
      );
    } else {
      lines.push(
        "Claude Code's default for this model and your settings; a change made in the session itself isn't reported.",
      );
    }
  }
  const tokens = s.usage?.thinking ?? 0;
  if (tokens) {
    const share = thinkingShare(s.usage);
    const last = s.lastThinkingAt ? `, most recently ${relativeTime(s.lastThinkingAt, now)}` : '';
    lines.push(
      `This session and its agents used ${formatTokens(tokens)} thinking tokens${share !== null ? ` (${share}% of the output)` : ''}${last}.`,
    );
  } else if (t?.mode !== 'off') {
    lines.push("No thinking tokens so far: with adaptive thinking that doesn't mean it is off.");
  }
  return lines;
}

/** The sentence behind the header's fast chip. */
export function explainFast(s: Pick<SessionSummary, 'owned' | 'live'>): string {
  return s.owned && s.live
    ? 'Fast mode is on: Claude Code reports its faster output mode as switched on for this session, or the latest request ran in it.'
    : "Fast mode is on: the latest request ran in Claude Code's faster output mode.";
}
