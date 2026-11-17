import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DEFAULT_IDE_PREFERENCES, normalizeIdePreferencesRecord } from '../src/lib/components/cm-ide-prefs.ts';
import {
  projectListEntries,
  projectTabs,
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

test('project tabs: Scratches first and not deletable, then the named projects in order', () => {
  const tabs = projectTabs(projects, 'b', false);
  assert.deepEqual(tabs.map((t) => t.id), ['scratches', 'a', 'b', 'c']);
  assert.deepEqual(tabs.map((t) => t.label), ['Scratches', 'Otto', 'Sinatra Java', 'Koala Golang']);
  assert.deepEqual(tabs.map((t) => t.deletable), [false, true, true, true]);
  assert.deepEqual(tabs.map((t) => t.active), [false, false, true, false]);
  assert.deepEqual(projectTabs([], null, false).map((t) => t.id), ['scratches']);
});

test('project tabs: Scratches is active on it; a plain folder activates no tab', () => {
  assert.deepEqual(projectTabs(projects, null, true).map((t) => t.active), [true, false, false, false]);
  // A stale project id never lights a second tab while Scratches is open.
  assert.deepEqual(projectTabs(projects, 'a', true).filter((t) => t.active).map((t) => t.id), ['scratches']);
  assert.equal(projectTabs(projects, null, false).some((t) => t.active), false);
});

test('"Show projects as tabs" is off by default; the old Scratches tab setting turns it on', () => {
  assert.equal(DEFAULT_IDE_PREFERENCES.projectTabs, false);
  assert.equal(normalizeIdePreferencesRecord({}).projectTabs, false);
  assert.equal(normalizeIdePreferencesRecord({ projectTabs: true }).projectTabs, true);
  assert.equal(normalizeIdePreferencesRecord({ scratchesTab: true }).projectTabs, true);
  assert.equal(normalizeIdePreferencesRecord({ scratchesTab: false }).projectTabs, false);
  // The new key wins once it is saved.
  assert.equal(normalizeIdePreferencesRecord({ scratchesTab: true, projectTabs: false }).projectTabs, false);
  assert.equal('scratchesTab' in normalizeIdePreferencesRecord({ scratchesTab: true }), false);
});
