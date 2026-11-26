import { test, expect } from '@playwright/test';
import { agentStep, chain, createWorkflow, settled, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, stepPane } from './workflowsUi';

// Old runs and deleted threads (ux.md §4.6, §16; test-plan.md §5.10): a run
// from before the upgrade (PTY sessions, no phases, no threads) shows "Older
// run" and its sessions open in Agents; a run whose step threads retention
// deleted shows the digest instead of the live conversation. The old-format
// row is served through page.route over a real run (test-plan.md §7 allows
// the frozen-old-type row to be seeded; this is that row's shape).

let api: WfApi;
test.beforeAll(async ({}, info) => {
  if (!(info.project.name === 'desktop-browser')) return;
  api = await wfApi();
});
test.afterAll(async () => {
  await api?.ctx.dispose();
});
test.beforeEach(async ({ page }, info) => {
  test.skip(!(info.project.name === 'desktop-browser'), 'not this project');
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, api.ws);
});

test('a run from before the upgrade says so and lists its sessions', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Old run', chain(agentStep('a', 'Old step')));
  const run = await startRun(api, wf);
  await waitRun(api, run, settled);
  await page.route(`**/api/v1/workflow-runs/${run}`, async (route) => {
    const res = await route.fetch();
    const r = await res.json();
    delete r.overview;
    r.nodes = r.nodes.map((n: Record<string, unknown>) => ({ ...n, phases: undefined, thread: undefined, sessions: n.node_id === 'a' ? ['sess_old_1'] : [] }));
    await route.fulfill({ response: res, json: r });
  });
  await openRun(page, wf, run, 'a');
  await expect(page.getByText('Older run')).toBeVisible();
  await expect(stepPane(page)).toContainText(/Session/);
});

test('a deleted thread shows the kept digest, read-only', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Retention', chain(agentStep('a', 'Answer')));
  const run = await startRun(api, wf);
  const done = await waitRun(api, run, settled);
  const node = done.nodes.find((n) => n.node_id === 'a')!;
  await page.route(`**/api/v1/workflow-runs/${run}/nodes/a`, async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    body.body.thread = { ...(node.thread ?? { thread_id: 'gone', runner: 'threads', provider: 'claude', turns: 1, usage: {} }), available: false, digest: { last_answer_excerpt: 'The answer the run kept.' } };
    await route.fulfill({ response: res, json: body });
  });
  await page.route(`**/api/v1/workflow-runs/${run}`, async (route) => {
    const res = await route.fetch();
    const r = await res.json();
    r.nodes = r.nodes.map((n: { node_id: string; thread?: Record<string, unknown> }) => (n.node_id === 'a' ? { ...n, thread: { ...(n.thread ?? { thread_id: 'gone', runner: 'threads', provider: 'claude', turns: 1, usage: {} }), available: false } } : n));
    await route.fulfill({ response: res, json: r });
  });
  await openRun(page, wf, run, 'a');
  await expect(page.getByTestId('wf-step-digest')).toBeVisible();
  await expect(page.getByTestId('wf-step-digest')).toContainText('The answer the run kept.');
  await expect(page.getByTestId('wf-step-conversation')).toHaveCount(0);
});
