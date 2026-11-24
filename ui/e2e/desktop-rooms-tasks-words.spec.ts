import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// Rooms and Scheduled Tasks (effort 2, D8): a lobby row always has an action,
// the empty lobby starts a room, a host who opens a room without its window's
// credential is not told to "ask the host"; a task's cron reads in words and
// "Fires next" says what fires first instead of repeating the list.

test.beforeEach(async ({}, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
});

const room = { room_id: 'r1', member_id: 'host', admission: 'admitted', session_id: 'ses_1', session_title: 'Build together', members: [] };

test('a lobby row without this window’s credential offers Open session', async ({ page }) => {
  await page.route('**/api/v1/rooms', (r) => r.fulfill({ json: [room] }));
  await openPage(page, 'rooms');
  await expect(page.getByRole('heading', { name: 'Build together' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open session' })).toBeVisible();
});

test('the empty lobby has a Start a room action', async ({ page }) => {
  await page.route('**/api/v1/rooms', (r) => r.fulfill({ json: [] }));
  await openPage(page, 'rooms');
  await expect(page.getByRole('button', { name: 'Start a room' })).toBeVisible();
});

test('the host opening a room without its credential is pointed back, not told to ask the host', async ({ page }) => {
  await openPage(page, 'rooms/r1');
  await expect(page.getByText('Open in another window')).toBeVisible();
  await expect(page.getByText(/Ask the host/)).toHaveCount(0);
  await page.getByRole('button', { name: 'Back to Rooms' }).click();
  await expect(page).toHaveURL(/#\/rooms$/);
});

test('a cron schedule reads in words; Fires next folds the repeated list', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  for (const [name, schedule] of [
    ['Weekday digest', { cadence: 'cron', expr: '30 8 * * 0,1,2,3,4' }],
    ['Hourly sweep', { cadence: 'interval', every_min: 60 }],
  ] as const) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/scheduled-tasks`, {
      data: { name, prompt: 'Report.', schedule, timezone: 'UTC', destination: { type: 'none' }, enabled: true },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
  }
  await ctx.dispose();
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), wsId);
  await openPage(page, 'scheduled-tasks');
  await expect(page.getByText('Sun–Thu at 08:30 · UTC')).toBeVisible();
  await expect(page.locator('code.cron')).toHaveCount(0);
  const radar = page.getByTestId('fires-next-radar');
  await expect(radar.locator('.radar-list')).toHaveCount(0);
  await radar.getByRole('button', { name: /All \d+ upcoming/ }).click();
  await expect(radar.locator('.radar-list')).toBeVisible();
});
