import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedSwarm } from './seed';

// Functional coverage for the swarm-enhancement feature: per-task GOALS + the
// leader verification surface, CHANNEL TRIGGERS, and the new board message kinds
// (worktree / shared / merge / verify / escalation). Two layers:
//   1. API round-trips through the real test daemon — proves the goal/trigger/
//      standing-goal routes work end to end (the strongest signal the backend is
//      wired: schema → repo → routes).
//   2. UI rendering of the seeded data — the Goals panel, the Settings modal
//      (standing goals + triggers), and the new feed-kind chips.
//
// NOTE: requires the test daemon to be the NEW build (these routes don't exist on
// older daemons) — run with OTTO_E2E_BIN pointing at a freshly-built ottod.

let workspaceId = '';
let swarmId = '';
let projectId = '';
let taskIds: string[] = [];
let base = '';
let api: APIRequestContext;
const swarmName = 'E2E Swarm';

async function postJson(ctx: APIRequestContext, url: string, body: unknown): Promise<any> {
  const res = await ctx.post(url, { data: body });
  expect(res.ok(), `POST ${url} -> ${res.status()}`).toBeTruthy();
  return res.json();
}

test.beforeAll(async () => {
  const a = await apiCtx();
  api = a.ctx;
  base = a.base;
  workspaceId = await seedWorkspace(api, base);
  const sw = await seedSwarm(api, base, workspaceId);
  swarmId = sw.swarmId;
  projectId = sw.projectId;
  taskIds = sw.taskIds;

  // Two explicit goals on the first task: a metric goal (the "under 2 min" case)
  // and a framework-swap goal.
  await postJson(api, `${base}/api/v1/swarm/tasks/${taskIds[0]}/goals`, {
    title: 'Runs under 2 minutes',
    description: 'The suite must complete in under 2 minutes.',
    metric: 'runtime_seconds',
    comparator: 'lte',
    target_value: 120,
    block_value: 300,
    max_retries: 2,
    blocking: true,
  });
  await postJson(api, `${base}/api/v1/swarm/tasks/${taskIds[0]}/goals`, {
    title: 'Use Playwright instead of Selenium',
    description: 'No Selenium imports remain.',
    blocking: true,
  });

  // A channel trigger binding a Slack channel to this swarm.
  await postJson(api, `${base}/api/v1/swarm/swarms/${swarmId}/triggers`, {
    channel: 'slack',
    keyword: '@team',
    repo_path: '/tmp/e2e-repo',
    auto_start: true,
    reply: true,
    enabled: true,
  });

  // Board messages exercising the new kinds (verification feed).
  const feed: { kind: string; body: string }[] = [
    { kind: 'worktree', body: '🌿 Dev created worktree `swarm/s1/a1` (base `swarm/s1/p1/int`) for "Build API endpoints".' },
    { kind: 'shared', body: '⚠️ Dev and Reviewer both modified shared file(s): `src/auth.ts` — coordinate before merge.' },
    { kind: 'verify', body: '🔎⚠️ Goal "Runs under 2 minutes" (measured: 2m31s): close but over target.' },
    { kind: 'merge', body: '✅ merged `swarm/s1/a1` → `swarm/s1/p1/int`.' },
    { kind: 'escalation', body: '🚫 Goal "Runs under 2 minutes" goal could not be achieved (after 2 attempts).' },
  ];
  for (const m of feed) {
    await postJson(api, `${base}/api/v1/swarm/swarms/${swarmId}/board`, { ...m, project_id: projectId });
  }
});

test.afterAll(async () => {
  await api?.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

// ---- API round-trips (full daemon stack) ---------------------------------

test('goals: create, list, patch, and verification state round-trip', async () => {
  const goals = await (await api.get(`${base}/api/v1/swarm/tasks/${taskIds[0]}/goals`)).json();
  expect(goals.length).toBeGreaterThanOrEqual(2);
  const metric = goals.find((g: any) => g.title === 'Runs under 2 minutes');
  expect(metric).toBeTruthy();
  expect(metric.metric).toBe('runtime_seconds');
  expect(metric.target_value).toBe(120);
  expect(metric.block_value).toBe(300);
  expect(metric.max_retries).toBe(2);
  expect(metric.blocking).toBe(true);
  expect(metric.status).toBe('pending');

  // Patch the target down; confirm it persists.
  const patched = await (
    await api.patch(`${base}/api/v1/swarm/goals/${metric.id}`, { data: { target_value: 90 } })
  ).json();
  expect(patched.target_value).toBe(90);

  // Verification state for the task (no controller running yet).
  const v = await (await api.get(`${base}/api/v1/swarm/tasks/${taskIds[0]}/verification`)).json();
  expect(v.running).toBe(false);
  expect(Array.isArray(v.goals)).toBe(true);
  expect(v.goals.length).toBeGreaterThanOrEqual(2);
});

test('standing goals: defaults are seeded on first read', async () => {
  const standing = await (await api.get(`${base}/api/v1/swarm/swarms/${swarmId}/standing-goals`)).json();
  expect(standing.length).toBeGreaterThanOrEqual(4);
  const titles = standing.map((g: any) => g.title.toLowerCase()).join(' ');
  expect(titles).toContain('reuse');
  expect(titles).toContain('duplication');
  // Replace the set; confirm the PUT round-trips.
  const replaced = await (
    await api.put(`${base}/api/v1/swarm/swarms/${swarmId}/standing-goals`, {
      data: { goals: [{ title: 'All tests pass', blocking: true }] },
    })
  ).json();
  expect(replaced.length).toBe(1);
  expect(replaced[0].title).toBe('All tests pass');
  expect(replaced[0].kind).toBe('standing');
});

test('triggers: the seeded channel trigger round-trips', async () => {
  const triggers = await (await api.get(`${base}/api/v1/swarm/swarms/${swarmId}/triggers`)).json();
  expect(triggers.length).toBeGreaterThanOrEqual(1);
  const t = triggers[0];
  expect(t.channel).toBe('slack');
  expect(t.keyword).toBe('@team');
  expect(t.repo_path).toBe('/tmp/e2e-repo');
  expect(t.enabled).toBe(true);
});

// ---- UI rendering --------------------------------------------------------

async function openSwarm(page: import('@playwright/test').Page): Promise<void> {
  await page.goto('/#/swarm');
  await expect(page.locator('.swarm-page')).toBeVisible({ timeout: 30_000 });
  const item = page.locator('.swarm-item', { hasText: swarmName }).first();
  await expect(item).toBeVisible({ timeout: 20_000 });
  await item.click();
  await expect(page.locator('.swarm-head')).toBeVisible({ timeout: 20_000 });
}

test('feed: the new verification message kinds render', async ({ page }) => {
  await openSwarm(page);
  await page.locator('.switcher .seg', { hasText: 'Feed' }).first().click();
  // The seeded worktree/merge/verify messages appear in the feed.
  await expect(page.getByText('created worktree', { exact: false }).first()).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText('merged', { exact: false }).first()).toBeVisible();
  await expect(page.getByText('close but over target', { exact: false }).first()).toBeVisible();
});

test('settings: standing goals tab + the Triggers tab show seeded data', async ({ page }) => {
  await openSwarm(page);
  // The swarm Settings button has a unique title (avoids matching other buttons).
  const settingsBtn = page.locator('button[title*="channel triggers"]').first();
  // On a phone the header controls collapse — expand them so Settings is reachable.
  if (!(await settingsBtn.isVisible())) {
    await page.locator('.head-toggle').first().click();
  }
  await expect(settingsBtn).toBeVisible({ timeout: 10_000 });
  await settingsBtn.click();
  const modal = page.locator('[role="dialog"]', { hasText: 'Swarm settings' }).first();
  await expect(modal).toBeVisible({ timeout: 15_000 });
  // The default "Standing goals" tab is present.
  await expect(modal.locator('.tab', { hasText: 'Standing goals' })).toBeVisible();
  // Switch to the Triggers tab → the seeded slack trigger (keyword @team) shows.
  await modal.locator('.tab', { hasText: 'Triggers' }).click();
  await expect(modal.getByText('@team', { exact: false }).first()).toBeVisible({ timeout: 10_000 });
});

test('goals panel: a task’s goals render with status', async ({ page }) => {
  await openSwarm(page);
  await page.locator('.switcher .seg', { hasText: 'Board' }).first().click();
  // The seeded goals are on the "Design data model" task — open THAT card's panel.
  const card = page.locator('.card', { hasText: 'Design data model' }).first();
  await expect(card).toBeVisible({ timeout: 15_000 });
  await card.locator('[aria-label="goals"]').click();
  const panel = page.locator('[role="dialog"]', { hasText: 'Goals' }).first();
  await expect(panel).toBeVisible({ timeout: 10_000 });
  await expect(panel.getByText('Runs under 2 minutes', { exact: false }).first()).toBeVisible({ timeout: 10_000 });
});
