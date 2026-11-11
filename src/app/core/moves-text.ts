// What the move screens say (docs/PLAN-MACHINES.md §5.8, spikes/machines/UX.md
// §7): a move's steps and states, its outcome, and the server's sentences —
// the preflight's checks and fixes, a failure's reason, a refusal — in the
// language shown. The server writes those in English
// (`src-tauri/src/machines/moves/*.rs`, `lease.rs`, `code/types.rs`
// `ApplyStop`); the ones it composes from a fixed pattern are matched here,
// names, paths and numbers kept (docs/I18N.md, "Server text drawn from a
// fixed list"), and anything else is shown as it came. Mirrors those files:
// change both together.

import { ApiError, MachineApiError, describeError } from './api.service';
import { computerNoun, locale, tr } from './i18n/i18n';
import {
  MoveCheck,
  MoveMachine,
  MovePreflight,
  MoveRecord,
  MoveState,
  canRetry,
  moveFinal,
  packageSizes,
  reachedAt,
  secondsLeft,
} from './moves';
import { formatBytes } from './data-jobs';

/** "this Mac" → "This Mac" at the start of a line. */
function capitalize(s: string): string {
  return s ? s.charAt(0).toLocaleUpperCase() + s.slice(1) : s;
}

type Rule = readonly [RegExp, (m: string[]) => string];

/** "this computer" as the server says it, in the app's words. */
const here = () => tr('this {computer}', { computer: computerNoun() });

/** The server's sentences of a move, and theirs in the language shown. */
const RULES: readonly Rule[] = [
  // ── Sentences around another one (first: the one inside would match
  // the patterns below whole) ──
  [
    /^It can't move yet: (.+)$/,
    (m) => tr("It can't move yet: {reason}", { reason: moveText(m[1]) }),
  ],
  [
    /^The code can't land here: (.+)$/,
    (m) => tr("The code can't land here: {reason}", { reason: moveText(m[1]) }),
  ],
  // ── The preflight: this computer's checks (source.rs) ──
  [
    /^A conversation of the API can't move to another computer yet\.$/,
    () => tr("A conversation of the API can't move to another computer yet."),
  ],
  [
    /^It runs in (.+) — Mother Claude can't stop it\. A copy can go on on (.+) as a new session while this one goes on here, or end it there first\.$/,
    (m) =>
      tr(
        "It runs in {app} — Mother Claude can't stop it. A copy can go on on {name} as a new session while this one goes on here, or end it there first.",
        { app: m[1], name: m[2] },
      ),
  ],
  [
    /^It runs without permission prompts: moving it needs the dangerous right here\.$/,
    () => tr('It runs without permission prompts: moving it needs the dangerous right here.'),
  ],
  [
    /^Carrying a folder's trust needs the dangerous permission \(set on the desktop\)\.$/,
    () => tr("Carrying a folder's trust needs the dangerous permission (set on the desktop)."),
  ],
  // The files git ignores that a project carries (ignored.rs, source.rs).
  [
    /^Carrying files git ignores needs the dangerous permission \(set on the desktop\)\.$/,
    () => tr('Carrying files git ignores needs the dangerous permission (set on the desktop).'),
  ],
  [
    /^The project lists more than (\d+) files git ignores: at most \d+ travel\.$/,
    (m) =>
      tr('The project lists more than {n} files git ignores: at most {n} travel.', { n: m[1] }),
  ],
  [
    /^(.+) is not a file inside the project, so it can't travel\.$/,
    (m) => tr("{file} is not a file inside the project, so it can't travel.", { file: m[1] }),
  ],
  [
    /^(.+) is larger than 1 MB: files git ignores travel only up to 1 MB each\.$/,
    (m) =>
      tr('{file} is larger than 1 MB: files git ignores travel only up to 1 MB each.', {
        file: m[1],
      }),
  ],
  [
    /^(.+) is a symbolic link, or inside one: only plain files travel\.$/,
    (m) => tr('{file} is a symbolic link, or inside one: only plain files travel.', { file: m[1] }),
  ],
  [
    /^(.+) is not a plain file: only plain files travel\.$/,
    (m) => tr('{file} is not a plain file: only plain files travel.', { file: m[1] }),
  ],
  [
    /^(.+) could not be read here: (.+)$/,
    (m) => tr('{file} could not be read here: {error}', { file: m[1], error: m[2] }),
  ],
  [
    /^(.+)'s Mother Claude can't take files git ignores yet\.$/,
    (m) => tr("{name}'s Mother Claude can't take files git ignores yet.", { name: m[1] }),
  ],
  [
    /^A git operation is under way in (.+) \((.+)\): finish it first, or move only the conversation\.$/,
    (m) =>
      tr(
        'A git operation is under way in {folder} ({operation}): finish it first, or move only the conversation.',
        { folder: m[1], operation: m[2] },
      ),
  ],
  [
    /^This repository has no remote named (.+): move the code machine to machine instead\.$/,
    (m) =>
      tr(
        'This repository has no remote named {remote}: move the code machine to machine instead.',
        {
          remote: m[1],
        },
      ),
  ],
  [
    /^Its folder is not in a git repository: only the conversation goes\.$/,
    () => tr('Its folder is not in a git repository: only the conversation goes.'),
  ],
  [
    /^(.+) is not in a git repository: only the conversation goes\.$/,
    (m) => tr('{folder} is not in a git repository: only the conversation goes.', { folder: m[1] }),
  ],
  [
    /^Its branch on (.+) has commits this computer doesn't\.$/,
    (m) =>
      tr("Its branch on {name} has commits this {computer} doesn't.", {
        name: m[1],
        computer: computerNoun(),
      }),
  ],
  [
    /^(.+) on (.+) has commits this computer doesn't\.$/,
    (m) =>
      tr("{branch} on {name} has commits this {computer} doesn't.", {
        branch: m[1],
        name: m[2],
        computer: computerNoun(),
      }),
  ],
  [
    /^The folder on (.+) is another repository \(its origin is (.+)\)\.$/,
    (m) =>
      tr('The folder on {name} is another repository (its origin is {origin}).', {
        name: m[1],
        origin: m[2],
      }),
  ],
  [
    /^Claude Code on (.+) is older \((.+), here (.+)\)\.$/,
    (m) =>
      tr('Claude Code on {name} is older ({there}, here {here}).', {
        name: m[1],
        there: m[2],
        here: m[3],
      }),
  ],
  [
    /^(.+)'s Mother Claude can't take sessions yet\.$/,
    (m) => tr("{name}'s Mother Claude can't take sessions yet.", { name: m[1] }),
  ],
  [
    /^(.+) can't be reached right now: (.+)$/,
    (m) => tr("{name} can't be reached right now: {error}", { name: m[1], error: m[2] }),
  ],
  [
    /^(.+) answered oddly: (.+)$/,
    (m) => tr('{name} answered oddly: {error}', { name: m[1], error: m[2] }),
  ],
  [
    /^(.+) has not given this computer the Move permission on the local network\.$/,
    (m) =>
      tr('{name} has not given this {computer} the Move permission on the local network.', {
        name: m[1],
        computer: computerNoun(),
      }),
  ],
  [
    /^(.+) has not given this computer the Move permission over the internet\.$/,
    (m) =>
      tr('{name} has not given this {computer} the Move permission over the internet.', {
        name: m[1],
        computer: computerNoun(),
      }),
  ],
  // ── Machines turned off (machines/mod.rs `using`, `serving`) ──
  [
    /^Machines is off on this computer: turn it on on the Machines page first\.$/,
    () =>
      tr('Machines is off on this {computer}: turn it on on the Machines page first.', {
        computer: computerNoun(),
      }),
  ],
  [/^Machines is off on (.+)\.$/, (m) => tr('Machines is off on {name}.', { name: m[1] })],
  // ── The preflight: the other computer's checks (target.rs, folder.rs) ──
  [
    /^(.+) has no usable account named (.+)\.$/,
    (m) => tr('{name} has no usable account named {account}.', { name: m[1], account: m[2] }),
  ],
  [
    /^(.+) has no account named (.*)\.$/,
    (m) => tr('{name} has no account named {account}.', { name: m[1], account: m[2] }),
  ],
  [
    /^(.+) on (.+) can't run sessions: (.+)$/,
    (m) =>
      tr("{account} on {name} can't run sessions: {error}", {
        account: m[1],
        name: m[2],
        error: m[3],
      }),
  ],
  [
    /^(.+) isn't signed in on (.+)\.$/,
    (m) => tr("{account} isn't signed in on {name}.", { account: m[1], name: m[2] }),
  ],
  [
    /^Couldn't check that (.+) is signed in on (.+)\.$/,
    (m) =>
      tr("Couldn't check that {account} is signed in on {name}.", { account: m[1], name: m[2] }),
  ],
  [
    /^(.+) runs this session right now\.$/,
    (m) => tr('{name} runs this session right now.', { name: m[1] }),
  ],
  [
    /^(.+) knows a newer home for this session \((.+)\)\.$/,
    (m) => tr('{name} knows a newer home for this session ({home}).', { name: m[1], home: m[2] }),
  ],
  [
    /^It runs without permission prompts: going on on (.+) needs its Dangerous permission for this computer\.$/,
    (m) =>
      tr(
        'It runs without permission prompts: going on on {name} needs its Dangerous permission for this {computer}.',
        { name: m[1], computer: computerNoun() },
      ),
  ],
  [
    /^(.+) is not a folder on this (?:Mac|PC|machine|computer)\.$/,
    (m) => tr('{folder} is not a folder there.', { folder: m[1] }),
  ],
  [
    /^(.+) has no folder for (.+): choose its folder there\.$/,
    (m) =>
      tr('{name} has no folder for {folder}: choose its folder there.', {
        name: m[1],
        folder: m[2],
      }),
  ],
  [
    /^(.+) doesn't trust (.+) yet\.$/,
    (m) => tr("{name} doesn't trust {folder} yet.", { name: m[1], folder: m[2] }),
  ],
  [
    /^Carrying the folder's trust needs (.+)'s Dangerous permission for this computer\.$/,
    (m) =>
      tr("Carrying the folder's trust needs {name}'s Dangerous permission for this {computer}.", {
        name: m[1],
        computer: computerNoun(),
      }),
  ],
  [
    /^Carrying files git ignores needs (.+)'s Dangerous permission for this computer\.$/,
    (m) =>
      tr("Carrying files git ignores needs {name}'s Dangerous permission for this {computer}.", {
        name: m[1],
        computer: computerNoun(),
      }),
  ],
  [
    /^(.+) had its own (.+): kept for Undo move\.$/,
    (m) => tr('{name} had its own {files}: kept for Undo move.', { name: m[1], files: m[2] }),
  ],
  [
    /^Changed on (.+) since the move, so left as they are: (.+)\.$/,
    (m) =>
      tr('Changed on {name} since the move, so left as they are: {files}.', {
        name: m[1],
        files: m[2],
      }),
  ],
  [
    /^A git operation is under way in (.+) \((.+)\)\.$/,
    (m) =>
      tr('A git operation is under way in {folder} ({operation}).', {
        folder: m[1],
        operation: m[2],
      }),
  ],
  [
    /^(.+) has changes of its own \((\d+) files?\)\. The code can't land in the project folder\.$/,
    (m) =>
      m[2] === '1'
        ? tr(
            "{folder} has changes of its own (1 file). The code can't land in the project folder.",
            {
              folder: m[1],
            },
          )
        : tr(
            "{folder} has changes of its own ({n} files). The code can't land in the project folder.",
            {
              folder: m[1],
              n: m[2],
            },
          ),
  ],
  [
    /^Its branch is checked out in another worktree there \((.+)\)\.$/,
    (m) => tr('Its branch is checked out in another worktree there ({folder}).', { folder: m[1] }),
  ],
  [
    /^A worktree of this session is still on (.+) \((.+)\): remove it there first\.$/,
    (m) =>
      tr('A worktree of this session is still on {name} ({folder}): remove it there first.', {
        name: m[1],
        folder: m[2],
      }),
  ],
  [
    /^(.+) on (.+) has no remote named (.+)\.$/,
    (m) =>
      tr('{folder} on {name} has no remote named {remote}.', {
        folder: m[1],
        name: m[2],
        remote: m[3],
      }),
  ],
  [
    /^(.+) on (.+) is not a git repository\.$/,
    (m) => tr('{folder} on {name} is not a git repository.', { folder: m[1], name: m[2] }),
  ],
  [
    /^(.+) has too little free space for it\.$/,
    (m) => tr('{name} has too little free space for it.', { name: m[1] }),
  ],
  [
    /^It had gone on on (.+) too: those turns are kept beside it \((.+)\)\.$/,
    (m) =>
      tr('It had gone on on {name} too: those turns are kept beside it ({file}).', {
        name: m[1],
        file: m[2],
      }),
  ],
  // ── Fixes ──
  [
    /^Update Mother Claude on (.+)\.$/,
    (m) => tr('Update Mother Claude on {name}.', { name: m[1] }),
  ],
  [
    /^Check that Mother Claude runs on (.+) and that both computers are online\.$/,
    (m) =>
      tr('Check that Mother Claude runs on {name} and that both computers are online.', {
        name: m[1],
      }),
  ],
  [
    /^Change it on (.+)'s Machines page\.$/,
    (m) => tr("Change it on {name}'s Machines page.", { name: m[1] }),
  ],
  [
    /^Add the account (.*) on (.+) and sign in, then try again\.$/,
    (m) =>
      tr('Add the account {account} on {name} and sign in, then try again.', {
        account: m[1],
        name: m[2],
      }),
  ],
  [
    /^Sign (.+) in on (.+) \(Accounts there\), then try again\.$/,
    (m) =>
      tr('Sign {account} in on {name} (Accounts there), then try again.', {
        account: m[1],
        name: m[2],
      }),
  ],
  [
    /^Sign (.+) in on (.+)\.$/,
    (m) => tr('Sign {account} in on {name}.', { account: m[1], name: m[2] }),
  ],
  [/^Stop it on (.+) first\.$/, (m) => tr('Stop it on {name} first.', { name: m[1] })],
  [
    /^Open it once with Claude Code on (.+), or carry the folder's trust \(needs the dangerous permission\)\.$/,
    (m) =>
      tr(
        "Open it once with Claude Code on {name}, or carry the folder's trust (needs the dangerous permission).",
        {
          name: m[1],
        },
      ),
  ],
  [
    /^Use a separate folder \(worktree\) there\.$/,
    () => tr('Use a separate folder (worktree) there.'),
  ],
  [/^Choose the project's folder there\.$/, () => tr("Choose the project's folder there.")],
  [
    /^Clone the project there first, or move only the conversation\.$/,
    () => tr('Clone the project there first, or move only the conversation.'),
  ],
  [
    /^Leave them here, or change the files the project carries \(Machines › Options › Each project\)\.$/,
    () =>
      tr(
        'Leave them here, or change the files the project carries (Machines › Options › Each project).',
      ),
  ],
  // ── Why a move stopped (source.rs, target.rs) ──
  [/^It was cancelled\.$/, () => tr('It was cancelled.')],
  [/^The move's record is gone\.$/, () => tr("The move's record is gone.")],
  [
    /^Another process started running it meanwhile, so it did not move\.$/,
    () => tr('Another process started running it meanwhile, so it did not move.'),
  ],
  [
    /^The session's process did not stop in time, so it did not move: a second process on the same conversation would write over it\.$/,
    () =>
      tr(
        "The session's process did not stop in time, so it did not move: a second process on the same conversation would write over it.",
      ),
  ],
  [/^It kept changing while it was packed\.$/, () => tr('It kept changing while it was packed.')],
  [
    /^The code could not be packed: (.+)$/,
    (m) => tr('The code could not be packed: {error}', { error: m[1] }),
  ],
  [
    /^The files git ignores could not be packed: (.+)$/,
    (m) => tr('The files git ignores could not be packed: {error}', { error: m[1] }),
  ],
  [
    /^The files git ignores can't land here: (.+)$/,
    (m) => tr("The files git ignores can't land here: {reason}", { reason: m[1] }),
  ],
  [
    /^(.+) can't be reached: (.+)$/,
    (m) => tr("{name} can't be reached: {error}", { name: m[1], error: m[2] }),
  ],
  [
    /^(.+) went offline while it was sent: (.*)$/,
    (m) => tr('{name} went offline while it was sent.', { name: m[1] }),
  ],
  [
    /^(.+) did not say it placed it: (.+)$/,
    (m) => tr('{name} did not say it placed it: {error}', { name: m[1], error: m[2] }),
  ],
  [/^Mother Claude stopped during the move\.$/, () => tr('Mother Claude stopped during the move.')],
  [
    /^The target checkout has changes of its own \((\d+) paths?\)\.$/,
    (m) => tr('The project folder there has changes of its own ({n} files).', { n: m[1] }),
  ],
  [
    /^A git operation is in progress in the target checkout \((.+)\)\.$/,
    (m) =>
      tr('A git operation is under way in the project folder there ({operation}).', {
        operation: m[1],
      }),
  ],
  [
    /^The target's (.+) has commits the snapshot's base lacks\.$/,
    (m) => tr("{branch} there has commits this move's code doesn't.", { branch: m[1] }),
  ],
  [
    /^The target has files of its own where the move would write \((.+)\); it ignores them, so git would overwrite them\.$/,
    (m) =>
      tr(
        'There are files of its own where the move would write ({files}); git ignores them there, so it would overwrite them.',
        {
          files: m[1],
        },
      ),
  ],
  [
    /^The move did not arrive whole: (.+)\.$/,
    (m) => tr('The move did not arrive whole: {reason}.', { reason: m[1] }),
  ],
  [
    /^A backup, restore or transfer is running here; try again when it has finished\.$/,
    () => tr('A backup, restore or transfer is running there; try again when it has finished.'),
  ],
  [
    /^This computer runs the session right now\.$/,
    () => tr('The other computer runs the session right now.'),
  ],
  [
    /^The move's account or folder was not found here\.$/,
    () => tr("The move's account or folder was not found there."),
  ],
  [
    /^(.+) refused \((\d+)\)\.$/,
    (m) => tr('{name} refused ({status}).', { name: m[1], status: m[2] }),
  ],
  // ── Refusals of the routes (mod.rs, source.rs, target.rs, lease.rs) ──
  [
    /^This session lives on (.+) now: open it there, or bring it back to this computer first\.$/,
    (m) =>
      tr('This session lives on {name} now: open it there, or bring it back first.', {
        name: m[1],
      }),
  ],
  [
    /^This session is moving to (.+) right now; it goes on there in a moment\.$/,
    (m) =>
      tr('This session is moving to {name} right now; it goes on there in a moment.', {
        name: m[1],
      }),
  ],
  [/^This session is moving already\.$/, () => tr('This session is moving already.')],
  [
    /^This move is (.+)'s to cancel: it started there\.$/,
    (m) => tr("This move is {name}'s to cancel: it started there.", { name: m[1] }),
  ],
  [/^This move has finished already\.$/, () => tr('This move has finished already.')],
  [/^This move has ended here\.$/, () => tr('This move has finished already.')],
  [
    /^It is being placed on (.+) now: once it is there, Undo move takes it back\.$/,
    (m) =>
      tr('It is being placed on {name} now: once it is there, Undo move takes it back.', {
        name: m[1],
      }),
  ],
  [/^This move is not running any more\.$/, () => tr('This move is not running any more.')],
  [
    /^Only a move that is on the other computer but did not start there is tried again; start a new move instead\.$/,
    () =>
      tr(
        'Only a move that is on the other computer but did not start there is tried again; start a new move instead.',
      ),
  ],
  [
    /^A copy is not undone: stop or delete it on the other computer\.$/,
    () => tr('A copy is not undone: stop or delete it on the other computer.'),
  ],
  [
    /^Only a move that finished can be undone\.$/,
    () => tr('Only a move that finished can be undone.'),
  ],
  [/^The undo of this move has expired\.$/, () => tr('The undo of this move has expired.')],
  [
    /^Only a move that came here is undone here\.$/,
    () => tr('Only a move that came here is undone here.'),
  ],
  [
    /^It moved again since: undo that move first\.$/,
    () => tr('It moved again since: undo that move first.'),
  ],
  [
    /^Another process runs it on (.+): stop it there first\.$/,
    (m) => tr('Another process runs it on {name}: stop it there first.', { name: m[1] }),
  ],
  [
    /^It is working on (.+) right now: try again when its turn ends\.$/,
    (m) => tr('It is working on {name} right now: try again when its turn ends.', { name: m[1] }),
  ],
  [
    /^It went on on (.+) after the move: bring it back instead \(an undo would drop what happened here\)\.$/,
    (m) =>
      tr(
        'It went on on {name} after the move: bring it back instead (an undo would drop what happened there).',
        {
          name: m[1],
        },
      ),
  ],
  [
    /^The code can't be put back here: (.+)$/,
    (m) => tr("The code can't be put back there: {error}", { error: m[1] }),
  ],
  [
    /^The conversation can't be put back here: (.+)$/,
    (m) => tr("The conversation can't be put back there: {error}", { error: m[1] }),
  ],
  [
    /^The files git ignores can't be put back here: (.+)$/,
    (m) => tr("The files git ignores can't be put back there: {error}", { error: m[1] }),
  ],
  [
    /^It lives on this computer already\.$/,
    () => tr('It lives on {here} already.', { here: here() }),
  ],
  [
    /^This computer has nothing of this conversation: it can't go on here\.$/,
    () =>
      tr("This {computer} has nothing of this conversation: it can't go on here.", {
        computer: computerNoun(),
      }),
  ],
  [
    /^(.+) is online: bring it back here instead\.$/,
    (m) => tr('{name} is online: bring it back here instead.', { name: m[1] }),
  ],
  [/^No such move here\.$/, () => tr('That move is not kept there any more.')],
  [
    /^Nothing of this session is on disk yet: it moves once it has a conversation\.$/,
    () => tr('Nothing of this session is on disk yet: it moves once it has a conversation.'),
  ],
  [
    /^Only Claude Code sessions \(UUID ids\) move to another computer\.$/,
    () => tr('Only Claude Code sessions move to another computer.'),
  ],
  [
    /^It goes on here from this computer's copy\. If (.+) went on with it while it was away, both versions are kept side by side when it next moves there\.$/,
    (m) =>
      tr(
        "It goes on here from this {computer}'s copy. If {name} went on with it while it was away, both versions are kept side by side when it next moves there.",
        { name: m[1], computer: computerNoun() },
      ),
  ],
  // ── Starting it there ──
  [
    /^(.+) has not taken it over yet(?: \(.+\))?: it will once it hears from this computer\.$/,
    (m) =>
      tr('{name} has not taken it over yet: it will once the two computers are in touch again.', {
        name: m[1],
      }),
  ],
  [/^It did not start there\.$/, () => tr('It did not start there.')],
  [/^(.+) answered (\d+)\.$/, (m) => tr('{name} answered {status}.', { name: m[1], status: m[2] })],
];

/**
 * One of the server's sentences about a move in the language shown, the
 * names, paths and numbers in it kept; anything not matched as it came.
 */
export function moveText(text: string | null | undefined): string {
  const t = (text ?? '').trim();
  if (!t) return '';
  for (const [re, words] of RULES) {
    const m = re.exec(t);
    if (m) return words([...m]);
  }
  return t;
}

/** Who a refusal concerns: the computer the session lives on (and starts
 * the move), and the one it goes to. */
export interface MoveNames {
  /** The computer the session lives on ('' for this one). */
  source?: string;
  /** The computer it goes to. */
  target?: string;
}

/**
 * A refused move call in words for a person: the routes' own codes worded
 * here, another computer's refusals as its scope worded them, the rest as
 * every page words failures (`describeError`).
 */
export function moveErrorText(e: unknown, names: MoveNames = {}): string {
  if (!(e instanceof ApiError)) return describeError(e).text;
  const source = names.source || here();
  const target = names.target || tr('that computer');
  switch (e.code) {
    case 'machines_unknown':
      // The computer it goes to is not paired with the one it lives on.
      return names.source
        ? tr("{target} isn't paired with {source}.", { target, source: names.source })
        : tr("{target} isn't paired with this {computer}.", { target, computer: computerNoun() });
    case 'grant_missing':
      if (e.detail('grant') === 'move') {
        return tr("{name} doesn't let this {computer} move its sessions.", {
          name: source,
          computer: computerNoun(),
        });
      }
      break;
    case 'move_blocked':
      return tr("It can't move yet: fix the checks first.");
    case 'move_underway':
      return tr('This session is moving already.');
    case 'move_not_found':
      return tr('That move is not kept there any more.');
    case 'move_finished':
      return tr('This move has finished already.');
    case 'session_lives_elsewhere':
      return tr('This session lives on {name} now: open it there, or bring it back first.', {
        name: e.detail('name') || target,
      });
    case 'session_moving':
      return tr('This session is moving to {name} right now; it goes on there in a moment.', {
        name: e.detail('name') || target,
      });
    case 'home_online':
      return names.source
        ? tr('{name} is online: bring it back here instead.', { name: names.source })
        : moveText(e.message);
    case 'not_elsewhere':
      return tr('It lives on {here} already.', { here: here() });
    case 'nothing_here':
      return tr("This {computer} has nothing of this conversation: it can't go on here.", {
        computer: computerNoun(),
      });
    case 'data_job_running':
      return tr('A backup, restore or transfer is running there; try again when it has finished.');
    case 'machines_off':
      // 409: Machines is off here; 403: on the computer the session lives on
      // (worded by its scope already) or the one it goes to.
      if (e.status === 409) {
        return tr('Machines is off on this {computer}: turn it on on the Machines page first.', {
          computer: computerNoun(),
        });
      }
      if (e instanceof MachineApiError) return e.message;
      return moveText(e.message) || describeError(e).text;
    case 'move_too_late':
    case 'move_not_undoable':
    case 'move_went_on':
    case 'move_not_retryable':
    case 'move_not_cancellable':
    case 'move_code_stopped':
    case 'move_code_changed':
    case 'move_ignored_stopped':
    case 'move_ignored_changed':
    case 'session_live':
    case 'session_busy':
    case 'invalid_move':
    case 'session_not_found':
    case 'invalid_session_id':
      return moveText(e.message) || describeError(e).text;
    default:
      break;
  }
  if (e instanceof MachineApiError) return e.message;
  const worded = moveText(e.message);
  return worded !== e.message.trim() ? worded : describeError(e).text;
}

/** A blocker's or a warning's sentence, and its fix. */
export function checkText(c: MoveCheck): { message: string; fix: string } {
  return { message: moveText(c.message), fix: moveText(c.fix) };
}

// ─────────────────────────────────────────────────────────────── Names

/**
 * The computers of a move as this computer says them. A record names them
 * as the computer that keeps it does (a move read through another
 * computer's scope calls this one by that computer's name for it): the
 * screens say *this Mac* for this computer and their own names for the
 * others (`MovesService.words`).
 */
export interface MoveWords {
  /** A computer of the move, in words. */
  name(m: MoveMachine): string;
  /** It is this computer. */
  isHere(m: MoveMachine): boolean;
  /** The session is in a turn now: the move waits for it to end. */
  turn?: boolean;
}

/** The record's own names, nothing said to be here (lists outside the app,
 * tests). */
export const RECORD_WORDS: MoveWords = { name: (m) => m.name, isHere: () => false };

// ─────────────────────────────────────────────────────────────── States

/** Why a move stopped, worded for whoever looks at it: a reason that came
 * from the computer it went to names that computer. */
export function failureText(m: MoveRecord, w: MoveWords = RECORD_WORDS): string {
  const e = m.error;
  if (!e) return '';
  const there = m.role === 'out' ? w.name(m.to) : '';
  switch (e.code) {
    case 'cancelled':
      return tr('It was cancelled.');
    case 'interrupted':
      return tr('Mother Claude stopped during the move.');
    case 'move_code_stopped': {
      const stop = /^The code can't land here: (.+)$/.exec(e.message.trim());
      const reason = moveText(stop ? stop[1] : e.message);
      return there
        ? tr("The code can't land on {name}: {reason}", { name: there, reason })
        : tr("The code can't land here: {reason}", { reason });
    }
    case 'move_ignored_stopped': {
      const stop = /^The files git ignores can't land here: (.+)$/.exec(e.message.trim());
      const reason = stop ? stop[1] : e.message;
      return there
        ? tr("The files git ignores can't land on {name}: {reason}", { name: there, reason })
        : tr("The files git ignores can't land here: {reason}", { reason });
    }
    case 'session_live_there':
      return there
        ? tr('{name} runs this session right now.', { name: there })
        : moveText(e.message);
    case 'data_job_running':
      return there
        ? tr(
            'A backup, restore or transfer is running on {name}; try again when it has finished.',
            {
              name: there,
            },
          )
        : moveText(e.message);
    default:
      return moveText(e.message);
  }
}

/** The name of a move, for lists and titles. */
export function moveTitle(m: MoveRecord): string {
  return m.title?.trim() || tr('A session');
}

/** A move's state in one or two words (its badge). */
export function moveStateLabel(m: MoveRecord): string {
  switch (m.state) {
    case 'resumed':
      return tr('Done');
    case 'failed':
      return tr('Failed');
    case 'cancelled':
      return tr('Cancelled');
    case 'undone':
      return tr('Undone');
    case 'landed':
    case 'committed':
      return m.resumeError ? tr("Didn't start there") : tr('Starting there');
    default:
      return tr('Moving');
  }
}

/** The badge's tone. */
export function moveTone(m: MoveRecord): 'running' | 'done' | 'failed' | 'quiet' {
  if (m.state === 'resumed') return 'done';
  if (m.state === 'failed' || canRetry(m)) return 'failed';
  if (moveFinal(m)) return 'quiet';
  return 'running';
}

/** "this Mac → DESKTOP-PC", "DESKTOP-PC → this Mac (a copy)". */
export function moveRoute(m: MoveRecord, w: MoveWords = RECORD_WORDS): string {
  const from = capitalize(w.name(m.from));
  const to = w.name(m.to);
  return m.asCopy ? tr('{from} → {to} (a copy)', { from, to }) : tr('{from} → {to}', { from, to });
}

/** How it ended (or where it is), in one sentence. */
export function moveSummary(m: MoveRecord, w: MoveWords = RECORD_WORDS): string {
  const title = moveTitle(m);
  const from = w.name(m.from);
  const to = w.name(m.to);
  switch (m.state) {
    case 'resumed':
      if (w.isHere(m.to) || (m.role === 'in' && !w.isHere(m.from))) {
        return m.asCopy
          ? tr('A copy of {title} came from {name}.', { title, name: from })
          : tr('{title} came from {name} and runs here now.', { title, name: from });
      }
      return m.asCopy
        ? tr('A copy of {title} runs on {name}.', { title, name: to })
        : tr('{title} runs on {name} now.', { title, name: to });
    case 'failed':
      return tr("Couldn't move {title}: {reason} It stays on {name}, as it was.", {
        title,
        reason: failureText(m, w),
        name: from,
      });
    case 'cancelled':
      return tr('The move was cancelled: {title} stays on {name}.', { title, name: from });
    case 'undone':
      return tr('Undone: {title} lives on {name} again.', { title, name: from });
    case 'landed':
    case 'committed':
      return m.resumeError
        ? tr("{title} is on {name} but didn't start there: {reason}", {
            title,
            name: to,
            reason: moveText(m.resumeError),
          })
        : tr('Starting {title} on {name}…', { title, name: to });
    default:
      return activeText(m, w);
  }
}

/** What a move under way is doing now. */
export function activeText(m: MoveRecord, w: MoveWords = RECORD_WORDS): string {
  const name = w.name(m.to);
  switch (m.state) {
    case 'preflight':
      return tr('Checking with {name}…', { name });
    case 'freezing':
      if (m.asCopy) return tr('Getting a copy ready for {name}…', { name });
      if (m.when === 'now') return tr('Interrupting the turn, then moving to {name}…', { name });
      return w.turn
        ? tr('Moving to {name} once this turn ends.', { name })
        : tr('Getting it ready to move to {name}…', { name });
    case 'packaging':
      return tr('Packing it for {name}…', { name });
    case 'sending':
      return m.bytesTotal > 0
        ? tr('Sending to {name}: {sent} of {total}', {
            name,
            sent: formatBytes(m.bytesSent),
            total: formatBytes(m.bytesTotal),
          })
        : tr('Sending to {name}…', { name });
    case 'landing':
      return tr('Placing it on {name}…', { name });
    default:
      return moveSummary(m, w);
  }
}

// ─────────────────────────────────────────────────────────────── Progress

/** One line of the progress list (UX §7.2). */
export interface ProgressLine {
  key: string;
  label: string;
  state: 'done' | 'active' | 'todo' | 'failed';
  /** When it began (ms). */
  at: number | null;
  /** The sending's share sent (0–1), while it sends. */
  bar?: number;
  /** "about 4 s", while it sends. */
  eta?: string;
}

const ORDER: readonly MoveState[] = [
  'preflight',
  'freezing',
  'packaging',
  'sending',
  'landing',
  'landed',
];

function rank(state: MoveState): number {
  if (state === 'committed') return 5;
  if (state === 'resumed') return 6;
  const i = ORDER.indexOf(state);
  return i < 0 ? 0 : i;
}

/** The second step's words: the session leaves the computer it ran on. */
function freezeLabel(m: MoveRecord, done: boolean, w: MoveWords): string {
  const here = w.isHere(m.from);
  const name = w.name(m.from);
  if (done) {
    if (m.asCopy) {
      return here ? tr('It goes on running here') : tr('It goes on running on {name}', { name });
    }
    if (m.wasRunning) return here ? tr('Stopped here') : tr('Stopped on {name}', { name });
    return here ? tr('Nothing to stop here') : tr('Nothing to stop on {name}', { name });
  }
  if (m.asCopy) return tr('Getting a copy ready');
  if (m.when === 'now') {
    return here
      ? tr('Interrupting the turn, then stopping it here')
      : tr('Interrupting the turn, then stopping it on {name}', { name });
  }
  if (w.turn) {
    return here
      ? tr('Stopping it here once this turn ends')
      : tr('Stopping it on {name} once this turn ends', { name });
  }
  return here ? tr('Stopping it here') : tr('Stopping it on {name}', { name });
}

/** The last step's words: it starts where it went. */
function startLabel(m: MoveRecord, state: ProgressLine['state'], w: MoveWords): string {
  const here = w.isHere(m.to);
  if (state === 'done') {
    if (m.asCopy) return here ? tr('A copy started here') : tr('A copy started there');
    return here ? tr('Started here') : tr('Started there');
  }
  if (state === 'failed') return here ? tr("Didn't start here") : tr("Didn't start there");
  return here ? tr('Starting it here') : tr('Starting it there');
}

/** The steps of a move, each done, under way, still to come, or where it
 * stopped. */
export function progressLines(
  m: MoveRecord,
  now = Date.now(),
  w: MoveWords = RECORD_WORDS,
): ProgressLine[] {
  const to = w.name(m.to);
  const sizes = packageSizes(m);
  const stoppedAt =
    m.state === 'failed' || m.state === 'cancelled' ? rank(m.error?.step ?? 'preflight') : -1;
  const at = m.state === 'undone' ? 6 : stoppedAt >= 0 ? stoppedAt : rank(m.state);
  const status = (i: number): ProgressLine['state'] => {
    if (stoppedAt >= 0) return i < stoppedAt ? 'done' : i === stoppedAt ? 'failed' : 'todo';
    if (m.state === 'resumed' || m.state === 'undone') return 'done';
    if (i < at) return 'done';
    if (i === at) return canRetry(m) && i === 5 ? 'failed' : 'active';
    return 'todo';
  };
  const left = secondsLeft(m, now);
  return [
    {
      key: 'preflight',
      label: status(0) === 'active' ? tr('Checking') : tr('Checked'),
      state: status(0),
      at: reachedAt(m, 'preflight') ?? m.createdAt,
    },
    {
      key: 'freezing',
      label: freezeLabel(m, status(1) === 'done', w),
      state: status(1),
      at: reachedAt(m, 'freezing'),
    },
    {
      key: 'packaging',
      label:
        status(2) === 'done'
          ? sizes.code > 0
            ? tr('Packed — conversation {conversation}, code {code}', {
                conversation: formatBytes(sizes.conversation),
                code: formatBytes(sizes.code),
              })
            : tr('Packed — conversation {conversation}', {
                conversation: formatBytes(sizes.conversation),
              })
          : tr('Packing it'),
      state: status(2),
      at: reachedAt(m, 'packaging'),
    },
    {
      key: 'sending',
      label:
        status(3) === 'done'
          ? tr('Sent {total}', { total: formatBytes(m.bytesTotal) })
          : m.bytesTotal > 0 && status(3) !== 'todo'
            ? tr('Sending {sent} of {total}', {
                sent: formatBytes(m.bytesSent),
                total: formatBytes(m.bytesTotal),
              })
            : tr('Sending'),
      state: status(3),
      at: reachedAt(m, 'sending'),
      ...(status(3) === 'active' && m.bytesTotal > 0
        ? {
            bar: Math.min(1, m.bytesSent / m.bytesTotal),
            ...(left ? { eta: tr('about {n} s', { n: left }) } : {}),
          }
        : {}),
    },
    {
      key: 'landing',
      label:
        status(4) === 'done'
          ? tr('Placed on {name}', { name: to })
          : tr('Placing it on {name}', { name: to }),
      state: status(4),
      at: reachedAt(m, 'landing'),
    },
    {
      key: 'start',
      label: startLabel(m, status(5), w),
      state: status(5),
      at: reachedAt(m, 'resumed') ?? reachedAt(m, 'landed'),
    },
  ];
}

/** A moment of a move, to the second: "14:05:12". */
export function stepClock(ms: number | null | undefined): string {
  if (!ms) return '';
  return new Date(ms).toLocaleTimeString(locale(), {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

/** What a sheet offers for a check's `offer`. */
export function offerLabel(offer: string | null | undefined, name: string): string {
  switch (offer) {
    case 'worktree':
    case 'worktree-or-reset':
      return tr('Use a separate folder (worktree)');
    case 'as-copy':
      return tr('Continue as a copy');
    case 'choose-folder':
      return tr('Choose its folder on {name}', { name });
    default:
      return '';
  }
}

/** The checks a preflight passed, said as such (UX §7.1's ✓ rows); `here`
 * when it comes to this computer. */
export function passedChecks(pre: MovePreflight, name: string, here = false): string[] {
  const out: string[] = [];
  const t = pre.target;
  const blocked = new Set([...pre.blockers, ...pre.warnings].map((c) => c.code));
  if (t?.account?.signedIn === true) {
    out.push(tr('{account} is signed in on {name}', { account: t.account.name, name }));
  }
  if (
    pre.code.repo &&
    pre.code.mode !== 'none' &&
    t?.code?.repo &&
    pre.code.origin &&
    t.code.origin &&
    !blocked.has('origin_differs')
  ) {
    out.push(
      here
        ? tr('The folder here is the same repository (same remote)')
        : tr('The folder there is the same repository (same remote)'),
    );
  }
  if (t?.folder?.trusted) {
    out.push(
      here
        ? tr('This {computer} trusts that folder', { computer: computerNoun() })
        : tr('{name} trusts that folder', { name }),
    );
  }
  return out;
}
