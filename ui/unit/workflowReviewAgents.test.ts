import { test } from 'node:test';
import assert from 'node:assert/strict';
import { reviewIds, reviewSessions, reviewAgentStatus } from '../src/modules/workflows/reviewAgents.ts';
import type { NodeRunState, Review } from '../src/lib/api/types.ts';

const node: NodeRunState = { node_id: 'review', status: 'success', logs: [], sessions: ['old-attempt'], review_ids: ['r'] };
const review: Review = { id: 'r', repo_id: 'repo', pr_number: 0, status: 'running', error: null, comments: [], created_at: '', agents: [
  { name: 'Summarizer', provider: 'codex', model: 'configured', status: 'running', note: '', comment_count: 0, session_id: 'new-attempt' },
] };

test('completed await:false node discovers a later summarizer and retains captured history', () => {
  assert.deepEqual(reviewIds(node), ['r']);
  assert.deepEqual(reviewSessions(node, { r: review }), ['old-attempt', 'new-attempt']);
});
test('reload resolves associations and retries from durable output on older runs', () => {
  const reloaded = JSON.parse(JSON.stringify({ ...node, review_ids: undefined, output: { reviews: [{ review_id: 'r' }] } }));
  assert.deepEqual(reviewIds(reloaded), ['r']);
  assert.deepEqual(reviewSessions(reloaded, { r: review }), ['old-attempt', 'new-attempt']);
});
test('review cancellation and fallback override suspended session lifecycle', () => {
  const agent = review.agents[0];
  assert.equal(reviewAgentStatus(review, agent), 'running');
  assert.equal(reviewAgentStatus({ ...review, status: 'cancelled' }, agent), 'cancelled');
  assert.equal(reviewAgentStatus({ ...review, status: 'done' }, { ...agent, status: 'done', fallback: true }), 'fallback');
});
