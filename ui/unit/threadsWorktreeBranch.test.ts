// A new thread's worktree branch (worktreeBranch.ts): a slug of the prompt,
// the repo's prefix when it is clear, a suffix on collision, dated without a prompt.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { branchPrefix, branchSlug, datedSlug, worktreeBranch } from '../src/modules/threads/worktreeBranch.ts';

test('the slug keeps the first meaningful words, kebab-case', () => {
  assert.equal(branchSlug('Fix the flaky checkout test!'), 'fix-flaky-checkout-test');
  assert.equal(branchSlug('Add a rate limit to the public search endpoint, 60/min'), 'add-rate-limit-public-search');
  assert.equal(branchSlug('Café crème: résumé @src/app.ts'), 'cafe-creme-resume-src-app');
  assert.equal(branchSlug('שלום עולם'), '');
  assert.ok(branchSlug('supercalifragilisticexpialidocious '.repeat(3)).length <= 40);
});

test('a prefix is used only when the repo clearly follows one', () => {
  assert.equal(branchPrefix(['main', 'feat/a', 'feat/b', 'fix/c'], 'main'), 'feat/');
  assert.equal(branchPrefix(['main', 'feat/a', 'fix/b', 'chore/c'], 'main'), '');
  assert.equal(branchPrefix(['main', 'feat/a'], 'main'), '');
  assert.equal(branchPrefix(['main', 'one', 'two', 'feat/a', 'feat/b'], 'main'), '');
});

test('collisions get a suffix; no prompt gives a dated name', () => {
  const now = new Date(2026, 10, 23, 18, 5);
  assert.equal(worktreeBranch({ prompt: 'Fix login', branches: ['main', 'fix-login', 'fix-login-2'], defaultBranch: 'main', now }), 'fix-login-3');
  assert.equal(worktreeBranch({ prompt: 'Fix login', branches: ['main', 'feat/x', 'feat/y'], defaultBranch: 'main', now }), 'feat/fix-login');
  assert.equal(worktreeBranch({ prompt: '', branches: ['main'], defaultBranch: 'main', now }), 'thread-2026-11-23-1805');
  assert.equal(datedSlug(now), 'thread-2026-11-23-1805');
});
