import type {ApiAssertion, ApiAutomationStep, ApiExtract} from '../../lib/api/types';

// The automation editor's working copy. Every step, check and saved value
// carries a client-only `key` so the `{#each}` blocks key by identity, not by
// slot: after a move the focused arrow travels with its step (an index key
// kept focus on the slot, which then showed the other step, so a keyboard
// user could never move a step more than one place — a11y-6-03). `key` never
// reaches the daemon: `savedSteps` strips it.

export type EditableAssertion = ApiAssertion & {key: string};
export type EditableExtract = ApiExtract & {key: string};
export interface EditableStep {
  key: string;
  request_id: ApiAutomationStep['request_id'];
  assertions: EditableAssertion[];
  extract: EditableExtract[];
}

let seq = 0;
/** A fresh client-only row key (unique for the page's lifetime). */
export function rowKey(): string {
  return `r${++seq}`;
}

export function editableAssertion(a: ApiAssertion): EditableAssertion {
  return {...a, key: rowKey()};
}

export function editableExtract(e: ApiExtract): EditableExtract {
  return {...e, key: rowKey()};
}

/** API-authored steps may omit optional assertion/extraction arrays. */
export function editableSteps(steps: ApiAutomationStep[]): EditableStep[] {
  return steps.map(step => ({
    key: rowKey(),
    request_id: step.request_id,
    assertions: (step.assertions ?? []).map(editableAssertion),
    extract: (step.extract ?? []).map(editableExtract),
  }));
}

/** The wire shape for Save: keys stripped, and incomplete "save a value" rows
 *  (both path and variable name are required) dropped. */
export function savedSteps(steps: EditableStep[]): ApiAutomationStep[] {
  return steps.map(s => ({
    request_id: s.request_id,
    assertions: s.assertions.map(({key: _key, ...a}) => a),
    extract: s.extract
      .filter(e => e.path.trim() !== '' && e.var.trim() !== '')
      .map(({key: _key, ...e}) => e),
  }));
}

/** The arrow to focus after moving a step from `from` by `dir` in a list of
 *  `len`: the same arrow on the moved step, unless the move put it at the edge
 *  where that arrow is disabled — then its other arrow. */
export function movedArrow(from: number, dir: -1 | 1, len: number): {index: number; arrow: 'up' | 'down'} {
  const index = from + dir;
  if (dir === -1) return {index, arrow: index === 0 ? 'down' : 'up'};
  return {index, arrow: index === len - 1 ? 'up' : 'down'};
}
