import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const importers = loadSource(new URL('../src/lib/api/importers.ts', import.meta.url), {
  './types': { isSecretRef: (v: unknown) => typeof v === 'object' && v !== null && typeof (v as { $secret?: unknown }).$secret === 'string' },
});

const env = {
  name: 'Prod',
  _postman_variable_scope: 'environment',
  values: [
    { key: 'base_url', value: 'https://api.example.com', enabled: true, type: 'default' },
    { key: 'signing', value: 's3cr3t', enabled: true, type: 'secret' },
    { key: 'api_token', value: 'tok-1', enabled: true, type: 'default' },
    { key: 'empty_secret', value: '', enabled: true, type: 'secret' },
    { key: 'off', value: 'x', enabled: false, type: 'secret' },
  ],
};

test('postman secret and credential-named variables import as Keychain secrets', () => {
  const parsed = importers.detectAndParse(JSON.stringify(env), 'prod.postman_environment.json');
  assert.equal(parsed.format, 'postman-env');
  assert.deepEqual([...parsed.secretKeys], ['signing', 'api_token']);
  const req = importers.environmentUpsertFor(parsed);
  assert.deepEqual({ ...req.variables }, { base_url: 'https://api.example.com', empty_secret: '' });
  assert.deepEqual({ ...req.secret_values }, { signing: 's3cr3t', api_token: 'tok-1' });
  assert.deepEqual([...req.secret_keys], ['signing', 'api_token']);
  // No secret value ever rides in the plain variables.
  assert.ok(!JSON.stringify(req.variables).includes('s3cr3t'));
});

test('an environment without secrets sends no secret values', () => {
  const req = importers.environmentUpsertFor({ format: 'postman-env', name: 'Dev', variables: { host: 'localhost' } });
  assert.deepEqual({ ...req.secret_values }, {});
  assert.deepEqual([...req.secret_keys], []);
});
