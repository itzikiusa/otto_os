// Shared status vocabulary (lib/status.ts): sessions, runs, environments.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { envTone, isResumable, runStatus, sentenceCase, sessionState, storyStage } from '../src/lib/status.ts';

test('storyStage: product stages map to one label/tone; done is success, not accent', () => {
  assert.deepEqual([storyStage('draft').label, storyStage('draft').tone], ['Draft', 'neutral']);
  assert.equal(storyStage('review').tone, 'warning');
  assert.equal(storyStage('approved').tone, 'success');
  assert.deepEqual([storyStage('done').label, storyStage('done').tone], ['Done', 'success']);
  assert.deepEqual([storyStage('tests_drafted').label, storyStage('planned').tone], ['Tests drafted', 'info']);
  const odd = storyStage('in-flight');
  assert.deepEqual([odd.key, odd.label, odd.tone], ['in_flight', 'In flight', 'neutral']);
  assert.equal(storyStage(null).key, 'unknown');
});

const agent = { kind: 'agent', provider_session_id: 'psid' };
const shell = { kind: 'shell', provider_session_id: null };

test('sessionState: live states map to one label/tone', () => {
  assert.deepEqual(
    [sessionState(agent, 'working').key, sessionState(agent, 'working').tone, sessionState(agent, 'working').live],
    ['working', 'success', true],
  );
  assert.equal(sessionState(agent, 'running').label, 'Running');
  assert.equal(sessionState(agent, 'idle').label, 'Idle');
  assert.equal(sessionState(null, undefined).key, 'idle');
  // Falls back to the row's own status when there is no live status.
  assert.equal(sessionState({ ...agent, status: 'working' }, undefined).key, 'working');
});

test('sessionState: needs-you is amber and wins over live states only', () => {
  const s = sessionState(agent, 'idle', true);
  assert.equal(s.key, 'needs-you');
  assert.equal(s.label, 'Needs you');
  assert.equal(s.tone, 'warning');
  assert.equal(sessionState(agent, 'working', true).key, 'needs-you');
  // No process left → suspended/ended, not "needs you".
  assert.equal(sessionState(agent, 'reconnectable', true).key, 'suspended');
});

test('sessionState: resumable exits are suspended, never amber; plain exits end', () => {
  const sus = sessionState(agent, 'exited');
  assert.equal(sus.key, 'suspended');
  assert.equal(sus.tone, 'neutral');
  assert.equal(sus.hint, 'Suspended — resumes on open');
  assert.ok(sus.resumable && sus.inactive);
  assert.equal(sessionState(shell, 'reconnectable').key, 'suspended');
  const ended = sessionState(shell, 'exited');
  assert.equal(ended.key, 'ended');
  assert.equal(ended.tone, 'neutral');
  assert.ok(!ended.resumable && ended.inactive);
});

test('sessionState: non-zero exit code is failed; zero follows resumability', () => {
  const f = sessionState(shell, 'exited', false, { exitCode: 2 });
  assert.equal(f.key, 'failed');
  assert.equal(f.tone, 'danger');
  assert.equal(f.label, 'Failed (exit 2)');
  assert.equal(sessionState(shell, 'exited', false, { exitCode: 0 }).key, 'ended');
  assert.equal(sessionState(agent, 'exited', false, { exitCode: 0 }).key, 'suspended');
});

test('sessionState: stale stops working/running claims from pulsing', () => {
  const s = sessionState(agent, 'working', false, { stale: true });
  assert.equal(s.key, 'stale');
  assert.equal(s.label, 'Reconnecting…');
  assert.ok(!s.live);
  assert.equal(sessionState(agent, 'idle', false, { stale: true }).key, 'idle');
  const ny = sessionState(agent, 'idle', true, { stale: true });
  assert.equal(ny.key, 'needs-you');
  assert.ok(!ny.live);
});

test('isResumable mirrors the sidebar rule', () => {
  assert.ok(isResumable(shell, 'reconnectable'));
  assert.ok(isResumable(agent, 'exited'));
  assert.ok(!isResumable({ kind: 'agent', provider_session_id: null }, 'exited'));
  assert.ok(!isResumable(agent, 'idle'));
});

test('runStatus normalises every module vocabulary', () => {
  for (const s of ['success', 'ok', 'done', 'succeeded', 'completed', 'OK', 'Passed']) {
    assert.deepEqual([runStatus(s).key, runStatus(s).label, runStatus(s).tone], ['succeeded', 'Succeeded', 'success'], s);
  }
  for (const s of ['error', 'failed', 'Failure']) assert.equal(runStatus(s).key, 'failed', s);
  for (const s of ['pending', 'queued']) assert.deepEqual([runStatus(s).label, runStatus(s).tone], ['Queued', 'neutral']);
  assert.deepEqual([runStatus('waiting').label, runStatus('waiting').tone], ['Waiting', 'warning']);
  assert.deepEqual([runStatus('running').tone, runStatus('running').live], ['info', true]);
  assert.equal(runStatus('in progress').key, 'running');
  for (const s of ['cancelled', 'canceled']) assert.equal(runStatus(s).label, 'Cancelled');
  assert.equal(runStatus('skipped').label, 'Skipped');
  // Running must never share the success tone (Mission Control bug).
  assert.notEqual(runStatus('running').tone, runStatus('succeeded').tone);
});

test('runStatus keeps unknown words readable and neutral', () => {
  assert.deepEqual(runStatus('needs_review'), { key: 'needs_review', label: 'Needs review', tone: 'neutral' });
  assert.equal(runStatus(null).label, 'Unknown');
  assert.equal(sentenceCase('waiting_on-user'), 'Waiting on user');
});

test('envTone: prod danger, staging warning, dev neutral', () => {
  assert.deepEqual([envTone('prod').key, envTone('prod').tone], ['prod', 'danger']);
  assert.equal(envTone('Production').key, 'prod');
  assert.deepEqual([envTone('staging').tone, envTone('stg').key], ['warning', 'staging']);
  assert.deepEqual([envTone('dev').tone, envTone(undefined).key], ['neutral', 'dev']);
  assert.deepEqual([envTone('sandbox').label, envTone('sandbox').tone], ['sandbox', 'neutral']);
});
