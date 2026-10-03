import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  findPlaceholder,
  resultDestinations,
  runInputExample,
  runInputSkeleton,
} from '../src/modules/workflows/runInput.ts';

test('Suggest never pre-fills outward or fake values', () => {
  const s = JSON.parse(runInputSkeleton(new Set(['review_run', 'agent'])));
  assert.ok(s.repos, 'repo graphs get a repos slot');
  for (const k of ['result_channel', 'result_chat', 'jira_ticket', 'working_directory']) {
    assert.equal(k in s, false, `${k} is not pre-filled`);
  }
  // Every value in the skeleton is a slot the user must replace.
  assert.equal(findPlaceholder(s), 'repos[0].repo');
});

test('the example is placeholder-only and lists the destinations', () => {
  const ex = JSON.parse(runInputExample(new Set(['product_plan'])));
  assert.ok('story_id' in ex && 'result_chat' in ex);
  assert.ok(findPlaceholder(ex));
});

test('placeholder values are found by path, real values pass', () => {
  assert.equal(findPlaceholder({ result_chat: '<channel id — optional>' }), 'result_chat');
  assert.equal(findPlaceholder({ jira_ticket: 'PROJ-0000' }), 'jira_ticket');
  assert.equal(findPlaceholder({ working_directory: '~/path/to/repo' }), 'working_directory');
  assert.equal(findPlaceholder({ msg: 'fix <b> tags', goals: ['a < b'], n: 3 }), null);
});

test('result destinations mirror the engine', () => {
  assert.deepEqual(resultDestinations({ result_channel: 'slack', result_chat: 'C1' }), ['Slack chat C1']);
  assert.deepEqual(resultDestinations({ result_chat: 'C1' }), [], 'no channel → nothing posted');
  assert.deepEqual(resultDestinations({ result_channel: 'telegram', result_chat: '42', result_thread: '7', result_webhook: 'https://x' }), [
    'Telegram chat 42 (thread 7)',
    'webhook https://x',
  ]);
  assert.deepEqual(resultDestinations(undefined), []);
});
