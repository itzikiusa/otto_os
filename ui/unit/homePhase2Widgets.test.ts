// Pure-TS unit tests for Phase 2 Home Live Widgets (`pr-queue` and `automations`):
// catalogue registration (`HOME_KINDS` / `isHomeKind` / `kindDef`), route builders,
// candidate repo filtering, ranking/sorting, truncation, and empty/error states.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { HOME_KINDS, isHomeKind, kindDef } from '../src/lib/home/kinds.ts';
import type {
  ActiveWorkflowRun,
  GitAccount,
  GoalLoop,
  PrSummary,
  Repo,
  Review,
  Workflow,
} from '../src/lib/api/types.ts';

const pluralMod = loadSource(new URL('../src/lib/plural.ts', import.meta.url), {});
const data = loadSource(new URL('../src/modules/home/boxes/homeBoxesData.ts', import.meta.url), {
  '../../../lib/plural': pluralMod,
});

const plain = <T>(v: T): T => JSON.parse(JSON.stringify(v));

function makeRepo(id: string, overrides: Partial<Repo> = {}): Repo {
  return {
    id,
    workspace_id: 'ws-1',
    name: id,
    path: `/work/${id}`,
    remote_url: `https://github.com/acme/${id}.git`,
    provider: 'github',
    git_account_id: 'acc-1',
    created_at: '2026-11-01T00:00:00Z',
    forge: 'github',
    ...overrides,
  };
}

function makePr(number: number, overrides: Partial<PrSummary> = {}): PrSummary {
  return {
    number,
    title: `PR #${number}`,
    author: 'alice',
    state: 'open',
    source_branch: `feat/pr-${number}`,
    target_branch: 'main',
    updated_at: '2026-11-19T12:00:00Z',
    url: `https://github.com/acme/repo/pull/${number}`,
    ...overrides,
  };
}

function makeReview(id: string, overrides: Partial<Review> = {}): Review {
  return {
    id,
    repo_id: 'repo-a',
    pr_number: 1,
    status: 'done',
    error: null,
    comments: [],
    agents: [],
    created_at: '2026-11-19T12:00:00Z',
    ...overrides,
  };
}

function makeLoop(id: string, overrides: Partial<GoalLoop> = {}): GoalLoop {
  return {
    id,
    workspace_id: 'ws-1',
    name: `Loop ${id}`,
    repo_path: '/work/repo-a',
    definition: {
      title: `Goal ${id}`,
      summary: '',
      objectives: [],
      acceptance_criteria: [],
      constraints: [],
      out_of_scope: [],
      success_signal: '',
    },
    limits: {
      max_iterations: 5,
      max_runtime_secs: 3600,
      per_phase_timeout_secs: 300,
      max_attempts_per_executor: 2,
    },
    config: {
      executors: [],
      planner: { provider: 'claude', model: '', prompt: '' },
      evaluator: { provider: 'claude', model: '', prompt: '' },
      digester: { provider: 'claude', model: '', prompt: '' },
      definer: { provider: 'claude', model: '', prompt: '' },
    },
    status: 'draft',
    phase: 'planning',
    iterations_started: 1,
    current_iteration: 1,
    progress_pct: 40,
    context_digest: '',
    elapsed_secs: 10,
    cost_usd: 0,
    created_by: 'u-1',
    created_at: '2026-11-19T10:00:00Z',
    updated_at: '2026-11-19T12:00:00Z',
    ...overrides,
  };
}

function makeWorkflow(id: string, overrides: Partial<Workflow> = {}): Workflow {
  return {
    id,
    workspace_id: 'ws-1',
    name: `Workflow ${id}`,
    description: '',
    instructions: '',
    graph: { nodes: [], edges: [] },
    created_by: 'u-1',
    created_at: '2026-11-19T09:00:00Z',
    updated_at: '2026-11-19T11:00:00Z',
    ...overrides,
  };
}

function makeRun(runId: string, overrides: Partial<ActiveWorkflowRun> = {}): ActiveWorkflowRun {
  return {
    run_id: runId,
    workflow_id: 'wf-1',
    workspace_id: 'ws-1',
    workflow_name: 'Deploy check',
    status: 'running',
    started_at: '2026-11-19T12:30:00Z',
    nodes_total: 4,
    nodes_done: 1,
    ...overrides,
  };
}

// ── HOME_KINDS & kind helpers ────────────────────────────────────────────────

test('HOME_KINDS registers pr-queue and automations with expected metadata', () => {
  assert.equal(HOME_KINDS.length, 7);
  const kinds = HOME_KINDS.map((k) => k.kind);
  assert.equal(new Set(kinds).size, HOME_KINDS.length, 'kind keys must be unique');

  assert.equal(isHomeKind('pr-queue'), true);
  assert.equal(isHomeKind('automations'), true);
  assert.equal(isHomeKind('unknown-widget'), false);
  assert.equal(isHomeKind(null), false);
  assert.equal(isHomeKind(42), false);

  const prDef = kindDef('pr-queue');
  assert.equal(prDef.label, 'PR review queue');
  assert.equal(prDef.icon, 'branch');
  assert.equal(prDef.feature, 'git');
  assert.equal(prDef.w, 6);
  assert.equal(prDef.h, 4);
  assert.equal(prDef.route, 'git');

  const autoDef = kindDef('automations');
  assert.equal(autoDef.label, 'Workflows & loops');
  assert.equal(autoDef.icon, 'split');
  assert.equal(autoDef.feature, 'workflows');
  assert.equal(autoDef.w, 6);
  assert.equal(autoDef.h, 4);
  assert.equal(autoDef.route, 'workflows');
});

// ── PR review queue helpers ──────────────────────────────────────────────────

test('prRoute builds encoded PR detail and local-review paths', () => {
  assert.equal(data.prRoute('repo/a', 17), 'git/repo%2Fa/pr/17');
  assert.equal(data.prRoute('repo-1', 42.9), 'git/repo-1/pr/42');
  assert.equal(data.prRoute('repo/a'), 'git/repo%2Fa/review');
  assert.equal(data.prRoute('repo-1', null), 'git/repo-1/review');
  assert.equal(data.prRoute('repo-1', 0), 'git/repo-1/review');
});

test('prCandidateRepos skips local-only and unrecognized-forge repos', () => {
  const repos: Repo[] = [
    makeRepo('bound-gh', { git_account_id: 'acc-1', provider: 'github', forge: 'github' }),
    makeRepo('unbound-gl', { git_account_id: null, provider: 'gitlab', forge: 'gitlab' }),
    makeRepo('unbound-bb', { git_account_id: null, provider: 'bitbucket', forge: 'bitbucket' }),
    makeRepo('local-only', { remote_url: null, provider: null, forge: null, git_account_id: null }),
    makeRepo('custom-host', { provider: null, forge: 'unrecognized', git_account_id: null }),
  ];
  const accounts: GitAccount[] = [
    {
      id: 'acc-1',
      user_id: 'u-1',
      provider: 'github',
      label: 'GH',
      username: 'alice',
      token_ref: 'k1',
      api_base_url: null,
      namespace: null,
      token_expires_at: null,
      created_at: '2026-11-01T00:00:00Z',
    },
    {
      id: 'acc-2',
      user_id: 'u-1',
      provider: 'gitlab',
      label: 'GL',
      username: 'alice',
      token_ref: 'k2',
      api_base_url: null,
      namespace: null,
      token_expires_at: null,
      created_at: '2026-11-01T00:00:00Z',
    },
  ];
  const filtered = plain(data.prCandidateRepos(repos, accounts)).map((r: Repo) => r.id);
  assert.deepEqual(filtered, ['bound-gh', 'unbound-gl']);
});

test('summarizePrQueue ranks active reviews first, counts stats, omits cancelled local reviews, and truncates cleanly', () => {
  const repoA = makeRepo('repo-a');
  const repoB = makeRepo('repo-b');

  const summary = plain(
    data.summarizePrQueue({
      repos: [repoA, repoB],
      prs: [
        {
          repo: repoA,
          pr: makePr(10, { title: 'Draft WIP', draft: true, updated_at: '2026-11-19T14:00:00Z' }),
        },
        {
          repo: repoA,
          pr: makePr(11, {
            title: 'Approved cleanup',
            review: { status: 'approved', approvals: 2 },
            updated_at: '2026-11-19T13:30:00Z',
          }),
        },
        {
          repo: repoB,
          pr: makePr(12, {
            title: 'Needs review PR',
            review: { status: 'waiting', approvals: 0 },
            updated_at: '2026-11-19T13:00:00Z',
          }),
        },
        {
          repo: repoB,
          pr: makePr(13, {
            title: 'Changes requested PR',
            review: { status: 'changes_requested', approvals: 0 },
            updated_at: '2026-11-19T12:00:00Z',
          }),
        },
        {
          repo: repoA,
          pr: makePr(14, { title: 'Live AI review PR', updated_at: '2026-11-19T11:00:00Z' }),
          review: makeReview('rev-14', { status: 'running', pr_number: 14 }),
        },
        {
          repo: repoB,
          pr: makePr(15, { title: 'Failed AI review PR', updated_at: '2026-11-19T11:30:00Z' }),
          review: makeReview('rev-15', { status: 'error', pr_number: 15, error: 'timeout' }),
        },
      ],
      localReviews: [
        {
          repo: repoB,
          review: makeReview('local-run', {
            repo_id: 'repo-b',
            pr_number: 0,
            status: 'running',
            created_at: '2026-11-19T14:10:00Z',
          }),
        },
        {
          repo: repoA,
          review: makeReview('local-cancel', {
            repo_id: 'repo-a',
            pr_number: 0,
            status: 'cancelled',
            created_at: '2026-11-19T14:20:00Z',
          }),
        },
      ],
      limit: 5,
    }),
  );

  assert.deepEqual(summary.stats, { openPrs: 6, reviewing: 2, repos: 2 });
  assert.equal(summary.totalRows, 7);
  assert.equal(summary.truncatedCount, 2);
  assert.deepEqual(
    summary.rows.map((r: { id: string; badgeLabel: string; live: boolean }) => [r.id, r.badgeLabel, r.live]),
    [
      ['local:repo-b:local-run', 'Reviewing', true],
      ['pr:repo-a:14', 'Reviewing', true],
      ['pr:repo-b:13', 'Changes', false],
      ['pr:repo-b:15', 'Review failed', false],
      ['pr:repo-b:12', 'Needs review', false],
    ],
  );
  assert.equal(summary.rows[0].route, 'git/repo-b/review');
  assert.equal(summary.rows[1].route, 'git/repo-a/pr/14');
});

test('summarizePrQueue handles empty input cleanly', () => {
  const empty = plain(data.summarizePrQueue({ repos: [], prs: [] }));
  assert.deepEqual(empty, {
    stats: { openPrs: 0, reviewing: 0, repos: 0 },
    rows: [],
    totalRows: 0,
    truncatedCount: 0,
  });
});

// ── Workflows & Goal Loops helpers ───────────────────────────────────────────

test('automationRoute builds encoded routes for loops, workflows, and runs', () => {
  assert.equal(data.automationRoute({ kind: 'goal-loop', id: 'loop 1' }), 'loops/loop%201');
  assert.equal(data.automationRoute({ kind: 'workflow', id: 'wf/2' }), 'workflows/wf%2F2');
  assert.equal(
    data.automationRoute({ kind: 'workflow-run', workflowId: 'wf/2', runId: 'run 9' }),
    'workflows/wf%2F2/runs/run%209',
  );
});

test('summarizeAutomations ranks waiting/blocked/running items first, deduplicates active workflow rows, and truncates', () => {
  const summary = plain(
    data.summarizeAutomations({
      activeRuns: [
        makeRun('run-live', {
          workflow_id: 'wf-running',
          workflow_name: 'Nightly CI',
          status: 'running',
          nodes_done: 2,
          nodes_total: 5,
          current_step: 'Run tests',
          started_at: '2026-11-19T13:00:00Z',
        }),
        makeRun('run-wait', {
          workflow_id: 'wf-approve',
          workflow_name: 'Prod release',
          status: 'running',
          waiting_approval: true,
          nodes_done: 1,
          nodes_total: 3,
          started_at: '2026-11-19T12:00:00Z',
        }),
      ],
      loops: [
        makeLoop('l-blocked', { status: 'blocked', current_iteration: 2, progress_pct: 60, updated_at: '2026-11-19T13:10:00Z' }),
        makeLoop('l-running', { status: 'running', current_iteration: 3, progress_pct: 75, updated_at: '2026-11-19T13:20:00Z' }),
        makeLoop('l-done', { status: 'succeeded', phase: 'done', progress_pct: 100, updated_at: '2026-11-19T11:00:00Z' }),
      ],
      workflows: [
        makeWorkflow('wf-running', { name: 'Nightly CI' }),
        makeWorkflow('wf-idle-1', {
          name: 'Triage PRs',
          description: 'Review incoming PRs',
          graph: {
            nodes: [{ id: 'n1', kind: 'trigger', name: 'Start', x: 0, y: 0, params: {} }],
            edges: [],
          },
          updated_at: '2026-11-19T10:00:00Z',
        }),
        makeWorkflow('wf-idle-3', {
          name: 'Release notes',
          graph: {
            nodes: [
              { id: 'n1', kind: 'a', name: '1', x: 0, y: 0, params: {} },
              { id: 'n2', kind: 'a', name: '2', x: 0, y: 0, params: {} },
              { id: 'n3', kind: 'a', name: '3', x: 0, y: 0, params: {} },
            ],
            edges: [],
          },
          updated_at: '2026-11-19T10:30:00Z',
        }),
      ],
      limit: 5,
    }),
  );

  // running = 2 active workflow runs + 1 running loop; workflows stat keeps all 3.
  assert.deepEqual(summary.stats, { running: 3, loops: 3, workflows: 3 });
  // wf-running is deduplicated from the static list: 2 runs + 3 loops + 2 idle workflows = 7.
  assert.equal(summary.totalRows, 7);
  assert.equal(summary.truncatedCount, 2);
  assert.deepEqual(
    summary.rows.map((r: { id: string; badgeLabel: string; live: boolean }) => [r.id, r.badgeLabel, r.live]),
    [
      ['loop:l-blocked', 'Blocked', false],
      ['run:run-wait', 'Needs approval', false],
      ['loop:l-running', 'Running', true],
      ['run:run-live', 'Running', true],
      ['loop:l-done', 'Succeeded', false],
    ],
  );
  assert.equal(summary.rows[0].route, 'loops/l-blocked');
  assert.equal(summary.rows[1].route, 'workflows/wf-approve/runs/run-wait');
  assert.equal(summary.rows[3].detail, 'Workflow · step 3/5 · Run tests');
});

test('summarizeAutomations formats workflow step counts and handles empty state', () => {
  const full = plain(
    data.summarizeAutomations({
      activeRuns: [],
      loops: [],
      workflows: [
        makeWorkflow('w1', {
          name: 'Single',
          graph: { nodes: [{ id: 'n1', kind: 'a', name: '1', x: 0, y: 0, params: {} }], edges: [] },
          updated_at: '2026-11-19T12:00:00Z',
        }),
        makeWorkflow('w0', {
          name: 'Blank',
          graph: { nodes: [], edges: [] },
          updated_at: '2026-11-19T11:00:00Z',
        }),
      ],
    }),
  );
  assert.deepEqual(
    full.rows.map((r: { id: string; badgeLabel: string; route: string }) => [r.id, r.badgeLabel, r.route]),
    [
      ['wf:w1', '1 step', 'workflows/w1'],
      ['wf:w0', 'Workflow', 'workflows/w0'],
    ],
  );

  const empty = plain(data.summarizeAutomations({ activeRuns: [], loops: [], workflows: [] }));
  assert.deepEqual(empty, {
    stats: { running: 0, loops: 0, workflows: 0 },
    rows: [],
    totalRows: 0,
    truncatedCount: 0,
  });
});
