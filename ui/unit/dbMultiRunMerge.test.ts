import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mergeMultiRunJob, type DbMultiRunJob } from '../src/lib/api/db-multirun-types.ts';

const item = (index: number, status: string) =>
  ({ index, target: 0, label: `r${index}`, values: [], statement_preview: '', is_write: false, status, has_result: false }) as unknown as DbMultiRunJob['items'][number];
const job = (over: Partial<DbMultiRunJob>): DbMultiRunJob =>
  ({
    id: 'j',
    status: 'running',
    engine: 'mysql',
    created_at: '',
    concurrency: 1,
    stop_on_error: true,
    read_only: false,
    statement_preview: '',
    summary: { total: 3, ok: 0, failed: 0, running: 0, pending: 3, skipped: 0, cancelled: 0 },
    targets: [{ index: 0 } as unknown as DbMultiRunJob['targets'][number]],
    items: [item(0, 'pending'), item(1, 'pending'), item(2, 'pending')],
    seq: 0,
    ...over,
  }) as DbMultiRunJob;

test('a partial answer replaces only its runs and keeps the targets', () => {
  const prev = job({});
  const next = job({ seq: 3, partial: true, targets: [], items: [item(1, 'ok')], status: 'running' });
  const m = mergeMultiRunJob(prev, next);
  assert.equal(m.seq, 3);
  assert.equal(m.partial, false);
  assert.equal(m.targets, prev.targets);
  assert.deepEqual(m.items.map((i) => i.status), ['pending', 'ok', 'pending']);
  assert.equal(m.items[0], prev.items[0], 'untouched runs keep their objects');
  assert.deepEqual(prev.items.map((i) => i.status), ['pending', 'pending', 'pending'], 'prev not mutated');
});

test('a full answer, another job or no previous job replaces everything', () => {
  const prev = job({});
  const full = job({ seq: 9, items: [item(0, 'ok')] });
  assert.equal(mergeMultiRunJob(prev, full), full);
  const other = job({ id: 'k', partial: true, items: [] });
  assert.equal(mergeMultiRunJob(prev, other), other);
  assert.equal(mergeMultiRunJob(null, other), other);
});
