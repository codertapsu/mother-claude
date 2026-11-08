import { useLanguage } from '../../../core/i18n/i18n';
import { ensureHelpLanguage, localize, localizedTopic } from '../help-localized';
import { HelpSectionSource, HelpTopicSource } from '../help-model';
import { inlineRuns } from '../help-text';
import { helpTopic } from '../help-topics';
import { VI_TOPICS_A } from './topics-a';
import { VI_TOPICS_B } from './topics-b';

/** The topics `topics-b.ts` translates; `topics-a.ts` has the others. */
const IDS = [
  'accounts',
  'move-data',
  'backup',
  'browser',
  'teams',
  'capacity',
  'claude-code-tools',
  'api',
  'troubleshooting',
];

const LISTS = ['points', 'steps', 'tips', 'notes'] as const;

/** The `code` runs of a string, sorted: commands and paths stay as they are. */
const codes = (text: string | undefined) =>
  inlineRuns(text ?? '')
    .filter((r) => r.kind === 'code')
    .map((r) => r.text)
    .sort();

/** Every string of a section, in order, the English and the Vietnamese side by side. */
function pairs(en: HelpSectionSource, vi: HelpSectionSource): [string, string | undefined][] {
  const out: [string, string | undefined][] = [[en.heading, vi.heading]];
  if (en.intro) out.push([en.intro, vi.intro]);
  for (const key of LISTS) (en[key] ?? []).forEach((x, j) => out.push([x, vi[key]?.[j]]));
  (en.terms ?? []).forEach((t, j) => {
    out.push([t.term, vi.terms?.[j]?.term]);
    out.push([t.text, vi.terms?.[j]?.text]);
  });
  return out;
}

/** Every string a topic shows. */
function strings(t: HelpTopicSource): string[] {
  return [
    t.title,
    t.summary,
    t.intro,
    ...(t.keywords ?? []),
    ...t.sections.flatMap((s) => [
      s.heading,
      s.intro ?? '',
      ...LISTS.flatMap((key) => s[key] ?? []),
      ...(s.terms ?? []).flatMap((x) => [x.term, x.text]),
      ...(s.links ?? []).map((l) => l.label),
    ]),
  ];
}

describe('Vietnamese Help, topics B', () => {
  afterEach(async () => {
    await useLanguage('en');
  });

  it('translates its topics, and only those', () => {
    expect(Object.keys(VI_TOPICS_B).sort()).toEqual([...IDS].sort());
    for (const id of Object.keys(VI_TOPICS_A)) {
      expect(IDS).withContext(`${id} is in topics-a`).not.toContain(id);
    }
  });

  for (const id of IDS) {
    describe(id, () => {
      const en = helpTopic(id)!;
      const vi = VI_TOPICS_B[id]!;

      it('exists, with the English id, icon and group', () => {
        expect(en).withContext('English topic').toBeDefined();
        expect(vi).withContext('Vietnamese topic').toBeDefined();
        expect(vi.id).toBe(id);
        expect(vi.icon).toBe(en.icon);
        expect(vi.group).toBe(en.group);
        expect(vi.title).not.toBe(en.title);
      });

      it('has the same sections as the English, with the same lists', () => {
        expect(vi.sections.length).toBe(en.sections.length);
        vi.sections.forEach((s, i) => {
          const e = en.sections[i];
          const where = `${id} §${i + 1} “${e.heading}”`;
          expect(s.id).withContext(`${where}: an own id`).toBeUndefined();
          expect(s.where).withContext(`${where}: where`).toBe(e.where);
          expect(s.embed).withContext(`${where}: embed`).toBe(e.embed);
          expect(!!s.intro).withContext(`${where}: intro`).toBe(!!e.intro);
          for (const key of LISTS) {
            expect(s[key]?.length ?? 0)
              .withContext(`${where}: ${key}`)
              .toBe(e[key]?.length ?? 0);
          }
          expect(s.terms?.length ?? 0)
            .withContext(`${where}: terms`)
            .toBe(e.terms?.length ?? 0);
          expect(s.links?.length ?? 0)
            .withContext(`${where}: links`)
            .toBe(e.links?.length ?? 0);
        });
      });

      it('keeps the code of every line, and closes every bold', () => {
        expect(codes(vi.intro)).withContext(`${id}: intro`).toEqual(codes(en.intro));
        vi.sections.forEach((s, i) => {
          for (const [e, v] of pairs(en.sections[i], s)) {
            expect(v)
              .withContext(`${id} §${i + 1}: “${e}”`)
              .toBeTruthy();
            expect(codes(v))
              .withContext(`${id} §${i + 1}: “${e}”`)
              .toEqual(codes(e));
          }
        });
        for (const text of strings(vi)) {
          expect(text.split('**').length % 2)
            .withContext(`${id}: “${text}”`)
            .toBe(1);
        }
      });

      it('lines up: localize() takes its words and the English ids and links', () => {
        const t = localize(en, vi);
        expect(t.title).toBe(vi.title);
        expect(t.summary).toBe(vi.summary);
        expect(t.sections.map((s) => s.heading)).toEqual(vi.sections.map((s) => s.heading));
        expect(t.sections.map((s) => s.id)).toEqual(en.sections.map((s) => s.id));
        const links = t.sections.flatMap((s) => s.links ?? []);
        const enLinks = en.sections.flatMap((s) => s.links ?? []);
        expect(links.map((l) => [l.path, l.fragment])).toEqual(
          enLinks.map((l) => [l.path, l.fragment]),
        );
        expect(links.map((l) => l.label)).toEqual(
          vi.sections.flatMap((s) => s.links ?? []).map((l) => l.label),
        );
        expect(t.keywords).toEqual([...(en.keywords ?? []), ...(vi.keywords ?? [])]);
      });
    });
  }

  it('shows them once Vietnamese is chosen, and English again after', async () => {
    await useLanguage('vi');
    ensureHelpLanguage();
    const last = IDS[IDS.length - 1];
    for (let i = 0; i < 200 && localizedTopic(last)?.title !== VI_TOPICS_B[last]?.title; i++) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    for (const id of IDS) {
      expect(localizedTopic(id)?.title).withContext(id).toBe(VI_TOPICS_B[id]?.title);
    }
    await useLanguage('en');
    for (const id of IDS) {
      expect(localizedTopic(id)?.title).withContext(id).toBe(helpTopic(id)?.title);
    }
  });
});
