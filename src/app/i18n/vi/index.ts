import type { Dict } from '../../core/i18n/i18n';
import common from './common.json';
import control from './control.json';
import copies from './copies.json';
import core from './core.json';
import data from './data.json';
import machines from './machines.json';
import moves from './moves.json';
import platform from './platform.json';
import runs from './runs.json';
import shared from './shared.json';
import shell from './shell.json';

/**
 * Vietnamese: English text → Tiếng Việt, one file per area of the app
 * (docs/I18N.md), merged. `common.json` holds the words every area uses
 * (Cancel, Close, the page names, time, nouns for plurals); an area's file
 * holds its own sentences. A key in two files must have one translation:
 * `npm run i18n:check` says when they differ.
 */
export const VI: Dict = {
  ...common,
  ...shell,
  ...shared,
  ...core,
  ...data,
  ...control,
  ...platform,
  ...machines,
  ...moves,
  ...copies,
  ...runs,
};
