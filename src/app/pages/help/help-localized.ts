import { computed, signal } from '@angular/core';

import { language } from '../../core/i18n/i18n';
import { HelpGroup, HelpGroupId, HelpTopic, HelpTopicSource } from './help-model';
import { HELP_GROUPS, HELP_TOPICS } from './help-topics';

/**
 * A language's Help: its group titles and its topics by id, each with the
 * same sections in the same order as the English topic (docs/I18N.md).
 */
export interface HelpTranslation {
  groups: Partial<Record<HelpGroupId, string>>;
  topics: Partial<Record<string, HelpTopicSource>>;
}

const vi = signal<HelpTranslation | null>(null);
let viLoading = false;

/** Load the Vietnamese Help the first time it is shown (its own chunk). */
export function ensureHelpLanguage(): void {
  if (language() !== 'vi' || vi() || viLoading) return;
  viLoading = true;
  import('./vi')
    .then((m) => vi.set(m.VI_HELP))
    .catch((err: unknown) => {
      viLoading = false;
      console.error(err);
    });
}

/** The topics in the language shown; one not translated yet stays English. */
export const helpTopics = computed<readonly HelpTopic[]>(() => {
  const t = language() === 'vi' ? vi() : null;
  return t ? HELP_TOPICS.map((en) => localize(en, t.topics[en.id])) : HELP_TOPICS;
});

export const helpGroups = computed<readonly HelpGroup[]>(() => {
  const t = language() === 'vi' ? vi() : null;
  return t ? HELP_GROUPS.map((g) => ({ ...g, title: t.groups[g.id] ?? g.title })) : HELP_GROUPS;
});

/** The topic of `/help/:topic` in the language shown. */
export function localizedTopic(id: string | null | undefined): HelpTopic | undefined {
  return id ? helpTopics().find((t) => t.id === id) : undefined;
}

/**
 * `en` in a translation: its sections keep the English ids (links and
 * `#fragments` stay valid in every language), its buttons and embeds. One
 * whose sections do not line up with the English stays English.
 */
export function localize(en: HelpTopic, src: HelpTopicSource | undefined): HelpTopic {
  if (!src || src.sections.length !== en.sections.length) return en;
  return {
    ...en,
    title: src.title,
    summary: src.summary,
    intro: src.intro,
    keywords: [...(en.keywords ?? []), ...(src.keywords ?? [])],
    sections: src.sections.map((s, i) => {
      const e = en.sections[i];
      return {
        ...s,
        id: e.id,
        where: e.where,
        embed: e.embed,
        links: e.links?.map((l, j) => ({ ...l, label: s.links?.[j]?.label ?? l.label })),
      };
    }),
  };
}
