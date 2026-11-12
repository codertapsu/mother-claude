import { localize } from '../help-localized';
import { HelpSectionSource } from '../help-model';
import { inlineRuns } from '../help-text';
import { HELP_TOPICS, helpTopic } from '../help-topics';
import { VI_HELP } from './index';
import { VI_TOPICS_A } from './topics-a';
import { VI_TOPICS_B } from './topics-b';
import { VI_TOPICS_C } from './topics-c';

/** The topics `topics-c.ts` translates: your computers (Machines). */
const IDS = [
  'machines-setup',
  'machines-pair',
  'machines-permissions',
  'machines-sessions',
  'machines-move',
  'machines-copies',
  'machines-sync',
  'machines-run',
  'machines-task',
  'machines-troubleshooting',
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

describe('Vietnamese Help: your computers', () => {
  it('translates its topics, and only those, and every English Machines topic', () => {
    expect(Object.keys(VI_TOPICS_C).sort()).toEqual([...IDS].sort());
    for (const id of [...Object.keys(VI_TOPICS_A), ...Object.keys(VI_TOPICS_B)]) {
      expect(IDS, `${id} is translated elsewhere`).not.toContain(id);
    }
    const english = HELP_TOPICS.filter(
      (t) => t.group === 'machines' || t.id.startsWith('machines'),
    );
    expect(english.map((t) => t.id).sort()).toEqual([...IDS].sort());
    for (const id of IDS) expect(VI_HELP.topics[id], id).toBe(VI_TOPICS_C[id]);
    expect(VI_HELP.groups.machines).toBeTruthy();
  });

  for (const id of IDS) {
    describe(id, () => {
      const en = helpTopic(id)!;
      const vi = VI_TOPICS_C[id]!;

      it('exists, with the English id, icon and group', () => {
        expect(en, 'the English topic').toBeDefined();
        expect(vi, 'the Vietnamese topic').toBeDefined();
        expect(vi.id).toBe(id);
        expect(vi.icon).toBe(en.icon);
        expect(vi.group).toBe(en.group);
        expect(vi.title).not.toBe(en.title);
        expect(vi.summary).toBeTruthy();
        expect(vi.intro).toBeTruthy();
      });

      it('has the sections of the English, each with as many items', () => {
        expect(vi.sections.length).toBe(en.sections.length);
        en.sections.forEach((e, i) => {
          const s = vi.sections[i];
          const where = `${id} › ${e.heading}`;
          expect(s.id, `${where}: its id comes from the English`).toBeUndefined();
          expect(s.where, `${where}: where`).toBe(e.where);
          expect(s.embed, `${where}: embed`).toBe(e.embed);
          expect(!!s.intro, `${where}: intro`).toBe(!!e.intro);
          for (const key of LISTS) {
            expect(s[key]?.length ?? 0, `${where}: ${key}`).toBe(e[key]?.length ?? 0);
          }
          expect(s.terms?.length ?? 0, `${where}: terms`).toBe(e.terms?.length ?? 0);
          expect(s.links?.length ?? 0, `${where}: links`).toBe(e.links?.length ?? 0);
          // The template tracks terms and links by their text.
          const terms = (s.terms ?? []).map((t) => t.term);
          expect(new Set(terms).size, `${where}: terms`).toBe(terms.length);
          const labels = (s.links ?? []).map((l) => l.label);
          expect(new Set(labels).size, `${where}: links`).toBe(labels.length);
        });
      });

      it('keeps the code of every line, and closes every bold', () => {
        expect(codes(vi.intro), `${id}: intro`).toEqual(codes(en.intro));
        vi.sections.forEach((s, i) => {
          for (const [e, v] of pairs(en.sections[i], s)) {
            expect(v, `${id} §${i + 1}: “${e}”`).toBeTruthy();
            expect(codes(v), `${id} §${i + 1}: “${e}”`).toEqual(codes(e));
            expect(v!.split('**').length % 2, `${id} §${i + 1}: “${v}”`).toBe(1);
          }
        });
      });

      it('lines up with the English, so localize() shows it in Vietnamese', () => {
        const t = localize(en, vi);
        expect(t.title).toBe(vi.title);
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
});
