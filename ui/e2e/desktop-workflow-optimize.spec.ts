import { test, expect } from '@playwright/test';
import { createWorkflow, prShape, wfApi, type WfApi } from './workflowsFixture';

// Optimize (addendum-chat-optimize.md §2.4, §2.6): the draft's changes in
// their effect groups with the lens hold unticked; unticking re-asks the
// draft and the After view changes; Not offered has reasons and no checkbox;
// Apply confirms and saves the next version with its note, the previous one
// stays in Versions and Restore brings it back; a save made elsewhere makes
// Apply wait for Check again. The graph is a small clone of the PR-review
// shape (three independent agent steps that only need the checkout).

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



test('the review page: groups, the lens hold unticked, re-asking on a change, Not offered', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Optimize', prShape());
  await page.goto(`/#/workflows/${wf}/optimize`);
  await expect(page.getByTestId('wf-opt-page')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('wf-opt-estimate')).toBeVisible();
  await expect(page.getByTestId('wf-opt-checks')).toContainText('Checked: valid');
  // The lens hold is offered, unticked.
  const lens = page.locator('[data-testid="wf-opt-change"][data-rule="review_declared_lenses"]');
  await expect(lens).toHaveCount(1);
  await expect(lens.getByRole('checkbox')).not.toBeChecked();
  const first = page.locator('[data-testid="wf-opt-change"][data-preselected="true"]').first();
  await expect(first).toBeVisible();
  const asked = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith(`/workflows/${wf}/optimize`) && !!r.postDataJSON()?.change_ids);
  await first.getByRole('checkbox').uncheck();
  await asked;
  // Not offered: the single-repo skip, whose edge already has a condition — a reason, no checkbox.
  const notOffered = page.getByTestId('wf-opt-not-offered');
  await notOffered.getByRole('button', { name: /Not offered/ }).click();
  await expect(notOffered.getByRole('listitem').filter({ hasText: 'already has a condition' })).toHaveCount(1);
  await expect(notOffered.getByRole('checkbox')).toHaveCount(0);
});

test('Apply confirms, saves the next version with its note; Restore brings the old graph back', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Optimize apply', prShape());
  const before = await api.get<{ version: number; graph: unknown }>(`/workflows/${wf}`);
  await page.goto(`/#/workflows/${wf}/optimize`);
  await page.getByTestId('wf-opt-apply').click();
  const dialog = page.getByRole('dialog', { name: new RegExp(`Save as version ${before.version + 1}`) });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: `Save version ${before.version + 1}` }).click();
  await expect(page).toHaveURL(new RegExp(`#/workflows/${wf}$`), { timeout: 15_000 });
  const after = await api.get<{ version: number }>(`/workflows/${wf}`);
  expect(after.version).toBe(before.version + 1);
  const versions = await api.get<{ version: number; note?: string }[]>(`/workflows/${wf}/versions?summary=true&limit=5`);
  expect(versions.find((v) => v.version === after.version)?.note ?? '').toMatch(/^Optimized by /);
  await api.post(`/workflows/${wf}/versions/${before.version}/restore`);
  expect((await api.get<{ graph: unknown }>(`/workflows/${wf}`)).graph).toEqual(before.graph);
});

test('a save made elsewhere makes Apply wait for Check again', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Optimize stale', prShape());
  await page.goto(`/#/workflows/${wf}/optimize`);
  await expect(page.getByTestId('wf-opt-page')).toBeVisible({ timeout: 20_000 });
  const cur = await api.get<{ graph: unknown }>(`/workflows/${wf}`);
  await api.ctx.patch(`${api.base}/api/v1/workflows/${wf}`, { data: { graph: cur.graph, instructions: 'changed elsewhere' } });
  await page.getByTestId('wf-opt-apply').click();
  await page.getByRole('dialog').getByRole('button', { name: /^Save version/ }).click();
  await expect(page.getByRole('button', { name: 'Check again' })).toBeVisible();
  await expect(page.getByTestId('wf-opt-apply')).toBeDisabled();
});
