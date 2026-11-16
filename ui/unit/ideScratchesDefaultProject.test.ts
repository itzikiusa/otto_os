import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DEFAULT_IDE_PREFERENCES, normalizeIdePreferencesRecord } from '../src/lib/components/cm-ide-prefs.ts';
import {
  pickReturnProject,
  projectListEntries,
  SCRATCHES_LOCKED,
  scratchesMenuItems,
} from '../src/modules/ide/shell/ideProjectList.ts';

const proj = (id: string, name: string, root = `/repos/${id}`) => ({ id, name, primary_root: root, folders: [root] });
const projects = [proj('a', 'Otto'), proj('b', 'Sinatra Java'), proj('c', 'Koala Golang')];

const ids = (entries: ReturnType<typeof projectListEntries>) =>
  entries.map((e) => (e.kind === 'scratches' ? 'scratches' : e.project.id));

test('Scratches is the first project, before every named one', () => {
  assert.deepEqual(ids(projectListEntries(projects, '')), ['scratches', 'a', 'b', 'c']);
  assert.deepEqual(ids(projectListEntries([], '  ')), ['scratches']);
});

test('the filter keeps Scratches only while it matches', () => {
  assert.deepEqual(ids(projectListEntries(projects, 'scrat')), ['scratches']);
  assert.deepEqual(ids(projectListEntries(projects, 'Default')), ['scratches']);
  assert.deepEqual(ids(projectListEntries(projects, 'java')), ['b']);
  assert.deepEqual(ids(projectListEntries(projects, '/repos/c')), ['c']);
  assert.deepEqual(ids(projectListEntries(projects, 'nothing')), []);
});

test('Scratches can be neither renamed nor deleted, and the menu says why', () => {
  const items = scratchesMenuItems().filter((i) => !i.separator);
  assert.deepEqual(items.map((i) => i.label), ['Rename project…', 'Delete project…']);
  for (const i of items) {
    assert.equal(i.disabled, true);
    assert.equal(i.title, SCRATCHES_LOCKED);
    assert.equal(i.action, undefined);
  }
});

test('the Project tab returns to the last real project while it exists', () => {
  const ws = [{ id: 'w1', root_path: '/ws/one' }];
  const last = { root: '/repos/b', workspaceId: null, projectId: 'b' };
  assert.deepEqual(pickReturnProject(last, projects, ws), last);
  const folder = { root: '/elsewhere', workspaceId: 'w9', projectId: null };
  assert.deepEqual(pickReturnProject(folder, projects, ws), folder);
  // The last project was deleted: the first named project, then a workspace folder.
  const gone = { root: '/repos/z', workspaceId: null, projectId: 'z' };
  assert.deepEqual(pickReturnProject(gone, projects, ws), { root: '/repos/a', workspaceId: null, projectId: 'a' });
  assert.deepEqual(pickReturnProject(gone, [], ws), { root: '/ws/one', workspaceId: 'w1', projectId: null });
  assert.equal(pickReturnProject(null, [], [{ id: 'w2', root_path: '' }]), null);
});

test('the Scratches tab setting is off by default and survives a round trip', () => {
  assert.equal(DEFAULT_IDE_PREFERENCES.scratchesTab, false);
  assert.equal(normalizeIdePreferencesRecord({}).scratchesTab, false);
  assert.equal(normalizeIdePreferencesRecord({ scratchesTab: true }).scratchesTab, true);
});
