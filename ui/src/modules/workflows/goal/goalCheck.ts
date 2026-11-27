// A goal run's criteria as rows (goal.md §3.5): each check step's latest
// settled output (CheckOutput, agreed-names.md §4) laid over its criteria,
// keyed by `(step_id, id)` so two check steps with the same criterion id stay
// distinct, with per-round history, numeric progress meters, and "Verify" for
// a criterion parked for a person. Pure; unit-tested in ui/unit/goalCheck.test.ts.
import type {
  CheckOutput,
  Criterion,
  CriterionPoint,
  CriterionProgress,
  CriterionResult,
  GoalCriterion,
  GoalProgress,
  NodeRunState,
  VerifyKind,
  WorkflowCheckpoint,
  WorkflowGraph,
} from '../../../lib/api/types';

/** The method chip's words (the form says "I check it"; the run page "You"). */
export const METHOD_WORD: Record<string, string> = { command: 'Command', agent: 'Agent', human: 'You', expr: 'Rule' };

/** Mirrors `VERIFY_OUTPUT_MARKER` in crates/otto-workflows/src/criteria.rs. */
const OUTPUT_MARKER = '\n--- verify output ---\n';
const MAX_HISTORY = 50;

export type CriterionState = 'met' | 'not_met' | 'verify' | 'pending';

/** A criterion tagged with the check step it belongs to (`<loop_id>.<step_index>` or node id). */
export interface ScopedCriterion extends Criterion {
  step_id: string;
  step_name: string;
}

export interface CriterionRow {
  /** Unique key `${step_id}::${id}` so identical criterion ids in separate check steps stay distinct. */
  key: string;
  step_id: string;
  step_name: string;
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
  /** Numeric progress, blocker label, and inspected values (when recorded). */
  progress?: CriterionProgress;
  /** Per-round history (ascending by iteration, bounded to 50). */
  history: CriterionPoint[];
  /** Rule expression (when `method === 'expr'`), used to distinguish floor vs ceiling rules. */
  expr?: string;
}

export function criterionKey(stepId: string, id: string): string {
  return `${stepId}::${id}`;
}

function isGenericCheckName(s: string): boolean {
  const lower = s.trim().toLowerCase();
  return !lower || lower === 'check' || lower === 'check criteria';
}

function isGenericLoopName(s: string): boolean {
  const lower = s.trim().toLowerCase();
  return !lower || lower === 'loop' || lower === 'work toward the goal' || lower === 'review → fix until passing' || lower === 'iterate';
}

function chooseStepLabel(loopName: string, innerName: string, checksInLoop: number, multiLoops: boolean): string {
  if (checksInLoop > 1) return multiLoops ? `${loopName} · ${innerName}` : innerName;
  if (!isGenericCheckName(innerName) && isGenericLoopName(loopName)) return innerName;
  return loopName;
}

/** The criteria of every check in a graph: a top-level check, or a loop's
 *  check step(s), each carrying its check-step key (`step_id`, `step_name`). */
export function graphCriteria(graph: Pick<WorkflowGraph, 'nodes'>): ScopedCriterion[] {
  type InnerStep = { kind: string; name?: string; params?: { criteria?: Criterion[] } };
  const goalLoops = graph.nodes.filter(
    (n) => n.kind === 'loop' && ((n.params as { steps?: InnerStep[] } | null)?.steps ?? []).some((s) => s.kind === 'check'),
  );
  const multiLoops = goalLoops.length > 1;
  const out: ScopedCriterion[] = [];
  for (const n of graph.nodes) {
    if (n.kind === 'loop') {
      const steps = (n.params as { steps?: InnerStep[] } | null)?.steps ?? [];
      const checksInLoop = steps.filter((s) => s.kind === 'check').length;
      if (checksInLoop === 0) continue;
      const loopName = n.name?.trim() || n.id;
      steps.forEach((s, k) => {
        if (s.kind !== 'check') return;
        const innerName = s.name?.trim() || `step${k}`;
        const step_id = `${n.id}.${k}`;
        const step_name = chooseStepLabel(loopName, innerName, checksInLoop, multiLoops);
        for (const c of s.params?.criteria ?? []) out.push({ ...c, step_id, step_name });
      });
    } else if (n.kind === 'check') {
      const step_name = n.name?.trim() || n.id;
      const criteria = (n.params as { criteria?: Criterion[] } | null)?.criteria ?? [];
      for (const c of criteria) out.push({ ...c, step_id: n.id, step_name });
    }
  }
  return out;
}

/** A `check` step's output (its criteria and verdict). */
export function isCheckOutput(v: unknown): v is CheckOutput {
  return !!v && typeof v === 'object' && Array.isArray((v as CheckOutput).criteria) && 'verdict' in v;
}

/** Stable check-step key for a loop checkpoint (`<loop_id>.<step_index>`). */
export function checkpointStepId(cp: Pick<WorkflowCheckpoint, 'loop_id' | 'step_index'>): string {
  return `${cp.loop_id}.${cp.step_index}`;
}

/** Each check step's newest settled attempt (keyed by `<loop_id>.<step_index>`).
 *  A checkpoint page may carry it without its output (`has_output`). */
export function newestCheckpointsByStep(checkpoints: readonly WorkflowCheckpoint[]): Map<string, WorkflowCheckpoint> {
  const best = new Map<string, WorkflowCheckpoint>();
  for (const cp of checkpoints) {
    if (cp.kind !== 'check' || cp.status !== 'success') continue;
    const key = checkpointStepId(cp);
    const cur = best.get(key);
    if (!cur || cp.iteration > cur.iteration) best.set(key, cp);
  }
  return best;
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

/** Whether the goal card reads the loop's checkpoints itself. A live run's
 *  page goes stale (the progress merge keeps the run read's first page while
 *  later iterations land); a settled run's read is whole — unless the card
 *  already read them while it ran, then once more for the last check. */
export function readsCheckpoints(run: { status: string; checkpoints?: readonly WorkflowCheckpoint[] }, readBefore: boolean): boolean {
  return !run.checkpoints?.length || readBefore || run.status === 'pending' || run.status === 'running';
}

/** Every check step's newest settled result, keyed by `step_id`
 *  (`<loop_id>.<step_index>` for inner checks, or top-level check `node_id`). */
export function checksByStep(
  nodes: readonly NodeRunState[],
  checkpoints?: readonly WorkflowCheckpoint[],
  criteria?: readonly (Criterion | ScopedCriterion)[],
): Map<string, CheckOutput> {
  const map = new Map<string, CheckOutput>();
  for (const [stepId, cp] of newestCheckpointsByStep(checkpoints ?? [])) {
    if (isCheckOutput(cp.output)) map.set(stepId, cp.output);
  }
  // Index known inner step_ids per loop so a settled loop's output.history
  // maps cleanly back to `<loop_id>.<step_index>` even without checkpoints.
  const loopSteps = new Map<string, string[]>();
  for (const c of criteria ?? []) {
    if (!('step_id' in c) || !c.step_id.includes('.')) continue;
    const loopId = c.step_id.slice(0, c.step_id.lastIndexOf('.'));
    const list = loopSteps.get(loopId) ?? [];
    if (!list.includes(c.step_id)) list.push(c.step_id);
    loopSteps.set(loopId, list);
  }
  for (const n of nodes) {
    if (isCheckOutput(n.output) && !map.has(n.node_id)) {
      map.set(n.node_id, n.output);
      continue;
    }
    const history = (n.output as { history?: { steps?: Record<string, unknown> }[] } | null | undefined)?.history;
    if (!history?.length) continue;
    const targetIds = loopSteps.get(n.node_id) ?? [`${n.node_id}.0`];
    for (const h of [...history].reverse()) {
      const checks = Object.values(h.steps ?? {}).filter(isCheckOutput);
      if (!checks.length) continue;
      if (checks.length === 1 && targetIds.length === 1) {
        if (!map.has(targetIds[0])) map.set(targetIds[0], checks[0]);
      } else {
        targetIds.forEach((sid, idx) => {
          const hit = checks[idx];
          if (hit && !map.has(sid)) map.set(sid, hit);
        });
      }
      if (targetIds.every((sid) => map.has(sid))) break;
    }
  }
  return map;
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

/** Build per-criterion history from check checkpoints, keyed by `${step_id}::${id}`. */
export function checkpointHistoryByCriterion(checkpoints?: readonly WorkflowCheckpoint[]): Map<string, CriterionPoint[]> {
  const byKey = new Map<string, Map<number, CriterionPoint>>();
  for (const cp of checkpoints ?? []) {
    if (cp.kind !== 'check' || cp.status !== 'success' || !isCheckOutput(cp.output)) continue;
    const stepId = checkpointStepId(cp);
    for (const r of cp.output.criteria) {
      const k = criterionKey(stepId, r.id);
      let rounds = byKey.get(k);
      if (!rounds) {
        rounds = new Map();
        byKey.set(k, rounds);
      }
      rounds.set(cp.iteration, { iteration: cp.iteration, met: r.met, value: r.progress?.value });
    }
  }
  const out = new Map<string, CriterionPoint[]>();
  for (const [k, rounds] of byKey) {
    const pts = [...rounds.values()].sort((a, b) => a.iteration - b.iteration);
    out.set(k, pts.length > MAX_HISTORY ? pts.slice(-MAX_HISTORY) : pts);
  }
  return out;
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

function isCheckMap(v: CheckOutput | ReadonlyMap<string, CheckOutput> | null): v is ReadonlyMap<string, CheckOutput> {
  return v instanceof Map;
}

/** One row per criterion, in the check node's order (keyed by `(step_id, id)`
 *  when multiple check steps exist, so identical ids across loops never share state). */
export function criterionRows(
  criteria: readonly (Criterion | ScopedCriterion)[],
  check: CheckOutput | ReadonlyMap<string, CheckOutput> | null,
  awaiting: readonly string[] = [],
  goal?: Pick<GoalProgress, 'criteria'>,
  checkpoints?: readonly WorkflowCheckpoint[],
): CriterionRow[] {
  const goalByKey = new Map<string, GoalCriterion>();
  for (const gc of goal?.criteria ?? []) goalByKey.set(criterionKey(gc.step_id, gc.id), gc);
  const cpHistory = checkpointHistoryByCriterion(checkpoints);

  const lookupResult = (stepId: string, id: string): CriterionResult | undefined => {
    if (!check) return undefined;
    if (isCheckMap(check)) return check.get(stepId)?.criteria.find((r) => r.id === id);
    return check.criteria.find((r) => r.id === id);
  };

  if (criteria.length) {
    return criteria.map((c) => {
      const step_id = 'step_id' in c ? c.step_id : '';
      const step_name = 'step_name' in c ? c.step_name : '';
      const key = criterionKey(step_id, c.id);
      const r = lookupResult(step_id, c.id);
      const gc = goalByKey.get(key) ?? (!step_id ? goal?.criteria?.find((x) => x.id === c.id) : undefined);
      const isAwaiting = gc ? Boolean(gc.awaiting) : awaiting.includes(c.id);
      const state: CriterionState = isAwaiting
        ? 'verify'
        : r
          ? r.met
            ? 'met'
            : 'not_met'
          : gc?.met != null
            ? gc.met
              ? 'met'
              : 'not_met'
            : 'pending';
      const progress = r?.progress ?? gc?.progress;
      const fromCp = cpHistory.get(key) ?? [];
      const fromGoal = gc?.history ?? [];
      const history = fromCp.length > fromGoal.length ? fromCp : fromGoal;
      return {
        key,
        step_id,
        step_name: step_name || gc?.step_name || '',
        id: c.id,
        text: c.text || r?.text || gc?.text || c.id,
        method: c.verify,
        state,
        ...splitEvidence(r?.evidence ?? ''),
        by: r?.verified_by ?? '',
        progress,
        history,
        ...(c.expr ? { expr: c.expr } : {}),
      };
    });
  }

  // Fallback when graph criteria were not provided: build rows from the check output(s) or overview goal.
  if (isCheckMap(check)) {
    const rows: CriterionRow[] = [];
    for (const [step_id, co] of check) {
      for (const r of co.criteria) {
        const key = criterionKey(step_id, r.id);
        const gc = goalByKey.get(key);
        const isAwaiting = gc ? Boolean(gc.awaiting) : awaiting.includes(r.id);
        const state: CriterionState = isAwaiting ? 'verify' : r.met ? 'met' : 'not_met';
        rows.push({
          key,
          step_id,
          step_name: gc?.step_name ?? '',
          id: r.id,
          text: r.text || r.id,
          method: r.verify,
          state,
          ...splitEvidence(r.evidence ?? ''),
          by: r.verified_by ?? '',
          progress: r.progress ?? gc?.progress,
          history: (cpHistory.get(key)?.length ?? 0) > (gc?.history?.length ?? 0) ? (cpHistory.get(key) ?? []) : (gc?.history ?? []),
        });
      }
    }
    if (rows.length) return rows;
  } else if (check) {
    return check.criteria.map((r) => {
      const state: CriterionState = awaiting.includes(r.id) ? 'verify' : r.met ? 'met' : 'not_met';
      return {
        key: criterionKey('', r.id),
        step_id: '',
        step_name: '',
        id: r.id,
        text: r.text || r.id,
        method: r.verify,
        state,
        ...splitEvidence(r.evidence ?? ''),
        by: r.verified_by ?? '',
        progress: r.progress,
        history: [],
      };
    });
  }

  return (goal?.criteria ?? []).map((gc) => {
    const key = criterionKey(gc.step_id, gc.id);
    const state: CriterionState = gc.awaiting ? 'verify' : gc.met == null ? 'pending' : gc.met ? 'met' : 'not_met';
    return {
      key,
      step_id: gc.step_id,
      step_name: gc.step_name ?? '',
      id: gc.id,
      text: gc.text || gc.id,
      method: 'expr',
      state,
      headline: '',
      detail: '',
      output: '',
      by: '',
      progress: gc.progress,
      history: gc.history ?? [],
    };
  });
}

/** Format a number cleanly: integers without decimals (`72`), fractions to 1 decimal (`9.5`). */
export function formatNum(v: number): string {
  if (!Number.isFinite(v)) return '0';
  if (Math.abs(v - Math.round(v)) < 1e-9) return String(Math.round(v));
  return v.toFixed(1).replace(/\.0$/, '');
}

/** Format `value / target` (with optional unit), e.g. `"72 / 95"` or `"64% / 80%"`. */
export function formatRatio(p?: CriterionProgress): string {
  if (!p || p.value == null || !Number.isFinite(p.value)) return '';
  const v = formatNum(p.value);
  const unit = p.unit?.trim() || '';
  if (p.target != null && Number.isFinite(p.target)) {
    const t = formatNum(p.target);
    if (unit === '%') return `${v}% / ${t}%`;
    return unit ? `${v} / ${t} ${unit}` : `${v} / ${t}`;
  }
  if (unit === '%') return `${v}%`;
  return unit ? `${v} ${unit}` : v;
}

/** True when a rule expression compares a metric against a ceiling (`<=`, `<`)
 *  or exact number (`== <num>`), so `value / target` would be misleading as a
 *  directional progress bar (e.g. `latency <= 200` at 300). */
export function isCeilingOrEqualityRule(expr?: string): boolean {
  if (!expr) return false;
  // `path <= 200`, `path < 5`, `path == 0`, `path != 0`, or flipped `200 >= path`, `5 > path`, `200 == path`, `0 != path`.
  return (
    /(?:<=|<|==|!=)\s*-?\d+(?:\.\d+)?\b/.test(expr) ||
    /\b-?\d+(?:\.\d+)?\s*(?:>=|>|==|!=)\s*[a-zA-Z_]/.test(expr)
  );
}

/** Meter fill percentage (0..100) for a directional floor numeric target, or
 *  `null` when no positive target exists or the rule is a ceiling/equality rule. */
export function progressPct(p?: CriterionProgress, state?: CriterionState, expr?: string): number | null {
  if (!p || p.value == null || p.target == null || !Number.isFinite(p.value) || !Number.isFinite(p.target) || p.target <= 0) {
    return null;
  }
  if (isCeilingOrEqualityRule(expr)) return null;
  if (state === 'met') return 100;
  return Math.max(0, Math.min(100, Math.round((p.value / p.target) * 100)));
}

/** Compact trend across the last three rounds with numeric values (e.g. `"35 → 58 → 72"`). */
export function recentTrend(history: readonly CriterionPoint[]): string {
  const nums = history.filter((pt) => pt.value != null && Number.isFinite(pt.value)).map((pt) => formatNum(pt.value!));
  if (nums.length < 2) return '';
  return nums.slice(-3).join(' → ');
}

/** Compact `key = value` pairs from `progress.values` for the evidence expander. */
export function observedPairs(p?: CriterionProgress): { key: string; value: string }[] {
  if (!p?.values) return [];
  return Object.entries(p.values).map(([key, val]) => ({
    key,
    value: typeof val === 'string' ? val : JSON.stringify(val),
  }));
}

/** The card's "Criteria m of t met", counted from the rows it lists so the
 *  line and the table never disagree; the run's own counts when it lists none. */
export function metCount(rows: readonly Pick<CriterionRow, 'state'>[], goal: { criteria_met: number; criteria_total: number }): { met: number; total: number } {
  if (!rows.length) return { met: goal.criteria_met, total: goal.criteria_total };
  return { met: rows.filter((r) => r.state === 'met').length, total: rows.length };
}

/** A goal's active time: the loop's own once it stopped; while it runs (the
 *  run says 0 then), what its settled attempts banked. */
export function activeMs(goal: { active_ms: number }, checkpoints: readonly Pick<WorkflowCheckpoint, 'active_ms'>[] = []): number {
  return goal.active_ms || checkpoints.reduce((sum, c) => sum + (c.active_ms ?? 0), 0);
}

export const STATE_WORD: Record<CriterionState, string> = { met: 'Met', not_met: 'Not met', verify: 'Verify', pending: 'Not checked' };
