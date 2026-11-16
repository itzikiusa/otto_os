// A scheduled task's run row (A11/A12): reasons without the daemon's error-kind
// prefix, summaries as plain text, and a running run that says so.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { runReason, runSummary } from '../src/modules/scheduled-tasks/runRow.ts';

test('a run reason drops the error-kind prefix', () => {
  assert.equal(runReason('internal: shell command exited with 3 after 1 attempt(s)'), 'shell command exited with 3 after 1 attempt(s)');
  assert.equal(runReason('conflict: a run of this task is already in progress'), 'a run of this task is already in progress');
});

test('a run summary is plain text, and a running run reads Running…', () => {
  assert.equal(runSummary({ status: 'ok', summary: '# Shell echo\nShell command exited with status `0`.' }), 'Shell command exited with status 0.');
  assert.equal(runSummary({ status: 'running', summary: '' }), 'Running…');
  assert.equal(runSummary({ status: 'ok', summary: '' }), 'No summary');
});
