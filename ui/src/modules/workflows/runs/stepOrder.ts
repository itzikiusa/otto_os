// The run page's steps in EXECUTION order (ux.md §4.2): started steps by
// start time, never-started ones in the topological order of the run's graph
// version; steps whose run intervals overlap form one "In parallel" group
// (never an ancestor and its descendant); a fan-out's children in reading
// order; the phone's fold of the done steps before the current one; and the
// pane's default selection per run state (§4.4). The ring (runOrbit.ts) is
// built from the same entries, so the two never disagree. Pure; unit-tested in
// ui/unit/workflowStepOrder.test.ts.
import type { ChildThread, NodeRunState, RunState, WorkflowGraph } from '../../../lib/api/types';

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

/** The run's steps as display entries (§4.2). */
export function orderSteps(nodes: readonly NodeRunState[], graph: WorkflowGraph, now: number): StepEntry[] {
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

const isSummarizer = (c: ChildThread) => /^summar/i.test(c.label) || c.id === 'summarizer';

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
    if (isSummarizer(c)) view.trailing.push(c);
    else if (c.status === 'running' || c.status === 'needs_you' || c.status === 'error') view.active.push(c);
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
      const child = s?.node.children?.find((c) => c.status === 'running');
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
