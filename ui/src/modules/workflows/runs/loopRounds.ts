// Loop round grouping, loop position lines, and named waiting threads for the
// run page (re-exported from stepOrder.ts; unit-tested in
// ui/unit/workflowStepOrder.test.ts). Pure.
import type {
  ChildThread,
  ChildThreadStatus,
  GoalProgress,
  LoopLimits,
  NodeRunState,
  WorkflowCheckpoint,
  WorkflowNode,
} from '../../../lib/api/types';
import { formatRatio } from '../goal/goalCheck.ts';
import { orderChildTree, type NestedChildView, type StepChild, type StepItem } from './stepOrder.ts';

/** A running thread or sub-agent a step (or summarizer) is currently waiting on. */
export interface WaitingThreadRef {
  /** Child id to pass to `onselect(stepId, childId)`, or null when only known from `activity.subagents`. */
  childId: string | null;
  label: string;
  provider: string;
  startedAt?: string;
}

/** One inner step inside a loop round (review, check, fix, Work, …). */
export interface LoopRoundStep {
  key: string;
  stepIndex: number;
  kind: string;
  name: string;
  status: ChildThreadStatus;
  /** Short status/result summary (e.g. `"72 / 95"`, `"9 of 12 reviewers done"`, `"not met"`). */
  summary: string;
  /** Primary child id to open when clicking this inner step row, when it maps to a thread. */
  childId: string | null;
  /** Ordered children nested under this inner step (reviewers + summarizer under review; attempts + sub-threads under fix). */
  children: NestedChildView;
  /** All children belonging to this inner step. */
  allChildren: StepChild[];
}

/** One round of a loop node (`Round 3 of 5`), grouping its inner steps and nested children. */
export interface LoopRound {
  iteration: number;
  maxIterations: number | null;
  /** True for the active/current round (open by default and highlighted). */
  current: boolean;
  status: ChildThreadStatus;
  /** Score/target ratio for this round (e.g. `"72 / 95"`), from goal progress or check/review output. */
  scoreText: string;
  /** Compact one-line summary for a folded earlier round (e.g. `"35 / 95 · 68 comments · fixed"`). */
  summary: string;
  /** Inner steps in definition order (review, check, fix, …). */
  steps: LoopRoundStep[];
}

/** Round-grouped view of a `loop` step's checkpoints and children. */
export interface LoopRoundsView {
  maxIterations: number | null;
  currentIteration: number | null;
  rounds: LoopRound[];
  /** Any children that could not be attributed to a round. */
  ungrouped: StepChild[];
}

const isSummarizer = (c: Pick<ChildThread, 'id' | 'label'>) => /^summar/i.test(c.label) || c.id === 'summarizer';
const isScopeAssessment = (c: Pick<ChildThread, 'label'>) => c.label.trim().toLowerCase() === 'scope assessment';
const isReviewerRow = (c: Pick<ChildThread, 'id' | 'label'>) => !isSummarizer(c) && !isScopeAssessment(c);

function reviewPrefixOf(id: string): string | null {
  const m = /^(.+)\.\d+$/.exec(id);
  if (!m) return null;
  const prefix = m[1];
  if (!prefix || prefix.startsWith('iter-')) return null;
  return prefix;
}

function logIterationPair(logs: readonly string[] | undefined): [number, number] | null {
  if (!logs?.length) return null;
  for (let i = logs.length - 1; i >= 0; i--) {
    const m = /^🔁 iteration (\d+)\/(\d+)/.exec(logs[i].trim());
    if (m) return [Number(m[1]), Number(m[2])];
  }
  return null;
}

function cpStatusToChild(status: string): ChildThreadStatus {
  if (status === 'success' || status === 'error' || status === 'running' || status === 'skipped' || status === 'pending') {
    return status;
  }
  return 'pending';
}

function formatScorePair(score: unknown, threshold: unknown): string {
  if (typeof score !== 'number' || !Number.isFinite(score)) return '';
  if (typeof threshold === 'number' && Number.isFinite(threshold)) {
    return formatRatio({ value: score, target: threshold });
  }
  return `${Math.round(score)}/100`;
}

function roundScoreFromGoal(goal: GoalProgress | null | undefined, loopId: string, iteration: number): string {
  if (!goal?.criteria?.length) return '';
  const prefix = `${loopId}.`;
  for (const gc of goal.criteria) {
    if (!gc.step_id.startsWith(prefix)) continue;
    const pt = gc.history?.find((p) => p.iteration === iteration);
    if (pt?.value != null && Number.isFinite(pt.value)) {
      const ratio = formatRatio({ value: pt.value, target: gc.progress?.target, unit: gc.progress?.unit });
      if (ratio) return ratio;
    }
  }
  return '';
}

function roundScoreFromCheck(cp: WorkflowCheckpoint | undefined): string {
  const out = cp?.output as { criteria?: { progress?: { value?: number; target?: number; unit?: string } }[] } | null | undefined;
  for (const r of out?.criteria ?? []) {
    const ratio = formatRatio(r.progress);
    if (ratio) return ratio;
  }
  return '';
}

function checkSummaryFromCp(cp: WorkflowCheckpoint | undefined): string {
  const out = cp?.output as {
    met?: number;
    total?: number;
    all_met?: boolean;
    criteria?: { met?: boolean; progress?: { value?: number; target?: number; unit?: string } }[];
  } | null | undefined;
  if (!out || typeof out !== 'object') return '';
  const ratio = roundScoreFromCheck(cp);
  if (out.all_met === true) return ratio ? `${ratio} · met` : 'met';
  if (ratio) return `${ratio} · not met`;
  if (typeof out.met === 'number' && typeof out.total === 'number' && out.total > 0) {
    return `${out.met} of ${out.total} met`;
  }
  return '';
}

function reviewFindingsCount(
  cp: WorkflowCheckpoint | undefined,
  view: NestedChildView,
): { count: number; word: 'comments' | 'findings' } | null {
  const out = cp?.output as { open?: unknown; total?: unknown } | null | undefined;
  const open = typeof out?.open === 'number' && Number.isFinite(out.open) ? out.open : null;
  const total = typeof out?.total === 'number' && Number.isFinite(out.total) ? out.total : null;
  const n = open ?? total;
  if (n !== null && n > 0) return { count: n, word: 'comments' };
  const sum = [...view.done, ...view.active].reduce(
    (acc, e) => acc + Number(/(\d+) findings?/.exec(e.child.note ?? '')?.[1] ?? 0),
    0,
  );
  return sum > 0 ? { count: sum, word: 'findings' } : null;
}

/** Group a `loop` step's checkpoints and children into per-round blocks in
 *  ascending round order, with inner steps (review, check, fix, …) in their
 *  definition order and each review's reviewers + summarizer nested under that
 *  round's review. Returns `null` for non-loop steps or loops with no rounds. */
export function groupLoopRounds(
  step: Pick<StepItem, 'id' | 'kind' | 'node'>,
  opts: {
    graphNode?: Pick<WorkflowNode, 'id' | 'kind' | 'params'> | null;
    checkpoints?: readonly WorkflowCheckpoint[];
    goal?: GoalProgress | null;
    limits?: LoopLimits | null;
  } = {},
): LoopRoundsView | null {
  if (step.kind !== 'loop') return null;

  const rawSteps = (opts.graphNode?.params as { steps?: { kind?: string; name?: string }[]; max_iterations?: number } | null | undefined)?.steps ?? [];
  const templateSteps = rawSteps.map((s, idx) => ({
    stepIndex: idx,
    kind: typeof s.kind === 'string' ? s.kind : '',
    name: typeof s.name === 'string' && s.name.trim() ? s.name.trim() : `step${idx}`,
  }));

  const logPair = logIterationPair(step.node.logs);
  const outObj = (step.node.output ?? null) as { max_iterations?: number; iterations?: number } | null;
  const paramMax = (opts.graphNode?.params as { max_iterations?: number } | null | undefined)?.max_iterations;
  const maxIterations: number | null =
    opts.limits?.max_iterations ??
    (typeof outObj?.max_iterations === 'number' && outObj.max_iterations > 0 ? outObj.max_iterations : null) ??
    (logPair?.[1] ? logPair[1] : null) ??
    (typeof paramMax === 'number' && paramMax > 0 ? paramMax : null) ??
    (opts.goal?.max_iterations ? opts.goal.max_iterations : null);

  const loopCps = (opts.checkpoints ?? []).filter((cp) => cp.loop_id === step.id);
  const reviewAssocRe = new RegExp(`^${step.id.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}#(\\d+)\\.(\\d+)#review\\.(\\d+)$`);

  const reviewToSlot = new Map<string, { iteration: number; stepIndex: number }>();
  const threadToSlot = new Map<string, { iteration: number; stepIndex: number }>();
  const cpBySlot = new Map<string, WorkflowCheckpoint>();
  const iterations = new Set<number>();

  for (const cp of loopCps) {
    const assoc = reviewAssocRe.exec(cp.node_id);
    if (assoc) {
      const iter = Number(assoc[1]);
      const sidx = Number(assoc[2]);
      if (iter > 0) iterations.add(iter);
      const rid = (cp.output as { review_id?: unknown } | null | undefined)?.review_id;
      if (typeof rid === 'string' && rid) reviewToSlot.set(rid, { iteration: iter, stepIndex: sidx });
      continue;
    }
    if (cp.iteration <= 0) continue;
    iterations.add(cp.iteration);
    cpBySlot.set(`${cp.iteration}:${cp.step_index}`, cp);
    if (cp.thread_id) threadToSlot.set(cp.thread_id, { iteration: cp.iteration, stepIndex: cp.step_index });
    const out = cp.output as { review_id?: unknown; reviews?: { review_id?: unknown }[] } | null | undefined;
    if (typeof out?.review_id === 'string' && out.review_id) {
      reviewToSlot.set(out.review_id, { iteration: cp.iteration, stepIndex: cp.step_index });
    }
    for (const r of out?.reviews ?? []) {
      if (typeof r?.review_id === 'string' && r.review_id) {
        reviewToSlot.set(r.review_id, { iteration: cp.iteration, stepIndex: cp.step_index });
      }
    }
  }

  const children = (step.node.children ?? []) as StepChild[];
  const byId = new Map(children.map((c) => [c.id, c] as const));
  const rootOf = (c: StepChild): StepChild => {
    let cur = c;
    const seen = new Set<string>([cur.id]);
    while (cur.parent_child_id && !seen.has(cur.parent_child_id) && byId.has(cur.parent_child_id)) {
      seen.add(cur.parent_child_id);
      cur = byId.get(cur.parent_child_id)!;
    }
    return cur;
  };

  // Fallback mapping for review_ids when checkpoint outputs were not loaded yet:
  // match each unique review_id prefix in launch/seen order to successive rounds' review_run step.
  const reviewStepIdx = templateSteps.find((s) => s.kind === 'review_run')?.stepIndex ?? 0;
  const seenReviews: string[] = [];
  for (const rid of step.node.review_ids ?? []) {
    if (rid && !seenReviews.includes(rid)) seenReviews.push(rid);
  }
  for (const c of children) {
    const rid = reviewPrefixOf(rootOf(c).id);
    if (rid && !seenReviews.includes(rid)) seenReviews.push(rid);
  }
  const usedReviewIters = new Set<number>();
  for (const slot of reviewToSlot.values()) {
    if (slot.stepIndex === reviewStepIdx) usedReviewIters.add(slot.iteration);
  }
  let nextIter = 1;
  for (const rid of seenReviews) {
    if (reviewToSlot.has(rid)) continue;
    while (usedReviewIters.has(nextIter)) nextIter++;
    reviewToSlot.set(rid, { iteration: nextIter, stepIndex: reviewStepIdx });
    usedReviewIters.add(nextIter);
    iterations.add(nextIter);
    nextIter++;
  }

  const iterLabelRe = /^(.+?)\s*·\s*iteration\s+(\d+)$/i;
  const synthIterRe = /^Iteration\s+(\d+)\b/i;
  for (const c of children) {
    const r = rootOf(c);
    const m = iterLabelRe.exec(r.label.trim());
    if (m) iterations.add(Number(m[2]));
    else {
      const sm = synthIterRe.exec(r.label.trim());
      if (sm) iterations.add(Number(sm[1]));
    }
  }
  for (const gc of opts.goal?.criteria ?? []) {
    if (!gc.step_id.startsWith(`${step.id}.`)) continue;
    for (const pt of gc.history ?? []) if (pt.iteration > 0) iterations.add(pt.iteration);
  }
  if (logPair?.[0] && step.node.status === 'running') iterations.add(logPair[0]);

  const sortedIters = [...iterations].sort((a, b) => a - b);
  if (sortedIters.length === 0) return null;

  const childrenBySlot = new Map<string, StepChild[]>();
  const extraStepMeta = new Map<string, { name: string; kind: string }>();
  const ungrouped: StepChild[] = [];
  const latestIter = sortedIters[sortedIters.length - 1];

  const findStepIndexByName = (iter: number, name: string): number => {
    const lower = name.toLowerCase();
    for (const cp of loopCps) {
      if (cp.iteration === iter && !reviewAssocRe.test(cp.node_id) && cp.name.toLowerCase() === lower) {
        return cp.step_index;
      }
    }
    const tmpl = templateSteps.find((t) => t.name.toLowerCase() === lower);
    if (tmpl) return tmpl.stepIndex;
    let idx = Math.max(templateSteps.length, 10);
    for (const [k, meta] of extraStepMeta) {
      if (k.startsWith(`${iter}:`) && meta.name.toLowerCase() === lower) return Number(k.split(':')[1]);
      if (k.startsWith(`${iter}:`)) idx = Math.max(idx, Number(k.split(':')[1]) + 1);
    }
    extraStepMeta.set(`${iter}:${idx}`, { name, kind: 'agent_prompt' });
    return idx;
  };

  const pushSlot = (iter: number, stepIndex: number, c: StepChild) => {
    const k = `${iter}:${stepIndex}`;
    const list = childrenBySlot.get(k);
    if (list) list.push(c);
    else childrenBySlot.set(k, [c]);
  };

  for (const c of children) {
    const root = rootOf(c);
    const rid = reviewPrefixOf(root.id);
    if (rid && reviewToSlot.has(rid)) {
      const slot = reviewToSlot.get(rid)!;
      pushSlot(slot.iteration, slot.stepIndex, c);
      continue;
    }
    if (root.thread_id && threadToSlot.has(root.thread_id)) {
      const slot = threadToSlot.get(root.thread_id)!;
      pushSlot(slot.iteration, slot.stepIndex, c);
      continue;
    }
    const m = iterLabelRe.exec(root.label.trim());
    if (m) {
      const iter = Number(m[2]);
      const sidx = findStepIndexByName(iter, m[1].trim());
      pushSlot(iter, sidx, c);
      continue;
    }
    const sm = synthIterRe.exec(root.label.trim());
    if (sm) {
      const iter = Number(sm[1]);
      pushSlot(iter, 0, c);
      continue;
    }
    if (step.node.status === 'running') {
      const runningCp = [...cpBySlot.values()].find((cp) => cp.iteration === latestIter && cp.status === 'running');
      const sidx = runningCp?.step_index ?? templateSteps.find((t) => t.kind === 'agent_prompt')?.stepIndex ?? 0;
      pushSlot(latestIter, sidx, c);
      continue;
    }
    ungrouped.push(c);
  }

  let currentIteration = latestIter;
  if (step.node.status === 'running') {
    for (let i = sortedIters.length - 1; i >= 0; i--) {
      const iter = sortedIters[i];
      const hasLiveCp = [...cpBySlot.values()].some((cp) => cp.iteration === iter && cp.status === 'running');
      const hasLiveKid = [...childrenBySlot.entries()].some(
        ([k, kids]) => k.startsWith(`${iter}:`) && kids.some((c) => c.status === 'running' || c.status === 'needs_you'),
      );
      if (hasLiveCp || hasLiveKid || logPair?.[0] === iter) {
        currentIteration = iter;
        break;
      }
    }
  }

  const rounds: LoopRound[] = sortedIters.map((iteration) => {
    const isCurrent = iteration === currentIteration;
    const stepIndices = new Set<number>();
    for (const [k] of cpBySlot) if (k.startsWith(`${iteration}:`)) stepIndices.add(Number(k.split(':')[1]));
    for (const [k] of childrenBySlot) if (k.startsWith(`${iteration}:`)) stepIndices.add(Number(k.split(':')[1]));
    if (isCurrent) for (const t of templateSteps) stepIndices.add(t.stepIndex);
    const orderedIndices = [...stepIndices].sort((a, b) => a - b);

    let reviewFindings: { count: number; word: 'comments' | 'findings' } | null = null;
    let reviewScoreFallback = '';
    let checkScoreFallback = '';
    let fixStatus: ChildThreadStatus | null = null;
    let checkPassed = false;

    const steps: LoopRoundStep[] = [];
    for (const stepIndex of orderedIndices) {
      const slotKey = `${iteration}:${stepIndex}`;
      const cp = cpBySlot.get(slotKey);
      const kids = childrenBySlot.get(slotKey) ?? [];
      const tmpl = templateSteps.find((t) => t.stepIndex === stepIndex);
      const extra = extraStepMeta.get(slotKey);
      const kind = cp?.kind || tmpl?.kind || extra?.kind || (kids.some((c) => reviewPrefixOf(c.id)) ? 'review_run' : 'agent_prompt');
      const name = cp?.name || tmpl?.name || extra?.name || (kind === 'review_run' ? 'review' : kind === 'check' ? 'check' : `step ${stepIndex + 1}`);
      const tree = orderChildTree(kids);

      const hasRunning = kids.some((c) => c.status === 'running');
      const hasNeedsYou = kids.some((c) => c.status === 'needs_you');
      const hasUnretriedError = kids.some((c) => c.status === 'error' && c.retried_attempt == null);
      const status: ChildThreadStatus = hasNeedsYou
        ? 'needs_you'
        : hasRunning
          ? 'running'
          : cp
            ? cpStatusToChild(cp.status)
            : hasUnretriedError
              ? 'error'
              : kids.length > 0 && kids.every((c) => c.status === 'success' || c.status === 'skipped' || (c.status === 'error' && c.retried_attempt != null))
                ? 'success'
                : 'pending';

      let summary = '';
      if (kind === 'review_run') {
        const reviewers = kids.filter((c) => (c.depth ?? 0) === 0 && isReviewerRow(c));
        const doneRev = reviewers.filter((c) => c.status === 'success' || c.status === 'error' || c.status === 'skipped').length;
        const summ = tree.trailing[0]?.child;
        const out = cp?.output as { score?: unknown; threshold?: unknown } | null | undefined;
        reviewScoreFallback = formatScorePair(out?.score, out?.threshold) || reviewScoreFallback;
        reviewFindings = reviewFindingsCount(cp, tree) ?? reviewFindings;
        if (status === 'running') {
          if (summ?.status === 'running') summary = 'summarizing';
          else if (reviewers.length > 0) summary = `${doneRev} of ${reviewers.length} reviewers done`;
        } else if (reviewScoreFallback) {
          summary = reviewFindings ? `${reviewScoreFallback} · ${reviewFindings.count} ${reviewFindings.word}` : reviewScoreFallback;
        } else if (reviewFindings) {
          summary = `${reviewFindings.count} ${reviewFindings.word}`;
        }
      } else if (kind === 'check') {
        checkScoreFallback = roundScoreFromCheck(cp) || checkScoreFallback;
        summary = checkSummaryFromCp(cp);
        const out = cp?.output as { all_met?: unknown } | null | undefined;
        if (out?.all_met === true) checkPassed = true;
      } else if (name.toLowerCase().startsWith('fix')) {
        fixStatus = status;
      }

      const directKids = kids.filter((c) => (c.depth ?? 0) === 0);
      const activeDirect = directKids.find((c) => c.status === 'running' || c.status === 'needs_you');
      const latestDirect = directKids.filter((c) => c.retried_attempt == null).at(-1) ?? directKids.at(-1);
      const childId =
        kind === 'review_run'
          ? (activeDirect?.id ?? tree.trailing[0]?.child.id ?? latestDirect?.id ?? null)
          : (activeDirect?.id ?? latestDirect?.id ?? null);

      steps.push({
        key: `${step.id}#${iteration}.${stepIndex}`,
        stepIndex,
        kind,
        name,
        status,
        summary,
        childId,
        children: tree,
        allChildren: kids,
      });
    }

    const roundStatus: ChildThreadStatus = steps.some((st) => st.status === 'needs_you')
      ? 'needs_you'
      : steps.some((st) => st.status === 'running')
        ? 'running'
        : steps.some((st) => st.status === 'error')
          ? 'error'
          : steps.length > 0 && steps.every((st) => st.status === 'success' || st.status === 'skipped')
            ? 'success'
            : 'pending';

    const scoreText = roundScoreFromGoal(opts.goal, step.id, iteration) || checkScoreFallback || reviewScoreFallback;
    const summaryParts: string[] = [];
    if (scoreText) summaryParts.push(scoreText);
    if (reviewFindings) summaryParts.push(`${reviewFindings.count} ${reviewFindings.word}`);
    if (checkPassed) summaryParts.push('passed');
    else if (fixStatus === 'success') summaryParts.push('fixed');
    else if (fixStatus === 'error') summaryParts.push('fix failed');
    else if (fixStatus === 'running') summaryParts.push('fixing');
    if (summaryParts.length === 0) {
      summaryParts.push(steps.map((st) => st.name).join(', '));
    }

    return {
      iteration,
      maxIterations,
      current: isCurrent,
      status: roundStatus,
      scoreText,
      summary: summaryParts.join(' · '),
      steps,
    };
  });

  return {
    maxIterations,
    currentIteration,
    rounds,
    ungrouped,
  };
}

/** A running loop's position in words for its header line:
 *  `"Round 3 of 5 · reviewing (9 of 12 reviewers done)"`, `"Round 3 of 5 · fixing"`,
 *  `"Round 3 of 5 · checking"`, `"Round 3 of 5 · summarizing"`. */
export function loopPositionLine(step: Pick<StepItem, 'kind' | 'node'>, view: LoopRoundsView | null): string {
  if (step.kind !== 'loop' || step.node.status !== 'running' || !view || view.rounds.length === 0) return '';
  const cur = view.rounds.find((r) => r.current) ?? view.rounds[view.rounds.length - 1];
  const prefix = cur.maxIterations ? `Round ${cur.iteration} of ${cur.maxIterations}` : `Round ${cur.iteration}`;
  const active = cur.steps.find((st) => st.status === 'running' || st.status === 'needs_you') ?? cur.steps.at(-1);
  if (!active) return prefix;

  if (active.kind === 'review_run') {
    const summ = active.children.trailing[0]?.child;
    if (summ?.status === 'running') return `${prefix} · summarizing`;
    const reviewers = active.allChildren.filter((c) => (c.depth ?? 0) === 0 && isReviewerRow(c));
    if (reviewers.length > 0) {
      const done = reviewers.filter((c) => c.status === 'success' || c.status === 'error' || c.status === 'skipped').length;
      return `${prefix} · reviewing (${done} of ${reviewers.length} reviewers done)`;
    }
    return `${prefix} · reviewing`;
  }
  if (active.kind === 'check') return `${prefix} · checking`;
  const lower = active.name.toLowerCase();
  if (lower.startsWith('fix')) return `${prefix} · fixing`;
  if (lower.startsWith('work')) return `${prefix} · working`;
  if (lower.startsWith('review')) return `${prefix} · reviewing`;
  return `${prefix} · ${active.name}`;
}

/** Whether a running step is explicitly waiting on sub-agents / child threads
 *  (e.g. `"Finishing — waiting on 3 sub-agent(s)"` or live `activity.subagents`). */
export function isWaitingOnSubagents(node: Pick<NodeRunState, 'status' | 'phase' | 'activity'>): boolean {
  if (node.status !== 'running') return false;
  const phaseText = node.activity?.phase?.toLowerCase() ?? '';
  if (phaseText.includes('sub-agent') || phaseText.includes('subagent') || phaseText.includes('waiting on ')) return true;
  if ((node.activity?.subagents ?? []).some((s) => s.status === 'running')) return true;
  return node.phase === 'finishing' && (node.activity?.pending_tasks ?? 0) > 0;
}

/** Threads / sub-agents a step is currently waiting on: running or needs_you
 *  child rows (each linkable by `childId`), plus any running `activity.subagents`
 *  not already matched to a child row. */
export function waitingThreads(
  node: Pick<NodeRunState, 'activity' | 'children'>,
  opts: { scopeChildren?: readonly StepChild[]; excludeSummarizer?: boolean } = {},
): WaitingThreadRef[] {
  const source = (opts.scopeChildren ?? node.children ?? []) as StepChild[];
  const out: WaitingThreadRef[] = [];
  const seenIds = new Set<string>();
  const seenLabels = new Set<string>();

  for (const c of source) {
    if (c.status !== 'running' && c.status !== 'needs_you') continue;
    if (opts.excludeSummarizer && isSummarizer(c)) continue;
    out.push({
      childId: c.id,
      label: c.label,
      provider: c.provider,
      ...(c.started_at ? { startedAt: c.started_at } : {}),
    });
    seenIds.add(c.id);
    if (c.thread_id) seenIds.add(c.thread_id);
    seenLabels.add(c.label.toLowerCase());
  }

  for (const sa of node.activity?.subagents ?? []) {
    if (sa.status !== 'running') continue;
    if (seenIds.has(sa.id) || seenLabels.has(sa.description.toLowerCase())) continue;
    const match = source.find((c) => c.id === sa.id || c.thread_id === sa.id);
    out.push({
      childId: match?.id ?? null,
      label: sa.description || sa.id,
      provider: match?.provider ?? '',
      ...(sa.started_at ? { startedAt: sa.started_at } : {}),
    });
  }
  return out;
}

/** Reviewers still running or queued ahead of a review's summarizer. */
export function waitingReviewers(children: readonly StepChild[]): { running: WaitingThreadRef[]; queued: number } {
  const running: WaitingThreadRef[] = [];
  let queued = 0;
  for (const c of children) {
    if ((c.depth ?? 0) !== 0 || !isReviewerRow(c)) continue;
    if (c.status === 'running' || c.status === 'needs_you') {
      running.push({
        childId: c.id,
        label: c.label,
        provider: c.provider,
        ...(c.started_at ? { startedAt: c.started_at } : {}),
      });
    } else if (c.status === 'queued' || c.status === 'pending') {
      queued++;
    }
  }
  return { running, queued };
}
