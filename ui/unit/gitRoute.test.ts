// Git page routes (modules/git/gitRoute.ts): Focus lives in the URL so the
// toggle, the crumb, repo links inside Focus, Back and reload agree.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { focusPath, parseGitRoute } from '../src/modules/git/gitRoute.ts';

test('#/git/focus is Focus; anything else leaves it', () => {
  assert.equal(parseGitRoute(['git', 'focus']).focus, true);
  assert.equal(parseGitRoute(['git']).focus, false);
  assert.equal(parseGitRoute(['git', 'r1']).focus, false);
  // A repo link inside Focus routes to the repo's PRs: Focus off, tab opened.
  assert.deepEqual(parseGitRoute(['git', 'r1', 'prs']), { focus: false, repo: 'r1', tab: 'prs', pr: false, legacyFocus: false });
});

test('legacy #/git/<repo>/focus opens the repo then rewrites to Focus', () => {
  assert.deepEqual(parseGitRoute(['git', 'r1', 'focus']), { focus: false, repo: 'r1', tab: null, pr: false, legacyFocus: true });
});

test('PR detail routes are recognised, not tabs', () => {
  assert.deepEqual(parseGitRoute(['git', 'r1', 'pr', '7']), { focus: false, repo: 'r1', tab: null, pr: true, legacyFocus: false });
  assert.equal(parseGitRoute(['git', 'r1', 'pr']).tab, 'pr');
});

test('focusPath', () => {
  assert.equal(focusPath(true), 'git/focus');
  assert.equal(focusPath(false), 'git');
});
