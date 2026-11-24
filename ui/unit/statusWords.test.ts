// The shared status vocabulary (lib/statusWords.ts): one word + tone per state.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resourceWord, sessionWord, statusWord, wordTone } from '../src/lib/statusWords.ts';

test('every word has exactly one tone; Working and the calm words have none', () => {
  for (const w of ['Approval', 'Question', 'Needs you', 'Limited', 'Needs sign-in']) assert.equal(wordTone(w), 'warning', w);
  for (const w of ['Failed', 'Can’t connect']) assert.equal(wordTone(w), 'danger', w);
  for (const w of ['Done', 'Connected']) assert.equal(wordTone(w), 'success', w);
  for (const w of ['Working', 'Working 18s', 'Idle', 'Stopped', 'Not checked', null, '']) assert.equal(wordTone(w), '', String(w));
});

test('a session row always says a word: needs you, failed, limited, working, done, idle, stopped', () => {
  assert.deepEqual(sessionWord({ key: 'needs-you' }), { word: 'Needs you', tone: 'warning' });
  assert.equal(sessionWord({ key: 'needs-you' }, { kinds: ['approval'] }).word, 'Approval');
  assert.equal(sessionWord({ key: 'idle' }, { kinds: ['question'] }).word, 'Question');
  assert.deepEqual(sessionWord({ key: 'failed', label: 'Failed' }), { word: 'Failed', tone: 'danger' });
  assert.deepEqual(sessionWord({ key: 'failed', label: 'Limited' }), { word: 'Limited', tone: 'warning' });
  assert.equal(sessionWord({ key: 'working' }, { workingSince: 1_000, now: 19_000 }).word, 'Working 18s');
  assert.equal(sessionWord({ key: 'working' }).word, 'Working');
  assert.deepEqual(sessionWord({ key: 'idle' }, { unread: true }), { word: 'Done', tone: 'success' });
  assert.equal(sessionWord({ key: 'idle' }).word, 'Idle');
  assert.equal(sessionWord({ key: 'running' }).word, 'Idle');
  assert.equal(sessionWord({ key: 'suspended' }).word, 'Idle');
  assert.equal(sessionWord({ key: 'stopped' }).word, 'Stopped');
  assert.equal(sessionWord({ key: 'ended' }).word, 'Stopped');
});

test('the Threads sidebar word stays null for a calm row (it shows the time)', () => {
  assert.equal(statusWord('rest'), null);
  assert.equal(statusWord('needs', { kinds: ['question', 'mcp_elicitation'] }), 'Question');
});

test('resources: Connected, Can’t connect, Needs sign-in, anything else Not checked', () => {
  assert.deepEqual(resourceWord('connected'), { word: 'Connected', tone: 'success' });
  assert.deepEqual(resourceWord('OK'), { word: 'Connected', tone: 'success' });
  assert.deepEqual(resourceWord('unreachable'), { word: 'Can’t connect', tone: 'danger' });
  assert.deepEqual(resourceWord('unauthorized'), { word: 'Needs sign-in', tone: 'warning' });
  for (const raw of ['unknown', '', null, undefined, 'weird']) assert.deepEqual(resourceWord(raw), { word: 'Not checked', tone: '' });
});
