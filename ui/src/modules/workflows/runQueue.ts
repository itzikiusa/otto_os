// The daemon-wide workflow run queue as the Running list shows it: a queued
// run's place in line ("2nd in line, behind PR Reviewer") and the one-line cap
// note. Pure — RunningRuns.svelte feeds it the active runs and the queue view
// (GET /workspaces/{wid}/workflow-runs/queue); unit-tested in
// unit/workflowRunQueue.test.ts.

import type { ActiveWorkflowRun, WorkflowRunQueue } from '../../lib/api/types';
import { plural } from '../../lib/plural.ts';

/** 1 → "1st", 2 → "2nd", 11 → "11th", 23 → "23rd". */
export function ordinal(n: number): string {
  const tens = n % 100;
  if (tens >= 11 && tens <= 13) return `${n}th`;
  switch (n % 10) {
    case 1:
      return `${n}st`;
    case 2:
      return `${n}nd`;
    case 3:
      return `${n}rd`;
    default:
      return `${n}th`;
  }
}

/** A queued run's place: the short chip ("2nd in line") and the full line
 *  ("Queued · 2nd in line, behind PR Reviewer, Deploy"). Runs ahead that
 *  belong to another workspace are not listed by the daemon; they are counted
 *  as "N other run(s)". `null` when `runId` is not queued. */
export function queuePlace(
  runId: string,
  queue: WorkflowRunQueue | null,
  active: ActiveWorkflowRun[],
): { chip: string; line: string } | null {
  const mine = queue?.queued.find((q) => q.run_id === runId);
  if (!mine) return null;
  if (mine.position <= 1) {
    return { chip: 'next in line', line: 'Queued · next in line — starts when a running run finishes' };
  }
  const names = new Map(active.map((r) => [r.run_id, r.workflow_name]));
  const ahead = queue!.queued
    .filter((q) => q.position < mine.position)
    .sort((a, b) => a.position - b.position)
    .map((q) => names.get(q.run_id))
    .filter((n): n is string => !!n);
  const others = mine.position - 1 - ahead.length;
  const behind = [...ahead];
  if (others > 0) behind.push(plural(others, 'other run'));
  const chip = `${ordinal(mine.position)} in line`;
  return { chip, line: `Queued · ${chip}, behind ${behind.join(', ')}` };
}

/** The one-line cap note under the Running header. */
export function capNote(max: number): string {
  return `Up to ${max} at once — other workflow runs wait in line.`;
}

/** Whether any listed run is still waiting for a slot (fresh `pending`, no
 *  step progress yet) — the queue view is only fetched then. */
export function hasQueued(active: ActiveWorkflowRun[]): boolean {
  return active.some((r) => r.status === 'pending' && r.nodes_done === 0 && !r.current_step);
}
