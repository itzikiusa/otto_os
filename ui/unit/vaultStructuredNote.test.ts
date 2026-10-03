import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  asList,
  matchByName,
  matchRepos,
  noteKind,
  previewText,
  sectionItems,
  sectionLines,
  structuredModel,
  workloadRouteKind,
  type NoteLike,
} from '../src/modules/vault/structuredNote.ts';

const note = (over: Partial<NoteLike['meta']>, raw = '', outgoing: NoteLike['outgoing'] = []): NoteLike => ({
  meta: { path: 'orders/orders-api.md', title: 'Orders API', okf_type: 'Service', frontmatter: {}, tags: [], headings: [], reserved: false, ...over },
  raw,
  outgoing,
});

test('noteKind recognises OKF types case/separator-insensitively and rejects plain notes', () => {
  assert.equal(noteKind('Service'), 'service');
  assert.equal(noteKind('API Endpoint'), 'api');
  assert.equal(noteKind('api_endpoint'), 'api');
  assert.equal(noteKind('Database Table'), 'data');
  assert.equal(noteKind('ADR'), 'decision');
  assert.equal(noteKind('Reference'), null);
  assert.equal(noteKind(null), null);
});

test('plain and reserved notes get no structured model', () => {
  assert.equal(structuredModel(note({ okf_type: null })), null);
  assert.equal(structuredModel(note({ reserved: true })), null);
});

test('frontmatter owners/status/environments/dependencies become header facts and cards', () => {
  const m = structuredModel(note({
    frontmatter: { owner: 'team-payments', status: 'stable', environments: ['staging', 'prod'], depends_on: ['ledger', 'kafka'] },
    headings: [{ level: 1, text: 'Overview' }, { level: 1, text: 'Dependencies' }],
  }, '', [{ raw_target: 'ledger', dst_path: 'ledger/index.md', alias: null }]))!;
  assert.equal(m.kind, 'service');
  assert.deepEqual(m.owners, ['team-payments']);
  assert.equal(m.status, 'stable');
  assert.deepEqual(m.cards.find((c) => c.key === 'environments')!.items.map((i) => i.value), ['staging', 'prod']);
  const deps = m.cards.find((c) => c.key === 'dependencies')!.items;
  assert.equal(deps[0].path, 'ledger/index.md', 'a dependency that is a resolved wikilink navigates');
  assert.equal(deps[1].path, undefined);
  assert.equal(m.sections.find((s) => s.label === 'Dependencies')!.present, true);
  assert.equal(m.sections.find((s) => s.label === 'Failure behavior')!.present, false);
});

test('cards fall back to the body section (bullets and first table column, wikilinks reduced)', () => {
  const raw = [
    '# Overview', 'x',
    '# Entry points',
    '| Endpoint | Purpose |', '|---|---|', '| `POST /orders` | create |', '| GET /orders/{id} | read |',
    '# Dependencies', '- [[ledger|Ledger service]]', '- Kafka `orders.v1`',
    '## Details', 'nested stays inside', '# Operations', '- not a dependency',
  ].join('\n');
  const m = structuredModel(note({}, raw))!;
  assert.deepEqual(m.cards.find((c) => c.key === 'endpoints')!.items.map((i) => i.value), ['POST /orders', 'GET /orders/{id}']);
  assert.deepEqual(m.cards.find((c) => c.key === 'dependencies')!.items.map((i) => i.value), ['Ledger service', 'Kafka orders.v1']);
});

test('sectionLines ignores headings inside code fences', () => {
  const lines = sectionLines(['# A', '```', '# Dependencies', '```', '# Dependencies', '- real'].join('\n'), ['dependenc']);
  assert.deepEqual(sectionItems(lines), ['real']);
});

test('API endpoint operation comes from resource; repo hints from a path resource', () => {
  const api = structuredModel(note({ okf_type: 'API Endpoint', frontmatter: { resource: 'post /v1/widgets' } }))!;
  assert.deepEqual(api.operation, { method: 'POST', path: '/v1/widgets' });
  assert.deepEqual(api.hints.repos, [], 'an HTTP operation is not a repo');
  const svc = structuredModel(note({ frontmatter: { resource: '/work/acme-orders', k8s: { cluster: 'prod-eu', namespace: 'orders', workload: 'orders-api' } } }))!;
  assert.deepEqual(svc.hints.repos, ['/work/acme-orders']);
  assert.equal(svc.hints.k8sCluster, 'prod-eu');
  assert.equal(svc.hints.k8sNamespace, 'orders');
  assert.ok(svc.hints.services.includes('orders-api'));
  assert.ok(svc.hints.services.includes('Orders API'), 'a service note matches by its own title');
});

test('asList flattens scalars, comma strings, object lists and maps', () => {
  assert.deepEqual(asList('a, b'), ['a', 'b']);
  assert.deepEqual(asList([{ name: 'x' }, 'y']), ['x', 'y']);
  assert.deepEqual(asList({ prod: 'https://p', staging: null }), ['prod: https://p', 'staging']);
  assert.deepEqual(asList(null), []);
});

test('name matching is normalised and ignores too-short hints', () => {
  const rows = [{ n: 'orders-api' }, { n: 'ordersapi-worker' }, { n: 'db' }];
  assert.deepEqual(matchByName(['Orders API'], rows, (r) => r.n), [{ n: 'orders-api' }]);
  assert.deepEqual(matchByName(['db'], rows, (r) => r.n), []);
});

test('repo matching by path, basename and remote slug', () => {
  const repos = [
    { name: 'acme-orders', path: '/work/acme-orders', remote_url: null },
    { name: 'other', path: '/x/other', remote_url: 'git@github.com:acme/ledger.git' },
    { name: 'misc', path: '/x/misc', remote_url: null },
  ];
  assert.deepEqual(matchRepos(['/work/acme-orders/'], repos).map((r) => r.name), ['acme-orders']);
  assert.deepEqual(matchRepos(['https://github.com/acme/ledger'], repos).map((r) => r.name), ['other']);
  assert.deepEqual(matchRepos(['nothing'], repos), []);
});

test('workload kinds map to resource-table routes; previews strip markup', () => {
  assert.equal(workloadRouteKind('StatefulSet'), 'statefulsets');
  assert.equal(workloadRouteKind('Deployment'), 'deployments');
  assert.equal(previewText('---\ntype: Service\n---\n# Title\nSee [[ledger|the ledger]] and `code`.\n```\nskip\n```'), 'See the ledger and code.');
});
