// The project badge (projectBadge.ts): two letters and a stable tone per name.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { projectBadge } from '../src/lib/projectBadge.ts';

test('two words give their initials; one word its first and last letter', () => {
  assert.equal(projectBadge('harbor-api').initials, 'HA');
  assert.equal(projectBadge('harbor').initials, 'HR');
  assert.equal(projectBadge('dotfiles').initials, 'DS');
  assert.equal(projectBadge('my_cool.project').initials, 'MC');
  assert.equal(projectBadge('x').initials, 'X');
  assert.equal(projectBadge('').initials, '?');
  assert.equal(projectBadge('קוד שלי').initials, 'קש');
});

test('the tone is stable per name and always one of the six categorical tokens', () => {
  assert.equal(projectBadge('harbor').tone, projectBadge('harbor').tone);
  const tones = new Set(['harbor', 'harbor-api', 'fieldnotes', 'dotfiles', 'otto', 'claude_ade', 'web', 'api'].map((n) => projectBadge(n).tone));
  for (const t of tones) assert.ok(t >= 1 && t <= 6);
  assert.ok(tones.size > 2, 'names spread over several tones');
});
