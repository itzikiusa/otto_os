import { test } from 'node:test';
import assert from 'node:assert/strict';
import { summaryFallback, summaryFallbackTitle } from '../src/modules/git/reviewFallback.ts';

const row = (provider: string, note: string) =>
  ({ name: 'summarizer', provider, model: '', status: 'done', note, comment_count: 3 }) as never;

test('error-handling-5-05: the stored cause and the real provider reach the tooltip', () => {
  const f = summaryFallback([
    row('claude', '2 findings'),
    row('codex', 'deterministic fallback (summarizer: codex is not logged in) — 3 final comments'),
  ]);
  assert.deepEqual(f, { provider: 'codex', cause: 'codex is not logged in' });
  const title = summaryFallbackTitle(f);
  assert.match(title, /The codex summarizer/);
  assert.match(title, /\(codex is not logged in\)/);
  assert.doesNotMatch(title, /claude/);
});

test('a pre-cause row and an empty list still read sensibly', () => {
  assert.deepEqual(summaryFallback([row('agy', 'deterministic fallback — 1 final comment')]), {
    provider: 'agy',
    cause: '',
  });
  assert.deepEqual(summaryFallback(undefined), { provider: 'claude', cause: '' });
  assert.doesNotMatch(summaryFallbackTitle(summaryFallback([])), /\(\)/);
});
