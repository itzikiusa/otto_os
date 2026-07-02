import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Live-run view stability E2E. A workflow run must stream step changes into the
// open view WITHOUT resetting what the user is looking at: an expanded step stays
// expanded (same DOM element — updates are merged, not remounted), switching to
// view another run is never stomped by an in-flight run's poll loop, the run
// `rev` is monotonic (the UI's stale-snapshot guard), and a human-approval pause
// is announced promptly (pause/approve emit events; nothing waits on a poll).

const V1 = '/api/v1';

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
    data: { name, description: 'e2e-live', graph: { nodes, edges } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function startRun(wfId: string): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, { data: {} });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function getRun(runId: string): Promise<any> {
  const g = await ctx.get(`${base}${V1}/workflow-runs/${runId}`);
  expect(g.ok(), await g.text()).toBeTruthy();
  return g.json();
}

async function waitRun(runId: string, timeoutMs = 60_000): Promise<any> {
  const deadline = Date.now() + timeoutMs;
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const run = await getRun(runId);
    if (run.status !== 'running' && run.status !== 'pending') return run;
    if (Date.now() > deadline) throw new Error(`run ${runId} did not finish: ${run.status}`);
    await new Promise((res) => setTimeout(res, 300));
  }
}

/** Open the workflows page with the seeded workspace + collapsed rail. */
async function gotoWorkflows(page: Page): Promise<void> {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  await page.goto('/#/workflows');
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

test('expanded step survives live step transitions — same DOM node, no reset', async ({
  page,
}) => {
  const wfId = await createWorkflow(
    'E2E LiveStable',
    [
      node('trigger', 'manual_trigger'),
      node('w1', 'delay', { ms: 3000 }),
      node('l1', 'log'),
      node('w2', 'delay', { ms: 3000 }),
      node('l2', 'log'),
    ],
    [edge('trigger', 'w1'), edge('w1', 'l1'), edge('l1', 'w2'), edge('w2', 'l2')],
  );
  // Navigate FIRST so the page is up before the (short) run starts — under
  // parallel workers a pre-started run can finish before the page loads.
  await gotoWorkflows(page);
  await expect(page.getByText('E2E LiveStable').first()).toBeVisible({ timeout: 15_000 });
  await startRun(wfId);
  const running = page.getByTestId('running-workflows');
  await expect(running).toBeVisible({ timeout: 15_000 });
  await running.getByText('E2E LiveStable').click();

  // The run detail is up; expand the FIRST step (the already-finished trigger)
  // while later steps are still running.
  const steps = page.locator('.run-detail details');
  await expect(steps.first()).toBeVisible({ timeout: 10_000 });
  await steps.first().locator('summary').click();
  await expect(steps.first()).toHaveAttribute('open', '');

  // Select a timeline step too — selection must survive live updates.
  const tlL1 = page.locator('.timeline .tl-step', { hasText: 'l1' });
  await tlL1.click();
  await expect(tlL1).toHaveClass(/active/);

  // Tag the expanded element so we can prove it is UPDATED IN PLACE, not
  // recreated, across subsequent step transitions.
  await page.evaluate(() => {
    document.querySelector('.run-detail details')?.setAttribute('data-e2e-tag', 'kept');
  });

  // Let the run finish (several node start/finish events + polls flow through).
  const label = page.locator('.timeline .tl-label');
  await expect(label).toContainText('success', { timeout: 30_000 });

  // The user's view was never reset: still expanded, same DOM element, same
  // timeline selection.
  await expect(steps.first()).toHaveAttribute('open', '');
  await expect(steps.first()).toHaveAttribute('data-e2e-tag', 'kept');
  await expect(tlL1).toHaveClass(/active/);
});

test('viewing another run is not stomped by an in-flight run started from the page', async ({
  page,
}) => {
  // First run (via API, no input) SUCCEEDS; the second (started from the page,
  // with fail=true) takes the failing branch — so a stomped view is unmistakable
  // (the viewed 'success' run would flip to running/error).
  const wfId = await createWorkflow(
    'E2E Stomp',
    [
      node('trigger', 'manual_trigger'),
      node('w', 'delay', { ms: 4000 }),
      node('bad', 'http_request', { method: 'GET', url: 'http://127.0.0.1:9/refused' }),
      node('done', 'log'),
    ],
    [
      edge('trigger', 'w'),
      edge('w', 'bad', 'run.input.fail == true'),
      edge('w', 'done', 'run.input.fail != true'),
    ],
  );
  // A finished, SUCCESSFUL run to inspect later.
  const oldRunId = await startRun(wfId);
  const oldRun = await waitRun(oldRunId);
  expect(oldRun.status).toBe('success');

  await gotoWorkflows(page);
  await page.getByText('E2E Stomp').first().click();

  // Start a NEW run from the page with the failing input (this is the path that
  // owns the in-page run driver).
  await page.getByRole('button', { name: 'Run…' }).click();
  await page.locator('.ri-text').fill('{ "fail": true }');
  await page.locator('.ri-actions').getByRole('button', { name: 'Run' }).click();
  const label = page.locator('.timeline .tl-label');
  await expect(label).toContainText('running', { timeout: 10_000 });

  // Now open the OLD, completed run from the Runs dropdown — the user wants to
  // inspect it while the new one keeps running in the background.
  await page.getByRole('button', { name: 'Runs' }).click();
  await page.locator('.runs-pop .run-item', { hasText: 'success' }).first().click();
  await expect(label).toContainText('success');

  // The in-flight run's updates must NOT replace the viewed (completed) run —
  // across its mid-run ticks AND its (failing) completion.
  await page.waitForTimeout(3000);
  await expect(label).toContainText('success');
  await page.waitForTimeout(3500);
  await expect(label).toContainText('success');
  await expect(page.locator('.timeline .tl-step[data-status="error"]')).toHaveCount(0);
});

test('run rev is monotonic while running (stale-snapshot guard contract)', async () => {
  const wfId = await createWorkflow(
    'E2E Rev',
    [
      node('trigger', 'manual_trigger'),
      node('w1', 'delay', { ms: 1200 }),
      node('l1', 'log'),
      node('w2', 'delay', { ms: 1200 }),
    ],
    [edge('trigger', 'w1'), edge('w1', 'l1'), edge('l1', 'w2')],
  );
  const runId = await startRun(wfId);
  const revs: number[] = [];
  for (let i = 0; i < 5; i++) {
    const run = await getRun(runId);
    expect(typeof run.rev, 'WorkflowRun carries a rev').toBe('number');
    revs.push(run.rev);
    await new Promise((res) => setTimeout(res, 700));
  }
  for (let i = 1; i < revs.length; i++) {
    expect(revs[i], `rev never goes backward (${revs.join(',')})`).toBeGreaterThanOrEqual(
      revs[i - 1],
    );
  }
  expect(revs[revs.length - 1], 'rev advances as steps transition').toBeGreaterThan(revs[0]);
  await waitRun(runId);
});

test('human-approval pause is announced promptly; approve resumes to success', async ({
  page,
}) => {
  const wfId = await createWorkflow(
    'E2E Approve',
    [
      node('trigger', 'manual_trigger'),
      node('gate', 'human_approval', { prompt: 'ok to continue?' }),
      node('done', 'log'),
    ],
    [edge('trigger', 'gate'), edge('gate', 'done')],
  );

  await gotoWorkflows(page);
  const runId = await startRun(wfId);

  // The pause must surface as a ⏸ badge on the Running sidebar row promptly —
  // driven by the pause EVENT, not by waiting for a poll to happen by.
  const running = page.getByTestId('running-workflows');
  await expect(running).toBeVisible({ timeout: 15_000 });
  await expect(running.getByTitle('waiting for approval')).toBeVisible({ timeout: 5_000 });

  // Open the run: the approval banner is up; approve it.
  await running.getByText('E2E Approve').click();
  const banner = page.locator('.approval-banner');
  await expect(banner).toBeVisible({ timeout: 10_000 });
  await banner.getByRole('button', { name: 'Approve' }).click();

  // The run resumes and completes IN PLACE (no re-navigation), the banner drops.
  const label = page.locator('.timeline .tl-label');
  await expect(label).toContainText('success', { timeout: 20_000 });
  await expect(banner).toHaveCount(0);
  const run = await getRun(runId);
  expect(run.status).toBe('success');
  expect((run.nodes ?? []).find((n: any) => n.node_id === 'gate').status).toBe('success');
});

test('error step auto-expands once; a user collapse is never fought by updates', async ({
  page,
}) => {
  // The failing node sits on its OWN branch so the delay branch keeps producing
  // transitions after the error lands (that's what used to fight the collapse).
  const wfId = await createWorkflow(
    'E2E ErrCollapse',
    [
      node('trigger', 'manual_trigger'),
      node('bad', 'http_request', { method: 'GET', url: 'http://127.0.0.1:9/refused' }),
      node('w1', 'delay', { ms: 4000 }),
      node('l1', 'log'),
    ],
    [edge('trigger', 'bad'), edge('trigger', 'w1'), edge('w1', 'l1')],
  );
  // Navigate first (see the LiveStable test) — the run is short.
  await gotoWorkflows(page);
  await expect(page.getByText('E2E ErrCollapse').first()).toBeVisible({ timeout: 15_000 });
  await startRun(wfId);
  const running = page.getByTestId('running-workflows');
  await expect(running).toBeVisible({ timeout: 15_000 });
  await running.getByText('E2E ErrCollapse').click();

  // The failed step auto-expands (error visibility)…
  const badStep = page.locator('.run-detail details[data-status="error"]');
  await expect(badStep).toBeVisible({ timeout: 15_000 });
  await expect(badStep).toHaveAttribute('open', '');

  // …the user collapses it — and later transitions must not re-expand it.
  await badStep.locator('summary').click();
  await expect(badStep).not.toHaveAttribute('open', '');
  const label = page.locator('.timeline .tl-label');
  await expect(label).toContainText('error', { timeout: 25_000 }); // run terminal (bad branch failed)
  await expect(badStep).not.toHaveAttribute('open', '');
});
