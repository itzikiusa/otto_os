import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedSwarm } from './seed';

// Desktop BROWSER: an empty Swarms rail must say WHICH workspace is empty, and
// help the operator find the swarms they actually created.
//
// REGRESSION (the report this pins): swarms are workspace-scoped, but the page
// only ever said "No swarms yet." — so landing on a different workspace (a fresh
// profile falls back to the first one) reads as "my two swarms are gone". The
// empty state now names the workspace and offers a cross-workspace probe that
// jumps straight to a workspace that has them.
//
// Only meaningful on the desktop-browser project; self-skips elsewhere.

test.setTimeout(120_000);

let api: APIRequestContext;
let base = '';
let withSwarms = '';
let withoutSwarms = '';
const swarmName = 'E2E Swarm';

test.beforeAll(async () => {
  const a = await apiCtx();
  api = a.ctx;
  base = a.base;
  withSwarms = await seedWorkspace(api, base);
  await seedSwarm(api, base, withSwarms);
  withoutSwarms = await seedWorkspace(api, base);
});

test.afterAll(async () => {
  await api?.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  // Boot on the workspace that has NO swarms — the operator's situation.
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, withoutSwarms);
});

test('empty rail names the workspace and finds swarms in the others', async ({ page }) => {
  await page.goto('/#/swarm');
  await expect(page.locator('.swarm-page')).toBeVisible({ timeout: 30_000 });

  // Both the rail and the main empty state must scope the statement to THIS
  // workspace rather than claiming there are no swarms at all.
  await expect(page.locator('.rail-list')).toContainText('No swarms in', { timeout: 20_000 });
  await expect(page.locator('.main')).toContainText('No swarms in');

  // The probe finds the sibling workspace and offers a jump.
  const probe = page.getByRole('button', { name: /Look in my other workspaces/ });
  await expect(probe).toBeVisible();
  await probe.click();

  const jump = page.locator('.ws-hits button').first();
  await expect(jump).toBeVisible({ timeout: 20_000 });
  await expect(jump).toContainText('swarm');
  await jump.click();

  // Landed on the workspace that owns the swarm — the rail lists it.
  await expect(page.locator('.swarm-item', { hasText: swarmName }).first()).toBeVisible({
    timeout: 20_000,
  });
});
