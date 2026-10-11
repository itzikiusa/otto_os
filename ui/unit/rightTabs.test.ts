// The Side panel's pure rules (lib/rightTabs.ts): which tabs the rail shows
// where, what the open panel shows when the chosen tab isn't here, the
// VS Code-style rail click, the Git/Files folder, and the persisted per-
// workspace folder picks.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  RIGHT_TABS,
  contextFolder,
  isPageTabId,
  parseFolderPicks,
  railClick,
  shownRightTab,
  visibleRightTabs,
  withFolderPick,
} from '../src/lib/rightTabs.ts';

const ids = (place: Parameters<typeof visibleRightTabs>[0]) => visibleRightTabs(place).map((t) => t.id);

test('beside an agent session every tab is on the rail, in order', () => {
  assert.deepEqual(ids({ module: 'agents', session: true, browserVersion: 'v2' }), RIGHT_TABS.map((t) => t.id));
});

test('off a session the session-only tabs are hidden; pages hide the tab they already are', () => {
  assert.deepEqual(ids({ module: 'home', session: false, browserVersion: 'v2' }), ['git', 'files', 'notes', 'browser', 'api']);
  assert.deepEqual(ids({ module: 'git', session: false, browserVersion: 'v2' }), ['files', 'notes', 'browser', 'api']);
  assert.deepEqual(ids({ module: 'api', session: false, browserVersion: 'v2' }), ['git', 'files', 'notes', 'browser']);
  assert.deepEqual(ids({ module: 'browser', session: false, browserVersion: 'v2' }), ['git', 'files', 'notes', 'api']);
  // The classic per-session browser needs a session.
  assert.deepEqual(ids({ module: 'home', session: false, browserVersion: 'v1' }), ['git', 'files', 'notes', 'api']);
  assert.ok(ids({ module: 'agents', session: true, browserVersion: 'v1' }).includes('browser'));
  // A connection terminal on the Agents page is not an agent session.
  assert.ok(!ids({ module: 'agents', session: false, browserVersion: 'v2' }).includes('activity'));
});

test('the open panel shows the chosen tab when it exists here, else the first one (choice kept)', () => {
  assert.equal(shownRightTab('activity', ['git', 'files', 'activity']), 'activity');
  assert.equal(shownRightTab('activity', ['git', 'files']), 'git');
  assert.equal(shownRightTab('page:vault-details', ['page:vault-details', 'git']), 'page:vault-details');
  assert.equal(shownRightTab('git', []), null);
});

test('rail click: another tab opens it, the shown one collapses, a collapsed rail reopens', () => {
  assert.deepEqual(railClick(false, 'git', 'files'), { open: true, tab: 'files' });
  assert.deepEqual(railClick(true, 'git', 'files'), { open: true, tab: 'files' });
  assert.deepEqual(railClick(true, 'files', 'files'), { open: false, tab: 'files' });
  assert.deepEqual(railClick(false, 'files', 'files'), { open: true, tab: 'files' });
});

test('Git/Files folder: the session cwd in a session, else the pick, else the workspace root', () => {
  assert.equal(contextFolder({ session: true, sessionCwd: '/repo/a', picked: '/other', workspaceRoot: '/ws' }), '/repo/a');
  assert.equal(contextFolder({ session: true, sessionCwd: '', workspaceRoot: '/ws' }), '/ws');
  assert.equal(contextFolder({ session: false, sessionCwd: '/repo/a', picked: '/other', workspaceRoot: '/ws' }), '/other');
  assert.equal(contextFolder({ session: false, sessionCwd: '/repo/a', picked: null, workspaceRoot: '/ws' }), '/ws');
  assert.equal(contextFolder({ session: false }), '');
});

test('folder picks round-trip per workspace and ignore malformed storage', () => {
  assert.deepEqual(parseFolderPicks(null), {});
  assert.deepEqual(parseFolderPicks('nope'), {});
  assert.deepEqual(parseFolderPicks('[1,2]'), {});
  assert.deepEqual(parseFolderPicks('{"a":"/x","b":3,"c":"rel/path","d":"~/y"}'), { a: '/x', d: '~/y' });
  let picks = withFolderPick({}, 'w1', '/code/one');
  picks = withFolderPick(picks, 'w2', '/code/two');
  assert.deepEqual(parseFolderPicks(JSON.stringify(picks)), { w1: '/code/one', w2: '/code/two' });
  assert.deepEqual(withFolderPick(picks, 'w1', null), { w2: '/code/two' });
});

test('page tab ids are namespaced', () => {
  assert.ok(isPageTabId('page:vault-details'));
  assert.ok(!isPageTabId('git'));
});
