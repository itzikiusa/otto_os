import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { FILE_REF_PREFIX, filesForSave, refSha, sha256Hex } from '../src/modules/canvas/canvasFileRefs.ts';

const img = `data:image/png;base64,${'A'.repeat(3 * 1024 * 1024)}`;
const sha = createHash('sha256').update(img).digest('hex');

test('refSha accepts only full sha256 refs', () => {
  assert.equal(refSha(FILE_REF_PREFIX + sha), sha);
  assert.equal(refSha(img), null);
  assert.equal(refSha(`${FILE_REF_PREFIX}abc`), null);
  assert.equal(refSha(undefined), null);
});

test('the client digest is the server content address (sha256 of the data-URL text)', async () => {
  assert.equal(await sha256Hex(img), sha);
});

test('a 3 MB image goes inline once, then every autosave sends a ref', () => {
  const known = new Map<string, string>();
  const files = { f1: { id: 'f1', mimeType: 'image/png', dataURL: img } };
  // First save: the server hasn't seen it.
  const first = filesForSave(files, known);
  assert.equal(first.inline.length, 1);
  assert.equal(first.files.f1.dataURL, img);
  // Learned after the save lands → the next 5 edits are small.
  known.set('f1', sha);
  for (let i = 0; i < 5; i++) {
    const next = filesForSave(files, known);
    assert.equal(next.inline.length, 0);
    const body = JSON.stringify({ doc: { source: JSON.stringify({ elements: [{ id: `e${i}` }], files: next.files }) } });
    assert.ok(body.length < 200 * 1024, `autosave body ${body.length} B`);
    assert.equal(next.files.f1.mimeType, 'image/png', 'other file fields are kept');
  }
});

test('an unresolved ref passes through and is not re-learned as inline', () => {
  const ref = { id: 'f2', dataURL: FILE_REF_PREFIX + sha };
  const out = filesForSave({ f2: ref }, new Map());
  assert.equal(out.files.f2.dataURL, ref.dataURL);
  assert.equal(out.inline.length, 0);
});
