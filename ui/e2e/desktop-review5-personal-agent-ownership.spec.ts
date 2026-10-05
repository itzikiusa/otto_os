import {test, expect, type Page, type Route} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';
import type {PersonalAgent} from '../src/lib/api/types';

test.use({serviceWorkers: 'block'});
let workspaceA = '', workspaceB = '';
test.beforeAll(async () => {
  const {ctx, base} = await apiCtx();
  workspaceA = await seedWorkspace(ctx, base);
  workspaceB = await seedWorkspace(ctx, base);
  await ctx.dispose();
});
async function select(page: Page, id: string) {
  await page.evaluate(async next => {
    const path = '/src/lib/stores/workspace.svelte.ts';
    void (await import(path)).ws.select(next);
  }, id);
}
function agent(name: string): PersonalAgent {
  return {id: name, name, workspace_id: workspaceA, avatar: '', soul_md: '', provider: 'claude',
    model: '', cwd: '', browser: false, delivery: {type: 'none'}, enabled: false,
    created_at: '2026-10-05T00:00:00Z', updated_at: '2026-10-05T00:00:00Z'};
}

for (const completion of ['old-success-while-loading', 'old-error-while-loading', 'old-success-after-current']) {
  test(`personal agent list A-B-A preserves the current visit: ${completion}`, async ({page}) => {
    const held: Route[] = [];
    await page.addInitScript(id => localStorage.setItem('otto_workspace', id), workspaceA);
    await page.route(`**/api/v1/workspaces/${workspaceA}/personal-agents`, route => { held.push(route); });
    await page.route(`**/api/v1/workspaces/${workspaceB}/personal-agents`, route => route.fulfill({json: []}));
    const scheduleRequests: string[] = [];
    await page.route('**/api/v1/personal-agents/*/schedules', route => {
      scheduleRequests.push(route.request().url());
      return route.fulfill({json: []});
    });
    await page.route('**/api/v1/personal-agents/*/autonomy', route => route.fulfill({json: {primary: false, proactive: false}}));
    await page.goto('/#/personal-agents');
    await expect.poll(() => held.length).toBe(1);
    await select(page, workspaceB);
    await expect(page.getByRole('status', {name: 'Loading personal agents'})).toHaveCount(0);
    await select(page, workspaceA);
    await expect.poll(() => held.length).toBe(2);
    const current = () => held[1].fulfill({json: [agent('Current agent') ]});
    if (completion === 'old-success-after-current') {
      await current();
      await expect(page.getByRole('button', {name: 'Current agent', exact: true})).toBeVisible();
    }
    const response = page.waitForResponse(r => r.url().endsWith(`/workspaces/${workspaceA}/personal-agents`));
    await held[0].fulfill(completion === 'old-error-while-loading'
      ? {status: 503, json: {error: 'Old visit failed'}}
      : {json: [agent('Obsolete agent')]});
    await response;
    // Wait for the actual response handler and Svelte render, not a timed sleep.
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    if (completion !== 'old-success-after-current') {
      await expect(page.getByRole('status', {name: 'Loading personal agents'})).toBeVisible();
      await expect(page.getByTestId('load-error')).toHaveCount(0);
      await current();
    }
    await expect(page.getByRole('button', {name: 'Current agent', exact: true})).toBeVisible();
    await expect(page.getByRole('button', {name: 'Obsolete agent', exact: true})).toHaveCount(0);
    await expect(page.getByTestId('load-error')).toHaveCount(0);
    expect(scheduleRequests.some(url => url.includes('Obsolete'))).toBe(false);
  });
}
