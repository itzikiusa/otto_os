// Pure ranking, summary and routing helpers for the Phase 2 Home live widgets
// (`PrQueueBox.svelte` and `AutomationsBox.svelte`). Kept free of Svelte / DOM
// imports so `node --test` exercises them directly.

import type {
  ActiveWorkflowRun,
  GitAccount,
  PrSummary,
  Repo,
  Review,
  Workflow,
} from '../../../lib/api/types';
import type { BadgeTone } from '../../../lib/status';
import { plural } from '../../../lib/plural';

export const DEFAULT_BOX_ROW_LIMIT = 8;

function parseTime(iso: string | null | undefined): number {
  if (!iso) return 0;
  const t = Date.parse(iso);
  return Number.isFinite(t) ? t : 0;
}

// ---------------------------------------------------------------------------
// PR Review Queue (`pr-queue`)
// ---------------------------------------------------------------------------

export interface PrQueueRawItem {
  repo: Pick<Repo, 'id' | 'name'>;
  pr: PrSummary;
  review?: Review | null;
}

export interface LocalReviewRawItem {
  repo: Pick<Repo, 'id' | 'name'>;
  review: Review;
}

export interface PrQueueRow {
  id: string;
  kind: 'pr' | 'local-review';
  repoId: string;
  repoName: string;
  prNumber?: number;
  title: string;
  detail: string;
  route: string;
  badgeLabel: string;
  badgeTone: BadgeTone;
  live: boolean;
  priority: number;
  updatedAt: string;
}

export interface PrQueueSummary {
  stats: {
    openPrs: number;
    reviewing: number;
    repos: number;
  };
  rows: PrQueueRow[];
  totalRows: number;
  truncatedCount: number;
}

/** Route to open a PR detail (`#/git/<repoId>/pr/<number>`) or a repo's review
 *  tab (`#/git/<repoId>/review`) when no PR number is present (local review). */
export function prRoute(repoId: string, prNumber?: number | null): string {
  const rid = encodeURIComponent(repoId);
  if (typeof prNumber === 'number' && Number.isFinite(prNumber) && prNumber > 0) {
    return `git/${rid}/pr/${Math.trunc(prNumber)}`;
  }
  return `git/${rid}/review`;
}

/** Filter registered repos that have a recognized forge and matching account
 *  so the PR box does not fire doomed forge calls against local-only repos. */
export function prCandidateRepos(
  repos: readonly Repo[],
  accounts: readonly GitAccount[] | null,
): Repo[] {
  const configuredProviders = accounts ? new Set(accounts.map((a) => a.provider)) : null;
  return repos.filter((r) => {
    const forge = r.provider ?? (r.forge && r.forge !== 'unrecognized' ? r.forge : null);
    return Boolean(forge && (r.git_account_id || (configuredProviders ? configuredProviders.has(forge) : r.provider)));
  });
}

function classifyPrRow(item: PrQueueRawItem): {
  priority: number;
  badgeLabel: string;
  badgeTone: BadgeTone;
  live: boolean;
} {
  const rev = item.review ?? null;
  if (rev?.status === 'running') {
    return { priority: 0, badgeLabel: 'Reviewing', badgeTone: 'info', live: true };
  }
  if (
    rev?.verdict === 'changes_requested' ||
    item.pr.review?.status === 'changes_requested' ||
    (typeof rev?.blocker_count === 'number' && rev.blocker_count > 0)
  ) {
    return { priority: 1, badgeLabel: 'Changes', badgeTone: 'warn', live: false };
  }
  if (rev?.status === 'error') {
    return { priority: 1, badgeLabel: 'Review failed', badgeTone: 'bad', live: false };
  }
  if (item.pr.draft) {
    return { priority: 4, badgeLabel: 'Draft', badgeTone: 'neutral', live: false };
  }
  if (item.pr.review?.status === 'waiting') {
    return { priority: 2, badgeLabel: 'Needs review', badgeTone: 'warn', live: false };
  }
  if (item.pr.review?.status === 'approved' || rev?.verdict === 'approved') {
    return { priority: 3, badgeLabel: 'Approved', badgeTone: 'ok', live: false };
  }
  return { priority: 2, badgeLabel: 'Open', badgeTone: 'neutral', live: false };
}

function classifyLocalReview(review: Review): {
  priority: number;
  badgeLabel: string;
  badgeTone: BadgeTone;
  live: boolean;
} {
  if (review.status === 'running') {
    return { priority: 0, badgeLabel: 'Reviewing', badgeTone: 'info', live: true };
  }
  if (review.status === 'error') {
    return { priority: 1, badgeLabel: 'Review failed', badgeTone: 'bad', live: false };
  }
  if (review.verdict === 'changes_requested' || (review.blocker_count ?? 0) > 0) {
    return { priority: 1, badgeLabel: 'Blockers', badgeTone: 'warn', live: false };
  }
  if (review.verdict === 'approved' || review.status === 'done') {
    return { priority: 3, badgeLabel: 'Reviewed', badgeTone: 'ok', live: false };
  }
  return { priority: 4, badgeLabel: 'Canceled', badgeTone: 'neutral', live: false };
}

/** Rank open PRs and active/recent local reviews into a compact Home queue. */
export function summarizePrQueue(input: {
  repos: readonly Pick<Repo, 'id' | 'name'>[];
  prs: readonly PrQueueRawItem[];
  localReviews?: readonly LocalReviewRawItem[];
  limit?: number;
}): PrQueueSummary {
  const limit = input.limit ?? DEFAULT_BOX_ROW_LIMIT;
  const rows: PrQueueRow[] = [];
  let reviewing = 0;

  for (const item of input.prs) {
    const cls = classifyPrRow(item);
    if (item.review?.status === 'running') reviewing += 1;
    const author = item.pr.author?.trim();
    const branch = item.pr.source_branch?.trim();
    const parts = [item.repo.name, `#${item.pr.number}`];
    if (author) parts.push(author);
    else if (branch) parts.push(branch);
    rows.push({
      id: `pr:${item.repo.id}:${item.pr.number}`,
      kind: 'pr',
      repoId: item.repo.id,
      repoName: item.repo.name,
      prNumber: item.pr.number,
      title: item.pr.title || `PR #${item.pr.number}`,
      detail: parts.join(' · '),
      route: prRoute(item.repo.id, item.pr.number),
      badgeLabel: cls.badgeLabel,
      badgeTone: cls.badgeTone,
      live: cls.live,
      priority: cls.priority,
      updatedAt: item.pr.updated_at || item.review?.created_at || '',
    });
  }

  for (const item of input.localReviews ?? []) {
    if (item.review.status === 'cancelled') continue;
    const cls = classifyLocalReview(item.review);
    if (item.review.status === 'running') reviewing += 1;
    rows.push({
      id: `local:${item.repo.id}:${item.review.id}`,
      kind: 'local-review',
      repoId: item.repo.id,
      repoName: item.repo.name,
      title: `Local review · ${item.repo.name}`,
      detail: item.review.summary_md?.trim() || `${item.repo.name} · AI review`,
      route: prRoute(item.repo.id),
      badgeLabel: cls.badgeLabel,
      badgeTone: cls.badgeTone,
      live: cls.live,
      priority: cls.priority,
      updatedAt: item.review.created_at || '',
    });
  }

  rows.sort((a, b) =>
    a.priority - b.priority ||
    parseTime(b.updatedAt) - parseTime(a.updatedAt) ||
    a.repoName.localeCompare(b.repoName) ||
    (a.prNumber ?? 0) - (b.prNumber ?? 0) ||
    a.id.localeCompare(b.id),
  );

  const bounded = limit >= 0 ? rows.slice(0, limit) : rows;
  return {
    stats: {
      openPrs: input.prs.length,
      reviewing,
      repos: input.repos.length,
    },
    rows: bounded,
    totalRows: rows.length,
    truncatedCount: Math.max(0, rows.length - bounded.length),
  };
}

// ---------------------------------------------------------------------------
// Workflows (`automations`) — goals are workflows too (goal.md §4.5)
// ---------------------------------------------------------------------------

export type AutomationRouteTarget =
  | { kind: 'workflow'; id: string }
  | { kind: 'workflow-run'; workflowId: string; runId: string };

/** Build the router path for a workflow or an active workflow run. */
export function automationRoute(target: AutomationRouteTarget): string {
  if (target.kind === 'workflow-run') {
    return `workflows/${encodeURIComponent(target.workflowId)}/runs/${encodeURIComponent(target.runId)}`;
  }
  return `workflows/${encodeURIComponent(target.id)}`;
}

export interface AutomationRow {
  id: string;
  kind: 'workflow-run' | 'workflow';
  /** A goal's run (its workflow was built from a goal). */
  goal: boolean;
  title: string;
  detail: string;
  route: string;
  badgeLabel: string;
  badgeTone: BadgeTone;
  live: boolean;
  priority: number;
  at: string;
}

export interface AutomationsSummary {
  stats: {
    running: number;
    workflows: number;
  };
  rows: AutomationRow[];
  totalRows: number;
  truncatedCount: number;
}

function runDetail(run: ActiveWorkflowRun): string {
  const parts = ['Workflow'];
  if (run.nodes_total > 0) {
    parts.push(`step ${Math.min(run.nodes_done + 1, run.nodes_total)}/${run.nodes_total}`);
  }
  if (run.current_step?.trim()) parts.push(run.current_step.trim());
  return parts.join(' · ');
}

/** Combine live workflow runs (goals' included) and the workspace's own
 *  workflows into one ranked summary for the Home Automations box. Goal
 *  workflows show only through their runs. */
export function summarizeAutomations(input: {
  activeRuns: readonly ActiveWorkflowRun[];
  workflows: readonly Workflow[];
  limit?: number;
}): AutomationsSummary {
  const limit = input.limit ?? DEFAULT_BOX_ROW_LIMIT;
  const rows: AutomationRow[] = [];
  const runningWorkflowIds = new Set<string>();
  const goalIds = new Set(input.workflows.filter((w) => w.origin === 'goal').map((w) => w.id));
  const authored = input.workflows.filter((w) => !goalIds.has(w.id));

  for (const run of input.activeRuns) {
    runningWorkflowIds.add(run.workflow_id);
    const waiting = run.waiting_approval === true;
    const isRunning = run.status === 'running';
    rows.push({
      id: `run:${run.run_id}`,
      kind: 'workflow-run',
      goal: goalIds.has(run.workflow_id),
      title: run.workflow_name || 'Workflow run',
      detail: runDetail(run),
      route: automationRoute({ kind: 'workflow-run', workflowId: run.workflow_id, runId: run.run_id }),
      badgeLabel: waiting ? 'Needs approval' : isRunning ? 'Running' : 'Queued',
      badgeTone: waiting ? 'warn' : isRunning ? 'info' : 'neutral',
      live: isRunning && !waiting,
      priority: waiting ? 0 : isRunning ? 1 : 2,
      at: run.started_at || '',
    });
  }

  for (const wf of authored) {
    if (runningWorkflowIds.has(wf.id)) continue;
    const nodeCount = wf.graph?.nodes?.length ?? 0;
    const stepLabel = nodeCount > 0 ? plural(nodeCount, 'step') : 'Workflow';
    rows.push({
      id: `wf:${wf.id}`,
      kind: 'workflow',
      goal: false,
      title: wf.name || 'Untitled workflow',
      detail: wf.description?.trim() || 'Workflow',
      route: automationRoute({ kind: 'workflow', id: wf.id }),
      badgeLabel: stepLabel,
      badgeTone: 'neutral',
      live: false,
      priority: 6,
      at: wf.updated_at || wf.created_at || '',
    });
  }

  rows.sort((a, b) =>
    a.priority - b.priority ||
    parseTime(b.at) - parseTime(a.at) ||
    a.title.localeCompare(b.title) ||
    a.id.localeCompare(b.id),
  );

  const bounded = limit >= 0 ? rows.slice(0, limit) : rows;
  return {
    stats: {
      running: input.activeRuns.length,
      workflows: authored.length,
    },
    rows: bounded,
    totalRows: rows.length,
    truncatedCount: Math.max(0, rows.length - bounded.length),
  };
}
