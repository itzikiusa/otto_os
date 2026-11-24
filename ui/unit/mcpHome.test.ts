import { test } from 'node:test';
import assert from 'node:assert/strict';
import { argsLine, pendingOf, recentDenials, summarySentences } from '../src/modules/mcp/mcpHome.ts';
import { catalogRows, firstSentence, toolState } from '../src/modules/mcp/toolCatalog.ts';

const tool = (name: string, category: string, enabled = true, mutating = false, extra = {}) => ({ name, category, enabled, mutating, description: `Does ${name}. Then more detail.`, ...extra });
const CATALOG = [tool('otto.git_status', 'Git'), tool('otto.git_push', 'Git', true, true), tool('otto.git_delete', 'Git', false, true), tool('otto.run_workflow', 'Workflows', true, true, { approval_exempt: true })];

test('the catalog opens collapsed: one row per category with its count', () => {
  const rows = catalogRows(CATALOG, new Set(), '');
  assert.deepEqual(rows.map((r) => (r.kind === 'cat' ? `${r.cat} ${r.on}/${r.total}` : r.key)), ['Git 2/3', 'Workflows 1/1']);
});

test('an open category lists its tools; a search opens every category it matches', () => {
  assert.deepEqual(catalogRows(CATALOG, new Set(['Git']), '').map((r) => r.key), ['cat:Git', 'tool:otto.git_status', 'tool:otto.git_push', 'tool:otto.git_delete', 'cat:Workflows']);
  const found = catalogRows(CATALOG, new Set(), 'push');
  assert.deepEqual(found.map((r) => r.key), ['cat:Git', 'tool:otto.git_push']);
  assert.equal(found[0].kind === 'cat' && found[0].total, 3, 'counts stay the category’s');
});

test('each tool says its state in words; descriptions show their first sentence', () => {
  assert.deepEqual(CATALOG.map((t) => toolState(t, true)), ['Allowed', 'Asks first', 'Off', 'Runs without asking']);
  assert.equal(toolState(CATALOG[1], false), 'Runs without asking');
  assert.equal(firstSentence('Search code for a literal query. Returns file:line matches.'), 'Search code for a literal query.');
  assert.equal(firstSentence('No full stop here\nsecond line'), 'No full stop here');
});

const NOW = Date.parse('2026-11-24T10:00:00Z');
const appr = (status: string, decidedDaysAgo: number | null = null) => ({
  kind: 'tool_call', tool: 'otto.git_push', title: 'Push', args_redacted_json: '{}', status,
  decided_at: decidedDaysAgo === null ? null : new Date(NOW - decidedDaysAgo * 86_400_000).toISOString(), expires_at: null,
});

test('a request says what it will do from its redacted arguments', () => {
  assert.equal(argsLine({ tool: 'otto.git_push', args_redacted_json: JSON.stringify({ repo: 'otto', branch: 'fx/x', force: false, files: ['a'], body: 'x'.repeat(80) }) }), 'otto.git_push · repo otto · branch fx/x · force false');
  assert.equal(argsLine({ tool: 'otto.create_pr', args_redacted_json: 'not json' }), 'otto.create_pr');
});

test('pending first; denials counted for the last 7 days only', () => {
  const list = [appr('pending'), appr('denied', 1), appr('denied', 9), appr('approved', 0)];
  assert.equal(pendingOf(list).length, 1);
  assert.equal(recentDenials(list, NOW), 1);
});

test('four sentences, one link each, in words', () => {
  const s = summarySentences({
    servers: [{ health_status: 'healthy' }, { health_status: 'unknown' }],
    tools: [
      { enabled: true, mutating: true },
      { enabled: true, mutating: false },
      { enabled: false, mutating: true },
    ],
    rules: 0,
    approvalsOn: true,
    approvals: [appr('denied', 2), appr('denied', 3)],
    now: NOW,
  });
  assert.deepEqual(s.map((x) => x.text), [
    '2 external servers: 1 connected, 1 not checked.',
    'Agents may use 2 of Otto’s 3 tools; 1 of them can change things.',
    'Every call that can change things asks a person first.',
    '2 calls denied in the last 7 days.',
  ]);
  assert.deepEqual(s.map((x) => x.to), ['mcp/servers', 'mcp/otto', 'mcp/otto', 'mcp/activity']);
});

test('what is still loading is left out; rules and an approvals-off switch read plainly', () => {
  const s = summarySentences({ servers: null, tools: null, rules: 3, approvalsOn: true, approvals: [], now: NOW });
  assert.deepEqual(s.map((x) => x.text), [
    'Calls that can change things ask a person first; 3 auto-approve rules let some run without asking.',
    'No calls were denied in the last 7 days.',
  ]);
  const off = summarySentences({ servers: [], tools: null, rules: 0, approvalsOn: false, approvals: [], now: NOW });
  assert.equal(off[0].text, 'No external MCP servers yet.');
  assert.match(off[1].text, /turned off/);
});
