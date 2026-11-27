// The run page's steps in EXECUTION order (ux.md §4.2): started steps by
// start time, never-started ones in the topological order of the run's graph
// version; steps whose run intervals overlap form one "In parallel" group
// (never an ancestor and its descendant); a fan-out's children in reading
// order; the phone's fold of the done steps before the current one; and the
// pane's default selection per run state (§4.4). The ring (runOrbit.ts) is
// built from the same entries, so the two never disagree. Pure; unit-tested in
// ui/unit/workflowStepOrder.test.ts.
import type { ChildThread, ChildThreadStatus, NodeRunState, RunState, RuntimeSession, WorkflowCheckpoint, WorkflowGraph } from '../../../lib/api/types';

/** A step's time bar inside the run's clock, in percent; clamped so a retry
 *  that ran past the clock or a skewed start never leaves the track. */
export function timeBar(s: StepItem, clock: { start: number; end: number }): { at: number; w: number } | null {
  if (s.start === null || s.end === null) return null;
  const end = Number.isFinite(s.end) ? s.end : clock.end;
  const span = Math.max(1, clock.end - clock.start);
  const at = Math.min(99.4, Math.max(0, ((s.start - clock.start) / span) * 100));
  return { at, w: Math.min(100 - at, Math.max(0.6, ((end - s.start) / span) * 100)) };
}

/** The pseudo-rows the pane can open besides a step. */
export const RESULT = '_result';

export interface StepItem {
  id: string;
  node: NodeRunState;
  /** The author's name (the node id when the graph no longer has it). */
  name: string;
  kind: string;
  /** Epoch ms; null before the step started. */
  start: number | null;
  /** Settled, or `now` while it runs; null before it started. */
  end: number | null;
}

export type StepEntry = { type: 'step'; step: StepItem } | { type: 'group'; steps: StepItem[] };

/** Steps of an entry, in order. */
export function entrySteps(e: StepEntry): StepItem[] {
  return e.type === 'step' ? [e.step] : e.steps;
}

/** Node ids in topological order (Kahn), ties in the graph's own order. */
export function topoOrder(graph: WorkflowGraph): string[] {
  const ids = graph.nodes.map((n) => n.id);
  const indeg = new Map(ids.map((id) => [id, 0]));
  for (const e of graph.edges) if (indeg.has(e.target) && indeg.has(e.source)) indeg.set(e.target, indeg.get(e.target)! + 1);
  const out: string[] = [];
  const ready = ids.filter((id) => indeg.get(id) === 0);
  while (ready.length) {
    const id = ready.shift()!;
    out.push(id);
    for (const e of graph.edges) {
      if (e.source !== id || !indeg.has(e.target)) continue;
      const d = indeg.get(e.target)! - 1;
      indeg.set(e.target, d);
      if (d === 0) {
        ready.push(e.target);
        ready.sort((a, b) => ids.indexOf(a) - ids.indexOf(b));
      }
    }
  }
  // A cycle (loop edges) leaves nodes behind: keep them in graph order.
  for (const id of ids) if (!out.includes(id)) out.push(id);
  return out;
}

function ancestorsOf(graph: WorkflowGraph): (id: string) => Set<string> {
  const memo = new Map<string, Set<string>>();
  const visit = (id: string, seen: Set<string>): Set<string> => {
    const hit = memo.get(id);
    if (hit) return hit;
    const acc = new Set<string>();
    for (const e of graph.edges) {
      if (e.target !== id || seen.has(e.source)) continue;
      acc.add(e.source);
      for (const a of visit(e.source, new Set(seen).add(id))) acc.add(a);
    }
    memo.set(id, acc);
    return acc;
  };
  return (id) => visit(id, new Set());
}

/** A step's number for "Step 3 of 6": the engine's own, else (a run from
 *  before steps were numbered) its place in the run's order. */
export function stepNumber(steps: readonly StepItem[], step: StepItem): number {
  return step.node.step_no ?? steps.indexOf(step) + 1;
}

/** A settled run has nothing running: a step an older engine left `running`
 *  (a run a daemon restart interrupted) reads as stopped — no spinner, no
 *  live timer — everywhere the page shows it. */
export function settledNodes(nodes: readonly NodeRunState[], live: boolean): readonly NodeRunState[] {
  if (live) return nodes;
  return nodes.map((n): NodeRunState => (n.status === 'running' ? { ...n, status: 'skipped', error_class: 'canceled', phase: undefined, wait: undefined, attention: undefined, activity: undefined } : n));
}

/** The run's steps as display entries (§4.2). `now` defaults to Infinity so a
 *  running step overlaps any later sibling without re-running on every 1s clock tick;
 *  `timeBar` resolves non-finite ends against the run's live clock. */
export function orderSteps(nodes: readonly NodeRunState[], graph: WorkflowGraph, now = Number.POSITIVE_INFINITY): StepEntry[] {
  const gnodes = new Map(graph.nodes.map((n) => [n.id, n]));
  const topo = topoOrder(graph);
  const rank = (id: string) => (topo.indexOf(id) + 1 || topo.length + 1);
  const ancestors = ancestorsOf(graph);
  const related = (a: string, b: string) => ancestors(a).has(b) || ancestors(b).has(a);
  const preds = (id: string) => graph.edges.filter((e) => e.target === id).map((e) => e.source);

  const items: StepItem[] = nodes.map((node) => {
    const g = gnodes.get(node.node_id);
    const start = node.started_at ? Date.parse(node.started_at) : NaN;
    const started = Number.isFinite(start);
    const end = !started ? null : node.status === 'running' ? now : start + (node.duration_ms ?? 0);
    return { id: node.node_id, node, name: g?.name || node.node_id, kind: g?.kind ?? '', start: started ? start : null, end };
  });
  const started = items.filter((s) => s.start !== null).sort((a, b) => a.start! - b.start! || rank(a.id) - rank(b.id));
  const waiting = items.filter((s) => s.start === null).sort((a, b) => rank(a.id) - rank(b.id));

  const entries: StepItem[][] = [];
  let groupEnd = -Infinity;
  for (const s of started) {
    const cur = entries[entries.length - 1];
    if (cur && s.start! < groupEnd && !cur.some((m) => related(m.id, s.id))) {
      cur.push(s);
      groupEnd = Math.max(groupEnd, s.end!);
    } else {
      entries.push([s]);
      groupEnd = s.end!;
    }
  }
  // A never-started step joins the parallel block of a sibling (one sharing
  // its predecessor): a pending one a RUNNING sibling's, a skipped one an
  // existing group's. Otherwise a skipped step follows its latest ancestor and
  // a pending one goes after every started step.
  const sibling = (m: StepItem, s: StepItem) => !related(m.id, s.id) && preds(m.id).some((p) => preds(s.id).includes(p));
  for (const s of waiting) {
    const pending = s.node.status === 'pending';
    const host = entries.find((g) => (pending ? g.some((m) => m.node.status === 'running' && sibling(m, s)) : g.length > 1 && g.some((m) => sibling(m, s))));
    if (host) {
      host.push(s);
      continue;
    }
    const anc = ancestors(s.id);
    const after = pending ? -1 : entries.reduce((at, g, i) => (g.some((m) => anc.has(m.id)) ? i : at), -1);
    if (after < 0) entries.push([s]);
    else entries.splice(after + 1, 0, [s]);
  }
  return entries.map((g) => (g.length === 1 ? { type: 'step', step: g[0] } : { type: 'group', steps: g }));
}

const isSummarizer = (c: Pick<ChildThread, 'id' | 'label'>) => /^summar/i.test(c.label) || c.id === 'summarizer';

/** What a step's child is, for the pane's crumb (ux.md §4.4, goal.md §3.5):
 *  a loop's iteration ("Work · iteration 2" from the engine, "Iteration 2 ·
 *  Work, Check" from the checkpoints) or a review's reviewer. Null for a row
 *  its own name already says: the summarizer, the scope assessment, an
 *  earlier turn, a thread the step's agent opened. */
export function childCrumb(stepKind: string | undefined, c: Pick<ChildThread, 'id' | 'label'>): string | null {
  const iteration = /^Iteration (\d+)\b/.exec(c.label) ?? /· iteration (\d+)$/.exec(c.label);
  if (iteration) return `Iteration ${iteration[1]}`;
  return stepKind === 'review_run' && !isSummarizer(c) && c.label !== 'Scope assessment' ? 'Reviewer' : null;
}

/** The noun of a fold of such children: "5 reviewers queued", "2 iterations done", "1 thread done". */
export function childNoun(stepKind: string | undefined, c: Pick<ChildThread, 'id' | 'label'>): string {
  const crumb = childCrumb(stepKind, c);
  return crumb === 'Reviewer' ? 'reviewer' : crumb ? 'iteration' : 'thread';
}

export interface ChildView {
  /** Working, waiting on a person and failed — first. */
  active: ChildThread[];
  queued: ChildThread[];
  done: ChildThread[];
  /** Findings the done children report ("14 findings" notes). */
  findings: number;
  /** The summarizer (or another trailing child), last. */
  trailing: ChildThread[];
}

/** A fan-out's children in reading order (§4.2 item 3). */
export function orderChildren(children: readonly ChildThread[]): ChildView {
  const view: ChildView = { active: [], queued: [], done: [], findings: 0, trailing: [] };
  for (const c of children) {
    const retried = c.status === 'error' && (c as StepChild).retried_attempt != null;
    if (isSummarizer(c)) view.trailing.push(c);
    else if (c.status === 'running' || c.status === 'needs_you' || (c.status === 'error' && !retried)) view.active.push(c);
    else if (c.status === 'queued' || c.status === 'pending') view.queued.push(c);
    else view.done.push(c);
  }
  for (const c of view.done) view.findings += Number(/(\d+) findings?/.exec(c.note ?? '')?.[1] ?? 0);
  return view;
}

const settled = (s: StepItem) => s.node.status === 'success' || s.node.status === 'skipped';

/** The phone's fold of the done steps before the first one that is not done:
 *  how many entries it hides and what its row says. Null under two entries. */
export function doneFold(entries: readonly StepEntry[]): { entries: number; done: number; skipped: number; ms: number } | null {
  let n = 0;
  while (n < entries.length && entrySteps(entries[n]).every(settled)) n++;
  if (n < 2 || n === entries.length) return null;
  const steps = entries.slice(0, n).flatMap(entrySteps);
  const ran = steps.filter((s) => s.start !== null);
  const ms = ran.length ? Math.max(...ran.map((s) => s.end!)) - Math.min(...ran.map((s) => s.start!)) : 0;
  return { entries: n, done: steps.filter((s) => s.node.status === 'success').length, skipped: steps.filter((s) => s.node.status === 'skipped').length, ms };
}

/** What the pane opens on when the person has not picked anything (§4.4). */
export function defaultSelection(state: RunState, entries: readonly StepEntry[], failedNodeId?: string): { node: string; child: string | null } {
  const steps = entries.flatMap(entrySteps);
  const trigger = steps.find((s) => s.kind.endsWith('_trigger')) ?? steps[0];
  const pick = (s: StepItem | undefined) => (s ? { node: s.id, child: null } : { node: RESULT, child: null });
  switch (state) {
    case 'running': {
      const s = steps.find((x) => x.node.status === 'running' && x.node.phase !== 'waiting' && x.node.phase !== 'queued')
        ?? steps.find((x) => x.node.status === 'running');
      // Container steps (review_run, loop) open their active child even when a
      // loop carries a thread; an agent step stays on its own conversation
      // even while its agent runs sub-threads.
      const child =
        s && (s.kind === 'review_run' || s.kind === 'loop' || !s.node.thread)
          ? s.node.children?.find((c) => c.status === 'running')
          : undefined;
      return s ? { node: s.id, child: child?.id ?? null } : pick(trigger);
    }
    case 'waiting':
      return pick(steps.find((x) => x.node.phase === 'waiting') ?? trigger);
    case 'queued':
      return pick(trigger);
    case 'needs_you':
      return pick(steps.find((x) => x.node.phase === 'needs_you' || (x.node.status === 'running' && x.node.attention)) ?? steps.find((x) => x.node.status === 'running'));
    case 'failed':
      return pick(steps.find((x) => x.id === failedNodeId) ?? steps.find((x) => x.node.status === 'error'));
    default:
      return { node: RESULT, child: null };
  }
}

/** A child row under a step: a reviewer, summarizer, loop iteration, or
 *  agent-opened sub-thread, optionally nested under another child row. */
export interface StepChild extends ChildThread {
  /** Nesting depth under the step (0 = direct child, 1 = grandchild, …). */
  depth?: number;
  /** Parent child id when nested (`depth > 0`); null/undefined for a direct child of the step. */
  parent_child_id?: string | null;
  /** 1-based attempt number when this failed row was followed by a later attempt of the same label. */
  retried_attempt?: number | null;
  /** Reason from checkpoint or node logs for a failed retried attempt, when available. */
  retried_reason?: string | null;
}

/** A live `RuntimeSession` status mapped to the step list's child status vocabulary. */
export function runtimeChildStatus(
  s: Pick<RuntimeSession, 'status' | 'last_turn_status' | 'background_tasks'>,
): ChildThreadStatus {
  if (s.status === 'awaiting_permission') return 'needs_you';
  if (s.status === 'working' || s.status === 'starting') return 'running';
  if (s.status !== 'closed' && (s.background_tasks?.length ?? 0) > 0) return 'running';
  if (s.last_turn_status === 'failed') return 'error';
  if (s.last_turn_status === 'interrupted') return 'skipped';
  if (s.last_turn_status === 'completed' || s.status === 'closed') return 'success';
  return s.status === 'dormant' ? 'skipped' : 'running';
}

function openerOf(s: Pick<RuntimeSession, 'meta'>): string | null {
  const co = s.meta?.child_of as { id?: unknown } | undefined;
  return typeof co?.id === 'string' && co.id ? co.id : null;
}

function providerSlugOf(s: Pick<RuntimeSession, 'provider' | 'meta'>): string {
  const slug = s.meta?.provider_slug;
  return typeof slug === 'string' && slug ? slug : s.provider;
}

/** Merge a step's recorded children (`NodeRunState.children`: reviewers,
 *  summarizer, loop iterations, settled sub-threads) with EVERY live/known
 *  runtime sub-thread belonging to that step's thread family (including
 *  multi-generation grandchildren), preserving parent→child nesting. */
export function mergeStepChildren(
  recorded: readonly ChildThread[],
  rootThreadId: string | null | undefined,
  sessions: readonly RuntimeSession[],
): StepChild[] {
  // A same-thread loop can record the same sub-thread across multiple iterations:
  // keep one canonical row per thread_id (or id) in first-seen order with the latest fields.
  const deduped: ChildThread[] = [];
  const seenKey = new Map<string, number>();
  for (const c of recorded) {
    const key = c.thread_id ? `t:${c.thread_id}` : `i:${c.id}`;
    const at = seenKey.get(key);
    if (at === undefined) {
      seenKey.set(key, deduped.length);
      deduped.push(c);
    } else {
      const prev = deduped[at];
      deduped[at] = { ...prev, ...c, id: prev.id };
    }
  }
  if (sessions.length === 0) return deduped.map((c) => ({ ...c, depth: 0, parent_child_id: null }));

  const bySessionId = new Map<string, RuntimeSession>();
  for (const s of sessions) bySessionId.set(s.id, s);

  const rootIds = new Set<string>();
  if (rootThreadId) rootIds.add(rootThreadId);
  const threadToChildId = new Map<string, string>();
  for (const c of deduped) {
    if (c.thread_id) {
      rootIds.add(c.thread_id);
      threadToChildId.set(c.thread_id, c.id);
    }
  }
  if (rootIds.size === 0) return deduped.map((c) => ({ ...c, depth: 0, parent_child_id: null }));

  const belongsToStep = (s: RuntimeSession): boolean => {
    if (rootThreadId && s.id === rootThreadId) return false;
    if (s.parent_id && rootIds.has(s.parent_id)) return true;
    const seen = new Set<string>([s.id]);
    let cur: string | null = openerOf(s);
    while (cur && !seen.has(cur)) {
      if (rootIds.has(cur)) return true;
      seen.add(cur);
      const next = bySessionId.get(cur);
      if (!next) return false;
      if (next.parent_id && rootIds.has(next.parent_id)) return true;
      cur = openerOf(next);
    }
    return false;
  };

  const family = sessions.filter(belongsToStep).sort((a, b) => a.created_at.localeCompare(b.created_at));
  const out: StepChild[] = [];
  const recordedIds = new Set<string>();
  const recordedThreadIds = new Set<string>();

  for (const c of deduped) {
    recordedIds.add(c.id);
    if (c.thread_id) recordedThreadIds.add(c.thread_id);
    const live = c.thread_id ? bySessionId.get(c.thread_id) : bySessionId.get(c.id);
    const opener = live ? openerOf(live) : null;
    // If an opener thread is also an explicit child row (e.g. a goal loop
    // mirroring its active iteration thread on `node.thread`), keep that child
    // row as the parent instead of collapsing to the bare step root.
    const parentChild = opener
      ? (threadToChildId.get(opener) ?? (opener !== rootThreadId && bySessionId.has(opener) ? opener : null))
      : null;
    out.push({
      ...c,
      ...(live && !c.note && c.status === 'running' ? { status: runtimeChildStatus(live) } : {}),
      parent_child_id: parentChild && parentChild !== c.id ? parentChild : null,
    });
  }

  for (const s of family) {
    if (recordedIds.has(s.id) || recordedThreadIds.has(s.id)) continue;
    const opener = openerOf(s);
    const parentChild = opener
      ? (threadToChildId.get(opener) ??
          (opener !== rootThreadId && (family.some((f) => f.id === opener) || recordedIds.has(opener)) ? opener : null))
      : null;
    const started = typeof s.meta?.last_turn_started_at === 'string' && s.meta.last_turn_started_at ? s.meta.last_turn_started_at : s.created_at;
    out.push({
      id: s.id,
      label: s.title || providerSlugOf(s),
      provider: providerSlugOf(s),
      status: runtimeChildStatus(s),
      thread_id: s.id,
      ...(s.model ? { model: s.model } : {}),
      ...(s.effort ? { effort: s.effort } : {}),
      ...(started ? { started_at: started } : {}),
      parent_child_id: parentChild && parentChild !== s.id ? parentChild : null,
    });
  }

  const byId = new Map(out.map((c) => [c.id, c] as const));
  for (const c of out) {
    let depth = 0;
    const seen = new Set<string>([c.id]);
    let pid = c.parent_child_id ?? null;
    while (pid && !seen.has(pid) && byId.has(pid) && depth < 8) {
      seen.add(pid);
      depth++;
      pid = byId.get(pid)!.parent_child_id ?? null;
    }
    if (!c.parent_child_id || !byId.has(c.parent_child_id)) c.parent_child_id = null;
    c.depth = depth;
  }
  return out;
}

/** Structural equality for merged step children so unrelated runtime-session
 *  ticks (e.g. `last_active_at` on the step's own thread) don't churn node
 *  references and re-render the step pane. */
export function sameStepChildren(a: readonly StepChild[], b: readonly StepChild[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    if (
      x.id !== y.id ||
      x.label !== y.label ||
      x.provider !== y.provider ||
      x.status !== y.status ||
      x.note !== y.note ||
      x.thread_id !== y.thread_id ||
      x.model !== y.model ||
      x.effort !== y.effort ||
      x.started_at !== y.started_at ||
      x.ended_at !== y.ended_at ||
      x.depth !== y.depth ||
      x.parent_child_id !== y.parent_child_id ||
      x.retried_attempt !== y.retried_attempt ||
      x.retried_reason !== y.retried_reason
    ) {
      return false;
    }
  }
  return true;
}

/** True when every non-`children` enumerable property on `base` still matches
 *  `cached`. Reads all keys of `base` so reactive proxies register dependencies
 *  and in-place `applyNodeSummary` mutations (status, phase, attention, deleted
 *  fields) invalidate any spread wrapper copy. */
export function sameNodeFields(base: NodeRunState, cached: NodeRunState): boolean {
  if (base === cached) return true;
  const baseRec = base as unknown as Record<string, unknown>;
  const cachedRec = cached as unknown as Record<string, unknown>;
  const baseKeys = Object.keys(baseRec);
  const cachedKeys = Object.keys(cachedRec);
  let baseCount = 0;
  for (const k of baseKeys) {
    if (k === 'children') continue;
    baseCount++;
    if (!Object.prototype.hasOwnProperty.call(cachedRec, k)) return false;
    if (!Object.is(baseRec[k], cachedRec[k])) return false;
  }
  let cachedCount = 0;
  for (const k of cachedKeys) {
    if (k === 'children') continue;
    cachedCount++;
  }
  return baseCount === cachedCount;
}

export interface MergedNodeCacheEntry {
  children: StepChild[];
  node: NodeRunState;
}

/** Reuse the previous merged child array across unrelated runtime-session ticks
 *  while never masking in-place mutations to the authoritative node fields. */
export function reuseMergedNode(
  base: NodeRunState,
  annotated: StepChild[],
  prev: MergedNodeCacheEntry | undefined,
): MergedNodeCacheEntry {
  if (annotated.length === 0 && !base.children?.length) {
    return { children: annotated, node: base };
  }
  const sameChildren = !!prev && sameStepChildren(prev.children, annotated);
  const stableChildren = sameChildren ? prev.children : annotated;
  if (sameChildren && sameNodeFields(base, prev.node)) {
    return prev;
  }
  const node = stableChildren !== base.children ? { ...base, children: stableChildren } : base;
  return { children: stableChildren, node };
}

/** Parse `"attempt N: <reason>"` from a checkpoint or node log list. */
function attemptReasonFromLogs(
  logs: readonly string[] | undefined,
  attempt: number,
  prefix?: { iteration: number; name: string } | null,
): string | null {
  if (!logs?.length) return null;
  const tag = prefix ? `[${prefix.iteration}/${prefix.name}]` : null;
  const re = new RegExp(`^attempt\\s+${attempt}:\\s*(.+)$`, 'i');
  for (const raw of logs) {
    let line = raw.trim();
    if (tag) {
      if (!line.startsWith(tag)) continue;
      line = line.slice(tag.length).trim();
    }
    const m = re.exec(line);
    if (m?.[1]?.trim()) return m[1].trim();
  }
  return null;
}

function parseIterationLabel(label: string): { stepName: string; iteration: number | null } {
  const iterMatch = /^(.+?)\s*·\s*iteration\s+(\d+)$/i.exec(label.trim());
  return iterMatch
    ? { stepName: iterMatch[1].trim(), iteration: Number(iterMatch[2]) }
    : { stepName: label.trim(), iteration: null };
}

/** Check whether checkpoint state or loop-scoped attempt logs prove that a
 *  failed child row was an earlier retried attempt of that checkpoint (as
 *  opposed to two independent agent-opened threads sharing a title/provider). */
function checkpointRetryEvidence(
  label: string,
  attempt: number,
  opts: { loopId?: string; checkpoints?: readonly WorkflowCheckpoint[]; nodeLogs?: readonly string[] },
): { confirmed: boolean; reason: string | null } {
  const { stepName, iteration } = parseIterationLabel(label);
  let confirmed = false;
  let reason: string | null = null;
  for (const cp of opts.checkpoints ?? []) {
    if (opts.loopId && cp.loop_id !== opts.loopId) continue;
    if (cp.name !== stepName) continue;
    if (iteration !== null) {
      if (cp.iteration !== iteration) continue;
    } else if ((cp.iteration ?? 0) > 0) {
      // A bare label without `· iteration N` cannot claim a loop iteration checkpoint.
      continue;
    }
    const cpReason = attemptReasonFromLogs(cp.logs, attempt);
    if (cpReason) {
      confirmed = true;
      if (!reason) reason = cpReason;
    }
    if ((cp.attempts ?? 1) > attempt) {
      confirmed = true;
    }
  }
  if (iteration !== null) {
    const prefixed = attemptReasonFromLogs(opts.nodeLogs, attempt, { iteration, name: stepName });
    if (prefixed) {
      confirmed = true;
      if (!reason) reason = prefixed;
    }
  }
  return { confirmed, reason };
}

/** Extract the failure reason for a retried child attempt from matching
 *  `checkpoints[].logs` (first) or the parent step's `logs` (fallback). */
export function retriedAttemptReason(
  label: string,
  attempt: number,
  opts: { loopId?: string; checkpoints?: readonly WorkflowCheckpoint[]; nodeLogs?: readonly string[] } = {},
): string | null {
  const { stepName, iteration } = parseIterationLabel(label);
  for (const cp of opts.checkpoints ?? []) {
    if (opts.loopId && cp.loop_id !== opts.loopId) continue;
    if (cp.name !== stepName) continue;
    if (iteration !== null && cp.iteration !== iteration) continue;
    const reason = attemptReasonFromLogs(cp.logs, attempt);
    if (reason) return reason;
  }
  if (iteration !== null) {
    const prefixed = attemptReasonFromLogs(opts.nodeLogs, attempt, { iteration, name: stepName });
    if (prefixed) return prefixed;
  }
  return attemptReasonFromLogs(opts.nodeLogs, attempt);
}

/** Mark failed top-level child rows that have checkpoint or loop-attempt-log
 *  evidence of being followed by a later retry attempt of the same checkpoint
 *  (`retried_attempt`, plus `retried_reason` when available). Independent
 *  agent-opened sub-threads that merely share a label keep their normal failure
 *  status, and the last attempt for any label always keeps normal wording. */
export function annotateRetriedChildren(
  children: readonly StepChild[],
  opts: { loopId?: string; checkpoints?: readonly WorkflowCheckpoint[]; nodeLogs?: readonly string[] } = {},
): StepChild[] {
  if (children.length < 2) return children as StepChild[];
  const groups = new Map<string, number[]>();
  for (let i = 0; i < children.length; i++) {
    const c = children[i];
    if (c.parent_child_id) continue;
    const list = groups.get(c.label);
    if (list) list.push(i);
    else groups.set(c.label, [i]);
  }
  let changed = false;
  const out = children.slice();
  for (const idxs of groups.values()) {
    if (idxs.length < 2) continue;
    for (let pos = 0; pos < idxs.length - 1; pos++) {
      const idx = idxs[pos];
      const c = out[idx];
      if (c.status !== 'error') continue;
      const attempt = pos + 1;
      const { confirmed, reason } = checkpointRetryEvidence(c.label, attempt, opts);
      if (!confirmed) continue;
      out[idx] = { ...c, retried_attempt: attempt, ...(reason ? { retried_reason: reason } : {}) };
      changed = true;
    }
  }
  return changed ? out : (children as StepChild[]);
}

export interface NestedChildEntry {
  child: StepChild;
  depth: number;
  /** Direct parent child's label when nested (`depth > 0`); null for direct children of the step. */
  parentLabel: string | null;
}

export interface NestedChildView {
  active: NestedChildEntry[];
  queued: NestedChildEntry[];
  done: NestedChildEntry[];
  findings: number;
  trailing: NestedChildEntry[];
}

/** Order a step's children (same buckets as {@link orderChildren}) while
 *  nesting each child's descendants immediately below it, one level per
 *  generation. A finished parent with a live descendant stays in `active` so
 *  working sub-threads are never hidden inside the done fold. */
export function orderChildTree(children: readonly StepChild[]): NestedChildView {
  const byId = new Map(children.map((c) => [c.id, c] as const));
  const kidsOf = new Map<string, StepChild[]>();
  const roots: StepChild[] = [];
  for (const c of children) {
    const pid = c.parent_child_id;
    if (pid && pid !== c.id && byId.has(pid)) {
      const list = kidsOf.get(pid);
      if (list) list.push(c);
      else kidsOf.set(pid, [c]);
    } else {
      roots.push(c);
    }
  }
  const isRetried = (c: StepChild): boolean => c.status === 'error' && c.retried_attempt != null;
  const hasLiveDescendant = (id: string, seen = new Set<string>()): boolean => {
    for (const k of kidsOf.get(id) ?? []) {
      if (seen.has(k.id)) continue;
      seen.add(k.id);
      if (k.status === 'running' || k.status === 'needs_you' || (k.status === 'error' && !isRetried(k))) return true;
      if (hasLiveDescendant(k.id, seen)) return true;
    }
    return false;
  };
  const expand = (root: StepChild): NestedChildEntry[] => {
    const res: NestedChildEntry[] = [];
    const seen = new Set<string>();
    const visit = (c: StepChild, depth: number, parentLabel: string | null): void => {
      if (seen.has(c.id) || depth > 8) return;
      seen.add(c.id);
      res.push({ child: c, depth, parentLabel });
      for (const k of kidsOf.get(c.id) ?? []) visit(k, depth + 1, c.label);
    };
    visit(root, 0, null);
    return res;
  };

  const view: NestedChildView = { active: [], queued: [], done: [], findings: 0, trailing: [] };
  for (const r of roots) {
    const branch = expand(r);
    if (isSummarizer(r)) view.trailing.push(...branch);
    else if (r.status === 'running' || r.status === 'needs_you' || (r.status === 'error' && !isRetried(r)) || hasLiveDescendant(r.id)) view.active.push(...branch);
    else if (r.status === 'queued' || r.status === 'pending') view.queued.push(...branch);
    else view.done.push(...branch);
  }
  for (const e of view.done) view.findings += Number(/(\d+) findings?/.exec(e.child.note ?? '')?.[1] ?? 0);
  return view;
}

/** Resolve a thread id clicked inside the pane ("Open parent" / sub-thread card)
 *  to a child id (or `null` for the bare step root). Explicit child rows win
 *  over `rootThreadId` so a goal loop whose `node.thread` mirrors its active
 *  iteration row selects the iteration conversation instead of the loop's Criteria tab. */
export function resolveThreadSelection(
  children: readonly Pick<StepChild, 'id' | 'thread_id'>[],
  rootThreadId: string | null | undefined,
  threadId: string,
): string | null {
  const match = children.find((c) => c.id === threadId || c.thread_id === threadId);
  if (match) return match.id;
  if (rootThreadId && threadId === rootThreadId) return null;
  return threadId;
}

/** Ancestor trail `[topChild, …, currentChild]` for the pane breadcrumb (empty when not found). */
export function childTrail(children: readonly StepChild[], childId: string | null | undefined): StepChild[] {
  if (!childId) return [];
  const cur = children.find((c) => c.id === childId || c.thread_id === childId);
  if (!cur) return [];
  const byId = new Map(children.map((c) => [c.id, c] as const));
  const chain: StepChild[] = [];
  const seen = new Set<string>();
  let node: StepChild | undefined = cur;
  while (node && !seen.has(node.id) && chain.length < 9) {
    seen.add(node.id);
    chain.push(node);
    node = node.parent_child_id ? byId.get(node.parent_child_id) : undefined;
  }
  return chain.reverse();
}

/** A loop step's iterations as child rows (goal.md §3.5), newest first —
 *  "Iteration 3 · Work, Check" — so the list and the ring show one entry per
 *  iteration like a fan-out. An iteration's state comes from its steps; it
 *  opens the conversation of its last step that had a thread. */
export function loopChildren(checkpoints: readonly WorkflowCheckpoint[], loopId: string): ChildThread[] {
  const byIter = new Map<number, WorkflowCheckpoint[]>();
  for (const c of checkpoints) if (c.loop_id === loopId) byIter.set(c.iteration, [...(byIter.get(c.iteration) ?? []), c]);
  return [...byIter.entries()].sort((a, b) => b[0] - a[0]).map(([n, steps]) => {
    steps.sort((a, b) => a.step_index - b.step_index);
    const st = steps.map((s) => s.status);
    const status: ChildThreadStatus = st.includes('error') ? 'error' : st.includes('running') ? 'running' : st.every((s) => s === 'success' || s === 'skipped') ? 'success' : 'pending';
    const thread = [...steps].reverse().find((s) => s.thread_id)?.thread_id;
    const ended = steps.map((s) => s.updated_at).sort().at(-1);
    return {
      id: `iter-${n}`, label: `Iteration ${n} · ${steps.map((s) => s.name).join(', ')}`, provider: '', status,
      ...(thread ? { thread_id: thread } : {}), ...(status !== 'running' && ended ? { ended_at: ended } : {}),
    };
  });
}

export {
  groupLoopRounds,
  isWaitingOnSubagents,
  loopPositionLine,
  waitingReviewers,
  waitingThreads,
  type LoopRound,
  type LoopRoundStep,
  type LoopRoundsView,
  type WaitingThreadRef,
} from './loopRounds.ts';
