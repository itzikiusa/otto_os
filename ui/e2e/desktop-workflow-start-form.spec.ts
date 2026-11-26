import { test, expect } from '@playwright/test';
import { expectFullyInViewport } from './helpers';
import { agentStep, chain, createWorkflow, settled, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun } from './workflowsUi';

// The start form (ux.md §7, §16): fields from the Start step's `inputs`,
// required validation, Run again pre-fills, the JSON escape round-trips, the
// modal fits the viewport, Run lands on the new run's page.

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

const inputs = [
  { key: 'pr', label: 'Pull request', required: true },
  { key: 'msg', label: 'Message to the agents', multiline: true },
];
function withInputs() {
  const g = chain(agentStep('one', 'One step'));
  g.nodes[0].params = { inputs };
  return g;
}

test('fields come from inputs; a required one holds Run; JSON round-trips; Run opens the run', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Start form', withInputs());
  await page.goto(`/#/workflows/${wf}`);
  await page.getByTestId('page-header').first().getByRole('button', { name: 'Run…' }).click();
  const form = page.getByTestId('wf-start-form');
  await expect(form).toBeVisible();
  await expectFullyInViewport(page, page.getByRole('dialog'));
  await expect(form.getByTestId('wf-start-field-pr')).toBeVisible();
  await expect(form.getByTestId('wf-start-field-msg')).toBeVisible();
  await page.getByTestId('wf-start-run').click();
  await expect(form).toContainText('Fill in Pull request');
  await form.getByTestId('wf-start-field-pr').fill('#412');
  await form.getByText('Input as JSON').click();
  await expect(form.getByTestId('wf-start-json')).toHaveValue(/"pr": "#412"/);
  await form.getByTestId('wf-start-json').fill('{"pr": "#413", "msg": "from json"}');
  await expect(form.getByTestId('wf-start-field-pr')).toHaveValue('#413');
  await page.getByTestId('wf-start-run').click();
  await expect(page).toHaveURL(new RegExp(`#/workflows/${wf}/runs/[^/]+$`), { timeout: 15_000 });
  await expect(page.getByTestId('wf-run-page')).toBeVisible();
});

test('Run again… fills the form from the run; the chat thread is offered, not chosen', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Run again', withInputs());
  // A Manual run names its chat destination with the result_* keys (channel / chat /
  // thread are a chat trigger's own and strip_reserved_input drops them here).
  const run = await startRun(api, wf, { pr: '#77', msg: 'look at retries', result_channel: 'slack', result_chat: 'C1', result_thread: '1.2' });
  const read = await waitRun(api, run, settled);
  expect(read.input).toMatchObject({ result_channel: 'slack', result_chat: 'C1' });
  await openRun(page, wf, run);
  await page.getByTestId('wf-run-primary').click();
  const form = page.getByTestId('wf-start-form');
  await expect(form).toContainText('Filled from the run of');
  await expect(form.getByTestId('wf-start-field-pr')).toHaveValue('#77');
  const sendTo = form.getByTestId('wf-start-send-to');
  await expect(sendTo).toHaveValue('page');
  await expect(sendTo.locator('option')).toHaveCount(2);
});
