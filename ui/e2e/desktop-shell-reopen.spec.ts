import { test, expect } from '@playwright/test';
import type { APIRequestContext, Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';

// Reopening a terminal whose shell is gone.
//
// Regression: a `shell` session has no provider resume args, so `ensure_live`
// used to no-op for it — reopening a terminal whose PTY had died (you typed
// `exit`, the app quit, the daemon restarted) left a dead screen that read as
// "this session doesn't exist any more", even though the row was intact and
// nothing had been lost. A login shell is cheap and stateless: it must respawn.

let wsA = '';
let api: { ctx: APIRequestContext; base: string } | null = null;

test.beforeAll(async () => {
  const a = await apiCtx();
  api = { ctx: a.ctx, base: a.base };
  wsA = await seedWorkspace(a.ctx, a.base);
});

test.afterAll(async () => {
  await api?.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsA);
});

/** Wait for the open session's terminal WS to attach. */
async function awaitTerminal(page: Page): Promise<void> {
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  await expect(
    page.locator('.term-overlay .badge').filter({ hasText: /connecting|reconnecting/ }),
  ).toHaveCount(0, { timeout: 20_000 });
}

/** Open a session route from scratch. */
async function openSession(page: Page, id: string): Promise<void> {
  await page.goto(`/#/agents/${id}`);
  await awaitTerminal(page);
}

async function status(id: string): Promise<string> {
  const r = await api!.ctx.get(`${api!.base}/api/v1/sessions/${id}`);
  expect(r.ok(), 'get session').toBeTruthy();
  return (await r.json()).status as string;
}

test('a terminal whose shell exited comes back live when reopened', async ({ page }) => {
  const id = await seedShellSession(api!.ctx, api!.base, wsA);
  await openSession(page, id);

  // Kill the shell the way the user would.
  const r = await api!.ctx.post(`${api!.base}/api/v1/sessions/${id}/input`, {
    data: { text: 'exit', submit: true },
  });
  expect(r.ok(), 'send exit').toBeTruthy();
  await expect(page.locator('.term-overlay .badge').filter({ hasText: /exited/ })).toBeVisible({
    timeout: 15_000,
  });
  await expect.poll(() => status(id), { timeout: 15_000 }).toBe('exited');

  // Open it again — the whole point: a working prompt, not a dead screen.
  // (A reload, not a re-`goto`: the URL already IS this session's route.)
  await page.reload();
  await awaitTerminal(page);
  await expect(page.locator('.term-overlay .badge').filter({ hasText: /exited/ })).toHaveCount(0, {
    timeout: 15_000,
  });
  await expect.poll(() => status(id), { timeout: 15_000 }).toMatch(/running|working|idle/);

  // And it really is a live shell: it answers.
  await api!.ctx.post(`${api!.base}/api/v1/sessions/${id}/input`, {
    data: { text: 'echo otto-is-back', submit: true },
  });
  await expect(page.locator('.term-host').first()).toContainText('otto-is-back', {
    timeout: 15_000,
  });
});
