import { test, expect } from '@playwright/test';
import { agentStep, cancelStarted, chain, createWorkflow, needsYou, settled, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, runsTab } from './workflowsUi';

// Needs you (ux.md §2.1, §4.3, §12; test-plan.md §7): a question answered
// from the run page; the usage-limit card (no buttons, the run continues at
// the reset time); the sidebar badge counts waiting runs.

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

test('a question waits in Needs you and is answered in the step\'s conversation', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Question', chain(agentStep('a', 'Ask the person', ['ask Which environment?', 'write $HANDOFF done'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, needsYou);
  await runsTab(page);
  await expect(page.getByTestId('wf-runs-needs').locator(`[data-run-id="${run}"]`)).toContainText('Question', { timeout: 20_000 });
  await openRun(page, wf, run);
  const card = page.locator('[data-testid="wf-run-card"][data-kind="question"]');
  await expect(card).toContainText('Which environment?');
  await card.getByRole('button', { name: 'Answer in the conversation' }).click();
  await expect(page.getByTestId('wf-step-tab-conversation')).toHaveAttribute('aria-selected', 'true');
  const convo = page.getByTestId('wf-step-conversation');
  await expect(convo).toBeVisible();
  // The read-only conversation keeps its question answerable (DR-021).
  const form = convo.getByTestId('runtime-question-card');
  await form.getByRole('textbox', { name: 'Your answer' }).fill('staging');
  await form.getByTestId('runtime-question-send').click();
  // The fake reads the answer and finishes the step (fake_script.rs `ask`).
  const done = await waitRun(api, run, settled);
  expect(done.status).toBe('success');
  await expect(convo.getByText('answer: staging').first()).toBeVisible();
});

test('a usage limit with a known reset has no buttons; the run says when it goes on', async ({ page }) => {
  // The fake's limit names its reset ("resets 3pm", fake_script.rs): the step parks until then.
  const wf = await createWorkflow(api, 'E2E Limited', chain(agentStep('a', 'Hit the limit', ['fail usage_limit'])));
  const run = await startRun(api, wf);
  const r = await waitRun(api, run, needsYou);
  expect(r.overview?.attention?.[0]).toMatchObject({ kind: 'usage_limit', resets_at: expect.any(String) });
  await openRun(page, wf, run);
  const card = page.locator('[data-testid="wf-run-card"][data-kind="limit"]');
  await expect(card).toBeVisible();
  await expect(card).toContainText('Otto continues it at');
  await expect(card.getByRole('button')).toHaveCount(0);
});

test('the sidebar badge counts runs that wait on a person', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Badge', chain(agentStep('a', 'Approve me', ['approve Bash'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, needsYou);
  await runsTab(page);
  await expect(page.getByTestId('wf-tab-runs')).toContainText(/\d/);
  await expect(page.getByTestId('sidebar-modules').getByRole('button', { name: /Workflows.*waiting on you/ })).toBeVisible();
});
