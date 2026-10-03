import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isTextualType, looksBinary, mimeOf } from '../src/modules/api/responseKind.ts';

test('declared text types are always text', () => {
  for (const ct of ['text/plain; charset=utf-8', 'application/json', 'application/problem+json', 'application/vnd.api+json', 'application/xml', 'image/svg+xml', 'application/x-www-form-urlencoded']) {
    assert.equal(isTextualType(ct), true, ct);
    assert.equal(looksBinary(ct, '���'), false, ct);
  }
  assert.equal(mimeOf(' Application/PDF ; q=1'), 'application/pdf');
});

test('binary payloads are detected by their decoded bytes', () => {
  const pdf = '%PDF-1.7\n' + '�\u0000\u0001'.repeat(200);
  assert.equal(looksBinary('application/pdf', pdf), true);
  assert.equal(looksBinary(null, pdf), true);
  // An octet-stream that is really plain text stays readable.
  assert.equal(looksBinary('application/octet-stream', 'hello world\n'.repeat(50)), false);
  assert.equal(looksBinary('application/octet-stream', ''), false);
});
