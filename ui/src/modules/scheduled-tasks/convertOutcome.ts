// Outcome of "Convert to workflow". The daemon creates the workflow, then its
// schedule trigger, then (optionally) pauses the task, and rolls the workflow
// back when either later step fails — so a success response always carries
// `trigger_id`. A response without one (an older daemon) means the workflow
// exists with no schedule: that is a failure, never "Converted".

export interface ConvertResult {
  workflow_id: string;
  trigger_id?: string | null;
}

export type ConvertOutcome =
  | { ok: true; title: string; body: string }
  | { ok: false; title: string; body: string };

export function convertOutcome(res: ConvertResult, name: string, pausedTask: boolean): ConvertOutcome {
  if (!res.trigger_id) {
    return {
      ok: false,
      title: `Couldn’t schedule “${name}” as a workflow`,
      body: 'The workflow was created without its schedule. Add a schedule trigger to it, or check that the task is still running.',
    };
  }
  return {
    ok: true,
    title: 'Converted to workflow',
    body: pausedTask ? `Created a workflow from “${name}” and paused the task.` : `Created a workflow from “${name}”.`,
  };
}
