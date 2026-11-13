import { test } from 'node:test';
import assert from 'node:assert/strict';
import { attachmentSupport, takesFile } from '../src/modules/threads/runtime/attachmentSupport.ts';

test('every structured runtime agent takes images; PDFs go to Claude and the ACP agents', () => {
  for (const p of ['claude', 'agy', 'gemini', 'grok', 'opencode']) assert.deepEqual(attachmentSupport(p), { images: true, files: true }, p);
  for (const p of ['codex', 'pi']) assert.deepEqual(attachmentSupport(p), { images: true, files: false }, p);
  assert.deepEqual(attachmentSupport('shell'), { images: false, files: false });
});

test('takesFile follows the provider: a PDF to Antigravity yes, to Codex no; text never', () => {
  assert.equal(takesFile('agy', { type: 'image/png' }), true);
  assert.equal(takesFile('agy', { type: 'application/pdf' }), true);
  assert.equal(takesFile('codex', { type: 'application/pdf' }), false);
  assert.equal(takesFile('claude', { type: 'text/plain' }), false);
});
