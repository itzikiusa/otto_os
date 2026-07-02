import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// Workflow run context files + repos declarations E2E (design 2026-07-02).
// Drives the API against the isolated OTTO_E2E daemon and asserts on the
// daemon's own OTTO_DATA_DIR (read from e2e/.auth-{SLOT}/daemon.json):
//
//  - a run materializes <data_dir>/workflow-context/<run_id>/ with the
//    wf-<run_id>-instruction.md brief, repos.json, and per-step handoff files
//    (step{N}-{name}.md + .output.json; loop iterations add -iter{X});
//  - an agent step's file carries the FULL stub reply (engine fallback — the
//    stub returns before any real agent could write its own summary);
//  - review_run succeeds on a repo whose only branch is `master` — the exact
//    production `git exited 128: fatal: ambiguous argument 'main'` regression —
//    both via default-branch detection and via a repos[] declaration;
//  - GET /workflow-runs/{id} surfaces the derived context_dir.

const V1 = '/api/v1';
const SLOT = process.env.OTTO_E2E_SLOT ?? '0';

let base = '';
let ctx: APIRequestContext;
let ws = '';

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

/** Run a workflow with the given input and poll its run row to a terminal status. */
async function runToCompletion(wfId: string, input?: unknown, deadlineMs = 90_000): Promise<any> {
  const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, {
    data: input === undefined ? {} : { input },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const runId = (await r.json()).id as string;
  const deadline = Date.now() + deadlineMs;
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

/** The daemon's per-run context dir, via the temp OTTO_DATA_DIR global-setup recorded. */
function ctxDir(runId: string): string {
  const meta = JSON.parse(
    readFileSync(join(process.cwd(), 'e2e', `.auth-${SLOT}`, 'daemon.json'), 'utf8'),
  ) as { dataDir: string };
  return join(meta.dataDir, 'workflow-context', runId);
}

/** A registered repo whose ONLY branch is `master` (explicit -b master — a bare
 *  `git init` follows the machine's init.defaultBranch, which would make the
 *  regression assertion nondeterministic), with one dirty file so a review has
 *  a non-empty diff. */
async function seedMasterOnlyRepo(): Promise<{ repoId: string; dir: string }> {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-master-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
  git('init', '-q', '-b', 'master');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  writeFileSync(join(dir, 'app.txt'), 'v1\n');
  git('add', '-A');
  git('commit', '-q', '-m', 'init on master');
  writeFileSync(join(dir, 'app.txt'), 'v1\nv2 uncommitted\n');
  const r = await ctx.post(`${base}${V1}/workspaces/${ws}/repos`, {
    data: { path: dir, name: 'e2e-master-only' },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return { repoId: (await r.json()).id as string, dir };
}

test.beforeEach(async ({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
});

test.beforeAll(async ({}, testInfo) => {
  if (testInfo.project.name !== 'desktop-browser') return;
  const a = await apiCtx();
  ctx = a.ctx;
  base = a.base;
  ws = await seedWorkspace(ctx, base);
});

test.afterAll(async () => {
  await ctx?.dispose();
});

test('run materializes instruction, repos.json and per-step files; context_dir surfaced', async () => {
  const { repoId, dir: repoDir } = await seedGitRepo(ctx, base, ws);
  const wfId = await createWorkflow(
    'E2E Context Files',
    [
      node('trigger', 'manual_trigger'),
      node('set', 'transform', { json: { note: 'ctx' } }),
      node('tail', 'log'),
    ],
    [edge('trigger', 'set'), edge('set', 'tail')],
  );
  const run = await runToCompletion(wfId, {
    repos: [{ repo: repoDir, type: 'worktree', name: repoDir }],
  });
  expect(run.status).toBe('success');
  // The derived context_dir rides the run detail and points at a real dir.
  const d = ctxDir(run.id);
  expect(run.context_dir).toBe(d);
  expect(existsSync(d)).toBe(true);

  // Instruction brief, named by the user's wf-{run_id} convention.
  const instruction = readFileSync(join(d, `wf-${run.id}-instruction.md`), 'utf8');
  expect(instruction).toContain('E2E Context Files');
  expect(instruction).toContain('repos.json');
  expect(instruction).toContain('trigger');

  // repos.json: the declared worktree entry, resolved to the registered repo.
  const repos = JSON.parse(readFileSync(join(d, 'repos.json'), 'utf8'));
  expect(Array.isArray(repos)).toBe(true);
  expect(repos[0].repo_id).toBe(repoId);
  expect(repos[0].worktree).toBeTruthy();
  expect(repos[0].error ?? null).toBeNull();

  // Per-step handoffs: every executed node left step{N}-{name}.md + raw output.
  const files = readdirSync(d);
  expect(files.some((f) => /^step\d+-trigger\.md$/.test(f)), files.join(', ')).toBe(true);
  expect(files.some((f) => /^step\d+-set\.md$/.test(f)), files.join(', ')).toBe(true);
  expect(files.some((f) => /^step\d+-set\.output\.json$/.test(f)), files.join(', ')).toBe(true);
  expect(files.some((f) => /^step\d+-tail\.md$/.test(f)), files.join(', ')).toBe(true);
  // The transform's raw output is intact JSON with the merged key.
  const setJson = files.find((f) => /^step\d+-set\.output\.json$/.test(f))!;
  expect(JSON.parse(readFileSync(join(d, setJson), 'utf8')).note).toBe('ctx');
});

test('agent step leaves an engine-fallback handoff file with the full stub reply', async () => {
  const wfId = await createWorkflow(
    'E2E Context Agent',
    [node('trigger', 'manual_trigger'), node('agent', 'agent_prompt', { prompt: 'context probe' })],
    [edge('trigger', 'agent')],
  );
  const run = await runToCompletion(wfId);
  expect(run.status).toBe('success');
  const d = ctxDir(run.id);
  const files = readdirSync(d);
  const agentMd = files.find((f) => /^step\d+-agent\.md$/.test(f));
  expect(agentMd, files.join(', ')).toBeTruthy();
  // The OTTO_E2E stub replies "OK" and returns before any real agent could
  // write its own summary — so this file IS the engine fallback, and it must
  // carry the reply verbatim (untruncated channel).
  expect(readFileSync(join(d, agentMd!), 'utf8')).toContain('OK');
});

test('review_run succeeds on a master-only repo — the exit-128 regression', async () => {
  // Two sequential reviews; each stub review pipeline (agents + summarizer,
  // with retry backoff) takes ~25-30s — well past the default 45s test cap.
  test.setTimeout(240_000);
  const { dir } = await seedMasterOnlyRepo();

  // A single reviewer lens keeps the stubbed pipeline fast (the default
  // config fans out several agents, each burning spawn-fail retries).
  const reviewParams = {
    await: true,
    timeout_s: 120,
    providers: ['claude'],
    lenses: ['correctness-review'],
  };

  // (a) No repos, no base — the historical hardcoded "main" fallback made
  // `git diff main` exit 128 here; default-branch detection must find master.
  const wfA = await createWorkflow(
    'E2E Review Master A',
    [
      node('trigger', 'manual_trigger'),
      node('review', 'review_run', reviewParams),
    ],
    [edge('trigger', 'review')],
  );
  const runA = await runToCompletion(wfA, { working_directory: dir }, 180_000);
  expect(runA.status, JSON.stringify(nodeState(runA, 'review'), null, 2)).toBe('success');
  expect(nodeState(runA, 'review').output.base).toBe('master');

  // (b) The same repo declared through repos[] (branch entry, no source) —
  // the registry supplies the target and the detected default becomes the base.
  const wfB = await createWorkflow(
    'E2E Review Master B',
    [
      node('trigger', 'manual_trigger'),
      node('review', 'review_run', reviewParams),
    ],
    [edge('trigger', 'review')],
  );
  const runB = await runToCompletion(
    wfB,
    { repos: [{ repo: dir, type: 'branch', name: 'master' }] },
    180_000,
  );
  expect(runB.status, JSON.stringify(nodeState(runB, 'review'), null, 2)).toBe('success');
  expect(nodeState(runB, 'review').output.base).toBe('master');
  // The reviewed reference lands in the run's repos.json registry too.
  const repos = JSON.parse(readFileSync(join(ctxDir(runB.id), 'repos.json'), 'utf8'));
  expect(repos[0].base).toBe('master');
});

test('loop iterations leave step{N}-{name}-iter{X} files', async () => {
  const wfId = await createWorkflow(
    'E2E Context Loop',
    [
      node('trigger', 'manual_trigger'),
      node('cycle', 'loop', {
        max_iterations: 2,
        steps: [{ kind: 'transform', name: 'tick', params: { json: { n: 1 } } }],
      }),
    ],
    [edge('trigger', 'cycle')],
  );
  const run = await runToCompletion(wfId);
  expect(run.status).toBe('success');
  const files = readdirSync(ctxDir(run.id));
  expect(files.some((f) => /^step\d+-tick-iter1\.md$/.test(f)), files.join(', ')).toBe(true);
  expect(files.some((f) => /^step\d+-tick-iter2\.md$/.test(f)), files.join(', ')).toBe(true);
  // The loop node itself also leaves its aggregate step file.
  expect(files.some((f) => /^step\d+-cycle\.md$/.test(f)), files.join(', ')).toBe(true);
});
