// Paste / drop into the vault editor: which files are taken, how clipboard
// images are named, and the client-side refusals (no request sent).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  MAX_ATTACHMENT_BYTES,
  carriesFiles,
  insertionFor,
  preflight,
  transferFiles,
  uploadName,
} from '../src/modules/vault/attachmentUpload.ts';

const file = (name: string, size = 10) => ({ name, size }) as unknown as File;

test('files come from `files`, else from file items (mobile Safari paste)', () => {
  const a = file('a.png');
  assert.deepEqual(transferFiles({ files: [a] }), [a]);
  const b = file('image.png');
  const items = [
    { kind: 'string', getAsFile: () => null },
    { kind: 'file', getAsFile: () => b },
    { kind: 'file', getAsFile: () => null },
  ];
  assert.deepEqual(transferFiles({ files: [], items }), [b]);
  assert.deepEqual(transferFiles({ files: [], items: [] }), [], 'a text paste is not taken');
  assert.deepEqual(transferFiles(null), []);
});

test('only a drag that carries files is accepted', () => {
  assert.equal(carriesFiles({ types: ['text/plain'] }), false);
  assert.equal(carriesFiles({ types: ['Files', 'text/uri-list'] }), true);
  assert.equal(carriesFiles(undefined), false);
});

test('a generic clipboard image name is left to the vault; real names are kept', () => {
  assert.equal(uploadName({ name: 'image.png' }, 'paste'), '');
  assert.equal(uploadName({ name: 'IMAGE.JPEG' }, 'paste'), '');
  assert.equal(uploadName({ name: 'image.png' }, 'drop'), 'image.png');
  assert.equal(uploadName({ name: 'Screenshot 2026-11-12.png' }, 'paste'), 'Screenshot 2026-11-12.png');
  assert.equal(uploadName({}, 'paste'), '');
});

test('size cap and empty files are refused before upload with a clear message', () => {
  assert.equal(preflight({ name: 'ok.png', size: 1024 }), null);
  assert.equal(preflight({ name: 'max.png', size: MAX_ATTACHMENT_BYTES }), null);
  assert.equal(
    preflight({ name: 'big.mov', size: MAX_ATTACHMENT_BYTES + 1 }),
    'big.mov is 20.1 MB; attachments can be at most 20 MB.',
  );
  assert.match(preflight({ name: 'huge.zip', size: 25 * 1024 * 1024 }) ?? '', /^huge\.zip is 25\.0 MB/);
  assert.equal(preflight({ name: '', size: 0 }), 'The file is empty.');
});

test('several files from one drop land on their own lines', () => {
  assert.equal(insertionFor('![[a.png]]', 0), '![[a.png]]');
  assert.equal(insertionFor('![[b.png]]', 1), '\n![[b.png]]');
});
