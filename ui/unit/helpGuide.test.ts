import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  chapterAt,
  extractShortcuts,
  isKeyCombo,
  looksLikeKeys,
  normalizeKeys,
  orderSections,
  parseFilmManifest,
  parseFrontmatter,
  parseSection,
  searchSections,
  splitChord,
  timeLabel,
  type GuideSection,
} from '../src/modules/help/guide.ts';

const AGENTS = `---
id: agents
title: Agents
group: Work            # Basics | Work | Automate | Build | Infrastructure | Insight | Plugins
route: agents          # router module to open, omit for basics
summary: "Run coding agents as live sessions."
---
## What it's for

Run claude, codex and shells side by side.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| \`⌘T\` | New session |
| \`⌘]\` / \`⌘[\` | Next / previous session |
| \`⌘⇧D\` | Split horizontally |

## Related
- [Git](#/walkthroughs/git)
`;

test('frontmatter: flat keys, trailing comments and quotes stripped, body split off', () => {
  const { meta, body } = parseFrontmatter(AGENTS);
  assert.equal(meta.id, 'agents');
  assert.equal(meta.group, 'Work');
  assert.equal(meta.route, 'agents');
  assert.equal(meta.summary, 'Run coding agents as live sessions.');
  assert.ok(body.startsWith("## What it's for"));
  assert.ok(!body.includes('---\nid:'));
});

test('frontmatter: CRLF + BOM tolerated; no block → whole text is the body', () => {
  const crlf = '﻿---\r\nid: x\r\ntitle: X: the sequel\r\n---\r\nHello\r\n';
  const { meta, body } = parseFrontmatter(crlf);
  assert.equal(meta.id, 'x');
  assert.equal(meta.title, 'X: the sequel', 'only the first colon splits');
  assert.equal(body.trim(), 'Hello');
  assert.deepEqual(parseFrontmatter('# Just markdown').meta, {});
  assert.equal(parseFrontmatter('---\nid: open\nno close').body, '---\nid: open\nno close');
});

test('parseSection: shortcuts from the Keys column, group/route/id fallbacks', () => {
  const s = parseSection('./sections/agents.md', AGENTS)!;
  assert.equal(s.id, 'agents');
  assert.equal(s.group, 'Work');
  assert.deepEqual(s.shortcuts, ['⌘T', '⌘]', '⌘[', '⌘⇧D']);

  const noGroup = parseSection('./sections/git.md', '---\ntitle: Git\ngroup: nonsense\n---\nx', (id) =>
    id === 'git' ? 'Build' : null,
  )!;
  assert.equal(noGroup.id, 'git', 'id falls back to the file name');
  assert.equal(noGroup.group, 'Build', 'unknown group → the sidebar section');
  assert.equal(noGroup.route, undefined);
  assert.equal(parseSection('./sections/x.md', '---\ntitle: X\n---\n')!.group, 'Basics');
  assert.equal(parseSection('./sections/x.md', 'no frontmatter at all'), null, 'no title → skipped');
  assert.equal(parseSection('./sections/i.md', '---\ntitle: I\ngroup: infra\n---\n')!.group, 'Infrastructure');
});

test('extractShortcuts reads every table under the heading, ignores other tables', () => {
  const body = `## Everything it can do
| Keys | Action |
|---|---|
| \`⌘Z\` | not a shortcut section |
## Keyboard shortcuts
**App**
| Keys | Action |
| --- | --- |
| \`⌃1\` – \`⌃4\` | Switch space |
**Terminal**
| Keys | Action |
|:--|:--|
| \`⌘F\` | Find |
## Tips and limits
`;
  assert.deepEqual(extractShortcuts(body), ['⌃1', '⌃4', '⌘F']);
  assert.deepEqual(extractShortcuts('## Keyboard shortcuts\nNone specific to this page.\n'), []);
});

function guide(id: string, title: string, extra: Partial<GuideSection> = {}): GuideSection {
  return { id, title, group: 'Work', summary: '', body: '', shortcuts: [], ...extra };
}

test('search ranks title over shortcut over summary over body', () => {
  const list = [
    guide('keyboard-shortcuts', 'Keyboard shortcuts', { body: 'Press ⌘K for the command bar', shortcuts: ['⌘K', '⌘T'] }),
    guide('command-bar', 'Command bar', { summary: 'The floating bar you open with ⌘K.', shortcuts: ['⌘K'] }),
    guide('git', 'Git', { body: 'Commit, push and open pull requests. commit commit' }),
    guide('agents', 'Agents', { summary: 'Sessions for every commit you make.' }),
  ];
  const byTitle = searchSections(list, 'git');
  assert.equal(byTitle[0].section.id, 'git');

  // A key combo query hits the Keys column (exact beats "contains").
  const keys = searchSections(list, '⌘K').map((h) => h.section.id);
  assert.deepEqual(keys.slice(0, 2), ['keyboard-shortcuts', 'command-bar'], 'ties keep display order');
  assert.equal(searchSections(list, '⌘K')[0].match?.kind, 'shortcut');

  // Words work as well as symbols.
  assert.equal(searchSections(list, 'cmd t')[0].section.id, 'keyboard-shortcuts');

  // Summary beats body.
  const commit = searchSections(list, 'commit').map((h) => h.section.id);
  assert.deepEqual(commit, ['agents', 'git']);
  assert.equal(searchSections(list, 'commit')[1].match?.kind, 'body');

  // A plain letter never matches through shortcut columns.
  assert.ok(!searchSections(list, 't').some((h) => h.match?.kind === 'shortcut'));

  // Fuzzy title, multi-word, empty and no-match.
  assert.equal(searchSections(list, 'kybrd')[0].section.id, 'keyboard-shortcuts');
  assert.equal(searchSections(list, 'open pull')[0].section.id, 'git');
  assert.equal(searchSections(list, '').length, list.length);
  assert.deepEqual(searchSections(list, 'zzzz-nothing'), []);
});

test('key helpers normalise, detect and split chords', () => {
  assert.equal(normalizeKeys('Cmd + Shift + D'), '⌘⇧d');
  assert.equal(normalizeKeys('⌘⇧D'), '⌘⇧d');
  assert.equal(normalizeKeys('ctrl+1'), '⌃1');
  assert.equal(normalizeKeys('option space'), '⌥space');
  assert.ok(looksLikeKeys('cmd k'));
  assert.ok(!looksLikeKeys('command bar tips'.replace('command', 'cmmand')));
  assert.deepEqual(splitChord('⌘⇧D'), ['⌘', '⇧', 'D']);
  assert.deepEqual(splitChord('⌘Enter'), ['⌘', 'Enter']);
  assert.deepEqual(splitChord('Esc'), ['Esc']);
  assert.ok(isKeyCombo('⌥Space'));
  assert.ok(!isKeyCombo('⌘'));
  assert.ok(!isKeyCombo('npm run check'));
});

test('orderSections: group order, Basics reading order, then sidebar order', () => {
  const s = (id: string, group: GuideSection['group']) => guide(id, id, { group });
  const ordered = orderSections(
    [s('git', 'Build'), s('keyboard-shortcuts', 'Basics'), s('agents', 'Work'), s('getting-started', 'Basics'), s('home', 'Work'), s('zzz', 'Basics')],
    ['home', 'agents', 'git'],
  ).map((x) => x.id);
  assert.deepEqual(ordered, ['getting-started', 'keyboard-shortcuts', 'zzz', 'home', 'agents', 'git']);
});

test('film manifest: missing/empty is handled, chapters validated and sorted', () => {
  assert.equal(parseFilmManifest(undefined), null);
  assert.equal(parseFilmManifest({ chapters: [] }), null, 'no file → no film');
  const placeholder = parseFilmManifest({ file: 'otto-tour.mp4', poster: 'p.jpg', captions: 'c.vtt', duration: 0, chapters: [] })!;
  assert.equal(placeholder.file, 'otto-tour.mp4');
  assert.deepEqual(placeholder.chapters, []);
  const f = parseFilmManifest({
    file: 'a.mp4',
    duration: 150,
    chapters: [
      { id: 'b', section: 'git', title: 'Git', start: 60, duration: 30 },
      { id: 'a', section: 'agents', title: 'Agents', start: 0, duration: 60 },
      { id: '', title: 'broken' },
      'junk',
    ],
  })!;
  assert.deepEqual(f.chapters.map((c) => c.id), ['a', 'b']);
  assert.equal(chapterAt(f.chapters, 10)?.id, 'a');
  assert.equal(chapterAt(f.chapters, 60)?.id, 'b');
  assert.equal(chapterAt(f.chapters, 95), null);
  assert.equal(timeLabel(125), '2:05');
});
