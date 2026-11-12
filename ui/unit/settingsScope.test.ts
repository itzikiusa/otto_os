import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  SETTINGS_SECTIONS,
  availableSections,
  filterSections,
  findSection,
  matchedSetting,
  scopeDescription,
} from '../src/modules/settings/sections.ts';
import { listenerIndicator, secretsUnencrypted } from '../src/modules/settings/daemonIndicators.ts';
import { archiveDaysError, archiveDaysFrom } from '../src/modules/settings/sessionArchive.ts';
import { channelAccess } from '../src/modules/settings/channelAccess.ts';

const all = availableSections({ can: () => true, isRoot: true });
const ids = (q: string): string[] => filterSections(all, q).map((s) => s.id);
const src = (f: string): string => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');

test('every section carries a scope and a non-empty setting index', () => {
  for (const s of SETTINGS_SECTIONS) {
    assert.ok(['device', 'me', 'workspace', 'everyone'].includes(s.scope), s.id);
    assert.ok(s.settings.length > 0, `${s.id} has no setting labels to search`);
  }
  const scope = (id: string) => findSection(id)!.scope;
  for (const id of ['appearance', 'snipping', 'browser']) assert.equal(scope(id), 'device', id);
  for (const id of ['tokens', 'git-accounts', 'jira']) assert.equal(scope(id), 'me', id);
  for (const id of ['channels', 'mcp-servers', 'context-soul', 'self-improvement']) assert.equal(scope(id), 'workspace', id);
  for (const id of ['notifications', 'daemon', 'providers', 'plugins', 'logs', 'trust-safety', 'backup']) {
    assert.equal(scope(id), 'everyone', id);
  }
  assert.match(scopeDescription(findSection('notifications')!), /^Everyone: /);
});

test('search finds individual settings and names the one it matched', () => {
  assert.equal(ids('idle timeout')[0], 'daemon');
  assert.equal(ids('font')[0], 'appearance');
  assert.equal(matchedSetting(findSection('appearance')!, 'font'), 'Terminal font');
  assert.equal(ids('listener')[0], 'daemon');
  // A section with the SETTING ranks above one that only has the keyword.
  assert.equal(ids('archive')[0], 'daemon');
  assert.equal(matchedSetting(findSection('daemon')!, 'archive'), 'Archive idle sessions');
  // The section's own name is never repeated as its "match".
  assert.equal(matchedSetting(findSection('daemon')!, 'daemon'), null);
});

test('secrets / encryption searches land on Trust & safety, not tokens', () => {
  for (const q of ['secret', 'secrets', 'encrypt', 'keychain', 'plaintext', 'security']) {
    assert.deepEqual(ids(q), ['trust-safety'], q);
  }
  // Tokens are still found by their own words.
  assert.ok(ids('pat').includes('tokens'));
  assert.equal(ids('token')[0], 'tokens');
});

test('listener indicator: what is BOUND, with the saved value pending a restart', () => {
  // No posture (non-root): only the saved value is known.
  assert.equal(listenerIndicator(true, null).text, 'network');
  assert.equal(listenerIndicator(undefined, null).text, 'loopback');
  // Saved on, still loopback-bound → pending, never a plain "network".
  const pendingOn = listenerIndicator(true, { network_listener: false, network_listener_restart_required: true });
  assert.equal(pendingOn.text, 'network (after restart)');
  assert.ok(pendingOn.pending);
  assert.match(pendingOn.title, /loopback only/);
  // Saved off while the network socket is still open → still says so.
  const pendingOff = listenerIndicator(false, { network_listener: true, network_listener_restart_required: true });
  assert.equal(pendingOff.text, 'loopback (after restart)');
  // Restart observed: the bound value, no pending state.
  const bound = listenerIndicator(true, { network_listener: true, network_listener_restart_required: false });
  assert.deepEqual([bound.text, bound.pending], ['network', false]);
  // A stale /meta value never overrides what the posture says is bound.
  assert.equal(listenerIndicator(true, { network_listener: false, network_listener_restart_required: false }).text, 'loopback');
});

test('secrets warning only for the plaintext store', () => {
  assert.equal(secretsUnencrypted({ mode: 'plaintext' }), true);
  assert.equal(secretsUnencrypted({ mode: 'encrypted' }), false);
  assert.equal(secretsUnencrypted({ mode: 'keychain' }), false);
  assert.equal(secretsUnencrypted(null), false);
});

test('auto-archive days: read as the daemon does, reject what it cannot store', () => {
  assert.equal(archiveDaysFrom({}), 0);
  assert.equal(archiveDaysFrom({ session_auto_archive_days: 14 }), 14);
  assert.equal(archiveDaysFrom({ session_auto_archive_days: '14' }), 0);
  assert.equal(archiveDaysFrom({ session_auto_archive_days: -3 }), 0);
  assert.equal(archiveDaysError(0), '');
  assert.equal(archiveDaysError(30), '');
  for (const bad of [-1, 1.5, null, Number.NaN, 1e9]) assert.notEqual(archiveDaysError(bad), '', String(bad));
});

test('channel bridges: owner configures, editors test/remove, viewers read', () => {
  assert.deepEqual(channelAccess(true, 'viewer'), { configure: true, editWorkspace: true, note: '' });
  const editor = channelAccess(false, 'editor');
  assert.equal(editor.configure, false);
  assert.equal(editor.editWorkspace, true);
  assert.match(editor.note, /owner/);
  const viewer = channelAccess(false, 'viewer');
  assert.deepEqual([viewer.configure, viewer.editWorkspace], [false, false]);
  assert.match(viewer.note, /viewer/);
  // The daemon gates these exactly so (owner for the upsert, Editor else).
  const http = readFileSync(new URL('../../crates/otto-channels/src/http.rs', import.meta.url), 'utf8');
  assert.match(http, /async fn upsert_integration[\s\S]*?require_owner_for_bridge/);
});

test('the restart reuses the desktop supervisor command, not a new route', () => {
  const restart = src('modules/settings/daemonRestart.ts');
  assert.match(restart, /invoke<string>\('daemon_restart'\)/);
  assert.match(src('modules/desktop/daemonState.ts'), /invoke<string>\('daemon_restart'\)/);
});
