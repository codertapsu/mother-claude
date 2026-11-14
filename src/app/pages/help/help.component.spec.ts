import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { routePaths, routes } from '../../app.routes';
import { AuthService } from '../../core/auth.service';
import { PLACES, SETTINGS_PAGES } from '../../core/nav';
import { HelpComponent } from './help.component';
import { inlineRuns, plainText } from './help-text';
import { HELP_GROUPS, HELP_TOPICS, helpTopic } from './help-topics';

describe('HelpComponent', () => {
  let harness: RouterTestingHarness;

  beforeEach(async () => {
    TestBed.configureTestingModule({ providers: [provideRouter(routes)] });
    TestBed.inject(AuthService).state.set('paired');
    harness = await RouterTestingHarness.create();
  });

  const el = () => harness.routeNativeElement as HTMLElement;
  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const all = (selector: string) => [...el().querySelectorAll<HTMLElement>(selector)];
  const url = () => TestBed.inject(Router).url;
  const search = (value: string) => {
    const input = el().querySelector<HTMLInputElement>('input[type="search"]')!;
    input.value = value;
    input.dispatchEvent(new Event('input'));
    harness.detectChanges();
  };
  const cardIds = () =>
    all('.topic-card').map(
      (a) =>
        a
          .getAttribute('href')
          ?.replace(/^.*\/help\//, '')
          .split('#')[0],
    );

  it('lists every topic on the landing, under its group', async () => {
    await harness.navigateByUrl('/help', HelpComponent);
    expect(cardIds()).toEqual(HELP_TOPICS.map((t) => t.id));
    expect(all('.group h2').map(text)).toEqual(HELP_GROUPS.map((g) => g.title));
    for (const t of HELP_TOPICS) {
      expect(text(el())).toContain(t.title);
    }
    expect(text(el().querySelector('#help-count'))).toBe(`${HELP_TOPICS.length} topics`);
  });

  it('groups the topics as the map: Basics, a group per place, then Troubleshooting', async () => {
    expect(HELP_GROUPS.map((g) => g.title)).toEqual([
      'Basics',
      'Your sessions',
      'Teams',
      'Your computers',
      'Accounts and data',
      'Settings and connections',
      'Troubleshooting',
    ]);
    // Each place's own topic is in its group (the Inbox's with the
    // sessions); the connections, Settings and its own pages are Settings
    // and connections, Backups is with the accounts' data.
    const groupOf = (id: string | undefined) => helpTopic(id)?.group;
    expect(PLACES.map((p) => [p.id, groupOf(p.help)])).toEqual([
      ['sessions', 'sessions'],
      ['inbox', 'sessions'],
      ['teams', 'teams'],
      ['computers', 'machines'],
      ['devices', 'settings'],
      ['elsewhere', 'settings'],
      ['browser', 'settings'],
      ['http-api', 'settings'],
      ['accounts', 'data'],
      ['backups', 'data'],
      ['settings', 'settings'],
    ]);
    for (const p of SETTINGS_PAGES) expect(groupOf(p.help), p.id).toBe('settings');
    await harness.navigateByUrl('/help', HelpComponent);
    const idsOf = (group: string) =>
      [
        ...(el()
          .querySelector(`#help-group-${group}`)
          ?.closest('section')
          ?.querySelectorAll('.topic-card') ?? []),
      ].map(
        (a) =>
          a
            .getAttribute('href')
            ?.replace(/^.*\/help\//, '')
            .split('#')[0],
      );
    expect(idsOf('basics')).toEqual(['getting-started', 'good-to-know']);
    expect(idsOf('sessions')).toEqual([
      'sessions',
      'session-page',
      'messaging',
      'approvals',
      'permissions',
      'new-session',
    ]);
    expect(idsOf('teams')).toEqual(['teams', 'capacity']);
    expect(idsOf('data')).toEqual(['accounts', 'move-data', 'backup']);
    expect(idsOf('settings')).toEqual([
      'settings',
      'phone',
      'internet',
      'foreign-sessions',
      'browser',
      'claude-code-tools',
      'api',
    ]);
    expect(idsOf('troubleshooting')).toEqual(['troubleshooting', 'machines-troubleshooting']);
  });

  it('groups the Machines topics under Your computers, and finds a move by its words', async () => {
    await harness.navigateByUrl('/help', HelpComponent);
    const group = el().querySelector('#help-group-machines')?.closest('section');
    expect(text(group?.querySelector('h2'))).toBe('Your computers');
    const ids = [...(group?.querySelectorAll('.topic-card') ?? [])].map(
      (a) =>
        a
          .getAttribute('href')
          ?.replace(/^.*\/help\//, '')
          .split('#')[0],
    );
    expect(ids).toEqual([
      'machines-setup',
      'machines-pair',
      'machines-more',
      'machines-permissions',
      'machines-sessions',
      'machines-move',
      'machines-copies',
      'machines-sync',
      'machines-run',
      'machines-task',
    ]);

    search('take over');
    expect(cardIds()).toContain('machines-move');
    search('revoke everywhere');
    expect(cardIds()).toContain('machines-more');
    search('introduce');
    expect(cardIds()).toContain('machines-more');
    search('keep in sync');
    expect(cardIds()).toContain('machines-sync');
    search('keep a copy');
    expect(cardIds()).toContain('machines-copies');
    search('machine_run');
    expect(cardIds()).toContain('machines-run');
    search('machine_task');
    expect(cardIds()).toContain('machines-task');
    // Its troubleshooting sits with the other troubleshooting.
    search('nearby');
    expect(cardIds()).toContain('machines-troubleshooting');
    expect(helpTopic('machines-troubleshooting')?.group).toBe('troubleshooting');
  });

  it('searches titles, summaries and the steps themselves, ignoring case', async () => {
    await harness.navigateByUrl('/help', HelpComponent);
    // Only a step of the browser topic says it.
    search('DEVELOPER mode');
    expect(cardIds()).toEqual(['browser']);
    expect(text(el().querySelector('.card-hit'))).toBe('In “Install the extension”');
    expect(all('.topic-card')[0].getAttribute('href')).toContain('#install-the-extension');
    expect(text(el().querySelector('#help-count'))).toBe(`1 of ${HELP_TOPICS.length} topics match`);

    // A title match needs no hint.
    search('teams of sessions');
    expect(cardIds()).toContain('teams');
    const teams = all('.topic-card').find((a) => a.getAttribute('href')?.endsWith('/help/teams'));
    expect(teams?.querySelector('.card-hit')).toBeNull();

    search('send now');
    expect(cardIds()).toContain('messaging');

    // Every word has to match, anywhere in the topic: Backup has the first,
    // the browser topic the second, Troubleshooting both.
    search('passphrase extension');
    expect(cardIds()).toEqual(['troubleshooting']);
  });

  it('says when nothing matches, and clears the search', async () => {
    await harness.navigateByUrl('/help', HelpComponent);
    search('zebra quantum');
    expect(all('.topic-card').length).toBe(0);
    expect(text(el().querySelector('.empty-state strong'))).toBe('No topics match “zebra quantum”');

    const clear = all('.empty-state button').find((b) => text(b) === 'Clear the search')!;
    clear.click();
    harness.detectChanges();
    expect(all('.topic-card').length).toBe(HELP_TOPICS.length);
    expect(el().querySelector<HTMLInputElement>('input[type="search"]')!.value).toBe('');
  });

  it('keeps the search while a topic is open', async () => {
    await harness.navigateByUrl('/help', HelpComponent);
    search('passphrase');
    await harness.navigateByUrl('/help/backup', HelpComponent);
    await harness.navigateByUrl('/help', HelpComponent);
    expect(el().querySelector<HTMLInputElement>('input[type="search"]')!.value).toBe('passphrase');
    expect(cardIds()).toContain('backup');
  });

  it('renders a topic: its steps, notes, links and contents', async () => {
    const topic = helpTopic('backup')!;
    await harness.navigateByUrl('/help/backup', HelpComponent);

    expect(text(el().querySelector('h1'))).toBe(topic.title);
    const steps = topic.sections.flatMap((s) => s.steps ?? []);
    const items = all('ol.steps > li');
    expect(items.length).toBe(steps.length);
    // Marks become elements; nothing is bound as HTML.
    expect(text(items[0])).toBe(plainText(steps[0]));
    expect(items[0].querySelector('strong')).not.toBeNull();

    const sections = all('section.sec');
    expect(sections.length).toBe(topic.sections.length);
    expect(all('nav.toc a').map(text)).toEqual(topic.sections.map((s) => s.heading));

    const links = topic.sections.flatMap((s) => s.links ?? []);
    const buttons = all('.links a');
    expect(buttons.map(text)).toEqual(links.map((l) => l.label));
    expect(buttons[0].getAttribute('href')).toBe('/backups');

    const notes = topic.sections.flatMap((s) => s.notes ?? []);
    expect(all('.callout[data-tone="amber"]').length).toBe(notes.length);
    expect(text(el().querySelector('.badge.where'))).toBe('Desktop app');
  });

  it('opens a section from the contents', async () => {
    await harness.navigateByUrl('/help/messaging', HelpComponent);
    const target = helpTopic('messaging')!.sections.find(
      (s) => s.heading === 'Send while Claude works',
    )!;
    const link = all('nav.toc a').find((a) => text(a) === target.heading)!;
    link.click();
    await harness.fixture.whenStable();
    harness.detectChanges();
    expect(url()).toBe(`/help/messaging#${target.id}`);
    expect(document.activeElement?.id).toBe(`${target.id}-h`);
  });

  it('links the previous and next topics', async () => {
    const [first, second] = HELP_TOPICS;
    await harness.navigateByUrl(`/help/${first.id}`, HelpComponent);
    expect(el().querySelector('.pg.prev')).toBeNull();
    const next = el().querySelector<HTMLAnchorElement>('.pg.next')!;
    expect(text(next)).toContain(second.title);
    expect(next.getAttribute('href')).toBe(`/help/${second.id}`);

    const last = HELP_TOPICS[HELP_TOPICS.length - 1];
    await harness.navigateByUrl(`/help/${last.id}`, HelpComponent);
    expect(el().querySelector('.pg.next')).toBeNull();
    expect(text(el().querySelector('.pg.prev'))).toContain(
      HELP_TOPICS[HELP_TOPICS.length - 2].title,
    );
  });

  it('falls back to the landing for a topic that does not exist', async () => {
    await harness.navigateByUrl('/help/no-such-topic');
    await harness.fixture.whenStable();
    harness.detectChanges();
    expect(url()).toBe('/help');
    expect(all('.topic-card').length).toBe(HELP_TOPICS.length);
  });

  it('sends the old Good to know page to its topic', async () => {
    await harness.navigateByUrl('/good-to-know', HelpComponent);
    expect(url()).toBe('/help/good-to-know');
    expect(text(el().querySelector('h1'))).toBe('Good to know');
    expect(el().querySelector('mc-limitations')).not.toBeNull();
  });
});

describe('Help content', () => {
  // A layout's pages under it (`/inbox` is the work layout's).
  const pagePaths = new Set(routePaths().map((p) => `/${p}`));

  it('gives every topic a unique id, a summary and sections', () => {
    const ids = HELP_TOPICS.map((t) => t.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const t of HELP_TOPICS) {
      expect(t.summary, t.id).toBeTruthy();
      expect(t.sections.length, t.id).toBeGreaterThan(0);
      const sectionIds = t.sections.map((s) => s.id);
      expect(new Set(sectionIds).size, t.id).toBe(sectionIds.length);
    }
  });

  it('links only to pages and topics that exist', () => {
    for (const t of HELP_TOPICS) {
      for (const link of t.sections.flatMap((s) => s.links ?? [])) {
        const where = `${t.id}: ${link.label}`;
        const target = link.path.match(/^\/help\/([^/]+)$/);
        if (target) {
          const topic = helpTopic(target[1]);
          expect(topic, where).toBeDefined();
          if (link.fragment) {
            expect(
              topic?.sections.map((s) => s.id),
              where,
            ).toContain(link.fragment);
          }
        } else {
          expect(pagePaths.has(link.path), where).toBe(true);
        }
      }
    }
  });

  it('splits bold and code marks into runs', () => {
    expect(inlineRuns('Press **Send now** or type `/`.')).toEqual([
      { kind: 'text', text: 'Press ' },
      { kind: 'strong', text: 'Send now' },
      { kind: 'text', text: ' or type ' },
      { kind: 'code', text: '/' },
      { kind: 'text', text: '.' },
    ]);
    expect(plainText('**Remote › Paired devices**')).toBe('Remote › Paired devices');
  });
});
