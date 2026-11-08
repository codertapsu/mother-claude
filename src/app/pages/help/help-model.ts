import { IconName } from '../../shared/icon.component';
import { slugify } from './help-text';

/** The headings the Help landing groups its topics under, in order. */
export type HelpGroupId =
  'basics' | 'sessions' | 'remote' | 'data' | 'browser-teams' | 'advanced' | 'troubleshooting';

export interface HelpGroup {
  id: HelpGroupId;
  title: string;
}

/** A button to the page a section is about ("Open Backup & sync"). */
export interface HelpLink {
  label: string;
  /** A route path: `/backup`, or `/help/<topic>` for another topic. */
  path: string;
  fragment?: string;
}

/** One entry of a definition list: a label the app shows, and its meaning. */
export interface HelpTerm {
  term: string;
  text: string;
}

/**
 * One section of a topic. Every string may use `**bold**` and `` `code` ``
 * (rendered as elements, never as HTML). `steps` are numbered, one action
 * each; `points` are plain facts.
 */
export interface HelpSectionSource {
  /** The fragment id; defaults to the heading as a slug. */
  id?: string;
  heading: string;
  /** Where it applies, shown as a label beside the heading. */
  where?: 'desktop';
  intro?: string;
  points?: string[];
  terms?: HelpTerm[];
  steps?: string[];
  tips?: string[];
  notes?: string[];
  links?: HelpLink[];
  /** A shared component shown in the section. */
  embed?: 'limitations';
}

export interface HelpSection extends HelpSectionSource {
  id: string;
}

export interface HelpTopicSource {
  /** The route: `/help/<id>`. */
  id: string;
  title: string;
  icon: IconName;
  group: HelpGroupId;
  /** One line on the landing card. */
  summary: string;
  /** A short paragraph under the title. */
  intro: string;
  /** More words the search finds the topic by. */
  keywords?: string[];
  sections: HelpSectionSource[];
}

export interface HelpTopic extends Omit<HelpTopicSource, 'sections'> {
  sections: HelpSection[];
}

/** Gives every section its fragment id (unique within the topic). */
export function resolveTopic(source: HelpTopicSource): HelpTopic {
  const seen = new Set<string>();
  const sections = source.sections.map((s) => {
    const base = s.id ?? slugify(s.heading);
    let id = base;
    for (let n = 2; seen.has(id); n++) id = `${base}-${n}`;
    seen.add(id);
    return { ...s, id };
  });
  return { ...source, sections };
}
