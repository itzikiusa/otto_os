// "Stuck?" recovery for a running step whose children all settled but which
// never moved on: stop the run, then (restart) re-enter it at that step once
// the engine has stopped. Both confirm first — they stop a live run.
import { api } from '../../lib/api/client';
import { retryRunNode } from '../../lib/api/workflows';
import type { WorkflowRun } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';

export async function recoverStuckStep(
  runId: string,
  nodeId: string,
  name: string,
  restart: boolean,
  onRunUpdated?: (run: WorkflowRun) => void,
): Promise<void> {
  const ok = await confirmer.ask(
    restart
      ? `Stop this run and re-run “${name}” and every step after it? Steps that already succeeded keep their results.`
      : `Stop this run and mark “${name}” as failed? Steps after it won’t run.`,
    { title: restart ? 'Restart stuck step' : 'Mark step failed', confirmLabel: restart ? 'Restart step' : 'Mark failed', danger: !restart },
  );
  if (!ok) return;
  try {
    await api.post(`/workflow-runs/${encodeURIComponent(runId)}/cancel`, {});
    if (!restart) {
      toasts.info('Run stopped', `${name} marked failed`);
      return;
    }
    // The engine needs a moment to stop the in-flight step ("still stopping"
    // 409) before the run can be re-entered.
    for (let i = 0; ; i++) {
      try {
        onRunUpdated?.(await retryRunNode(runId, nodeId, true));
        break;
      } catch (e) {
        if (i >= 14) throw e;
        await new Promise((r) => setTimeout(r, 2000));
      }
    }
    toasts.info('Restarting the step…', name);
  } catch (e) {
    toastError(restart ? 'Couldn’t restart the step' : 'Couldn’t stop the run', e);
  }
}
