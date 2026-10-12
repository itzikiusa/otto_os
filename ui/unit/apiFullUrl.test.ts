// The API client's Params tab shows the URL as sent (lib/api/apiVars.ts
// `fullRequestUrl`): enabled, named rows appended and form-encoded the way the
// daemon's reqwest `.query()` does, known {{variables}} filled in, the rest
// left readable.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fillKnownVars, fullRequestUrl, type VarEnv } from '../src/lib/api/apiVars.ts';

const row = (key: string, value: string, enabled = true) => ({ key, value, enabled });

test('enabled params are appended in order; disabled and unnamed rows are skipped', () => {
  const url = fullRequestUrl('https://gwsinatra-stg.groovegaming.com/games/attribute/get/1.0/list', [
    row('type', 'FEATURE'),
    row('page', '0'),
    row('size', '100'),
    row('debug', 'true', false),
    row('', 'orphan'),
    row('sort', 'name'),
    row('order', 'asc'),
  ]);
  assert.equal(url, 'https://gwsinatra-stg.groovegaming.com/games/attribute/get/1.0/list?type=FEATURE&page=0&size=100&sort=name&order=asc');
});

test('keys and values are form-encoded like the daemon (space → +, reserved chars escaped)', () => {
  const url = fullRequestUrl('https://x.test/s', [row('q', 'a b&c=d/é'), row('tag[]', "it's*-._~")]);
  assert.equal(url, "https://x.test/s?q=a+b%26c%3Dd%2F%C3%A9&tag%5B%5D=it%27s*-._%7E");
  // An empty value still sends the key.
  assert.equal(fullRequestUrl('https://x.test', [row('flag', '')]), 'https://x.test?flag=');
});

test('an existing query string is extended, a fragment stays last', () => {
  assert.equal(fullRequestUrl('https://x.test/p?a=1', [row('b', '2')]), 'https://x.test/p?a=1&b=2');
  assert.equal(fullRequestUrl('https://x.test/p?', [row('b', '2')]), 'https://x.test/p?b=2');
  assert.equal(fullRequestUrl('https://x.test/p?a=1&', [row('b', '2')]), 'https://x.test/p?a=1&b=2');
  assert.equal(fullRequestUrl('https://x.test/p#top', [row('b', '2')]), 'https://x.test/p?b=2#top');
  assert.equal(fullRequestUrl('  https://x.test/p  ', []), 'https://x.test/p');
});

test('known variables are filled in (session over environment, nested); unknown, secret and built-ins stay raw', () => {
  const env: VarEnv = { name: 'stg', variables: { host: 'https://{{sub}}.test', sub: 'api', size: '100' }, secret_keys: ['token'] };
  const url = fullRequestUrl('{{host}}/list', [
    row('size', '{{size}}'),
    row('page', '{{page}}'),
    row('token', '{{token}}'),
    row('id', '{{$guid}}'),
    row('who', '{{user}} x'),
  ], { user: 'ada lovelace' }, env);
  assert.equal(url, 'https://api.test/list?size=100&page={{page}}&token={{token}}&id={{$guid}}&who=ada+lovelace+x');
  // No resolver: everything stays as typed.
  assert.equal(fullRequestUrl('{{host}}/x', [row('a', '{{v}}')]), '{{host}}/x?a={{v}}');
});

test('fillKnownVars stops on a self-referencing value instead of looping', () => {
  const env: VarEnv = { name: 'e', variables: { a: '{{a}}!' }, secret_keys: [] };
  assert.equal(fillKnownVars('{{a}}', {}, env), '{{a}}!!!');
});

test('an API key sent in the query is shown last, its value always masked', () => {
  const env: VarEnv = { name: 'e', variables: { keyname: 'api_key' }, secret_keys: [] };
  const plain = fullRequestUrl('https://x.test/a', [row('q', '1')], {}, env, { type: 'api_key', key: '{{keyname}}', value: 'sk-live-123', in: 'query' });
  assert.equal(plain, 'https://x.test/a?q=1&api_key=••••');
  assert.ok(!plain.includes('sk-live'));
  const secret = fullRequestUrl('https://x.test/a', [], {}, null, { type: 'api_key', key: 'token', value: { $secret: 'ref' }, in: 'query' });
  assert.equal(secret, 'https://x.test/a?token=••••');
  // In a header, or no key: nothing in the URL.
  assert.equal(fullRequestUrl('https://x.test/a', [], {}, null, { type: 'api_key', key: 'X-Key', value: 'v', in: 'header' }), 'https://x.test/a');
  assert.equal(fullRequestUrl('https://x.test/a', [], {}, null, { type: 'api_key', key: ' ', value: 'v', in: 'query' }), 'https://x.test/a');
  assert.equal(fullRequestUrl('https://x.test/a', [], {}, null, { type: 'bearer', token: 't' }), 'https://x.test/a');
});
