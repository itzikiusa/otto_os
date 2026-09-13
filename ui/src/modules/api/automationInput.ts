import type {ApiAutomationStep} from '../../lib/api/types';

/** API-authored steps may omit optional assertion/extraction arrays. */
export function editableSteps(steps: ApiAutomationStep[]): ApiAutomationStep[] {
  return steps.map(step => ({
    request_id: step.request_id,
    assertions: (step.assertions ?? []).map(assertion => ({...assertion})),
    extract: (step.extract ?? []).map(extract => ({...extract})),
  }));
}
