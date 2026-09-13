import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildTriggerSpec, defaultTriggerForm, EVENT_KINDS, formFromTrigger } from '../src/modules/workflows/triggerForm.ts';
import { effectiveRetry, updateRetry, clearRetry } from '../src/modules/workflows/retryPolicy.ts';

test('explicit zero survives save while use-default retains agent retry policy', () => {
  const node = { id: 'agent', kind: 'agent_prompt', name: '', x: 0, y: 0, params: {}, retry: null };
  assert.equal(effectiveRetry(node).max_attempts, 2);
  const saved = JSON.parse(JSON.stringify(updateRetry(node, 'max_attempts', 0)));
  assert.equal(saved.max_attempts, 0);
  assert.equal(effectiveRetry({ ...node, retry: saved }).max_attempts, 0);
});

test('default event uses a supported canonical identifier', () => {
  const form = { ...defaultTriggerForm(), kind: 'event' as const };
  assert.equal(buildTriggerSpec(form).event_kind, 'review_changed');
  assert.ok(EVENT_KINDS.some(([kind]) => kind === form.eventKind));
});
test('editing cron destinations preserves cursor and extension settings', () => {
  const trigger = { id: 't', workflow_id: 'wf', kind: 'schedule' as const, enabled: true, created_at: '',
    spec: { cadence: 'cron', expr: '30 8 * * 1', timezone: 'Asia/Jerusalem', last_run: 'cursor', custom: 42,
      result_channel: 'slack', result_chat: 'C123', result_thread: '123.456' } };
  const form = formFromTrigger(trigger);
  form.cron = '30 10 * * 1';
  const saved = buildTriggerSpec(form, trigger.spec);
  assert.equal(saved.expr, '30 10 * * 1');
  for (const key of ['timezone', 'last_run', 'custom', 'result_channel', 'result_chat', 'result_thread'] as const) {
    assert.equal(saved[key], trigger.spec[key]);
  }
});
test('event filters and cleared destinations round trip', () => {
  const form = { ...defaultTriggerForm(), kind: 'event' as const, filter: '{"status":"done"}' };
  assert.deepEqual(buildTriggerSpec(form).filter_json, { status: 'done' });
  assert.equal(buildTriggerSpec(form, { result_webhook: 'https://old.example' }).result_webhook, '');
  assert.throws(() => buildTriggerSpec({ ...form, filter: '[]' }), /JSON object/);
});

test('use default clears legacy nested policies without removing step parameters', () => {
  const node = { id: 'agent', kind: 'agent_prompt', name: '', x: 0, y: 0, params: { prompt: 'keep', retry: { max_attempts: 0 } }, retry: null };
  clearRetry(node);
  assert.deepEqual(node.params, { prompt: 'keep' });
  assert.equal(effectiveRetry(node).max_attempts, 2);
});
