// SA-04 lazy half: a live delta's elided tool result is completed on expand
// through `GET …/transcript/tool/{id}` (cached, failures retried).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function load(get: (path: string) => Promise<unknown>) {
  return loadSource(new URL('../src/modules/agents/conversation/toolDetail.ts', import.meta.url), {
    '../../../lib/api/client': { api: { get } },
  });
}

const result = (text: string, extra: Record<string, unknown> = {}) => ({
  ok: true, text, truncated: false, bytes: 90_000, image_ids: [], patch: null, file_path: null, ...extra,
});
const block = (r: unknown) => ({ kind: 'tool_call', id: 't1', name: 'Bash', tool: 'shell', title: 'x', input: {}, result: r });

test('effectiveResult: full result only while the block is still an elided preview', () => {
  const m = load(async () => ({}));
  const preview = block(result('abc', { elided: true }));
  const full = result('abc…everything');
  assert.equal(m.isElided(preview), true);
  assert.equal(m.effectiveResult(preview, null), preview.result, 'not loaded yet → preview');
  assert.equal(m.effectiveResult(preview, full), full, 'loaded → full');
  const whole = block(result('whole from a later delta'));
  assert.equal(m.isElided(whole), false);
  assert.equal(m.effectiveResult(whole, full), whole.result, 'a non-elided block wins over a stale fetch');
  assert.equal(m.isElided(block(null)), false);
});

test('fetchToolResult hits the tool route once per step and retries after a failure', async () => {
  const calls: string[] = [];
  let fail = true;
  const m = load(async (path: string) => {
    calls.push(path);
    if (fail) throw new Error('503');
    return block(result('FULL'));
  });
  await assert.rejects(m.fetchToolResult('s 1', 'toolu/9'));
  fail = false;
  const r = await m.fetchToolResult('s 1', 'toolu/9');
  assert.equal(r.text, 'FULL');
  await m.fetchToolResult('s 1', 'toolu/9');
  assert.deepEqual(calls, [
    '/sessions/s%201/transcript/tool/toolu%2F9',
    '/sessions/s%201/transcript/tool/toolu%2F9',
  ], 'failure not cached; success cached; ids URL-encoded');
});
