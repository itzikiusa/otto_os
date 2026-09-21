import type { NodeRunState, Review, ReviewAgentState } from '../../lib/api/types';

/** Current runs persist the association before waiting. Older completed runs
 * carry it in their output (including multi-repository review steps). */
export function reviewIds(node: NodeRunState): string[] {
  const ids = new Set(node.review_ids ?? []);
  function visit(value: unknown): void {
    if (!value || typeof value !== 'object') return;
    if (Array.isArray(value)) { value.forEach(visit); return; }
    const obj = value as Record<string, unknown>;
    if (typeof obj.review_id === 'string') ids.add(obj.review_id);
    Object.values(obj).forEach(visit);
  }
  visit(node.output);
  return [...ids];
}

export function reviewSessions(node: NodeRunState, reviews: Record<string, Review>): string[] {
  return [...new Set([
    ...(node.sessions ?? []),
    ...reviewIds(node).flatMap((id) => reviews[id]?.agents.flatMap((a) => a.session_id ? [a.session_id] : []) ?? []),
  ])];
}

export function reviewAgentStatus(review: Review, agent: ReviewAgentState): string {
  if (review.status === 'cancelled') return 'cancelled';
  if (agent.fallback) return 'fallback';
  if (review.status === 'error' && ['running', 'waiting', 'pending'].includes(agent.status)) return 'error';
  return agent.status;
}
