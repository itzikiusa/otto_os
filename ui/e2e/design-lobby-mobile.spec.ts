import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Design Hall lobby on a phone (iPhone / Android projects; portrait and
// landscape). Seeds a few designs (one archived) through the API and checks
// the LAYOUT and the reachable controls at phone width: the prompt line fits
// (the brief on its own row, Generate visible), the Group-by + filter chips
// wrap instead of scrolling the page, the rail stacks under the grid, every
// menu stays inside the viewport, and Archived → Unarchive works by touch.

const V1 = '/api/v1';
const stamp = `${Date.now().toString(36)}${process.pid.toString(36)}`;
const ARCHIVED = `Phone archived ${stamp}`;
let wsId = '';

async function frame(ctx: APIRequestContext, base: string, title: string): Promise<string> {
  const r = await ctx.post(`${base}${V1}/design/artifacts`, {
    data: { workspace_id: wsId, format: 'html', studio: 'frames', title, content: `<!doctype html><h1>${title}</h1>` },
  });
  expect(r.ok(), `create → ${r.status()} ${await r.text()}`).toBeTruthy();
  return (await r.json()).artifact.id;
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  for (let i = 0; i < 3; i++) await frame(ctx, base, `Phone frame ${i} ${stamp}`);
  const gone = await frame(ctx, base, ARCHIVED);
  expect((await ctx.delete(`${base}${V1}/design/artifacts/${gone}`)).ok()).toBeTruthy();
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsId);
});

test('the lobby fits a phone: prompt, filters, grid and rail', async ({ page }) => {
  await page.goto('/#/design');
  await expect(page.getByTestId('design-all')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('design-artifact-card').first()).toBeVisible({ timeout: 15_000 });

  // The prompt: brief, studio picker, ⋯ and Generate are all on screen.
  for (const id of ['design-prompt', 'design-prompt-studio', 'design-prompt-more', 'design-prompt-generate']) {
    await expectFullyInViewport(page, page.getByTestId(id), id);
  }
  await expect(page.getByTestId('design-search')).toBeVisible(); // leads the body on a phone
  await expect(page.getByTestId('design-grid-filters')).toBeVisible();
  await expect(page.getByTestId('design-needs-review')).toBeAttached();
  await expectNoHorizontalOverflow(page);

  // The prompt's ⋯ popover and the header ⋯ stay inside the viewport.
  await page.getByTestId('design-prompt-more').click();
  const menu = page.locator('.ctx-menu');
  await expect(menu.getByRole('menuitem', { name: /Draft only/ })).toBeVisible();
  await expectFullyInViewport(page, menu, 'prompt options');
  await page.keyboard.press('Escape');
  await page.getByTestId('design-more').click();
  await expect(menu.getByRole('menuitem', { name: 'Brand Kit' })).toBeVisible();
  await expectFullyInViewport(page, menu, 'header more');
  await page.keyboard.press('Escape');
});

test('Archived → Unarchive by touch on a phone', async ({ page }) => {
  await page.goto('/#/design');
  await expect(page.getByTestId('design-all')).toBeVisible({ timeout: 30_000 });
  await page.getByTestId('design-filter-archived').click();
  const card = page.getByTestId('design-artifact-card').filter({ hasText: ARCHIVED });
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card.getByTestId('design-card-archived')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await card.getByTestId('design-card-unarchive').click();
  await expect(card).toHaveCount(0);
});
