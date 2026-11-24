// A thread lists every MCP server its agent has, with a status word and,
// when it is not connected, why — nothing missing silently.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mcpChanged, mcpServerRows, mcpSummary } from '../src/modules/threads/runtime/mcpServers.ts';
import { nativeServerNames } from '../src/modules/threads/runtime/nativeServers.ts';

const s = (name: string, source: string, state: string, reason?: string) => ({ name, source, state, reason }) as never;

test('problems first, each with its word and reason; then Otto, the workspace, the CLI', () => {
  const rows = mcpServerRows(
    [
      s('otto', 'otto', 'connected'),
      s('jira', 'workspace', 'added'),
      s('probe', 'workspace', 'unsupported', 'Gemini can’t take it: its command “probe” isn’t on the daemon’s PATH'),
      s('own-docs', 'cli', 'needs_auth', 'Sign in with `claude mcp`'),
      s('github', 'cli', 'connected'),
    ],
    [{ name: 'linear', health_status: 'healthy', health_error: null, tools: 2 }, { name: 'down', health_status: 'unhealthy', health_error: 'connection refused', tools: 1 }],
    'Gemini',
  );
  assert.deepEqual(rows.map((r) => [r.name, r.status]), [
    ['probe', 'Not supported by Gemini'],
    ['down', 'Failed'],
    ['own-docs', 'Needs sign-in'],
    ['otto', 'Connected'],
    ['jira', 'Added'],
    ['linear', 'Connected'],
    ['github', 'Connected'],
  ]);
  assert.equal(rows.find((r) => r.name === 'jira')!.reason, 'Gemini doesn’t report whether it connected');
  assert.equal(rows.find((r) => r.name === 'linear')!.from, 'Through Otto · 2 tools');
  assert.equal(rows.find((r) => r.name === 'github')!.from, 'Gemini’s own config');
  assert.equal(rows.find((r) => r.name === 'down')!.reason, 'connection refused');
  assert.equal(mcpSummary(rows), 'MCP · 7 servers · 3 need attention');
});

test('a governed server says so when the thread has no Otto tools', () => {
  const rows = mcpServerRows([s('otto', 'otto', 'disabled', 'Otto’s tools are off for this workspace')], [{ name: 'linear', health_status: 'unknown', health_error: null, tools: 1 }], 'Claude');
  const linear = rows.find((r) => r.name === 'linear')!;
  assert.equal(linear.status, 'Not available');
  assert.match(linear.reason!, /through Otto’s tools/);
  assert.equal(mcpSummary([]), 'MCP · no servers');
});

test('"MCP servers changed" only when an enabled server was added or removed since the start', () => {
  const spawn = [s('otto', 'otto', 'connected'), s('jira', 'workspace', 'added'), s('own', 'cli', 'connected')];
  assert.equal(mcpChanged(spawn, ['jira']), false);
  assert.equal(mcpChanged(spawn, ['jira', 'linear']), true);
  assert.equal(mcpChanged(spawn, []), true);
  assert.equal(mcpChanged([], ['jira']), false, 'nothing to compare before the first start');
});

test('the workspace list is read once for every open thread within 15 s', async () => {
  let reads = 0;
  const get = async () => { reads += 1; return [{ name: 'a', enabled: true }, { name: 'b', enabled: false }]; };
  assert.deepEqual(await nativeServerNames('ws-cache', get, 1_000), ['a']);
  assert.deepEqual(await nativeServerNames('ws-cache', get, 2_000), ['a']);
  assert.equal(reads, 1);
  await nativeServerNames('ws-cache', get, 20_000);
  assert.equal(reads, 2);
});
