// The editor's step rules (pure): which earlier step's thread a step may
// continue, which continuers clash, where "+ Step" wires a new step, and the
// review step's per-reviewer model picks (ux.md §9.3; engine.md §4.5;
// runtime.md §7.3 / agreed-names.md §4).
import type { WorkflowEdge, WorkflowGraph, WorkflowNode, WorkflowValidationIssue } from '../../../lib/api/types';
import { providerName } from './nodeSummary.ts';

type Params = Record<string, unknown>;
const str = (v: unknown): string => (typeof v === 'string' ? v.trim() : '');

/** The provider a step runs on (empty pick = the workspace default). */
export function stepProvider(node: WorkflowNode, defaultProvider: string): string {
  return str((node.params as Params | null)?.provider) || defaultProvider;
}

/** Every step `id` comes after (its ancestors along the edges). */
export function ancestors(graph: WorkflowGraph, id: string): Set<string> {
  const seen = new Set<string>();
  const stack = [id];
  while (stack.length) {
    const cur = stack.pop()!;
    for (const e of graph.edges) if (e.target === cur && !seen.has(e.source)) (seen.add(e.source), stack.push(e.source));
  }
  return seen;
}

/** An agent's pick; empty strings = Default. */
export interface StepModelValue {
  provider: string;
  model: string;
  effort: string;
}

export interface ContinueChoice {
  node: WorkflowNode;
  /** Why it can't be picked (another provider), else null. */
  reason: string | null;
}

/** The earlier agent steps a step may continue: ancestors that are agent
 *  steps, the ones on another provider listed with the reason. */
export function continueChoices(graph: WorkflowGraph, node: WorkflowNode, defaultProvider: string): ContinueChoice[] {
  const before = ancestors(graph, node.id);
  const mine = stepProvider(node, defaultProvider);
  return graph.nodes
    .filter((n) => before.has(n.id) && n.kind === 'agent_prompt')
    .map((n) => {
      const theirs = stepProvider(n, defaultProvider);
      return {
        node: n,
        reason: theirs === mine ? null : `${n.name || n.id} is a ${providerName(theirs)} step — pick a ${providerName(mine)} step`,
      };
    });
}

/** Steps that continue one thread and can run at the same time (neither
 *  comes after the other): the save names them. */
export function continueClashes(graph: WorkflowGraph): string[] {
  const out: string[] = [];
  const byName = (id: string): string => graph.nodes.find((n) => n.id === id)?.name || id;
  const continuers = graph.nodes.filter((n) => str((n.params as Params | null)?.continue_from));
  for (let i = 0; i < continuers.length; i++) {
    for (let j = i + 1; j < continuers.length; j++) {
      const a = continuers[i];
      const b = continuers[j];
      const src = str((a.params as Params).continue_from);
      if (src !== str((b.params as Params).continue_from)) continue;
      if (ancestors(graph, a.id).has(b.id) || ancestors(graph, b.id).has(a.id)) continue;
      out.push(`${a.name || a.id} and ${b.name || b.id} both continue ${byName(src)}’s thread and can run at the same time. Make one of them start a new thread.`);
    }
  }
  return out;
}

/** "+ Step": `added` goes after `after` (a step id; null = the graph has no
 *  step to follow) and takes over its unconditional outgoing edges — a
 *  conditional edge reads its own step's output, so it stays where it is. */
export function wireAfter(edges: readonly WorkflowEdge[], after: string | null, added: string, newId: (s: string, t: string) => string): WorkflowEdge[] {
  if (!after) return [...edges];
  const moved = edges.map((e) => (e.source === after && !e.condition ? { ...e, source: added, id: newId(added, e.target) } : e));
  return [...moved, { id: newId(after, added), source: after, target: added }];
}

/** The engine's step-file slug (otto_workflows::context::slug): lowercase,
 *  [a-z0-9-], ≤ 40 characters, "step" when nothing is left. */
export function slug(name: string): string {
  const s = name.toLowerCase().replace(/[^a-z0-9]/g, '-').replace(/-+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40).replace(/^-+|-+$/g, '');
  return s || 'step';
}

/** The file a step writes when every step before it runs: `step{N}-{slug}.md`,
 *  N its place in the engine's order (Kahn, sources in graph order, successors
 *  in edge order — workflow_engine `topo_order`), the trigger being step 1. */
export function stepFile(graph: WorkflowGraph, id: string): string | null {
  const indeg = new Map(graph.nodes.map((n) => [n.id, 0]));
  for (const e of graph.edges) if (indeg.has(e.source) && indeg.has(e.target)) indeg.set(e.target, indeg.get(e.target)! + 1);
  const queue = graph.nodes.filter((n) => indeg.get(n.id) === 0).map((n) => n.id);
  const order: string[] = [];
  while (queue.length) {
    const cur = queue.shift()!;
    order.push(cur);
    for (const e of graph.edges) {
      if (e.source !== cur || !indeg.has(e.target)) continue;
      const left = indeg.get(e.target)! - 1;
      indeg.set(e.target, left);
      if (left === 0) queue.push(e.target);
    }
  }
  const at = order.indexOf(id);
  const node = graph.nodes.find((n) => n.id === id);
  return at < 0 || !node ? null : `step${at + 1}-${slug(node.name || node.kind)}.md`;
}

// --- review_run: reviewer picks -------------------------------------------

export interface ModelPick {
  model?: string;
  effort?: string;
}
export interface ReviewerParams {
  lens?: string;
  providers?: string[];
  model?: string;
  effort?: string;
  picks?: Record<string, ModelPick>;
  instructions?: string;
}

/** One reviewer row's own pick for `provider`: its `picks[provider]`, else
 *  the row's model/effort when `provider` is the lens's primary (first)
 *  provider — the review engine's `resolve_pick` (agreed-names §4). */
export function reviewerPick(row: ReviewerParams, provider: string): ModelPick {
  const own = row.picks?.[provider];
  if (own) return own;
  return (row.providers ?? [])[0] === provider ? { model: row.model, effort: row.effort } : {};
}

/** What a reviewer of `provider` runs on: its own pick, else the review-wide
 *  default for that provider, else the provider's default (empty). */
export function resolvedReviewerPick(row: ReviewerParams, provider: string, nodePicks: Record<string, ModelPick> | undefined): ModelPick {
  const own = reviewerPick(row, provider);
  if (own.model || own.effort) return own;
  return nodePicks?.[provider] ?? {};
}

const cleanPick = (p: ModelPick): ModelPick | null => {
  const out: ModelPick = {};
  if (p.model) out.model = p.model;
  if (p.effort) out.effort = p.effort;
  return Object.keys(out).length ? out : null;
};

/** The row with `picks[provider]` set (an empty pick removes the key). */
export function withReviewerPick(row: ReviewerParams, provider: string, pick: ModelPick): ReviewerParams {
  const picks = { ...(row.picks ?? {}) };
  const clean = cleanPick(pick);
  if (clean) picks[provider] = clean;
  else delete picks[provider];
  const { picks: _old, ...rest } = row;
  // The primary provider's legacy model/effort would win over an empty pick.
  if ((row.providers ?? [])[0] === provider) {
    delete rest.model;
    delete rest.effort;
  }
  return Object.keys(picks).length ? { ...rest, picks } : rest;
}

/** The review-wide `picks` with `provider` set (empty → removed; none → undefined). */
export function withNodePick(picks: Record<string, ModelPick> | undefined, provider: string, pick: ModelPick): Record<string, ModelPick> | undefined {
  const next = { ...(picks ?? {}) };
  const clean = cleanPick(pick);
  if (clean) next[provider] = clean;
  else delete next[provider];
  return Object.keys(next).length ? next : undefined;
}

/** Whether an issue stops a save or a run (DR-028): a save only on a `save`
 *  issue (a draft keeps its missing pieces), a run on `save` and `run`, a
 *  warning never. */
export function issueBlocks(issue: WorkflowValidationIssue, at: 'save' | 'run'): boolean {
  return issue.class === 'save' || (at === 'run' && issue.class === 'run');
}

/** Skills a step names that are not installed for its provider. */
export function missingSkills(skills: readonly string[], installed: ReadonlySet<string> | null): string[] {
  return installed ? skills.filter((s) => !installed.has(s)) : [];
}
