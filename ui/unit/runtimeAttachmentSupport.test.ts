import { test } from 'node:test';
import assert from 'node:assert/strict';
import { attachmentSupport, capsOf, takesFile } from '../src/modules/threads/runtime/attachmentSupport.ts';

const acp = { images: true, files: true, steer: false };
const codex = { images: true, files: false, steer: true };

test('attachments follow the reported capabilities, not the provider name', () => {
  assert.deepEqual(attachmentSupport(acp), { images: true, files: true });
  assert.deepEqual(attachmentSupport(codex), { images: true, files: false });
  assert.deepEqual(attachmentSupport(null), { images: false, files: false });
});

test('takesFile: a PDF only where files are taken; text never', () => {
  assert.equal(takesFile(acp, { type: 'image/png' }), true);
  assert.equal(takesFile(acp, { type: 'application/pdf' }), true);
  assert.equal(takesFile(codex, { type: 'application/pdf' }), false);
  assert.equal(takesFile(acp, { type: 'text/plain' }), false);
});

test("capsOf prefers the session's own report over the provider baseline", () => {
  const live = { images: false, files: false, steer: false };
  assert.equal(capsOf(live, acp), live);
  assert.equal(capsOf(undefined, codex), codex);
  assert.deepEqual(capsOf(null, null), { images: false, files: false, steer: false, efforts: [] });
});
