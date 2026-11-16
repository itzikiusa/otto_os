// The "Agents N" badge counts exactly the green rows the Agents section
// renders (review D-02: badge 1 next to three green dots).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { bucketSessions, visibleWorkingCount } from '../src/lib/stores/sessionBuckets.ts';
import type { Session } from '../src/lib/api/types.ts';

let n = 0;
function mk(p: Partial<Session>): Session {
  n++;
  return {
    id: `s${n}`,
    workspace_id: 'w1',
    kind: 'agent',
    provider: 'claude',
    title: `t${n}`,
    status: 'idle',
    cwd: '/tmp',
    provider_session_id: null,
    connection_id: null,
    created_by: 'u',
    created_at: '2026-11-16T00:00:00Z',
    last_active_at: '2026-11-16T00:00:00Z',
    archived: false,
    meta: {},
    ...p,
  } as Session;
}

test('other-workspace rows count while the all-workspaces view lists them', () => {
  const here = mk({ status: 'working' });
  const otherA = mk({ workspace_id: 'w2', status: 'idle' });
  const otherB = mk({ workspace_id: 'w3', status: 'idle' });
  const b = bucketSessions([here]);
  const statusMap = { [otherA.id]: 'working', [otherB.id]: 'working' };
  const listed = [b.plainAgent, b.scratch, [otherA], [otherB]];
  assert.equal(visibleWorkingCount(listed, statusMap, {}), 3, 'three green rows, badge 3');
  // Toggle off: those rows are not rendered, so they are not counted.
  assert.equal(visibleWorkingCount([b.plainAgent, b.scratch], statusMap, {}), 1);
});

test('connection terminals and background sessions never count', () => {
  const ssh = mk({ kind: 'connection', status: 'working' });
  const review = mk({ status: 'working', meta: { source: 'review' } });
  const agent = mk({ status: 'idle' });
  const b = bucketSessions([ssh, review, agent]);
  assert.equal(b.foregroundActive.includes(ssh), true, 'the old badge scope held it');
  assert.equal(visibleWorkingCount([b.plainAgent, b.scratch], {}, {}), 0);
});

test('the live status wins over the row, and a needs-you dot is not green', () => {
  const a = mk({ status: 'working' });
  const b = mk({ status: 'idle' });
  const c = mk({ status: 'idle' });
  const statusMap = { [a.id]: 'idle', [b.id]: 'working', [c.id]: 'working' };
  assert.equal(visibleWorkingCount([[a, b, c]], statusMap, { [c.id]: true }), 1);
});

test('the scratch group counts and a row listed twice counts once', () => {
  const s = mk({ workspace_id: 'scratch', status: 'working' });
  const b = bucketSessions([s]);
  assert.deepEqual(b.scratch, [s]);
  assert.equal(visibleWorkingCount([b.plainAgent, b.scratch, [s]], {}, {}), 1);
});

test('the store badge is derived from the rendered groups', () => {
  const src = readFileSync(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), 'utf8');
  const decl = src.slice(src.indexOf('workingCount: number'), src.indexOf('needsYouCount: number'));
  assert.match(decl, /visibleWorkingCount\(\[this\.buckets\.plainAgent, this\.buckets\.scratch/);
  assert.match(decl, /this\.allWorkspaces \? this\.otherWsGroups/);
  assert.doesNotMatch(decl, /foregroundActive/);
});

