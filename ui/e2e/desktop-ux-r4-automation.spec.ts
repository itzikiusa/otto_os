import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block', viewport: { width: 1440, height: 900 } });
async function setup(page: Page) {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  return { ctx, base, ws };
}

test('rooms: pending creation prevents duplicate Enter and preserves newer name', async ({ page }) => {
  const { ctx, ws } = await setup(page); await ctx.dispose();
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  let posts = 0;
  await page.route(`**/api/v1/workspaces/${ws}/agent-rooms`, async r => {
    if (r.request().method() !== 'POST') return r.continue();
    posts++; const response = await r.fetch(); await gate; await r.fulfill({ response });
  });
  await page.goto('/#/personal-agents/rooms');
  const input = page.getByLabel('New channel name');
  await input.fill('Review team'); await input.press('Enter');
  await expect.poll(() => posts).toBe(1);
  await input.fill('Release team'); await input.press('Enter');
  try { await expect.poll(() => posts).toBe(1); } finally { release(); }
  await expect(page.getByRole('button', { name: 'Review team 0 agents' })).toBeVisible();
  await expect(input).toHaveValue('Release team');
});

test('workflow canvas: keyboard selects a step and saved input survives reload', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  const workflow = await (await ctx.post(`${base}/api/v1/workspaces/${ws}/workflows`, { data: {
    name: 'Keyboard release checklist', graph: {
      nodes: [{ id: 'start', kind: 'manual_trigger', name: 'Release input', x: 40, y: 40, params: {} },
        { id: 'approval', kind: 'human_approval', name: 'Approve release checklist', x: 340, y: 40, params: { prompt: 'Approve synthetic release evidence?' } },
        { id: 'done', kind: 'log', name: 'Record decision', x: 640, y: 40, params: {} }],
      edges: [{ id: 'e1', source: 'start', target: 'approval' }, { id: 'e2', source: 'approval', target: 'done' }],
    },
  } })).json();
  await page.goto('/#/workflows');
  await page.getByTestId(`wf-row-${workflow.id}`).locator('.row-main').click();
  const node = page.locator('.node-main', { hasText: 'Release input' });
  await node.focus(); await page.keyboard.press('Enter');
  const input = page.getByLabel('Message / prompt', { exact: true });
  await expect(input).toBeVisible();
  await input.fill('Review long acceptance checklist before release');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
  await page.reload();
  await page.locator('.node-main', { hasText: 'Release input' }).focus(); await page.keyboard.press('Space');
  await expect(input).toHaveValue('Review long acceptance checklist before release');
  await ctx.dispose();
});
