// Node-inspector helpers (pure): read a node's params for the per-kind forms,
// and build the next value a form writes back through the page's `onParam`.
// Split out of WorkflowsPage.svelte with the forms (inspector/<kind>.svelte).
import type { BrokerCluster, Connection, IssueAccount, TopicSummary, WorkflowNode } from '../../../lib/api/types';
import type { ProductStory } from '../../../lib/api/types/product';
import { AUTO_VERTICAL_ENGINES } from '../../../lib/db-view-prefs.ts';
import { envTone } from '../../../lib/status.ts';

type Params = Record<string, unknown>;

/** One choice in an id picker (inspector/IdPicker.svelte). */
export interface IdOption {
  id: string;
  label: string;
  title?: string;
}

/** Writes one param of the selected node (the page's `onParam`: empty → removed). */
export type SetParam = (field: string, value: unknown) => void;

function params(node: WorkflowNode | null | undefined): Params | undefined {
  return node?.params as Params | undefined;
}

export function paramStr(node: WorkflowNode | null | undefined, field: string): string {
  const v = params(node)?.[field];
  return typeof v === 'string' ? v : v != null ? String(v) : '';
}

export function paramNum(node: WorkflowNode | null | undefined, field: string, def: number): number {
  const v = params(node)?.[field];
  return typeof v === 'number' ? v : typeof v === 'string' && v !== '' ? Number(v) : def;
}

export function paramJson(node: WorkflowNode | null | undefined, field: string): string {
  const v = params(node)?.[field];
  if (v == null) return '';
  return typeof v === 'string' ? v : JSON.stringify(v, null, 2);
}

export function paramBool(node: WorkflowNode | null | undefined, field: string, def = false): boolean {
  const v = params(node)?.[field];
  return typeof v === 'boolean' ? v : def;
}

/** A string[] param rendered one-per-line (e.g. review goals). */
export function paramLines(node: WorkflowNode | null | undefined, field: string): string {
  const v = params(node)?.[field];
  if (Array.isArray(v)) return v.filter((x) => typeof x === 'string').join('\n');
  return typeof v === 'string' ? v : '';
}

/** A string[] param rendered comma-separated (e.g. skills, providers, lenses). */
export function paramList(node: WorkflowNode | null | undefined, field: string): string {
  const v = params(node)?.[field];
  if (Array.isArray(v)) return v.filter((x) => typeof x === 'string').join(', ');
  return typeof v === 'string' ? v : '';
}

/** The node's params with `field` set — an empty value removes the key. */
export function withParam(current: unknown, field: string, value: unknown): Params {
  const next = { ...((current as Params) ?? {}) };
  if (value === '' || value === null || value === undefined) delete next[field];
  else next[field] = value;
  return next;
}

/** A JSON field's typed text: the parsed value, or the raw string while it
 *  isn't valid JSON (the user sees what they typed; the engine rejects it on run). */
export function jsonValue(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
}

/** One-per-line text → string[] (blank lines dropped; none → removed). */
export function linesValue(raw: string): string[] | undefined {
  const lines = raw.split('\n').map((s) => s.trim()).filter((s) => s !== '');
  return lines.length ? lines : undefined;
}

/** Comma-separated text → string[] (none → removed). */
export function listValue(raw: string): string[] | undefined {
  const items = raw.split(',').map((s) => s.trim()).filter((s) => s !== '');
  return items.length ? items : undefined;
}

/** `list` with `item` toggled in or out. */
export function toggled(list: readonly string[], item: string): string[] {
  return list.includes(item) ? list.filter((x) => x !== item) : [...list, item];
}

// --- review_run rich config: per-lens reviewers + summarizer + scoring -----
// Mirrors the PR-review config (each lens gets its own provider set + optional
// custom instructions), plus a generic per-severity scoring guideline.
export interface ReviewerRow {
  lens?: string;
  providers?: string[];
  instructions?: string;
}

export function reviewers(node: WorkflowNode | null | undefined): ReviewerRow[] {
  const p = params(node);
  return Array.isArray(p?.reviewers) ? (p?.reviewers as ReviewerRow[]) : [];
}

export function summarizerField(node: WorkflowNode | null | undefined, field: 'provider' | 'instructions'): string {
  const s = params(node)?.summarizer as Params | undefined;
  const v = s?.[field];
  return typeof v === 'string' ? v : '';
}

/** The `summarizer` object with `field` set (empty → removed; nothing left → undefined). */
export function withSummarizer(node: WorkflowNode | null | undefined, field: string, value: string): Params | undefined {
  const next = { ...((params(node)?.summarizer as Params) ?? {}) };
  if (value === '') delete next[field];
  else next[field] = value;
  return Object.keys(next).length ? next : undefined;
}

export function scoringField(node: WorkflowNode | null | undefined, sev: 'bug' | 'warn' | 'info', def: number): number {
  const s = params(node)?.scoring as Params | undefined;
  const v = s?.[sev];
  return typeof v === 'number' ? v : def;
}

export function withScoring(node: WorkflowNode | null | undefined, sev: string, value: number): Params {
  return { ...((params(node)?.scoring as Params) ?? {}), [sev]: value };
}

// --- loop sub-steps: each step is a node-like {kind,name,params}. The loop
// runs them in order each iteration. Agent-running sub-steps (agent_prompt,
// prepare_context, review_run, product_*, canvas) get a real Provider/Model
// editor — no more hand-editing raw JSON to set a sub-step's provider. ---
export interface LoopStep {
  kind: string;
  name?: string;
  params?: Params;
}
// Sub-step kinds that spawn a single agent → Provider + Model (+ Prompt).
export const LOOP_AGENT_KINDS = [
  'agent_prompt',
  'prepare_context',
  'product_analyze',
  'product_rewrite',
  'product_plan',
  'canvas',
];
// Kinds a loop sub-step can be (agent kinds + review_run + the common
// non-agent utilities). Anything else stays editable via Advanced JSON.
export const LOOP_STEP_KINDS = [...LOOP_AGENT_KINDS, 'review_run', 'http_request', 'db_query', 'delay', 'transform'];

// Kinds that render their OWN Provider control (inline or a richer multi-agent
// config). EVERY OTHER kind gets the shared universal Provider+Model block, so
// NO node is ever missing a provider selector — including git_pr (drafts the PR
// message with an agent), swarm_task, and the utility nodes.
export const KINDS_WITH_OWN_PROVIDER = [
  'agent_prompt',
  'prepare_context',
  'product_analyze',
  'product_rewrite',
  'product_plan',
  'canvas',
  'review_run',
  'self_improve',
  'loop',
  'budget_gate',
];

export function loopSteps(node: WorkflowNode | null | undefined): LoopStep[] {
  const p = params(node);
  return Array.isArray(p?.steps) ? (p?.steps as LoopStep[]) : [];
}

export function loopStepParam(step: LoopStep | undefined, key: string): string {
  const v = (step?.params ?? {})[key];
  return typeof v === 'string' ? v : v != null ? String(v) : '';
}

export function loopStepParamList(step: LoopStep | undefined, key: string): string {
  const v = (step?.params ?? {})[key];
  if (Array.isArray(v)) return v.filter((x) => typeof x === 'string').join(', ');
  return typeof v === 'string' ? v : '';
}

/** The step's params with `key` set — empty (incl. an empty list) removes it. */
export function withLoopStepParam(step: LoopStep, key: string, value: unknown): Params {
  const next = { ...(step.params ?? {}) };
  if (value == null || value === '' || (Array.isArray(value) && value.length === 0)) delete next[key];
  else next[key] = value;
  return next;
}

/** The step's params with the summarizer provider set (blank → no summarizer). */
export function withLoopStepSummarizer(step: LoopStep, provider: string): Params {
  const next = { ...(step.params ?? {}) };
  if (provider.trim() === '') delete next.summarizer;
  else next.summarizer = { ...((next.summarizer as Params) ?? {}), provider };
  return next;
}

export function loopStepSummarizer(step: LoopStep | undefined): string {
  const s = (step?.params ?? {}).summarizer as Params | undefined;
  return typeof s?.provider === 'string' ? s.provider : '';
}

// --- self_improve: reflection can run on one or more agents; the node's
// `providers` array overrides the workspace's Self-Improvement providers. Empty
// ⇒ use the configured set (Settings → Self-Improvement). ---
export function selfImproveProviders(node: WorkflowNode | null | undefined): string[] {
  const p = params(node);
  return Array.isArray(p?.providers) ? (p.providers as string[]) : [];
}

// --- picker options: what each id picker shows instead of the raw id -------
/** DB-Explorer connections as "name · engine · environment", in words
 *  ("orders · MySQL · Production"), never the wire enums. */
export function connectionOptions(rows: readonly Connection[] | null): IdOption[] | null {
  return rows?.map((c) => {
    const engine = AUTO_VERTICAL_ENGINES.find((e) => e.id === c.kind)?.label ?? c.kind;
    const env = envTone(c.environment);
    return { id: c.id, label: `${c.name} · ${engine} · ${env.hint ?? env.label}` };
  }) ?? null;
}

export function clusterOptions(rows: readonly BrokerCluster[] | null): IdOption[] | null {
  return rows?.map((c) => ({ id: c.id, label: c.name, title: c.bootstrap_servers })) ?? null;
}

/** A topic is addressed by its name. */
export function topicOptions(rows: readonly TopicSummary[] | null): IdOption[] | null {
  return rows?.map((t) => ({ id: t.name, label: t.name })) ?? null;
}

/** Product stories as "KEY — title". */
export function storyOptions(rows: readonly ProductStory[] | null): IdOption[] | null {
  return rows?.map((s) => ({ id: s.id, label: s.source_key ? `${s.source_key} — ${s.title}` : s.title, title: s.url })) ?? null;
}

/** The person's Jira (Atlassian — Jira + Confluence) accounts, by label. */
export function jiraAccountOptions(rows: readonly IssueAccount[] | null): IdOption[] | null {
  return rows?.filter((a) => a.provider === 'jira').map((a) => ({ id: a.id, label: a.label || a.email, title: a.base_url })) ?? null;
}
