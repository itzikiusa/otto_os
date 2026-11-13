import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { keptCopiesNote, PROVIDER_BACKUP_FOLDER } from '../src/modules/skills-lab/bundledText.ts';

const kept = {
  copy: '/Users/me/.claude/skills/grill',
  kept_at: '/Users/me/Library/Application Support/Otto/library/provider-skills-backup/claude/grill-1763000000-01ARZ3NDEKTSV4RRFFQ69G5FAV',
};

test('no kept copies, no toast', () => {
  assert.equal(keptCopiesNote(undefined), null);
  assert.equal(keptCopiesNote([]), null);
});

test('the toast names the edited copy and where it was kept', () => {
  const note = keptCopiesNote([kept]);
  assert.ok(note);
  const [title, body] = note;
  assert.equal(title, 'Kept your edited copy');
  assert.match(body, /Your edited copy in \/Users\/me\/\.claude\/skills\/grill was kept at .*provider-skills-backup\/claude\/grill-/);
  assert.equal(keptCopiesNote([kept, { ...kept, copy: '/Users/me/.codex/skills/grill' }])?.[0], 'Kept 2 edited copies');
  assert.match(PROVIDER_BACKUP_FOLDER, /provider-skills-backup/);
});

test('a save reports the kept copy even when the person moved on to another skill', async () => {
  const noted: unknown[] = []; let published = 0;
  const state = componentFunctions(new URL('../src/modules/skills-lab/SkillEditor.svelte', import.meta.url), ['save'], {
    editable: true, saving: false, loading: false, loadError: null, binary: false, dirty: true,
    name: 'grill', source: 'library', currentFile: 'SKILL.md', content: 'new', original: 'old', loadGeneration: 1, disposed: false,
    skillLabApi: { putFile: async () => { state.loadGeneration++; return { files: [], kept_copies: [kept] }; } },
    onsaved() { published++; }, toasts: { success() {}, info() {} }, toastError() {}, sha256Hex: () => 'base',
    noteKeptCopies(k: unknown) { noted.push(k); },
  });
  await state.save();
  assert.deepEqual(JSON.parse(JSON.stringify(noted)), [[kept]]);
  assert.equal(published, 0, 'the stale selection is still not touched');
});
