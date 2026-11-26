// Invented sample runs for the run page's pure helpers (runs/stepOrder.ts,
// runs/runOrbit.ts, runs/runWords.ts): a PR-review-shaped graph and a run of it
// at the moment its review fans out. No real names, repositories or people.
import type { ChildThread, NodeRunState, WorkflowGraph } from '../src/lib/api/types';

export const T0 = Date.parse('2026-11-20T10:29:00Z');
export const at = (min: number): string => new Date(T0 + min * 60_000).toISOString();

const n = (id: string, kind: string, name: string) => ({ id, kind, name, x: 0, y: 0, params: {} });

/** start → ticket → checkout → (vault ∥ gates ∥ cross-repo) → review → passed? → post → notify */
export const PR_GRAPH: WorkflowGraph = {
  nodes: [
    n('n1', 'manual_trigger', 'Start PR review'),
    n('n2', 'http_request', 'Fetch Jira ticket'),
    n('n3', 'git_checkout', 'Check out PR branch'),
    n('n4', 'prepare_context', 'Gather context from the Vault'),
    n('n5', 'script', 'Go PR gates'),
    n('n6', 'agent_prompt', 'Cross-repo contract check'),
    n('n7', 'review_run', 'Multi-agent PR review'),
    n('n8', 'condition', 'Passed?'),
    n('n9', 'post_review_comments', 'Post inline PR comments'),
    n('n10', 'channel_notify', 'Notify the thread'),
  ],
  edges: [
    ['n1', 'n2'], ['n2', 'n3'], ['n3', 'n4'], ['n3', 'n5'], ['n3', 'n6'],
    ['n4', 'n7'], ['n5', 'n7'], ['n6', 'n7'], ['n7', 'n8'], ['n8', 'n9'], ['n9', 'n10'],
  ].map(([source, target], i) => ({ id: `e${i}`, source, target })),
};

/** A node that started `startMin` after T0 and ran `ms` (running when `ms` is null). */
export function node(id: string, status: NodeRunState['status'], startMin: number | null, ms: number | null, extra: Partial<NodeRunState> = {}): NodeRunState {
  return {
    node_id: id, status, logs: [],
    ...(startMin === null ? {} : { started_at: at(startMin) }),
    ...(ms === null ? {} : { duration_ms: ms }),
    ...extra,
  };
}

export function reviewers(done: number, running: number, queued: number): ChildThread[] {
  const kids: ChildThread[] = [];
  for (let i = 0; i < done; i++) kids.push({ id: `d${i}`, label: `Lens ${i} · Codex`, provider: 'codex', status: 'success', note: `${i + 1} findings` });
  for (let i = 0; i < running; i++) kids.push({ id: `r${i}`, label: `Busy ${i} · Codex`, provider: 'codex', status: 'running', started_at: at(6) });
  for (let i = 0; i < queued; i++) kids.push({ id: `q${i}`, label: `Later ${i} · Claude`, provider: 'claude', status: 'queued' });
  kids.push({ id: 'summarizer', label: 'Summarizer · Claude', provider: 'claude', status: 'queued' });
  return kids;
}

/** The PR review run 12 minutes in: three steps ran in parallel, the review fans out. */
export function fanoutNodes(done = 9, running = 3, queued = 0): NodeRunState[] {
  return [
    node('n1', 'success', 0, 0),
    node('n2', 'success', 0, 400),
    node('n3', 'success', 0.1, 38_000),
    node('n4', 'success', 0.8, 192_000),
    node('n5', 'success', 0.8, 64_000),
    node('n6', 'skipped', null, null, { brief: 'Only one repository in this run' }),
    node('n7', 'running', 4, null, { phase: 'working', children: reviewers(done, running, queued) }),
    node('n8', 'pending', null, null),
    node('n9', 'pending', null, null),
    node('n10', 'pending', null, null),
  ];
}
