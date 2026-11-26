// The start-a-run form (modules/workflows/runs/startForm.ts): fields from the
// Start step's inputs or the chat keys, the payload (empty optionals left
// out, goals as a list, the message also the prompt — what a chat message
// with the same values gives the run), Run again's prefill (this page, not
// the old thread, unless picked), and JSON both ways.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildInput, chatOrigin, DEFAULT_FIELDS, fromJson, prefill, startFields, startProblem } from '../src/modules/workflows/runs/startForm.ts';

/** What the chat trigger puts in a run's input (crates/otto-workflows/src/chat.rs). */
const chatInput = {
  trigger: 'chat', origin_workspace_id: 'w1', channel: 'slack', chat: 'C0123', thread: '1794.2', user: 'U9', name: 'PR Reviewer',
  msg: 'Check the cap', prompt: 'Check the cap', jira_ticket: 'PAY-907', working_directory: null, base: 'main',
  pr: 'payments-api #412', pr_branch: null, relevant_info: [], goals: ['Cap per session', 'No N+1'], raw: 'Action: Workflow …',
};

test('fields: the declared inputs, else the chat keys in the chat message’s order', () => {
  assert.deepEqual(startFields({}).map((f) => f.key), ['pr', 'jira_ticket', 'pr_branch', 'base', 'goals', 'msg']);
  assert.deepEqual(startFields({ working_directory: '~/src/app' }).at(-1), { key: 'working_directory', label: 'Working folder', placeholder: '~/src/app' });
  const declared = startFields({ inputs: [{ key: 'ticket', label: 'Ticket', required: true }, { key: 'notes', multiline: true }, { key: 'Bad Key' }] });
  assert.deepEqual(declared, [{ key: 'ticket', label: 'Ticket', required: true }, { key: 'notes', label: 'notes', multiline: true }]);
  const goals = startFields({ inputs: [{ key: 'goals', label: 'Goals', multiline: true }] });
  assert.deepEqual(buildInput(goals, { values: { goals: 'a\nb' }, extra: {} }), { goals: ['a', 'b'] }, 'a declared goals input is a list too');
});

test('payload: empty optionals left out, goals one per line → a list, the message is also the prompt', () => {
  const input = buildInput(DEFAULT_FIELDS, { values: { pr: ' #412 ', jira_ticket: '', goals: 'Cap per session\n\n No N+1 ', msg: 'Check the cap' }, extra: {} });
  assert.deepEqual(input, { pr: '#412', goals: ['Cap per session', 'No N+1'], msg: 'Check the cap', prompt: 'Check the cap' });
  // The keys and values a chat message with the same fields gives the run.
  const fromChat = Object.fromEntries(Object.entries(chatInput).filter(([k, v]) => ['pr', 'jira_ticket', 'base', 'goals', 'msg', 'prompt'].includes(k) && v !== null));
  const same = buildInput(DEFAULT_FIELDS, { values: { pr: 'payments-api #412', jira_ticket: 'PAY-907', base: 'main', goals: 'Cap per session\nNo N+1', msg: 'Check the cap' }, extra: {} });
  assert.deepEqual(same, fromChat);
});

test('Run again: the earlier input fills the form; who/where/raw and the result routing are dropped', () => {
  const s = prefill(DEFAULT_FIELDS, chatInput);
  assert.deepEqual(s.values, { msg: 'Check the cap', jira_ticket: 'PAY-907', base: 'main', pr: 'payments-api #412', goals: 'Cap per session\nNo N+1' });
  assert.deepEqual(s.extra, { relevant_info: [] }, 'keys without a field stay (in JSON)');
  const again = buildInput(DEFAULT_FIELDS, s);
  for (const k of ['channel', 'chat', 'thread', 'trigger', 'result_chat']) assert.ok(!(k in again), `${k} is not sent: the result goes to this page`);
});

test('the original thread is offered from the run’s input and, when picked, routes the result there', () => {
  const origin = chatOrigin(chatInput);
  assert.deepEqual(origin, { channel: 'slack', chat: 'C0123', thread: '1794.2' });
  assert.equal(chatOrigin({ msg: 'manual' }), null);
  assert.deepEqual(chatOrigin({ result_channel: 'telegram', result_chat: '-100', thread: 'x' }), { channel: 'telegram', chat: '-100' });
  const input = buildInput(DEFAULT_FIELDS, { values: { pr: '#5' }, extra: {} }, origin);
  assert.deepEqual(input, { pr: '#5', result_channel: 'slack', result_chat: 'C0123', result_thread: '1794.2' });
});

test('JSON both ways: fields and extra keys round-trip; bad JSON is an error, not a payload', () => {
  const state = { values: { pr: '#7', goals: 'a\nb' }, extra: { repos: [{ repo: 'app' }] } };
  const text = JSON.stringify(buildInput(DEFAULT_FIELDS, state), null, 2);
  const back = fromJson(DEFAULT_FIELDS, text);
  assert.deepEqual(back, state);
  assert.deepEqual(buildInput(DEFAULT_FIELDS, back as typeof state), JSON.parse(text));
  assert.match((fromJson(DEFAULT_FIELDS, '{"pr":') as { error: string }).error, /isn’t valid JSON/);
  assert.match((fromJson(DEFAULT_FIELDS, '[1]') as { error: string }).error, /JSON object/);
  assert.deepEqual(fromJson(DEFAULT_FIELDS, '  '), { values: {}, extra: {} });
});

test('Run is held for a required field or a template value', () => {
  const fields = startFields({ inputs: [{ key: 'ticket', label: 'Ticket', required: true }] });
  assert.equal(startProblem(fields, { values: {}, extra: {} }, {}), 'Fill in Ticket.');
  const state = { values: { ticket: 'X-1' }, extra: { repo: '<repo id>' } };
  assert.match(startProblem(fields, state, buildInput(fields, state))!, /placeholder in “repo”/);
  assert.equal(startProblem(fields, { values: { ticket: 'X-1' }, extra: {} }, { ticket: 'X-1' }), null);
});
