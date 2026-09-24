import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  bodyKey,
  categoryCounts,
  filterGroups,
  groupSkills,
  metaList,
  namesNeedingBodies,
  normalizeBody,
  parseFrontmatter,
  sourceCounts,
} from '../src/modules/skills-lab/skillGroups.ts';

const BODY = '---\ndescription: Review API docs\ncategory: review\n---\n# vault-api-review\n';

const library = [
  { name: 'vault-api-review', description: 'Review API docs', body: BODY, category: 'review' },
  { name: 'insights', description: 'Usage report', body: '---\nversion: 2\n---\n# insights', category: 'insights' },
  { name: 'solo', description: 'Only here', body: '# solo', category: '' },
];
const bundled = [
  { name: 'insights', category: 'insights', version: 3, description: 'Usage report', installed_version: 2, state: 'update_available' },
  { name: 'grill', category: 'review', version: 1, description: 'Grill a diff', installed_version: null, state: 'not_installed' },
];
const provider = [
  { provider: 'claude', name: 'vault-api-review', category: 'review', description: 'Review API docs' },
  { provider: 'codex', name: 'vault-api-review', category: 'provider', description: 'Review API docs' },
  { provider: 'claude', name: 'docx', category: 'provider', description: 'Word documents' },
  { provider: 'codex', name: 'docx', category: 'provider', description: 'Word documents' },
];

test('one group per name with variants in display order (no ×3 rows)', () => {
  const g = groupSkills(library, bundled, provider);
  const names = g.map((x) => x.name);
  assert.equal(new Set(names).size, names.length);
  const var3 = g.find((x) => x.name === 'vault-api-review')!;
  assert.deepEqual(var3.variants.map((v) => v.source), ['library', 'claude', 'codex']);
  assert.equal(var3.category, 'review');
  assert.equal(var3.reference, 'library');
  // "provider" is a placeholder category, not a real one.
  assert.equal(g.find((x) => x.name === 'docx')!.category, 'uncategorized');
  assert.equal(g.find((x) => x.name === 'solo')!.category, 'uncategorized');
});

test('sync states: unknown until bodies are read, then in sync or drifted', () => {
  let g = groupSkills(library, bundled, provider);
  assert.equal(g.find((x) => x.name === 'vault-api-review')!.sync, 'unknown');
  assert.equal(g.find((x) => x.name === 'solo')!.sync, 'single');
  assert.equal(g.find((x) => x.name === 'grill')!.sync, 'single');

  const bodies = {
    [bodyKey('claude', 'vault-api-review')]: BODY.replace(/\n/g, '\r\n') + '   \n',
    [bodyKey('codex', 'vault-api-review')]: BODY + '\nExtra line\n',
  };
  g = groupSkills(library, bundled, provider, bodies);
  const v = g.find((x) => x.name === 'vault-api-review')!;
  assert.equal(v.sync, 'drifted');
  assert.deepEqual(v.driftedSources, ['codex']);
  assert.deepEqual(v.drift, ['Codex copy differs from Library']);

  const same = groupSkills(library, bundled, provider, { ...bodies, [bodyKey('codex', 'vault-api-review')]: BODY });
  assert.equal(same.find((x) => x.name === 'vault-api-review')!.sync, 'in_sync');
});

test('bundled version drift marks the library copy drifted', () => {
  const g = groupSkills(library, bundled, provider).find((x) => x.name === 'insights')!;
  assert.equal(g.sync, 'drifted');
  assert.deepEqual(g.driftedSources, ['bundled']);
  assert.match(g.drift[0], /Bundled v3 is newer than the library copy \(v2\)/);
  const upToDate = groupSkills(library, [{ ...bundled[0], state: 'up_to_date', installed_version: 3 }], []);
  assert.equal(upToDate.find((x) => x.name === 'insights')!.sync, 'in_sync');
});

test('provider-only copies compare against the first provider', () => {
  const g = groupSkills([], [], provider, { [bodyKey('claude', 'docx')]: 'a', [bodyKey('codex', 'docx')]: 'b' });
  const d = g.find((x) => x.name === 'docx')!;
  assert.equal(d.reference, 'claude');
  assert.equal(d.sync, 'drifted');
});

test('filters: search, category, source, drifted-only; counts', () => {
  const g = groupSkills(library, bundled, provider, { [bodyKey('claude', 'docx')]: 'a', [bodyKey('codex', 'docx')]: 'b' });
  assert.deepEqual(filterGroups(g, { query: 'WORD' }).map((x) => x.name), ['docx']);
  assert.deepEqual(filterGroups(g, { category: 'review' }).map((x) => x.name), ['grill', 'vault-api-review']);
  assert.deepEqual(filterGroups(g, { source: 'codex' }).map((x) => x.name).sort(), ['docx', 'vault-api-review']);
  assert.deepEqual(filterGroups(g, { driftedOnly: true }).map((x) => x.name).sort(), ['docx', 'insights']);
  assert.equal(filterGroups(g, {}).length, g.length);
  assert.deepEqual(categoryCounts(g)[0], ['review', 2]);
  assert.deepEqual(sourceCounts(g).map(([s]) => s), ['library', 'claude', 'codex', 'bundled']);
});

test('namesNeedingBodies lists only provider copies that have something to compare to', () => {
  const g = groupSkills(library, bundled, provider);
  const need = namesNeedingBodies(g, {});
  assert.deepEqual(
    need.map((n) => `${n.source}:${n.name}`).sort(),
    ['claude:docx', 'claude:vault-api-review', 'codex:docx', 'codex:vault-api-review'],
  );
  assert.equal(namesNeedingBodies(g, { [bodyKey('claude', 'docx')]: 'x' }).length, 3);
});

test('normalizeBody ignores line endings and trailing whitespace only', () => {
  assert.equal(normalizeBody('a  \r\nb\r\n\n'), 'a\nb');
  assert.notEqual(normalizeBody('a b'), normalizeBody('a  b'));
});

test('parseFrontmatter reads scalars, block scalars and lists', () => {
  const fm = parseFrontmatter(
    [
      '---',
      'name: vault-api-review',
      'description: >',
      '  Review the API docs',
      '  of a vault.',
      'allowed-tools: Read, Grep, Bash(git diff:*, git log:*)',
      'triggers:',
      '  - "review api"',
      '  - api docs',
      'tags: [docs, "api"]',
      'version: 3',
      '---',
      '# Body',
    ].join('\n'),
  );
  const m = Object.fromEntries(fm.meta);
  assert.equal(m.name, 'vault-api-review');
  assert.equal(m.description, 'Review the API docs of a vault.');
  assert.deepEqual(m.triggers, ['review api', 'api docs']);
  assert.deepEqual(m.tags, ['docs', 'api']);
  assert.equal(m.version, '3');
  assert.deepEqual(metaList(m['allowed-tools']), ['Read', 'Grep', 'Bash(git diff:*, git log:*)']);
  assert.equal(fm.body, '# Body');
});

test('a leaked block-scalar marker is not a description', () => {
  const g = groupSkills([], [], [{ provider: 'claude', name: 'x', category: '', description: '|' }, { provider: 'codex', name: 'y', category: '', description: '>-' }]);
  assert.deepEqual(g.map((x) => x.description), ['', '']);
});

test('parseFrontmatter without frontmatter returns the whole text', () => {
  assert.deepEqual(parseFrontmatter('# Just markdown'), { meta: [], body: '# Just markdown' });
  assert.deepEqual(parseFrontmatter(''), { meta: [], body: '' });
  assert.deepEqual(metaList(undefined), []);
});
