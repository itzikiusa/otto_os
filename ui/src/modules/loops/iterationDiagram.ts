import type { GoalLoop } from '../../lib/api/types';
import type { DiagramEdge, DiagramNode } from './diagramLayout';

export const iterationPhases = [
  { id: 'planning', title: 'Plan', note: 'Choose the next work', explanation: 'The planner chooses a bounded next step from the goal, acceptance criteria and earlier feedback.' },
  { id: 'executing', title: 'Execute', note: 'Make the change', explanation: 'The configured executors work sequentially, keeping their changes and results in the retained working directory.' },
  { id: 'evaluating', title: 'Evaluate', note: 'Check the evidence', explanation: 'The evaluator checks acceptance criteria against actual evidence. Passing criteria can complete the goal; missing evidence keeps it open.' },
  { id: 'digesting', title: 'Digest', note: 'Carry feedback forward', explanation: 'The loop records what changed, what remains and what to try next. Another iteration starts only while work and budget remain.' },
];
export const iterationEdges: DiagramEdge[] = [
  { from: 'planning', to: 'executing' }, { from: 'executing', to: 'evaluating' },
  { from: 'evaluating', to: 'digesting' }, { from: 'digesting', to: 'planning', feedback: true },
];

export function iterationNodes(loop?: Pick<GoalLoop, 'phase' | 'status'>): DiagramNode[] {
  const phase = loop?.phase === 'waiting' ? 'executing' : loop?.phase;
  return iterationPhases.map(({ id, title, note }) => {
    if (loop?.status === 'succeeded' && id === 'evaluating') return { id, title, note: 'Goal met', tone: 'passed' };
    if (!loop || loop.status === 'draft' || phase !== id) return { id, title, note };
    if (loop.status === 'running') return { id, title, note: loop.phase === 'waiting' ? 'Waiting on an agent' : 'Running', tone: 'running' };
    const labels = { paused: 'Paused', blocked: 'Blocked', exhausted: 'Budget reached', failed: 'Failed', stopped: 'Stopped', succeeded: 'Finished' };
    return { id, title, note: labels[loop.status], tone: loop.status === 'blocked' || loop.status === 'failed' ? 'blocked' : undefined };
  });
}
