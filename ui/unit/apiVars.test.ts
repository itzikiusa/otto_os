import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  collectionPaths,
  methodTone,
  parseSetCookies,
  requestTexts,
  resolveVar,
  splitUrl,
  splitVars,
  statusTone,
  statusWord,
  varNames,
} from '../src/lib/api/apiVars.ts';
import { searchTree, childPath, preview } from '../src/lib/api/jsonTree.ts';
import { forDuplicate } from '../src/lib/api/apiSecretShapes.ts';

const env = { name: 'Staging', variables: { base_url: 'https://s.example', id: '1' }, secret_keys: ['api_token'] };

test('variables resolve in daemon order: built-ins, session, environment, secret, missing', () => {
  assert.equal(resolveVar('$guid', { $guid: 'x' }, env).kind, 'dynamic');
  assert.deepEqual(
    [resolveVar('id', { id: '9' }, env).value, resolveVar('id', {}, env).value],
    ['9', '1'],
  );
  const secret = resolveVar('api_token', {}, env);
  assert.equal(secret.kind, 'secret');
  assert.equal(secret.value, undefined, 'a secret value is never exposed');
  assert.match(resolveVar('nope', {}, env).detail, /Not set in “Staging”/);
  assert.match(resolveVar('nope', {}, null).detail, /No environment/);
});

test('variable discovery and highlight segments', () => {
  assert.deepEqual(varNames('{{base_url}}/v1/{{ id }}', 'Bearer {{api_token}}', '{{id}}'), ['base_url', 'id', 'api_token']);
  const segs = splitVars('{{base_url}}/users/{{id}}');
  assert.deepEqual(segs.map((s) => [s.text, s.isVar]), [['{{base_url}}', true], ['/users/', false], ['{{id}}', true]]);
  assert.equal(segs.map((s) => s.text).join(''), '{{base_url}}/users/{{id}}', 'segments rebuild the input exactly');
  const texts = requestTexts({
    url: '{{a}}', body: '{{b}}', auth: { type: 'bearer', token: '{{c}}' },
    headers: [{ key: 'X', value: '{{d}}', enabled: false }], query: [{ key: 'q', value: '{{e}}' }],
  });
  assert.deepEqual(varNames(...texts), ['a', 'e', 'b', 'c'], 'disabled rows are not sent');
});

test('method and status tones carry words, not only colour', () => {
  assert.deepEqual(['get', 'post', 'patch', 'delete', 'head'].map(methodTone), ['get', 'post', 'put', 'delete', 'other']);
  assert.deepEqual([200, 302, 404, 503, null].map(statusTone), ['ok', 'redirect', 'client', 'server', 'none']);
  assert.equal(statusWord(422), 'Client error');
});

test('collection paths follow the tree order', () => {
  const c = (id: string, name: string, parent_id: string | null, position = 0) =>
    ({ id, name, parent_id, position, workspace_id: 'w', created_at: '' }) as never;
  const paths = collectionPaths([c('b', 'Zeta', null), c('a', 'Alpha', null), c('a1', 'Child', 'a')]);
  assert.deepEqual(paths.map((p) => p.path), ['Alpha', 'Alpha / Child', 'Zeta']);
  assert.equal(paths[1].depth, 1);
});

test('url split and Set-Cookie parsing', () => {
  assert.deepEqual(splitUrl('https://api.x.dev/v1/users?limit=2'), { host: 'api.x.dev', path: '/v1/users?limit=2' });
  assert.deepEqual(splitUrl('{{base}}/x'), { host: '', path: '{{base}}/x' });
  const cookies = parseSetCookies([
    { key: 'Set-Cookie', value: 'session=abc; Path=/; HttpOnly' },
    { key: 'Content-Type', value: 'x' },
  ]);
  assert.deepEqual(cookies, [{ name: 'session', value: 'abc', attributes: 'Path=/; HttpOnly' }]);
});

test('JSON tree search finds keys and values and opens their ancestors', () => {
  const doc = { data: [{ id: 'cus_1', email: 'ada@example.com' }], 'odd key': 1 };
  const { hits, open } = searchTree(doc, 'ADA@');
  assert.deepEqual([...hits], ['$.data[0].email']);
  assert.deepEqual([...open].sort(), ['$', '$.data', '$.data[0]']);
  assert.ok(searchTree(doc, 'odd').hits.has('$["odd key"]'));
  assert.equal(searchTree(doc, '  ').hits.size, 0);
  assert.equal(childPath('$', 'a-b'), '$["a-b"]');
  assert.equal(preview([1, 2]), '[2 items]');
  assert.equal(preview({ a: 1 }), '{1 key}');
});

test('duplicating a request never copies another request’s Keychain marker', () => {
  const d = {
    auth: { type: 'bearer' as const, token: { $secret: 'otto.api.request.r1' } },
    headers: [{ key: 'Authorization', value: '{{api_token}}' }],
    query: [],
  };
  const out = forDuplicate(d as never) as typeof d & { blanked: boolean };
  assert.equal(out.blanked, true);
  assert.deepEqual(out.auth, { type: 'bearer', token: '' });
  assert.equal(out.headers[0].value, '{{api_token}}');
  assert.notEqual(out.headers, d.headers, 'rows are copied, not shared');
  const plain = forDuplicate({ auth: { type: 'bearer', token: '{{t}}' }, headers: [], query: [] } as never) as { blanked: boolean };
  assert.equal(plain.blanked, false);
});
