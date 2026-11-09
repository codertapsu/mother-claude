import { localize } from '../help-localized';
import { HelpSectionSource } from '../help-model';
import { inlineRuns } from '../help-text';
import { helpTopic } from '../help-topics';
import { VI_HELP } from './index';
import { VI_TOPICS_A } from './topics-a';

/** The topics `topics-a.ts` translates. */
const IDS = [
  'getting-started',
  'good-to-know',
  'settings',
  'sessions',
  'session-page',
  'messaging',
  'approvals',
  'new-session',
  'phone',
  'internet',
  'foreign-sessions',
];

const LISTS = ['points', 'terms', 'steps', 'tips', 'notes', 'links'] as const;

/** Every string a reader sees in a section. */
function strings(s: HelpSectionSource): string[] {
  return [
    s.heading,
    s.intro ?? '',
    ...(s.points ?? []),
    ...(s.terms ?? []).flatMap((t) => [t.term, t.text]),
    ...(s.steps ?? []),
    ...(s.tips ?? []),
    ...(s.notes ?? []),
    ...(s.links ?? []).map((l) => l.label),
  ];
}

/** The `code` of a section, sorted: commands and paths stay as they are. */
function code(s: HelpSectionSource): string[] {
  return strings(s)
    .flatMap((text) => inlineRuns(text).filter((r) => r.kind === 'code'))
    .map((r) => r.text)
    .sort();
}

describe('Vietnamese Help: the basics, sessions and remote topics', () => {
  for (const id of IDS) {
    describe(id, () => {
      const en = helpTopic(id)!;
      const vi = VI_TOPICS_A[id]!;

      it('exists, with the English id, icon and group', () => {
        expect(en, 'the English topic').toBeDefined();
        expect(vi).toBeDefined();
        expect(vi.id).toBe(id);
        expect(vi.icon).toBe(en.icon);
        expect(vi.group).toBe(en.group);
        expect(vi.summary).toBeTruthy();
        expect(vi.intro).toBeTruthy();
      });

      it('has the sections of the English, each with as many steps, links and items', () => {
        expect(vi.sections.length).toBe(en.sections.length);
        en.sections.forEach((e, i) => {
          const s = vi.sections[i];
          const where = `${id} › ${e.heading}`;
          expect(s.heading, where).toBeTruthy();
          expect(s.id, `${where}: its id comes from the English`).toBeUndefined();
          expect(s.where, where).toBe(e.where);
          expect(s.embed, where).toBe(e.embed);
          expect(!!s.intro, `${where}: intro`).toBe(!!e.intro);
          for (const list of LISTS) {
            expect(s[list]?.length ?? 0, `${where}: ${list}`).toBe(e[list]?.length ?? 0);
          }
          expect(code(s), `${where}: code`).toEqual(code(e));
          // The template tracks terms and links by their text.
          const terms = (s.terms ?? []).map((t) => t.term);
          expect(new Set(terms).size, `${where}: terms`).toBe(terms.length);
          const labels = (s.links ?? []).map((l) => l.label);
          expect(new Set(labels).size, `${where}: links`).toBe(labels.length);
          for (const text of strings(s)) {
            expect(text.split('**').length % 2, `${where}: unclosed bold in “${text}”`).toBe(1);
          }
        });
      });

      it('lines up with the English, so localize() shows it in Vietnamese', () => {
        const t = localize(en, vi);
        expect(t.title).toBe(vi.title);
        expect(t.title).not.toBe(en.title);
        expect(t.sections.map((s) => s.id)).toEqual(en.sections.map((s) => s.id));
        expect(t.sections.map((s) => s.heading)).toEqual(vi.sections.map((s) => s.heading));
        const links = t.sections.flatMap((s) => s.links ?? []);
        const enLinks = en.sections.flatMap((s) => s.links ?? []);
        expect(links.map((l) => [l.path, l.fragment])).toEqual(
          enLinks.map((l) => [l.path, l.fragment]),
        );
        expect(links.map((l) => l.label)).toEqual(
          vi.sections.flatMap((s) => s.links ?? []).map((l) => l.label),
        );
      });

      it('adds Vietnamese keywords, not the English ones again', () => {
        const english = new Set((en.keywords ?? []).map((k) => k.toLowerCase()));
        expect(vi.keywords?.length).toBeGreaterThan(0);
        for (const k of vi.keywords ?? []) {
          expect(english.has(k.toLowerCase()), k).toBe(false);
        }
      });
    });
  }

  it('holds exactly these topics, and the Vietnamese Help uses them', () => {
    expect(Object.keys(VI_TOPICS_A).sort()).toEqual([...IDS].sort());
    for (const id of IDS) {
      expect(VI_HELP.topics[id], id).toBe(VI_TOPICS_A[id]);
    }
  });
});
