import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { agentStep, chain, createWorkflow, wfApi, type WfApi } from './workflowsFixture';

// Optimize on a phone (addendum-chat-optimize.md §2.5): touch check and
// uncheck, the After outline in place of the canvas, Apply confirms; nothing
// overflows the screen.

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

test('tick and untick by touch; the After outline; Apply asks first', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Phone optimize', chain(agentStep('a', 'Gather'), agentStep('b', 'Check'), agentStep('c', 'Write')));
  await page.goto(`/#/workflows/${wf}/optimize`);
  await expect(page.getByTestId('wf-opt-page')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('[data-testid="wf-opt-canvas"][data-view="after"]')).toBeVisible();
  const change = page.getByTestId('wf-opt-change').first();
  if (await change.count()) {
    const box = change.getByRole('checkbox');
    const was = await box.isChecked();
    await box.tap();
    await expect(box).toBeChecked({ checked: !was });
  }
  await expectNoHorizontalOverflow(page);
  const apply = page.getByTestId('wf-opt-apply');
  if (await apply.isEnabled()) {
    await apply.tap();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expectFullyInViewport(page, dialog);
    await dialog.getByRole('button', { name: 'Cancel' }).tap();
  }
});
