// api-contracts-5-02: the reset dry-run's `warning` (group has live consumers)
// reaches the UI and blocks Reset while it holds; a clean preview, no preview
// or a stale preview for another group does not.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resetWarning } from '../src/modules/brokers/resetWarning.ts';
import type { DryRunResp } from '../src/modules/brokers/types.ts';

const preview = (warning?: string, group = 'g'): DryRunResp => ({
  group,
  partitions: [],
  total_lag_before: 10,
  total_lag_after: 0,
  ...(warning === undefined ? {} : { warning }),
});

test('a dry-run warning for the selected group blocks the reset', () => {
  const text = 'consumer group g is Stable; stop its consumers first';
  assert.equal(resetWarning(preview(text), 'g'), text);
});

test('no warning, a blank one, no preview or another group never blocks', () => {
  assert.equal(resetWarning(preview(), 'g'), null);
  assert.equal(resetWarning(preview('  '), 'g'), null);
  assert.equal(resetWarning(null, 'g'), null);
  assert.equal(resetWarning(preview('busy', 'other'), 'g'), null);
});
