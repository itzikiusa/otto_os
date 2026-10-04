import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SchemaVersionCache } from '../src/modules/brokers/schemaVersionCache.ts';

test('version cache reuses immutable selected versions with bounded bytes and entries', () => {
  const cache = new SchemaVersionCache(1024, 2);
  const body = (version: number, schema = '{}') => ({version,subject:'s',id:version,schema_type:'JSON',schema});
  cache.set('/a', body(1)); cache.set('/a', body(2));
  assert.equal(cache.get('/a', 1)?.id, 1);
  cache.set('/a', body(3));
  assert.equal(cache.get('/a', 2), null);
  assert.equal(cache.get('/b', 1), null);
  cache.set('/a', body(4, 'x'.repeat(1024)));
  assert.equal(cache.get('/a', 4), null);
  assert.ok(cache.bytes <= 1024);
});
