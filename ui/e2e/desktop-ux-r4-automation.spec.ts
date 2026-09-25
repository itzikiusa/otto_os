import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedSwarm } from './seed';
import { expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';

test.use({ serviceWorkers: 'block', viewport: { width: 1440, height: 900 } });
async function setup(page: Page) {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const fixture = await seedSwarm(ctx, base, ws);
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  return { ctx, base, ws, ...fixture };
}

test('desktop-to-tablet swarm: detail gains rail width and navigation preserves selection and focus', async ({ page }) => {
  const { ctx } = await setup(page); await ctx.dispose();
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/swarm');
  await page.getByRole('tab', { name: 'Feed', exact: true }).click();
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Keep this draft across layout changes');
  await page.locator('.swarm-item.active').focus();
  await expect(page.locator('.swarm-item.active')).toBeFocused();
  await page.setViewportSize({ width: 834, height: 1112 });
  await page.evaluate(() => document.documentElement.dir = 'rtl');
  await expect(page.locator('.rail-list')).toBeHidden();
  const toggle = page.getByRole('button', { name: 'Toggle swarms list' });
  await expect(toggle).toBeFocused();
  expect((await page.locator('.swarm-page .main').boundingBox())!.width).toBeGreaterThan(560);
  await toggle.press('Enter');
  const selected = page.locator('.swarm-item.active');
  await expect(selected).toBeVisible();
  await selected.focus(); await selected.press('Enter');
  await expect(toggle).toBeFocused();
  await expect(page.locator('.rail-list')).toBeHidden();
  await expect(page.getByRole('tab', { name: 'Feed', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('Keep this draft across layout changes');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: '/tmp/otto-ux-r4-automation-screens/tablet-rtl-feed.png' });
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(page.locator('.swarm-item.active')).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('Keep this draft across layout changes');
});

test('swarm board: pending creation prevents duplicate Enter and preserves newer title', async ({ page }) => {
  const { ctx, projectId } = await setup(page); await ctx.dispose();
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  let posts = 0;
  await page.route(`**/api/v1/swarm/projects/${projectId}/tasks`, async r => {
    if (r.request().method() !== 'POST') return r.continue();
    posts++; const response = await r.fetch(); await gate; await r.fulfill({ response });
  });
  await page.goto('/#/swarm');
  await page.getByRole('tab', { name: 'Board', exact: true }).click();
  await page.getByRole('button', { name: 'Add task', exact: true }).click();
  const input = page.getByLabel('New task title');
  await input.fill('Submitted task'); await input.press('Enter');
  await expect.poll(() => posts).toBe(1);
  await input.fill('Next task draft'); await input.press('Enter');
  try { await expect.poll(() => posts).toBe(1); } finally { release(); }
  await expect(page.getByText('Submitted task', { exact: true })).toBeVisible();
  await expect(input).toHaveValue('Next task draft');
});

test('swarm board: project change clears selection from the old board', async ({ page }) => {
  const { ctx, base, swarmId, projectId } = await setup(page);
  const second = await (await ctx.post(`${base}/api/v1/swarm/swarms/${swarmId}/projects`, { data: { name: 'Second project' } })).json();
  const tasks = await (await ctx.get(`${base}/api/v1/swarm/projects/${projectId}/tasks`)).json();
  expect(tasks.length).toBeGreaterThan(0);
  await ctx.dispose();
  let secondLoaded!: () => void;
  const secondReady = new Promise<void>(resolve => { secondLoaded = resolve; });
  await page.route(`**/api/v1/swarm/projects/${second.id}/tasks`, async r => { await r.fulfill({ json: [] }); secondLoaded(); });
  await page.route(`**/api/v1/swarm/projects/${projectId}/tasks`, async r => {
    await secondReady;
    // Complete the empty sibling first; the populated result must merge into the current cache.
    await new Promise(resolve => setTimeout(resolve, 150));
    await r.fulfill({ json: tasks });
  });
  await page.goto('/#/swarm');
  await page.getByRole('tab', { name: 'Board', exact: true }).click();
  await page.getByLabel('Project', { exact: true }).selectOption(projectId);
  await page.getByLabel('Select “Plan release”').check();
  await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
  await page.getByLabel('Project', { exact: true }).selectOption(second.id);
  await expect(page.getByText('1 selected', { exact: true })).toHaveCount(0);
});

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
  const input = page.getByLabel('New room name');
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
  const node = page.locator('.node', { hasText: 'Release input' });
  await node.focus(); await page.keyboard.press('Enter');
  const input = page.getByLabel('Message / prompt', { exact: true });
  await expect(input).toBeVisible();
  await input.fill('Review long acceptance checklist before release');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
  await page.reload();
  await page.locator('.node', { hasText: 'Release input' }).focus(); await page.keyboard.press('Space');
  await expect(input).toHaveValue('Review long acceptance checklist before release');
  await ctx.dispose();
});

test('swarm board: keyboard assignment and move preserve dependencies, rejected run can retry', async ({ page }) => {
  const { ctx, base, projectId, taskIds } = await setup(page);
  const target = taskIds[2];
  let runs = 0;
  await page.route(`**/api/v1/swarm/tasks/${target}/run`, r => {
    runs++;
    return runs === 1 ? r.fulfill({ status: 409, json: { code: 'conflict', message: 'Dependency must finish before this task can run' } })
      : r.fulfill({ json: { id: 'synthetic-queued-run' } });
  });
  await page.goto('/#/swarm'); await page.getByRole('tab', { name: 'Board', exact: true }).click();
  const card = page.locator('.kb-card', { hasText: 'Wire up frontend' });
  await card.focus(); await card.press('Enter');
  await page.getByRole('menuitem', { name: 'Assign to Ada', exact: true }).click();
  await expect(card).toContainText('Ada');
  await card.focus(); await card.press('Enter');
  await page.getByRole('menuitem', { name: 'Move to Blocked', exact: true }).click();
  await expect(page.locator('.column', { has: page.locator('.col-head', { hasText: 'Blocked' }) })).toContainText('Wire up frontend');
  const saved = await (await ctx.get(`${base}/api/v1/swarm/projects/${projectId}/tasks`)).json();
  expect(saved.find((t: { id: string }) => t.id === target).depends_on).toEqual([taskIds[1]]);
  await card.focus(); await card.press('Enter'); await page.getByRole('menuitem', { name: 'Run now', exact: true }).click();
  await expect(page.getByText('Dependency must finish before this task can run')).toBeVisible();
  await card.focus(); await card.press('Enter'); await page.getByRole('menuitem', { name: 'Run now', exact: true }).click();
  await expect(page.getByText('Task queued', { exact: true })).toBeVisible();
  expect(runs).toBe(2);
  await ctx.dispose();
});


test('tablet swarm: picker collapses with focus return and phone resize retains the draft', async ({ page }) => {
  const { ctx } = await setup(page); await ctx.dispose();
  await page.setViewportSize({ width: 834, height: 1112 });
  await page.addInitScript(() => {
    localStorage.setItem('otto_theme', 'warm'); localStorage.setItem('otto_scheme', 'dark');
  });
  await page.goto('/#/swarm');
  await page.evaluate(() => document.documentElement.dir = 'rtl');
  await expect(page.getByRole('tab', { name: 'Org', exact: true })).toBeVisible();
  await expect(page.locator('.rail-list')).toBeHidden();
  expect((await page.locator('.swarm-page .main').boundingBox())!.width).toBeGreaterThan(560);
  const toggle = page.getByRole('button', { name: 'Toggle swarms list' });
  await toggle.focus(); await toggle.press('Enter');
  const selected = page.locator('.swarm-item.active');
  await expect(selected).toBeVisible(); await selected.focus(); await selected.press('Enter');
  await expect(page.locator('.rail-list')).toBeHidden(); await expect(toggle).toBeFocused();
  await page.getByRole('tab', { name: 'Feed', exact: true }).click();
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await input.fill('Keep this draft across layout changes');
  await page.screenshot({ path: '/tmp/otto-ux-r4-automation-screens/warm-dark-tablet-rtl-feed.png' });
  await page.setViewportSize({ width: 375, height: 812 });
  await expect(input).toHaveValue('Keep this draft across layout changes'); await expect(input).toBeFocused();
  await expect(page.getByRole('tab', { name: 'Feed', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expectFullyInViewport(page, input); await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: '/tmp/otto-ux-r4-automation-screens/warm-dark-phone-rtl-feed.png' });
});
