// When the IDE editor re-reads a file's HEAD copy for its change bars (review P2-013): the git
// state's first load must not cost a second read, and must not hide the first real change.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isSettlingDigest, needsGitBaseRead, settledGitBaseDigest } from '../src/modules/ide/editor/ideGitBaseDigest.ts';

const CLEAN = 'r1:main:0:0:3:clean';
const COMMITTED = 'r1:main:0:0:4:clean';

test('the first load of the git state (pending → no status → known) does not re-read', () => {
  assert.equal(needsGitBaseRead('pending', 'norepo:1', true), false);
  assert.equal(needsGitBaseRead('norepo:1', 'r1:nostatus:3', true), false);
  assert.equal(needsGitBaseRead('r1:nostatus:3', CLEAN, true), false);
  assert.equal(needsGitBaseRead(CLEAN, CLEAN, true), false);
});

test('nothing held yet, or a real change after the first load, reads', () => {
  assert.equal(needsGitBaseRead(undefined, 'pending', false), true);
  assert.equal(needsGitBaseRead(CLEAN, COMMITTED, true), true);
});

test('a read started while git was still loading stands for the digest reached meanwhile, so the next commit re-reads', () => {
  // read #1 starts at 'pending'; git finishes loading before it answers
  const persisted = settledGitBaseDigest('pending', CLEAN);
  assert.equal(persisted, CLEAN);
  assert.equal(isSettlingDigest(persisted), false);
  // the first real commit must re-read (with 'pending' persisted it was swallowed)
  assert.equal(needsGitBaseRead(persisted, COMMITTED, true), true);
});

test('a read started at a real digest keeps standing for it', () => {
  assert.equal(settledGitBaseDigest(CLEAN, COMMITTED), CLEAN);
});
