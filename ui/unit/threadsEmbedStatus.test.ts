// What an embedded thread reports to its host (`onstatus`,
// lib/runtimeThreadPort.ts): one mapping from what the view knows.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { embedStatus } from '../src/modules/threads/runtime/embedStatus.ts';

const base = { status: 'idle' as const, backgroundTasks: 0, turnRunning: false, pendingRequests: 0 };

test('idle when nothing runs or waits (and before the row loads)', () => {
  assert.equal(embedStatus(base), 'idle');
  assert.equal(embedStatus({ ...base, status: undefined }), 'idle');
  assert.equal(embedStatus({ ...base, status: 'dormant' }), 'idle');
});

test('working: a running turn, or a working or starting session', () => {
  assert.equal(embedStatus({ ...base, turnRunning: true }), 'working');
  assert.equal(embedStatus({ ...base, status: 'working' }), 'working');
  assert.equal(embedStatus({ ...base, status: 'starting' }), 'working');
  // A turn that runs while background work runs is still working.
  assert.equal(embedStatus({ ...base, turnRunning: true, backgroundTasks: 2 }), 'working');
});

test('background: an idle agent with only background work left is calm, not working', () => {
  assert.equal(embedStatus({ ...base, backgroundTasks: 2 }), 'background');
  assert.equal(embedStatus({ ...base, status: 'awaiting_permission', backgroundTasks: 1 }), 'needs-you');
  assert.equal(embedStatus({ ...base, status: 'closed', backgroundTasks: 1 }), 'closed');
});

test('needs-you outranks working: a pending approval or question, or awaiting permission', () => {
  assert.equal(embedStatus({ ...base, turnRunning: true, pendingRequests: 1 }), 'needs-you');
  assert.equal(embedStatus({ ...base, status: 'awaiting_permission' }), 'needs-you');
});

test('closed outranks everything', () => {
  assert.equal(embedStatus({ ...base, status: 'closed', turnRunning: true, pendingRequests: 3 }), 'closed');
});
