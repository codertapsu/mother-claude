import { Pipe, PipeTransform } from '@angular/core';

import { Params, tr } from './i18n';

/**
 * `{{ 'Back up now' | t }}`, `{{ 'Backed up {time}' | t: { time: ago } }}`,
 * `[attr.aria-label]="'Close' | t"`: the English text in the language shown
 * (docs/I18N.md). Impure, so that a language switch shows at once: a pure
 * pipe would keep its first answer for the same text.
 */
@Pipe({ name: 't', pure: false })
export class TPipe implements PipeTransform {
  transform(text: string, params?: Params | null): string {
    return tr(text, params);
  }
}
