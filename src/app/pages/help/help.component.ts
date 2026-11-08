import { ViewportScroller } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DOCUMENT,
  ElementRef,
  Injectable,
  afterRenderEffect,
  computed,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { map } from 'rxjs';

import { plural } from '../../core/data-jobs';
import { TPipe } from '../../core/i18n/t.pipe';
import { IconComponent } from '../../shared/icon.component';
import { LimitationsComponent } from '../../shared/limitations.component';
import { HelpGroup, HelpSection, HelpTopic } from './help-model';
import { plainText, searchTerms } from './help-text';
import { HelpTextComponent } from './help-text.component';
import { ensureHelpLanguage, helpGroups, helpTopics, localizedTopic } from './help-localized';

/** The search as typed: it is still there after reading a topic. */
@Injectable({ providedIn: 'root' })
export class HelpSearchState {
  readonly query = signal('');
}

interface IndexedSection {
  id: string;
  heading: string;
  hay: string;
}

interface IndexedTopic {
  topic: HelpTopic;
  /** Title, summary, intro and keywords. */
  head: string;
  sections: IndexedSection[];
  all: string;
}

/** A topic on the landing, and the section a search found in it. */
export interface HelpHit {
  topic: HelpTopic;
  section?: { id: string; heading: string };
}

interface HitGroup {
  group: HelpGroup;
  hits: HelpHit[];
}

function lower(parts: (string | undefined)[]): string {
  return parts
    .filter((p): p is string => !!p)
    .map((p) => plainText(p).toLowerCase())
    .join('\n');
}

function sectionText(s: HelpSection): string {
  return lower([
    s.heading,
    s.intro,
    ...(s.points ?? []),
    ...(s.terms ?? []).flatMap((t) => [t.term, t.text]),
    ...(s.steps ?? []),
    ...(s.tips ?? []),
    ...(s.notes ?? []),
    ...(s.links ?? []).map((l) => l.label),
  ]);
}

/** The search index of the topics in the language shown. */
const INDEX = computed<IndexedTopic[]>(() =>
  helpTopics().map((topic) => {
    const head = lower([topic.title, topic.summary, topic.intro, ...(topic.keywords ?? [])]);
    const sections = topic.sections.map((s) => ({
      id: s.id,
      heading: s.heading,
      hay: sectionText(s),
    }));
    return { topic, head, sections, all: [head, ...sections.map((s) => s.hay)].join('\n') };
  }),
);

/** Topics matching every word of `query` (all of them for an empty one). */
export function searchHelp(query: string): HelpHit[] {
  const terms = searchTerms(query);
  const hits: HelpHit[] = [];
  for (const it of INDEX()) {
    if (!terms.every((t) => it.all.includes(t))) continue;
    const inHead = !terms.length || terms.every((t) => it.head.includes(t));
    const sec = inHead ? undefined : it.sections.find((s) => terms.every((t) => s.hay.includes(t)));
    hits.push({ topic: it.topic, section: sec && { id: sec.id, heading: sec.heading } });
  }
  return hits;
}

/**
 * The Help center. `/help`: every topic as a card, grouped, with a search
 * over titles, summaries and the steps themselves. `/help/:topic`: one
 * topic — its sections with numbered steps, tips, notes and buttons to the
 * pages they are about, a table of contents (sticky beside the text on wide
 * screens, above it on phones) and the previous and next topics. A topic
 * that does not exist falls back to `/help`. The content is typed data
 * (`help-topics.ts`), rendered as text: nothing is bound as HTML.
 */
@Component({
  selector: 'mc-help',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, IconComponent, LimitationsComponent, HelpTextComponent, TPipe],
  templateUrl: './help.component.html',
  styleUrl: './help.component.scss',
})
export class HelpComponent {
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly scroller = inject(ViewportScroller);
  private readonly doc = inject(DOCUMENT);
  protected readonly search = inject(HelpSearchState);

  private readonly titleEl = viewChild<ElementRef<HTMLElement>>('topicTitle');

  private readonly topicId = toSignal(this.route.paramMap.pipe(map((p) => p.get('topic'))), {
    initialValue: this.route.snapshot.paramMap.get('topic'),
  });
  private readonly fragment = toSignal(this.route.fragment, {
    initialValue: this.route.snapshot.fragment,
  });

  protected readonly topic = computed(() => localizedTopic(this.topicId()));
  protected readonly groupTitle = computed(
    () => helpGroups().find((g) => g.id === this.topic()?.group)?.title ?? '',
  );
  protected readonly prev = computed(() => this.neighbour(-1));
  protected readonly next = computed(() => this.neighbour(1));

  protected readonly query = this.search.query;
  protected readonly hits = computed(() => searchHelp(this.query()));
  protected readonly groups = computed<HitGroup[]>(() => {
    const hits = this.hits();
    return helpGroups()
      .map((group) => ({
        group,
        hits: hits.filter((h) => h.topic.group === group.id),
      }))
      .filter((g) => g.hits.length);
  });
  protected readonly total = computed(() => helpTopics().length);
  /** "28 topics", in the language shown. */
  protected readonly totalLabel = computed(() => plural(this.total(), 'topic'));

  constructor() {
    // Vietnamese topics load the first time they are shown.
    effect(() => ensureHelpLanguage());
    // An unknown topic: the landing, at its own address.
    effect(() => {
      const id = this.topicId();
      if (id && !this.topic()) {
        untracked(() => void this.router.navigate(['/help'], { replaceUrl: true }));
      }
    });
    // A new topic starts at its top (or at the section its link names), with
    // its heading focused for screen readers; the landing starts at the top.
    afterRenderEffect(() => {
      const id = this.topicId();
      const fragment = this.fragment();
      untracked(() => this.arrive(id, fragment));
    });
  }

  protected setQuery(event: Event): void {
    this.query.set((event.target as HTMLInputElement).value);
  }

  protected clearQuery(input: HTMLInputElement): void {
    this.query.set('');
    input.focus();
  }

  /** A table-of-contents link: the router scrolls on the first click, but
   * a second click on the same link is the same URL, which it ignores. */
  protected jump(sectionId: string): void {
    this.scrollToSection(sectionId);
  }

  private arrive(id: string | null, fragment: string | null): void {
    if (id && fragment && this.scrollToSection(fragment)) return;
    this.scroller.scrollToPosition([0, 0]);
    if (id) this.titleEl()?.nativeElement.focus({ preventScroll: true });
  }

  private scrollToSection(sectionId: string): boolean {
    const anchor = this.doc.getElementById(sectionId);
    if (!anchor?.closest('mc-help')) return false;
    anchor.scrollIntoView({ block: 'start' });
    this.doc.getElementById(`${sectionId}-h`)?.focus({ preventScroll: true });
    return true;
  }

  private neighbour(step: -1 | 1): HelpTopic | undefined {
    const t = this.topic();
    if (!t) return undefined;
    const all = helpTopics();
    return all[all.findIndex((x) => x.id === t.id) + step];
  }
}
