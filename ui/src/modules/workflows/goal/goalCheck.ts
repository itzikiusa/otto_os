// A goal run's criteria as rows (goal.md §3.5): the latest settled check's
// output (CheckOutput, agreed-names.md §4) laid over the check node's criteria,
// with "Verify" for a criterion parked for a person. Pure; unit-tested in
// ui/unit/goalCheck.test.ts.
import type { CheckOutput, Criterion, NodeRunState, VerifyKind, WorkflowCheckpoint } from '../../../lib/api/types';

/** The method chip's words (the form says "I check it"; the run page "You"). */
export const METHOD_WORD: Record<string, string> = { command: 'Command', agent: 'Agent', human: 'You', expr: 'Rule' };

/** Mirrors `VERIFY_OUTPUT_MARKER` in crates/otto-workflows/src/criteria.rs. */
const OUTPUT_MARKER = '\n--- verify output ---\n';

export type CriterionState = 'met' | 'not_met' | 'verify' | 'pending';

export interface CriterionRow {
  id: string;
  text: string;
  method: VerifyKind;
  state: CriterionState;
  /** The evidence's first line. */
  headline: string;
  /** The rest of the evidence (without the command output). */
  detail: string;
  /** A command's output tail. */
  output: string;
  /** `command` · `rule` · `agent` · a person's name. */
  by: string;
}

/** A `check` step's output (its criteria and verdict). */
export function isCheckOutput(v: unknown): v is CheckOutput {
  return !!v && typeof v === 'object' && Array.isArray((v as CheckOutput).criteria) && 'verdict' in v;
}

/** A loop's newest settled check attempt (newest iteration, latest step).
 *  A checkpoint page may carry it without its output (`has_output`). */
export function newestCheckpoint(checkpoints: readonly WorkflowCheckpoint[]): WorkflowCheckpoint | null {
  let best: WorkflowCheckpoint | null = null;
  for (const cp of checkpoints) {
    if (cp.kind !== 'check' || cp.status !== 'success') continue;
    if (!best || cp.iteration > best.iteration || (cp.iteration === best.iteration && cp.step_index > best.step_index)) best = cp;
  }
  return best;
}

/** The newest check result of the run: a loop's check checkpoints, a settled
 *  loop's last history entry, or a standalone check node's output. */
export function latestCheck(nodes: readonly NodeRunState[], checkpoints?: readonly WorkflowCheckpoint[]): CheckOutput | null {
  const cp = newestCheckpoint(checkpoints ?? []);
  if (cp && isCheckOutput(cp.output)) return cp.output;
  for (const n of nodes) {
    if (isCheckOutput(n.output)) return n.output;
    const history = (n.output as { history?: { steps?: Record<string, unknown> }[] } | null | undefined)?.history;
    for (const h of [...(history ?? [])].reverse()) {
      const hit = Object.values(h.steps ?? {}).find(isCheckOutput);
      if (hit) return hit;
    }
  }
  return null;
}

function splitEvidence(evidence: string): Pick<CriterionRow, 'headline' | 'detail' | 'output'> {
  const at = evidence.indexOf(OUTPUT_MARKER);
  const words = (at < 0 ? evidence : evidence.slice(0, at)).trim();
  const nl = words.indexOf('\n');
  return {
    headline: nl < 0 ? words : words.slice(0, nl),
    detail: nl < 0 ? '' : words.slice(nl + 1).trim(),
    output: at < 0 ? '' : evidence.slice(at + OUTPUT_MARKER.length),
  };
}

/** One row per criterion, in the check node's order (the result's own order
 *  when the graph has none). */
export function criterionRows(criteria: readonly Criterion[], check: CheckOutput | null, awaiting: readonly string[] = []): CriterionRow[] {
  const results = new Map((check?.criteria ?? []).map((r) => [r.id, r]));
  const list: readonly { id: string; text: string; verify: VerifyKind }[] = criteria.length ? criteria : (check?.criteria ?? []);
  return list.map((c) => {
    const r = results.get(c.id);
    const state: CriterionState = awaiting.includes(c.id) ? 'verify' : r ? (r.met ? 'met' : 'not_met') : 'pending';
    return { id: c.id, text: c.text || r?.text || c.id, method: c.verify, state, ...splitEvidence(r?.evidence ?? ''), by: r?.verified_by ?? '' };
  });
}

export const STATE_WORD: Record<CriterionState, string> = { met: 'Met', not_met: 'Not met', verify: 'Verify', pending: 'Not checked yet' };
