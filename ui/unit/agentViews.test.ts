import { test } from 'node:test';
import assert from 'node:assert/strict';
import { broadcastScope, matchesSavedView } from '../src/modules/agents/viewFilters.ts';

test('scratch broadcasts are allowed while mixed and missing recipients are refused', () => {
  const sessions = [{ id: 'a', workspace_id: 'scratch' }, { id: 'b', workspace_id: 'scratch' }, { id: 'c', workspace_id: 'project' }];
  assert.equal(broadcastScope(['a', 'b'], sessions), 'scratch');
  assert.equal(broadcastScope(['a', 'c'], sessions), null);
  assert.equal(broadcastScope(['a', 'unknown'], sessions), null);
  assert.equal(broadcastScope([], sessions), null);
});

test('saved view filters combine provider, repository and minimum cost', () => {
  const item = { id: 'work', session_id: 'a', cost_usd: 3 };
  const sessions = [{ id: 'a', provider: 'codex', cwd: '/repo' }];
  assert.equal(matchesSavedView(item, { provider: 'codex', repo: '/repo', min_cost_usd: 2 }, sessions), true);
  assert.equal(matchesSavedView(item, { provider: 'claude', min_cost_usd: 2 }, sessions), false);
  assert.equal(matchesSavedView(item, { repo: '/other', min_cost_usd: 2 }, sessions), false);
  assert.equal(matchesSavedView(item, { min_cost_usd: 4 }, sessions), false);
  assert.equal(matchesSavedView({ id: 'non-session' }, { provider: 'codex' }, sessions), false);
});
