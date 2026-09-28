// K8s fleet live refresh keeps "Load more" pages (r3-02-04).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { liveRefreshLimit } from '../src/modules/kubernetes/monitor/monitor-util.ts';

test('a live refresh re-reads every paged-in row', () => {
  assert.equal(liveRefreshLimit(true, 600, 200, 2000, false), 600);
  assert.equal(liveRefreshLimit(true, 0, 200, 2000, false), 200);
  assert.equal(liveRefreshLimit(true, 150, 200, 2000, false), 200);
});

test('a live refresh never aborts a Load more, nor truncates past the server cap', () => {
  assert.equal(liveRefreshLimit(true, 600, 200, 2000, true), null);
  assert.equal(liveRefreshLimit(true, 2200, 200, 2000, false), null);
  assert.equal(liveRefreshLimit(true, 2000, 200, 2000, false), 2000);
});

test('user-driven loads and appends are one page', () => {
  assert.equal(liveRefreshLimit(false, 600, 200, 2000, false), 200);
  assert.equal(liveRefreshLimit(false, 600, 200, 2000, true), 200);
});
