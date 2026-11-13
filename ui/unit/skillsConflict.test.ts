import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { sha256Hex } from '../src/modules/skills-lab/sha256.ts';

// The editor's `if_hash` must equal the daemon's SHA-256 of the file bytes,
// or every save would read as a conflict.
test('sha256Hex matches SHA-256 of the UTF-8 bytes', () => {
  assert.equal(sha256Hex(''), 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
  assert.equal(sha256Hex('abc'), 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
  for (const text of ['---\ndescription: x\n---\n# body\n', 'é ✓ 🙂 עברית', 'x'.repeat(55), 'y'.repeat(56), 'z'.repeat(64), 'w'.repeat(1000)]) {
    assert.equal(sha256Hex(text), createHash('sha256').update(text, 'utf8').digest('hex'), JSON.stringify(text.slice(0, 20)));
  }
});

import { componentFunctions } from './componentFunctions.ts';
const editor = new URL('../src/modules/skills-lab/SkillEditor.svelte', import.meta.url);
class ApiError extends Error { status: number; constructor(status: number) { super('conflict'); this.status = status; } }
const quiet = { success() {}, info() {} };

test('a stale save keeps the draft, sends the base hash and shows their version', async () => {
  const sent: any[] = []; let toasted = 0; let published = 0;
  const state = componentFunctions(editor, ['save', 'loadTheirs'], {
    editable: true, saving: false, loading: false, loadError: null, binary: false, dirty: true,
    name: 'skill', source: 'library', currentFile: 'references/a.md', content: 'mine', original: 'base text',
    loadGeneration: 1, disposed: false, conflict: null, ApiError, sha256Hex,
    skillLabApi: {
      putFile: async (_n: string, body: unknown) => { sent.push(body); throw new ApiError(409); },
      getFile: async () => ({ path: 'references/a.md', content: 'theirs', binary: false, hash: 'their-hash' }),
    },
    onsaved() { published++; }, toasts: quiet, toastError() { toasted++; }, noteKeptCopies() {},
  });
  await state.save();
  assert.equal(sent[0].if_hash, sha256Hex('base text'));
  assert.equal(state.content, 'mine', 'the person’s text is never dropped');
  assert.equal(state.original, 'base text');
  assert.deepEqual(JSON.parse(JSON.stringify(state.conflict)), { path: 'references/a.md', theirs: 'theirs', hash: 'their-hash' });
  assert.equal(toasted, 0); assert.equal(published, 0);
});

test('Keep mine writes over their version using their hash; Reload theirs swaps the text', async () => {
  const sent: any[] = [];
  const state = componentFunctions(editor, ['keepMine', 'reloadTheirs'], {
    saving: false, name: 'skill', source: 'library', currentFile: 'references/a.md', content: 'mine', original: 'base',
    loadGeneration: 1, disposed: false, docKey: 0, files: [], ApiError, sha256Hex,
    conflict: { path: 'references/a.md', theirs: 'theirs', hash: 'their-hash' },
    skillLabApi: { putFile: async (_n: string, body: unknown) => { sent.push(body); return { files: [] }; } },
    onsaved() {}, toasts: quiet, toastError() {}, noteKeptCopies() {},
  });
  await state.keepMine();
  assert.equal(sent[0].if_hash, 'their-hash'); assert.equal(sent[0].content, 'mine');
  assert.equal(state.conflict, null); assert.equal(state.original, 'mine');

  state.conflict = { path: 'references/a.md', theirs: 'theirs', hash: 'h2' }; state.content = 'draft';
  state.reloadTheirs();
  assert.equal(state.content, 'theirs'); assert.equal(state.original, 'theirs'); assert.equal(state.conflict, null);
});

import { deleteQuestion, installedNote, replaceQuestion } from '../src/modules/skills-lab/bundledText.ts';

test('one install question: first install asks nothing, an edited copy is a danger confirm that backs up', () => {
  assert.equal(replaceQuestion('grill', 'not_installed', 3), null);
  const update = replaceQuestion('grill', 'update_available', 3)!;
  assert.equal(update[1].danger, false); assert.match(update[0], /backed up to .*skills-backup/);
  for (const state of ['modified', 'ahead']) {
    const [message, opts] = replaceQuestion('grill', state, 3)!;
    assert.equal(opts.danger, true, state); assert.equal(opts.confirmLabel, 'Back up & replace');
    assert.match(message, /Cancel keeps your edited copy/);
  }
  assert.match(installedNote({ backed_up: true, backup_path: '/x/skills-backup/grill-1' }, true), /grill-1/);
  assert.equal(installedNote({ backed_up: false, backup_path: null }, false), 'You can edit it now.');
});

test('deleting a skill says where it goes and is a danger confirm', () => {
  const [message, opts] = deleteQuestion('grill');
  assert.equal(opts.danger, true);
  assert.match(message, /skills-backup/);
  assert.match(message, /skills you made yourself in those folders are not touched/);
});
