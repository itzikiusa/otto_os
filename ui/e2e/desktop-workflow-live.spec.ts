import { test, expect } from '@playwright/test';
import { agentStep, cancelStarted, chain, createWorkflow, needsYou, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, runsTab } from './workflowsUi';

// Live updates (ux.md §3, §16): a state change reaches the DOM within a
// second of its event; events that do not end a run cause no Recent list
// request; an open running step's body is fetched at most once a second.

let api: WfApi;
test.beforeAll(async ({}, info) => {
  if (!(info.project.name === 'desktop-browser')) return;
  api = await wfApi();
});
test.afterEach(async () => {
  await cancelStarted(api);
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

test('a state change is on screen within a second; non-terminal events fetch no list', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Live', chain(agentStep('a', 'Ask first', ['approve Bash', 'sleep 3000'])));
  await runsTab(page);
  await expect(page.getByTestId('wf-runs-recent')).toBeVisible({ timeout: 20_000 });
  const lists: number[] = [];
  page.on('request', (r) => { if (/\/workspaces\/[^/]+\/workflow-runs\?/.test(r.url())) lists.push(Date.now()); });
  const run = await startRun(api, wf);
  await waitRun(api, run, needsYou);
  const seen = Date.now();
  await expect(page.getByTestId('wf-runs-needs').locator(`[data-run-id="${run}"]`)).toBeVisible({ timeout: 1_500 });
  expect(Date.now() - seen).toBeLessThan(1_500);
  expect(lists, 'no Recent refetch for a run that has not ended').toEqual([]);
});

test('an open running step fetches its body at most once a second', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Live body', chain(agentStep('a', 'Stream', ['stream 4000 8000'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, (r) => r.nodes.some((n) => n.node_id === 'a' && n.status === 'running'));
  const bodies: number[] = [];
  page.on('request', (r) => { if (r.url().includes(`/workflow-runs/${run}/nodes/a`)) bodies.push(Date.now()); });
  await openRun(page, wf, run, 'a');
  await page.getByTestId('wf-step-tab-output').click();
  await page.waitForTimeout(5_000); // ui-guards: allow (counting requests over a fixed window)
  // The stream changes the node all the time: the body is read again, but no
  // more than once a second (2–6 reads in 5 s; 0 or 1 would prove nothing).
  expect(bodies.length, `${bodies.length} body reads`).toBeGreaterThanOrEqual(2);
  expect(bodies.length).toBeLessThanOrEqual(6);
  const gaps = bodies.slice(1).map((t, i) => t - bodies[i]);
  expect(gaps.every((g) => g >= 900), `body fetches ${gaps.join(', ')} ms apart`).toBeTruthy();
});
