// The Goal form's fields ⇄ GoalSpec (goal.md §3.3, §5.3). Pure; unit-tested in
// ui/unit/goalForm.test.ts. The server validates the spec again and builds the
// workflow (Start → loop { Work → Check criteria }).
import type { AgentChoice, Criterion, GoalLoop, GoalSpec, Repo } from '../../../lib/api/types';
import { templateGoal, type GoalTemplateId } from './goalTemplates.ts';

/** How a criterion is checked, in the form ("Rule" is canvas-only). */
export type FormVerify = 'command' | 'agent' | 'human';

export const VERIFY_LABEL: Record<FormVerify, string> = {
  command: 'A command passes',
  agent: 'An agent finds evidence',
  human: 'I check it',
};

/** Active time choices (seconds) and their words. */
export const ACTIVE_OPTIONS: readonly { s: number; label: string }[] = [
  { s: 900, label: '15m' }, { s: 1800, label: '30m' }, { s: 3600, label: '1h' },
  { s: 7200, label: '2h' }, { s: 14_400, label: '4h' }, { s: 28_800, label: '8h' },
];

export const MAX_ITERATIONS = 50;
const MIN_ACTIVE_S = 900;
const MAX_ACTIVE_S = 28_800;
const STEP_TIMEOUT_S = 1200;

export interface ModelChoice {
  provider: string;
  model: string;
  effort: string;
}

export interface GoalFields {
  goal: string;
  title: string;
  /** The title follows the goal's first line until the person edits it. */
  titleEdited: boolean;
  criteria: Criterion[];
  work: 'repo' | 'research';
  repoId: string;
  /** A branch; empty = the checkout's current commit. */
  base: string;
  iterations: number;
  activeS: number;
  worker: ModelChoice;
  checker: ModelChoice;
  finalReview: boolean;
  /** Continuing an older loop: its kept work (the server resolves the path). */
  continueLoop?: { id: string; path: string; research: boolean };
  from?: 'insights';
}

const noModel = (): ModelChoice => ({ provider: '', model: '', effort: '' });

export function emptyFields(): GoalFields {
  return {
    goal: '', title: '', titleEdited: false, criteria: [newCriterion([])], work: 'repo', repoId: '', base: '',
    iterations: 5, activeS: 3600, worker: noModel(), checker: noModel(), finalReview: false,
  };
}

/** The goal's first line, cut to a title. */
export function titleFromGoal(goal: string): string {
  const line = goal.trim().split('\n')[0].trim();
  return line.length > 120 ? `${line.slice(0, 119).trimEnd()}…` : line;
}

/** Typing the goal: the title follows until it was edited. */
export function setGoal(f: GoalFields, goal: string): GoalFields {
  return { ...f, goal, title: f.titleEdited ? f.title : titleFromGoal(goal) };
}

/** A blank criterion with the next free `c<n>` id. */
export function newCriterion(existing: readonly Criterion[]): Criterion {
  let n = existing.length + 1;
  while (existing.some((c) => c.id === `c${n}`)) n += 1;
  return { id: `c${n}`, text: '', verify: 'agent', how: '' };
}

/** Swap a criterion with its neighbour (the up / down buttons). */
export function moveCriterion(list: readonly Criterion[], i: number, d: -1 | 1): Criterion[] {
  const j = i + d;
  if (j < 0 || j >= list.length) return [...list];
  const next = [...list];
  [next[i], next[j]] = [next[j], next[i]];
  return next;
}

/** What still keeps Start goal off, first problem first (the field names the
 *  form's test ids use). The server checks the rest. */
export function firstProblem(f: GoalFields): { field: string; text: string } | null {
  if (!f.goal.trim()) return { field: 'goal', text: 'Say what the goal is.' };
  if (!f.criteria.length) return { field: 'criteria', text: 'Add at least one criterion.' };
  const i = f.criteria.findIndex((c) => !c.text.trim());
  if (i >= 0) return { field: 'criteria', text: `Criterion ${i + 1} needs what must be true.` };
  const j = f.criteria.findIndex((c) => c.verify === 'command' && !c.command?.trim());
  if (j >= 0) return { field: 'criteria', text: `Criterion ${j + 1} needs its command.` };
  if (!f.continueLoop && f.work === 'repo' && !f.repoId) return { field: 'work', text: 'Pick a repository, or choose Research.' };
  return null;
}

function choice(m: ModelChoice): AgentChoice | undefined {
  const c: AgentChoice = {};
  if (m.provider) c.provider = m.provider;
  if (m.model) c.model = m.model;
  if (m.effort) c.effort = m.effort;
  return Object.keys(c).length ? c : undefined;
}

function cleanCriterion(c: Criterion): Criterion {
  const out: Criterion = { id: c.id, text: c.text.trim(), verify: c.verify, how: c.verify === 'command' ? '' : c.how.trim() };
  if (c.verify === 'command') out.command = c.command?.trim() ?? '';
  return out;
}

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, Math.round(n)));

export function toSpec(f: GoalFields): GoalSpec {
  const research = f.continueLoop ? f.continueLoop.research : f.work === 'research';
  const spec: GoalSpec = {
    title: (f.title.trim() || titleFromGoal(f.goal)).slice(0, 120),
    goal: f.goal.trim(),
    criteria: f.criteria.map(cleanCriterion),
    work: research ? { kind: 'research' } : { kind: 'build', repo_id: f.repoId, ...(f.base && !f.continueLoop ? { base: f.base } : {}) },
    budget: { iterations: clamp(f.iterations, 1, MAX_ITERATIONS), active_s: clamp(f.activeS, MIN_ACTIVE_S, MAX_ACTIVE_S), step_timeout_s: STEP_TIMEOUT_S },
  };
  const worker = choice(f.worker);
  const checker = choice(f.checker);
  if (worker) spec.worker = worker;
  if (checker) spec.checker = checker;
  if (f.finalReview) spec.final_review = true;
  if (f.continueLoop) spec.continue_loop_id = f.continueLoop.id;
  if (f.from) spec.from = f.from;
  return spec;
}

/** "This goal will…" — where it works and that nothing leaves the machine. */
export function willLine(f: GoalFields, repoName: string | null): string {
  const tail = 'Nothing is committed to your branches, pushed or posted.';
  if (f.continueLoop) return `Work in this loop’s kept work (${f.continueLoop.path}). ${tail}`;
  if (f.work === 'research') return `Work in a research folder of its own, with no repository. ${tail}`;
  return `Work in an isolated copy of ${repoName ?? 'the repository'} (branch otto-wf/…). ${tail}`;
}

/** Apply a "Start from…" template: its criteria and budget; the goal stays. */
export function applyTemplate(f: GoalFields, id: GoalTemplateId): GoalFields {
  const t = templateGoal(id);
  const active = ACTIVE_OPTIONS.find((o) => o.s >= t.activeS)?.s ?? MAX_ACTIVE_S;
  return { ...f, criteria: t.criteria, iterations: Math.min(MAX_ITERATIONS, t.iterations), activeS: active, work: t.research ? 'research' : f.work };
}

/** The Insights handoff (goal.md §4.2): Goal = the prompt, Title = its title. */
export function fromStaged(draft: { title: string; prompt: string }): GoalFields {
  const f = setGoal(emptyFields(), draft.prompt);
  return draft.title.trim() ? { ...f, title: draft.title.trim().slice(0, 120), titleEdited: true, from: 'insights' } : { ...f, from: 'insights' };
}

/** A criterion id the goal accepts (`[a-z0-9-]{1,64}`). */
function criterionId(raw: string, i: number): string {
  const id = raw.toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64);
  return id || `c${i + 1}`;
}

/** Continue an older loop as a goal (goal.md §5.3, mirrors the server's
 *  `goal_spec_from_loop_args`): its criteria and agents, and the budget it had
 *  left (at least 1 iteration and 15 minutes). Work in = the loop's kept work. */
export function fromLoop(loop: GoalLoop, repos: readonly Pick<Repo, 'id' | 'path'>[] = []): GoalFields {
  const d = loop.definition;
  const lim = loop.limits;
  const exec = loop.config.executors[0];
  const ev = loop.config.evaluator;
  const research = loop.config.mode === 'research';
  return {
    goal: (d.summary || d.title || loop.name).trim(),
    title: (loop.name || d.title).slice(0, 120),
    titleEdited: true,
    criteria: d.acceptance_criteria.map((c, i): Criterion => {
      const id = criterionId(c.id, i);
      if (c.verify_kind === 'command') return { id, text: c.text, verify: 'command', how: '', command: c.verify_cmd ?? '' };
      return { id, text: c.text, verify: c.verify_kind === 'human' ? 'human' : 'agent', how: c.verify };
    }),
    work: research ? 'research' : 'repo',
    repoId: repos.find((r) => r.path === loop.repo_path)?.id ?? '',
    base: '',
    iterations: clamp(lim.max_iterations - loop.iterations_started, 1, MAX_ITERATIONS),
    activeS: clamp(lim.max_runtime_secs - loop.elapsed_secs, MIN_ACTIVE_S, MAX_ACTIVE_S),
    worker: { provider: exec?.provider ?? '', model: exec?.model ?? '', effort: '' },
    checker: { provider: ev?.provider ?? '', model: ev?.model ?? '', effort: '' },
    finalReview: !!loop.config.require_review,
    continueLoop: { id: loop.id, path: loop.worktree_path || loop.repo_path, research },
  };
}

/** Loops a person may continue: not finished, not already continued. */
export function canContinue(loop: Pick<GoalLoop, 'status'>): boolean {
  return ['draft', 'paused', 'blocked', 'exhausted'].includes(loop.status);
}
