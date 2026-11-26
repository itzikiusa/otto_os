import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { agentStep, chain, createWorkflow, wfApi, type WfApi } from './workflowsFixture';

// The editor on a phone (ux.md §9, §10, §16): a read-only outline of the
// steps, a step's settings in a sheet that Done saves, and no control that
// adds, connects or deletes steps.

let api: WfApi;
test.beforeAll(async ({}, info) => {
  if (!(/phone|iphone/.test(info.project.name))) return;
  api = await wfApi();
});
test.afterAll(async () => {
  await api?.ctx.dispose();
});
test.beforeEach(async ({ page }, info) => {
  test.skip(!(/phone|iphone/.test(info.project.name)), 'not this project');
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, api.ws);
});

test('outline, a step sheet that saves on Done, nothing that edits the graph', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Phone editor', chain(agentStep('a', 'Gather'), agentStep('b', 'Write')));
  await page.goto(`/#/workflows/${wf}`);
  const outline = page.getByTestId('wf-phone-outline');
  await expect(outline).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole('button', { name: /^Step$|Add step|Connect/ })).toHaveCount(0);
  await expect(page.getByTestId('wf-editor-canvas')).toHaveCount(0);
  await outline.getByRole('button', { name: /Write/ }).tap();
  const sheet = page.getByTestId('wf-step-sheet');
  await expect(sheet).toBeVisible();
  await expectFullyInViewport(page, sheet);
  await sheet.getByRole('textbox', { name: 'Name' }).fill('Write the summary');
  const saved = page.waitForRequest((r) => r.method() === 'PATCH' && r.url().endsWith(`/workflows/${wf}`));
  await page.getByRole('button', { name: 'Done' }).tap();
  const body = (await saved).postDataJSON();
  expect(body.graph.nodes.find((n: { id: string }) => n.id === 'b').name).toBe('Write the summary');
  await expectNoHorizontalOverflow(page);
});
