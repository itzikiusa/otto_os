import { execFileSync } from 'node:child_process';
import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// Workflow orchestrator E2E. Drives the API against the isolated OTTO_E2E daemon
// (agent turns return a deterministic canned reply + a session id, so agent nodes
// complete offline), then renders the Workflows page in the desktop browser.
//
// The API portion proves the NEW orchestrator behavior end-to-end through the
// real route → engine → repo stack: conditional branching (branch-skip ≠ error),
// the bounded fix→review-style loop, per-step agent SESSIONS surfaced on the run,
// the run→Proof-Pack link, workflow versioning + restore, and converting a
// scheduled task into a workflow. The UI portion is a desktop smoke test.

const V1 = '/api/v1';

let base = '';
let ctx: APIRequestContext;
let ws = '';
let branchWfId = '';
let loopWfId = '';
let sessionWfId = '';

interface Node {
  id: string;
  kind: string;
  name?: string;
  x?: number;
  y?: number;
  params?: unknown;
}
interface Edge {
  id: string;
  source: string;
  target: string;
  condition?: string;
}

function node(id: string, kind: string, params?: unknown): Node {
  return { id, kind, name: id, x: 0, y: 0, params: params ?? null };
}
function edge(source: string, target: string, condition?: string): Edge {
  return { id: `${source}-${target}`, source, target, condition };
}

async function createWorkflow(name: string, nodes: Node[], edges: Edge[]): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workspaces/${ws}/workflows`, {
    data: { name, description: 'e2e', graph: { nodes, edges } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

/** Run a workflow and poll its run row to a terminal status. */
async function runToCompletion(wfId: string): Promise<any> {
  const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, { data: {} });
  expect(r.ok(), await r.text()).toBeTruthy();
  const runId = (await r.json()).id as string;
  const deadline = Date.now() + 60_000;
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const g = await ctx.get(`${base}${V1}/workflow-runs/${runId}`);
    expect(g.ok(), await g.text()).toBeTruthy();
    const run = await g.json();
    if (run.status !== 'running' && run.status !== 'pending') return run;
    if (Date.now() > deadline) throw new Error(`run ${runId} did not finish: ${run.status}`);
    await new Promise((res) => setTimeout(res, 500));
  }
}

function nodeState(run: any, id: string): any {
  return (run.nodes ?? []).find((n: any) => n.node_id === id);
}

/** Poll an already-started run (by id) to a terminal status. */
async function waitRun(runId: string): Promise<any> {
  const deadline = Date.now() + 60_000;
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const g = await ctx.get(`${base}${V1}/workflow-runs/${runId}`);
    expect(g.ok(), await g.text()).toBeTruthy();
    const run = await g.json();
    if (run.status !== 'running' && run.status !== 'pending') return run;
    if (Date.now() > deadline) throw new Error(`run ${runId} did not finish: ${run.status}`);
    await new Promise((res) => setTimeout(res, 300));
  }
}

// Run on the desktop-browser project only (the API assertions don't need a
// browser, and the UI smoke is desktop-only) — mobile projects skip.
test.beforeEach(async ({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
});

test.beforeAll(async ({}, testInfo) => {
  if (testInfo.project.name !== 'desktop-browser') return;
  const a = await apiCtx();
  ctx = a.ctx;
  base = a.base;
  ws = await seedWorkspace(ctx, base);

  // Branching workflow: trigger → agent (session) → set score → condition →
  // {passed | failed} via edge conditions on the condition's `result`.
  branchWfId = await createWorkflow(
    'E2E Branching',
    [
      node('trigger', 'manual_trigger'),
      node('agent', 'agent_prompt', { prompt: 'say hi' }),
      node('setscore', 'transform', { json: { score: 90 } }),
      node('cond', 'condition', { expr: 'score >= 80' }),
      node('passed', 'log'),
      node('failed', 'log'),
    ],
    [
      edge('trigger', 'agent'),
      edge('agent', 'setscore'),
      edge('setscore', 'cond'),
      edge('cond', 'passed', 'output.result == true'),
      edge('cond', 'failed', 'output.result == false'),
    ],
  );

  // Loop workflow: a transform step sets score=90; the loop's `until` is met on
  // the first iteration.
  loopWfId = await createWorkflow(
    'E2E Loop',
    [
      node('trigger', 'manual_trigger'),
      node('loopn', 'loop', {
        max_iterations: 3,
        until: 'last.score >= 80',
        steps: [{ kind: 'transform', name: 'set', params: { json: { score: 90 } } }],
      }),
    ],
    [edge('trigger', 'loopn')],
  );

  // Dedicated single-run workflow for the session assertion (its own agent prompt
  // → guaranteed cache miss → the agent actually runs and spawns a session).
  sessionWfId = await createWorkflow(
    'E2E Session',
    [node('trigger', 'manual_trigger'), node('agent', 'agent_prompt', { prompt: 'unique session probe' })],
    [edge('trigger', 'agent')],
  );
});

test.afterAll(async () => {
  await ctx?.dispose();
});

test('conditional branching: true branch runs, false branch is branch-skipped (not error)', async () => {
  const run = await runToCompletion(branchWfId);
  expect(run.status).toBe('success');
  expect(nodeState(run, 'passed').status).toBe('success');
  expect(nodeState(run, 'failed').status).toBe('skipped');
  // The skip reason is "branch not taken", not an upstream failure.
  const failedLogs = (nodeState(run, 'failed').logs ?? []).join(' ');
  expect(failedLogs.toLowerCase()).toContain('branch not taken');
});

test('agent node surfaces an openable session on its step', async () => {
  const run = await runToCompletion(sessionWfId);
  const agent = nodeState(run, 'agent');
  expect(agent.status).toBe('success');
  expect(Array.isArray(agent.sessions) && agent.sessions.length >= 1).toBeTruthy();
});

test('a finished run links a Proof Pack with evidence', async () => {
  const run = await runToCompletion(branchWfId);
  expect(run.proof_pack_id, 'run links a proof pack').toBeTruthy();
  const p = await ctx.get(`${base}${V1}/proof-packs/${run.proof_pack_id}`);
  expect(p.ok(), await p.text()).toBeTruthy();
  const detail = await p.json();
  expect((detail.artifacts ?? []).length).toBeGreaterThan(0);
});

test('loop stops when the until-expression is satisfied', async () => {
  const run = await runToCompletion(loopWfId);
  expect(run.status).toBe('success');
  const out = nodeState(run, 'loopn').output;
  expect(out.satisfied).toBe(true);
  expect(out.iterations).toBe(1);
});

test('workflow versioning: graph edits snapshot, restore appends a version', async () => {
  // v1 exists from create.
  let v = await ctx.get(`${base}${V1}/workflows/${loopWfId}/versions`);
  expect(v.ok()).toBeTruthy();
  let versions = await v.json();
  expect(versions.length).toBe(1);
  expect(versions[0].version).toBe(1);

  // A graph-changing PATCH bumps to v2.
  const patch = await ctx.patch(`${base}${V1}/workflows/${loopWfId}`, {
    data: { graph: { nodes: [node('trigger', 'manual_trigger')], edges: [] } },
  });
  expect(patch.ok(), await patch.text()).toBeTruthy();
  expect((await patch.json()).version).toBe(2);

  v = await ctx.get(`${base}${V1}/workflows/${loopWfId}/versions`);
  versions = await v.json();
  expect(versions.length).toBe(2);
  expect(versions[0].version).toBe(2);

  // Restore v1 → a NEW version (3) whose graph equals v1's.
  const restore = await ctx.post(`${base}${V1}/workflows/${loopWfId}/versions/1/restore`, {
    data: { note: 'back to original' },
  });
  expect(restore.ok(), await restore.text()).toBeTruthy();
  expect((await restore.json()).version).toBe(3);
  const wf = await (await ctx.get(`${base}${V1}/workflows/${loopWfId}`)).json();
  expect(wf.graph.nodes.find((n: any) => n.id === 'loopn'), 'v1 loop node restored').toBeTruthy();
});

test('convert a scheduled task into a workflow', async () => {
  const t = await ctx.post(`${base}${V1}/workspaces/${ws}/scheduled-tasks`, {
    data: {
      name: 'E2E Convertible',
      prompt: 'Summarize the repo.',
      schedule: { cadence: 'interval', every_min: 60 },
      destination: { type: 'none' },
      enabled: true,
    },
  });
  expect(t.ok(), await t.text()).toBeTruthy();
  const taskId = (await t.json()).id;

  const c = await ctx.post(`${base}${V1}/scheduled-tasks/${taskId}/convert-to-workflow`, {
    data: { disable_task: true },
  });
  expect(c.ok(), await c.text()).toBeTruthy();
  const { workflow_id } = await c.json();
  expect(workflow_id).toBeTruthy();

  const wf = await (await ctx.get(`${base}${V1}/workflows/${workflow_id}`)).json();
  const kinds = wf.graph.nodes.map((n: any) => n.kind);
  expect(kinds).toContain('manual_trigger');
  expect(kinds).toContain('agent_prompt');
  // A schedule trigger mirroring the cadence was created.
  const trigs = await (await ctx.get(`${base}${V1}/workflows/${workflow_id}/triggers`)).json();
  expect(trigs.some((tr: any) => tr.kind === 'schedule')).toBeTruthy();
});

test('manual_trigger node fields seed the run input', async () => {
  // Fields configured on the Start node become the run input (overridable by /run).
  const wfId = await createWorkflow(
    'E2E Trigger Fields',
    [
      {
        id: 'trigger',
        kind: 'manual_trigger',
        name: 'Start',
        x: 0,
        y: 0,
        params: { msg: 'do the thing', goals: ['g1', 'g2'], working_directory: '~/x' },
      },
      node('echo', 'log'),
    ],
    [edge('trigger', 'echo')],
  );
  const run = await runToCompletion(wfId);
  expect(run.status).toBe('success');
  const out = nodeState(run, 'trigger').output;
  expect(out.msg).toBe('do the thing');
  expect(out.goals).toEqual(['g1', 'g2']);
  expect(out.working_directory).toBe('~/x');
});

test('orchestrator example templates are offered', async () => {
  const r = await ctx.get(`${base}${V1}/workflows/templates`);
  expect(r.ok()).toBeTruthy();
  const ids = (await r.json()).map((t: any) => t.id);
  expect(ids).toContain('write-tests');
  expect(ids).toContain('implement-feature');
  expect(ids).toContain('po-lifecycle');
});

test('code templates auto-open the PR on review pass (no human approval)', async () => {
  const tmpls = await (await ctx.get(`${base}${V1}/workflows/templates`)).json();
  for (const id of ['write-tests', 'implement-feature']) {
    const t = tmpls.find((x: any) => x.id === id);
    expect(t, `template ${id}`).toBeTruthy();
    const kinds = t.graph.nodes.map((n: any) => n.kind);
    // The manual-approval step was dropped — the review passing IS the approval.
    expect(kinds, `${id} has no human_approval`).not.toContain('human_approval');
    // git_pr opens the real PR (per-step opt-in).
    const pr = t.graph.nodes.find((n: any) => n.kind === 'git_pr');
    expect(pr, `${id} has a git_pr`).toBeTruthy();
    expect(pr.params.open, `${id} PR opens`).toBe(true);
    // The edge into the PR step is GATED on the review having passed.
    const prEdge = t.graph.edges.find((e: any) => e.target === pr.id);
    expect(prEdge?.condition, `${id} PR edge is conditional`).toBeTruthy();
    // The loop is REVIEW-first then fix (design §E): review precedes fix, and
    // the until references the review step by name.
    const loop = t.graph.nodes.find((n: any) => n.kind === 'loop');
    const stepKinds = loop.params.steps.map((s: any) => s.kind);
    expect(stepKinds[0], `${id} reviews before fixing`).toBe('review_run');
    expect(stepKinds).toContain('agent_prompt');
    expect(loop.params.until, `${id} until references the review step`).toContain('steps.review');
    // Per-lens multi-agent reviewers + a summarizer + a scoring guideline (design §F/§G).
    const review = loop.params.steps.find((s: any) => s.kind === 'review_run');
    expect(Array.isArray(review.params.reviewers), `${id} review has reviewers`).toBeTruthy();
    expect(review.params.reviewers.length).toBeGreaterThan(0);
    // At least one reviewer fans out to multiple providers (e.g. claude+codex).
    expect(
      review.params.reviewers.some((r: any) => Array.isArray(r.providers) && r.providers.length >= 2),
      `${id} has a multi-provider reviewer`,
    ).toBeTruthy();
    expect(review.params.summarizer?.provider, `${id} review has a summarizer`).toBeTruthy();
    expect(review.params.scoring, `${id} review has a scoring guideline`).toBeTruthy();
    // The template offers improvements as a separate, terminal block (design §I).
    expect(kinds, `${id} offers improvements`).toContain('self_improve');
  }
});

test('per-step skills + multi-provider/lens review params round-trip', async () => {
  // The new node params persist through save → reload (the contract the engine
  // reads at run time: skills injection, multi-provider/lens review, auto-PR gate).
  const wfId = await createWorkflow(
    'E2E Step Skills',
    [
      node('trigger', 'manual_trigger'),
      {
        id: 'impl',
        kind: 'agent_prompt',
        name: 'impl',
        x: 0,
        y: 0,
        params: { prompt: 'implement', skills: ['golang-feature-implementation'] },
      },
      {
        id: 'rev',
        kind: 'review_run',
        name: 'rev',
        x: 0,
        y: 0,
        params: {
          repo_id: 'r',
          threshold: 85,
          providers: ['claude', 'codex'],
          lenses: ['correctness-review', 'security-review'],
          require_pass: true,
        },
      },
      { id: 'pr', kind: 'git_pr', name: 'pr', x: 0, y: 0, params: { open: true } },
    ],
    [edge('trigger', 'impl'), edge('impl', 'rev'), edge('rev', 'pr', 'output.passed == true')],
  );
  const wf = await (await ctx.get(`${base}${V1}/workflows/${wfId}`)).json();
  const byId = (id: string) => wf.graph.nodes.find((n: any) => n.id === id);
  expect(byId('impl').params.skills).toEqual(['golang-feature-implementation']);
  expect(byId('rev').params.providers).toEqual(['claude', 'codex']);
  expect(byId('rev').params.lenses).toEqual(['correctness-review', 'security-review']);
  expect(byId('rev').params.threshold).toBe(85);
  expect(byId('rev').params.require_pass).toBe(true);
  expect(byId('pr').params.open).toBe(true);
  const prEdge = wf.graph.edges.find((e: any) => e.target === 'pr');
  expect(prEdge.condition).toBe('output.passed == true');
});

test('review/PR repo_id derives from the working_directory (no "missing repo_id")', async () => {
  // A workflow given ONLY a working_directory (no repo_id) must still resolve a
  // repo for review/PR steps. The engine seeds repo_id into the run input from
  // the working_directory; the manual_trigger node emits that input as output.
  const { repoId, dir } = await seedGitRepo(ctx, base, ws);
  const wfId = await createWorkflow(
    'E2E Repo Derive',
    [
      {
        id: 'trigger',
        kind: 'manual_trigger',
        name: 'Start',
        x: 0,
        y: 0,
        params: { working_directory: dir },
      },
      node('echo', 'log'),
    ],
    [edge('trigger', 'echo')],
  );
  const run = await runToCompletion(wfId);
  expect(run.status).toBe('success');
  expect(nodeState(run, 'trigger').output.repo_id, 'repo_id derived from working_directory').toBe(
    repoId,
  );
});

test('the implementer publishes its working directory; review_run publishes the reference', async () => {
  // The reference (repo/base/worktree) flows FROM the implementer TO the
  // reviewer/PR: the agent step emits `working_directory`, and review_run emits
  // the exact `repo_id`/`base`/`worktree` it used — so a downstream git_pr
  // inherits them with nothing re-typed.
  const { repoId, dir } = await seedGitRepo(ctx, base, ws);
  // Create the base branch so the review starts cleanly (empty diff vs develop).
  execFileSync('git', ['-C', dir, 'branch', 'develop'], { stdio: 'ignore' });
  const wfId = await createWorkflow(
    'E2E Reference Flow',
    [
      {
        id: 'trigger',
        kind: 'manual_trigger',
        name: 'Start',
        x: 0,
        y: 0,
        params: { working_directory: dir, base: 'develop' },
      },
      node('implement', 'agent_prompt', { prompt: 'do the work' }),
      // review_run with no repo_id/base/worktree — must inherit from the run.
      node('review', 'review_run', { await: false }),
    ],
    [edge('trigger', 'implement'), edge('implement', 'review')],
  );
  const run = await runToCompletion(wfId);
  // The implementer reports where it worked.
  expect(nodeState(run, 'implement').output.working_directory, 'agent publishes its cwd').toBe(dir);
  // The reviewer inherits + publishes the exact reference for the PR.
  const rev = nodeState(run, 'review').output;
  expect(rev.repo_id, 'review inherits repo from the implementer').toBe(repoId);
  expect(rev.base, 'review inherits the run base').toBe('develop');
  expect(rev.worktree, 'review reviews where the implementer worked').toBe(dir);
});

test('active-runs endpoint lists an in-flight run, then drops it on completion', async () => {
  const wfId = await createWorkflow(
    'E2E Active',
    [node('trigger', 'manual_trigger'), node('wait', 'delay', { ms: 3000 }), node('done', 'log')],
    [edge('trigger', 'wait'), edge('wait', 'done')],
  );
  const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, { data: {} });
  expect(r.ok(), await r.text()).toBeTruthy();
  const runId = (await r.json()).id as string;

  // While running it appears in the workspace-wide active list, with progress.
  // Poll until the engine has populated the node states (nodes_total = 3) — there
  // is a brief `pending` window right after create where nodes_json is still `[]`.
  let mine: any = null;
  const deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    const a = await ctx.get(`${base}${V1}/workspaces/${ws}/workflow-runs/active`);
    expect(a.ok(), await a.text()).toBeTruthy();
    const found = (await a.json()).find((x: any) => x.run_id === runId);
    if (found && found.nodes_total === 3) {
      mine = found;
      break;
    }
    await new Promise((res) => setTimeout(res, 150));
  }
  expect(mine, 'run appeared in the active list with its steps populated').toBeTruthy();
  expect(mine.workflow_name).toBe('E2E Active');
  expect(['pending', 'running']).toContain(mine.status);
  expect(mine.nodes_done).toBeLessThanOrEqual(mine.nodes_total);

  // Once terminal it leaves the active list.
  await waitRun(runId);
  const a2 = await ctx.get(`${base}${V1}/workspaces/${ws}/workflow-runs/active`);
  const active2 = await a2.json();
  expect(active2.find((x: any) => x.run_id === runId), 'finished run is no longer active').toBeFalsy();
});

// NOTE on hiding workflow sessions from the Agents list (design §A): this can't
// be faithfully exercised in the offline E2E harness. Workflow agent turns
// short-circuit (`run_session_turn` returns a synthetic id, no row), and a real
// agent session can't be created either — the daemon points CLAUDE_BIN at a
// nonexistent path, so `manager.create` deletes the row when the PTY fails to
// spawn. The behavior is a one-line client filter (`meta.source !== 'workflow'`)
// added alongside the 8 existing identical source-filters in
// `ui/src/lib/stores/workspace.svelte.ts`, and the engine already stamps
// `meta.source = "workflow"` (workflow_engine.rs → run_session_turn → create).

// --- Desktop UI smoke -------------------------------------------------------

test.describe('workflows page (desktop)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
    await page.addInitScript((wsId) => {
      localStorage.setItem('otto_workspace', wsId as string);
      localStorage.setItem('otto_rail_expanded', '0');
    }, ws);
  });

  test('renders the workflows page and lists a seeded workflow', async ({ page }) => {
    await page.goto('/#/workflows');
    await expect(page.getByText('E2E Branching').first()).toBeVisible({ timeout: 30_000 });
  });

  test('templates live in a dropdown (room freed); no always-open "Game templates"', async ({
    page,
  }) => {
    await page.goto('/#/workflows');
    await expect(page.getByText('E2E Branching').first()).toBeVisible({ timeout: 30_000 });
    // The old always-open template list is gone (room freed for Workflows/Running).
    await expect(page.getByText('Game templates')).toHaveCount(0);
    // Templates are reachable via a collapsed dropdown.
    const btn = page.getByRole('button', { name: /Templates/ });
    await expect(btn).toBeVisible();
    await btn.click();
    await expect(page.getByText('Write tests for a story').first()).toBeVisible();
  });

  test('Running list shows an in-flight run; the detail auto-updates to success', async ({
    page,
  }) => {
    const wfId = await createWorkflow(
      'E2E Live',
      [node('trigger', 'manual_trigger'), node('wait', 'delay', { ms: 6000 }), node('done', 'log')],
      [edge('trigger', 'wait'), edge('wait', 'done')],
    );
    const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, { data: {} });
    expect(r.ok(), await r.text()).toBeTruthy();

    await page.goto('/#/workflows');
    // The Running section lists the in-flight run.
    const running = page.getByTestId('running-workflows');
    await expect(running).toBeVisible({ timeout: 15_000 });
    await expect(running.getByText('E2E Live')).toBeVisible();

    // Open it from the Running list, then watch the status flip to success IN
    // PLACE — no re-navigation — once the delay elapses (live auto-update).
    await running.getByText('E2E Live').click();
    const label = page.locator('.timeline .tl-label');
    await expect(label).toBeVisible({ timeout: 10_000 });
    await expect(label).toContainText('success', { timeout: 25_000 });
  });
});
