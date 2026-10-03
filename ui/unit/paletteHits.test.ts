// ⌘K palette search hits: every action shown has a handler (review 06 F4).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { hitActions, hitContext, hitRoute, normalizeHitAction } from '../src/lib/floatingBar.ts';

test('open is first, unknown server actions are hidden', () => {
  assert.deepEqual(hitActions({ actions: ['rerun', 'send-to-agent', 'open'] }), ['open', 'send-to-agent']);
  assert.deepEqual(hitActions({ actions: ['open', 'review'] }), ['open']);
});

test("the canvas hit's legacy 'Open in Canvas' label still opens", () => {
  assert.equal(normalizeHitAction('Open in Canvas'), 'open');
  assert.deepEqual(hitActions({ actions: ['Open in Canvas'] }), ['open']);
  assert.equal(hitRoute({ kind: 'canvas', id: 'c1' }), 'canvas');
});

test('a repo hit routes to its repo', () => {
  assert.equal(hitRoute({ kind: 'repo', id: 'r9' }), 'git/r9');
});

test('context names the kind, title, subtitle and id', () => {
  assert.equal(
    hitContext({ kind: 'api_request', id: 'a1', title: 'Get user', subtitle: 'GET /users/1' }),
    'api request: Get user\nGET /users/1\n(api_request id a1)',
  );
  assert.equal(hitContext({ kind: 'repo', id: 'r', title: 'otto', subtitle: null }), 'repo: otto\n(repo id r)');
});
