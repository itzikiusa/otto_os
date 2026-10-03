// Agents-page session list (perf section 13, F1/F2): the one-pass sidebar
// bucketing, the shown-list query, `?ids=` chunking, and the UI mirror of the
// daemon's background-source list.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  BACKGROUND_SOURCES,
  SCRATCH_ID,
  bucketSessions,
  idChunks,
  isForeground,
  isShownKind,
  shownListQuery,
} from '../src/lib/stores/sessionBuckets.ts';
import { SCRATCH_WORKSPACE_ID } from '../src/lib/stores/sessionScope.ts';
import type { Session } from '../src/lib/api/types.ts';

let n = 0;
function mk(p: Partial<Session> & { meta?: Record<string, unknown> }): Session {
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
    created_at: `2026-01-01T00:00:${String(n % 60).padStart(2, '0')}Z`,
    last_active_at: `2026-01-01T00:${String(n % 60).padStart(2, '0')}:00Z`,
    archived: false,
    meta: {},
    ...p,
  } as Session;
}

test('BACKGROUND_SOURCES mirrors the Rust BACKGROUND_SESSION_SOURCES exactly', () => {
  const rs = readFileSync(new URL('../../crates/otto-core/src/domain.rs', import.meta.url), 'utf8');
  const m = /pub const BACKGROUND_SESSION_SOURCES: \[&str; \d+\] = \[([\s\S]*?)\];/.exec(rs);
  assert.ok(m, 'array found');
  const rust = [...m![1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
  assert.deepEqual([...BACKGROUND_SOURCES], rust);
});

test('the inlined scratch id matches sessionScope', () => {
  assert.equal(SCRATCH_ID, SCRATCH_WORKSPACE_ID);
});

test('isForeground / isShownKind follow the daemon rule', () => {
  assert.equal(isForeground(mk({})), true);
  assert.equal(isForeground(mk({ meta: { source: 7 } })), true, 'non-string source is foreground');
  assert.equal(isForeground(mk({ meta: { source: 'review' } })), false);
  assert.equal(isShownKind(mk({ meta: { source: 'channel' } })), true, 'channel tickets are listed');
  assert.equal(isShownKind(mk({ meta: { source: 'review' } })), false);
  assert.equal(isShownKind(mk({ kind: 'connection', meta: { source: 'review' } })), true);
});

test('bucketSessions: one pass reproduces every sidebar list', () => {
  const fg = mk({});
  const review = mk({ meta: { source: 'review' } });
  const arch = mk({ archived: true });
  const conn = mk({ kind: 'connection' });
  const scratchFg = mk({ workspace_id: SCRATCH_WORKSPACE_ID });
  const scratchBg = mk({ workspace_id: SCRATCH_WORKSPACE_ID, meta: { source: 'insights' } });
  const tgOld = mk({ meta: { source: 'channel', channel: 'telegram' }, last_active_at: '2026-01-01T00:00:00Z' });
  const tgNew = mk({ meta: { source: 'channel', channel: 'telegram' }, last_active_at: '2026-02-01T00:00:00Z' });
  const sl = mk({ meta: { source: 'channel', channel: 'slack' } });
  const all = [fg, review, arch, conn, scratchFg, scratchBg, tgOld, tgNew, sl];
  const b = bucketSessions(all);
  // The legacy per-list filters, verbatim, as the oracle.
  const notArch = all.filter((s) => !s.archived);
  assert.deepEqual(b.active, notArch);
  assert.deepEqual(b.foregroundActive, notArch.filter(isForeground));
  const agent = all.filter((s) => !s.archived && s.kind === 'agent' && s.workspace_id !== SCRATCH_WORKSPACE_ID);
  assert.deepEqual(b.agent, agent);
  assert.deepEqual(b.plainAgent, agent.filter(isForeground));
  assert.deepEqual(
    b.scratch,
    all.filter((s) => !s.archived && s.kind === 'agent' && s.workspace_id === SCRATCH_WORKSPACE_ID && isForeground(s)),
  );
  assert.deepEqual(b.connection, [conn]);
  assert.deepEqual(b.telegram, [tgNew, tgOld], 'newest first');
  assert.deepEqual(b.slack, [sl]);
});

test('bucketSessions on 2,360 rows stays well under a frame', () => {
  const rows: Session[] = [];
  for (let i = 0; i < 2000; i++) rows.push(mk({ meta: { source: 'review' } }));
  for (let i = 0; i < 300; i++) rows.push(mk({ archived: true }));
  for (let i = 0; i < 60; i++) rows.push(mk({}));
  bucketSessions(rows); // warm
  const t0 = performance.now();
  for (let i = 0; i < 20; i++) bucketSessions(rows);
  const per = (performance.now() - t0) / 20;
  assert.ok(per < 8, `bucketSessions took ${per.toFixed(2)} ms per pass`);
});

test('shownListQuery always asks for the foreground rows + channel tickets', () => {
  assert.equal(shownListQuery(), '?archived=false&foreground=true&with_sources=channel');
  assert.equal(
    shownListQuery(['swarm', 'channel', 'canvas_assist']),
    '?archived=false&foreground=true&with_sources=channel,canvas_assist,swarm',
  );
});

test('idChunks splits at the daemon cap of 64', () => {
  const ids = Array.from({ length: 130 }, (_, i) => `id${i}`);
  const chunks = idChunks(ids);
  assert.deepEqual(chunks.map((c) => c.length), [64, 64, 2]);
  assert.deepEqual(chunks.flat(), ids);
  assert.deepEqual(idChunks([]), []);
});
