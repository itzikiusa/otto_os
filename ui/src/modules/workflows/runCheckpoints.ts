// Revision- and generation-aware checkpoint state shared between the run step
// list (loop rounds) and GoalCard (ARCH-1). Keeps loop checkpoints live as
// iterations advance or after an in-place retry, and requests full checkpoint
// detail ONLY for the `check` and `review_run` outputs those views actually read.
// Pure; unit-tested in ui/unit/workflowLoopRounds.test.ts.
import type { WorkflowCheckpoint, WorkflowRun } from '../../lib/api/types';
import { isCheckOutput } from './goal/goalCheck.ts';

export interface CheckpointStamp {
  runId: string;
  rev: number;
  generation: number;
}

export function checkpointStamp(run: Pick<WorkflowRun, 'id' | 'checkpoint_rev' | 'checkpoint_generation'>): CheckpointStamp {
  return {
    runId: run.id,
    rev: run.checkpoint_rev ?? 0,
    generation: run.checkpoint_generation ?? 0,
  };
}

/** Whether the shared checkpoint list needs a `/checkpoints` page refresh for `run`. */
export function needsCheckpointRefresh(
  run: Pick<WorkflowRun, 'id' | 'summary' | 'checkpoint_rev' | 'checkpoint_generation' | 'checkpoints'> & {
    overview?: { goal?: unknown } | null;
  },
  loaded: CheckpointStamp | null,
): boolean {
  const rev = run.checkpoint_rev ?? 0;
  const gen = run.checkpoint_generation ?? 0;
  const hasCps = (run.checkpoints?.length ?? 0) > 0;
  if (!loaded || loaded.runId !== run.id) {
    // Full run read already includes checkpoints unless rev says pages exist and none came down.
    if (!run.summary && (hasCps || rev === 0)) return false;
    return rev > 0 || gen > 0 || !!run.overview?.goal;
  }
  if (loaded.generation !== gen) return true;
  if (loaded.rev !== rev) return true;
  // A retry-cleared summary with checkpoints still missing on a goal/loop run.
  if (run.summary && !hasCps && (rev > 0 || !!run.overview?.goal)) return true;
  return false;
}

/** Whether a checkpoint row needs its full detail body loaded for StepList or GoalCard.
 *  Only `check` (settled, missing CheckOutput) and `review_run` (missing review_id/score/open)
 *  outputs are used by those views; agent/script/delay/condition bodies are skipped. */
export function needsCheckpointDetail(cp: WorkflowCheckpoint): boolean {
  if (cp.kind === 'check') return cp.status === 'success' && !isCheckOutput(cp.output);
  if (cp.kind === 'review_run') return cp.output == null;
  return false;
}

/** Merge newly paged summary checkpoints with previously loaded full bodies for
 *  the same generation, returning the merged array and ONLY the rows that still
 *  need `workflowCheckpointDetail`. */
export function mergeCheckpointDetails(
  items: readonly WorkflowCheckpoint[],
  cached: ReadonlyMap<string, WorkflowCheckpoint>,
): { merged: WorkflowCheckpoint[]; toFetch: WorkflowCheckpoint[] } {
  const merged: WorkflowCheckpoint[] = [];
  const toFetch: WorkflowCheckpoint[] = [];
  for (const item of items) {
    const hit = cached.get(item.node_id);
    if (
      hit &&
      hit.output != null &&
      hit.updated_at === item.updated_at &&
      hit.status === item.status &&
      hit.attempts === item.attempts
    ) {
      merged.push(hit);
      continue;
    }
    merged.push(item);
    if (needsCheckpointDetail(item)) {
      toFetch.push(item);
    }
  }
  return { merged, toFetch };
}
