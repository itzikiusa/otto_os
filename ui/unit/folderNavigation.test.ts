import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyHistory, recordFolder, historyTarget, folderCrumbs, parseShortcuts, rememberFolder, toggleFavorite } from '../src/lib/components/folderNavigation.ts';

test('Back and Forward preserve history until a different folder is opened', () => {
  let history = emptyHistory();
  assert.equal(historyTarget(history, -1), null);
  for (const path of ['/Users', '/Users/itziklavon', '/Users/itziklavon/go_deposit']) history = recordFolder(history, path);
  assert.equal(historyTarget(history, 1), null);
  history = { ...history, index: historyTarget(history, -1)! };
  assert.equal(history.paths[history.index], '/Users/itziklavon');
  assert.equal(history.paths[historyTarget(history, 1)!], '/Users/itziklavon/go_deposit');
  assert.equal(recordFolder(history, '/Users/itziklavon'), history);
  history = recordFolder(history, '/Users/itziklavon/another');
  assert.deepEqual(history.paths, ['/Users', '/Users/itziklavon', '/Users/itziklavon/another']);
  assert.equal(historyTarget(history, 1), null);
});

test('each breadcrumb targets its exact ancestor including spaces, Unicode, and root', () => {
  assert.deepEqual(folderCrumbs('/Users/Itzik Lavon/שלום/project'), [
    { label: '/', path: '/' }, { label: 'Users', path: '/Users' },
    { label: 'Itzik Lavon', path: '/Users/Itzik Lavon' },
    { label: 'שלום', path: '/Users/Itzik Lavon/שלום' },
    { label: 'project', path: '/Users/Itzik Lavon/שלום/project' },
  ]);
  assert.deepEqual(folderCrumbs('/'), [{ label: '/', path: '/' }]);
});

test('shortcut persistence tolerates corrupt data and bounds unique recent folders', () => {
  assert.deepEqual(parseShortcuts('not json'), { favorites: [], recents: [] });
  assert.deepEqual(parseShortcuts(JSON.stringify({ favorites: ['/one', '/one', false, 'relative'], recents: null })),
    { favorites: ['/one'], recents: [] });
  let shortcuts = parseShortcuts(null);
  for (let i = 0; i < 25; i++) shortcuts = rememberFolder(shortcuts, `/folder${i}`);
  shortcuts = rememberFolder(shortcuts, '/folder20');
  assert.equal(shortcuts.recents.length, 20);
  assert.equal(shortcuts.recents[0], '/folder20');
  assert.equal(shortcuts.recents.filter(path => path === '/folder20').length, 1);
  shortcuts = toggleFavorite(shortcuts, '/folder20');
  assert.deepEqual(parseShortcuts(JSON.stringify(shortcuts)), shortcuts);
  assert.deepEqual(toggleFavorite(shortcuts, '/folder20').favorites, []);
});
