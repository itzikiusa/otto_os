// The run page's writes (ux.md §4.5, engine.md §10.1, goal.md): thin calls
// with the run page's confirmations and toasts. Every one leaves the run on
// screen live through RunSync's event path.
import { api } from '../../../lib/api/client';
import { rereviewWorkflowRun, retryRunNode } from '../../../lib/api/workflows';
import { confirmer } from '../../../lib/confirm.svelte';
import { loadIdeRuntimeSessionPort } from '../../../lib/runtimeThreadPort';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';

const run = (id: string) => `/workflow-runs/${encodeURIComponent(id)}`;

async function attempt(what: string, done: string | null, call: () => Promise<unknown>): Promise<boolean> {
  try {
    await call();
    if (done) toasts.success(done);
    return true;
  } catch (e) {
    toastError(`Couldn’t ${what}`, e);
    return false;
  }
}

/** "Retry step" re-runs the step and what follows it; "Retry only this step" just the step. */
export function retryStep(runId: string, nodeId: string, downstream = true): Promise<boolean> {
  return attempt('retry the step', 'Retrying the step', () => retryRunNode(runId, nodeId, downstream));
}

/** Stop a running run, after a confirmation. */
export async function stopRun(runId: string): Promise<boolean> {
  const ok = await confirmer.ask('Running agents are stopped; files written so far are kept.', { title: 'Stop run?', confirmLabel: 'Stop run', danger: true });
  return ok && attempt('stop the run', 'Run stopped', () => api.post(`${run(runId)}/cancel`, {}));
}

/** Cancel a queued run: nothing started, no confirmation. */
export function cancelQueued(runId: string): Promise<boolean> {
  return attempt('cancel the run', 'Run canceled', () => api.post(`${run(runId)}/cancel`, {}));
}

export function skipStep(runId: string, nodeId: string): Promise<boolean> {
  return attempt('skip the step', 'Step skipped', () => api.post(`${run(runId)}/nodes/${encodeURIComponent(nodeId)}/skip`, {}));
}

export function pauseRun(runId: string): Promise<boolean> {
  return attempt('pause the run', 'Run paused', () => api.post(`${run(runId)}/pause`, {}));
}

/** Resume a held run; `extend` raises the loop's caps (new totals). */
export function resumeRun(runId: string, extend?: { iterations?: number; active_s?: number }): Promise<boolean> {
  return attempt('resume the run', 'Run resumed', () => api.post(`${run(runId)}/resume`, extend ? { extend } : {}));
}

/** "Add a note for the next step…": the note reaches the next agent step. */
export async function steerRun(runId: string): Promise<boolean> {
  const note = await confirmer.promptText('The next agent step reads this note before it starts.', { title: 'Add a note for the next step', confirmLabel: 'Add note', placeholder: 'Also check the migration rollback' });
  return !!note?.trim() && attempt('add the note', 'Note added', () => api.post(`${run(runId)}/steer`, { note: note.trim() }));
}

export function rereview(runId: string): Promise<boolean> {
  return attempt('start the re-review', 'Re-review started', () => rereviewWorkflowRun(runId));
}

/** Record a "you check it" criterion. */
export function verifyCriterion(runId: string, nodeId: string, criterionId: string, met: boolean, evidence: string): Promise<boolean> {
  return attempt('record the check', met ? 'Recorded as met' : 'Recorded as not met', () =>
    api.post(`${run(runId)}/nodes/${encodeURIComponent(nodeId)}/criteria/${encodeURIComponent(criterionId)}/verify`, { met, evidence }));
}

/** Answer a step thread's approval (`via: provider | mcp`) by its request id. */
export function answerRequest(threadId: string, requestId: string, approve: boolean): Promise<boolean> {
  return attempt(approve ? 'approve' : 'deny', null, async () => {
    const port = await loadIdeRuntimeSessionPort();
    await port.runtimeThreads.respond(threadId, requestId, approve ? 'accept' : 'decline');
  });
}
