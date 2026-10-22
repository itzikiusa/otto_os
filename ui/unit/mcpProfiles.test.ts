import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  canonical,
  connectionConfigs,
  draftOf,
  duplicateName,
  emptyDraft,
  groupOf,
  profileAllows,
  profileSummary,
  profileTools,
  selectionHint,
  selectionLabel,
  sessionProfileIds,
  storedProfileIds,
  toggleGroup,
  toggleProfile,
  toggleTool,
  validateDraft,
} from '../src/lib/mcpProfiles.ts';

const groups = [
  { id: 'database', label: 'Database', description: '', prefixes: ['db_'], tools: ['list_connections', 'query_db_readonly'] },
  { id: 'git', label: 'Git & PRs', description: '', prefixes: [], tools: ['list_repos', 'create_pr', 'merge_pr'] },
  { id: 'aws', label: 'AWS', description: '', prefixes: ['aws_'], tools: ['aws_s3_list_buckets'] },
];

function profile(id: string, over: Record<string, unknown> = {}): any {
  return {
    id, name: id[0].toUpperCase() + id.slice(1), description: '', all_tools: false, groups: [id],
    include_tools: [], exclude_tools: [], builtin: false, locked: false, is_default: false,
    created_at: '', updated_at: '', ...over,
  };
}
const full = profile('full', { name: 'Full', all_tools: true, groups: [], locked: true, builtin: true, is_default: true });
const all = [full, profile('database'), profile('git')];

test('canonical names and group membership mirror the daemon', () => {
  for (const n of ['otto.create_pr', 'otto_create_pr', 'create_pr']) assert.equal(canonical(n), 'create_pr');
  assert.equal(groupOf(groups, 'otto_db_schema')?.id, 'database', 'prefix family');
  assert.equal(groupOf(groups, 'otto.list_connections')?.id, 'database');
  assert.equal(groupOf(groups, 'k8s_logs'), undefined);
});

test('a profile is its groups plus includes minus excludes, Full is everything', () => {
  const db = profile('database', { include_tools: ['list_repos'], exclude_tools: ['query_db_readonly'] });
  assert.ok(profileAllows(groups, db, 'otto_db_query'));
  assert.ok(profileAllows(groups, db, 'otto_list_repos'));
  assert.ok(!profileAllows(groups, db, 'otto.query_db_readonly'));
  assert.ok(!profileAllows(groups, db, 'create_pr'));
  assert.ok(profileAllows(groups, full, 'anything'));
  assert.deepEqual(profileTools(groups, db), ['list_connections', 'list_repos']);
  assert.equal(profileSummary(groups, full), 'All tools');
  assert.equal(profileSummary(groups, profile('git')), 'Git & PRs · 3 tools');
  assert.equal(profileSummary(groups, profile('full', { all_tools: true, exclude_tools: ['merge_pr'] })), 'All tools except 1');
});

test('the picker preselects the default, multi-selects, and never empties', () => {
  assert.deepEqual(sessionProfileIds({ mcp_profiles: ['database', '', 3, 'git'] }), ['database', 'git']);
  assert.deepEqual(sessionProfileIds({}), []);
  assert.deepEqual(sessionProfileIds(null), []);
  let ids = ['full'];
  ids = toggleProfile(ids, 'database', 'full');
  assert.deepEqual(ids, ['full', 'database']);
  assert.equal(selectionHint(ids, all), 'Full already includes every tool.');
  ids = toggleProfile(ids, 'full', 'full');
  assert.deepEqual(ids, ['database']);
  assert.deepEqual(toggleProfile(ids, 'database', 'full'), ['full'], 'clearing the last falls back to the default');
  assert.equal(selectionHint(['database', 'git'], all), '');
});

test('exactly the default is stored as nothing, so it follows a default change', () => {
  assert.deepEqual(storedProfileIds(['full'], all, 'full'), []);
  assert.deepEqual(storedProfileIds(['database', 'git'], all, 'full'), ['database', 'git']);
  assert.deepEqual(storedProfileIds(['gone', 'git'], all, 'full'), ['git']);
  assert.deepEqual(storedProfileIds(['gone'], all, 'full'), []);
  assert.equal(selectionLabel([], all, 'full'), 'Full (default)');
  assert.equal(selectionLabel(['database', 'git'], all, 'full'), 'Database + Git');
  assert.equal(selectionLabel([], [], 'full'), 'Default');
});

test('the editor validates, duplicates and flips single tools against their groups', () => {
  assert.equal(validateDraft(emptyDraft(), all, null), 'Give the profile a name.');
  assert.match(validateDraft({ ...emptyDraft(), name: 'DATABASE', groups: ['git'] }, all, null), /already exists/);
  assert.equal(validateDraft({ ...draftOf(all[1]), name: 'Database' }, all, 'database'), '', 'its own name is fine');
  assert.equal(validateDraft({ ...emptyDraft(), name: 'x' }, all, null), 'Pick at least one tool group or tool.');
  assert.equal(validateDraft({ ...emptyDraft(), name: 'x', all_tools: true }, all, null), '');
  assert.equal(validateDraft({ ...emptyDraft(), name: 'x'.repeat(61), groups: ['git'] }, all, null), 'Keep the name under 60 characters.');

  assert.equal(duplicateName('Database', all), 'Database copy');
  assert.equal(duplicateName('Database', [...all, profile('dbc', { name: 'Database copy' })]), 'Database copy 2');

  let d = toggleGroup(emptyDraft(), 'git');
  assert.deepEqual(d.groups, ['git']);
  d = toggleTool(groups, d, 'merge_pr');
  assert.deepEqual(d.exclude_tools, ['merge_pr'], 'a grouped tool becomes an exclude');
  d = toggleTool(groups, d, 'otto_merge_pr');
  assert.deepEqual(d.exclude_tools, [], 'flipping back drops the override');
  d = toggleTool(groups, d, 'query_db_readonly');
  assert.deepEqual(d.include_tools, ['query_db_readonly'], 'an outside tool becomes an include');
  assert.deepEqual(toggleGroup(d, 'git').groups, []);
});

test('each profile gets its own outside connection; Full stays the plain one', () => {
  const db = connectionConfigs(all[1], 'http://127.0.0.1:7700/');
  assert.equal(db.label, 'Otto · Database');
  assert.equal(db.httpUrl, 'http://127.0.0.1:7700/api/v1/mcp/http?profile=database');
  const json = JSON.parse(db.stdioJson);
  assert.deepEqual(json.mcpServers['otto-database'].args, ['mcp-server', '--profile', 'database']);
  assert.match(db.codexToml, /\[mcp_servers\.otto_database\]/);
  assert.match(db.claudeHttp, /claude mcp add --transport http otto-database "http:\/\/127\.0\.0\.1:7700\/api\/v1\/mcp\/http\?profile=database"/);
  const plain = connectionConfigs(full, 'http://127.0.0.1:7700');
  assert.equal(plain.httpUrl, 'http://127.0.0.1:7700/api/v1/mcp/http');
  assert.deepEqual(JSON.parse(plain.stdioJson).mcpServers.otto.args, ['mcp-server']);
});
