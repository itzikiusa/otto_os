import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// "Design every state": a FAILED list load must render inline ("Couldn't load
// X" + Retry, via the shared LoadState), never masquerade as the empty state
// ("No scheduled tasks yet", "No external servers yet"), and Retry must
// recover once the endpoint answers again. The endpoint is forced to 500 with
// `page.route` — the isolated daemon itself is never broken.
// Desktop-browser project only.

let wsId = '';

test.beforeAll(async () => {
  const a = await apiCtx();
  wsId = await seedWorkspace(a.ctx, a.base);
  // One task so the recovered page has a row to prove it loaded for real.
  const r = await a.ctx.post(`${a.base}/api/v1/workspaces/${wsId}/scheduled-tasks`, {
    data: {
      name: 'Load-state probe',
      prompt: 'noop',
      schedule: { cadence: 'interval', every_min: 60 },
      destination: { type: 'none' },
      enabled: false,
    },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  await a.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  // A cold Vite dev server can take a while to serve the first boot.
  test.setTimeout(150_000);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsId);
});

/**
 * Boot the app on a neutral page FIRST, then fail GETs matching `pattern` with
 * a 500 and navigate in-app (hash change, no reload) to `route`. Routing is
 * installed after boot so the interception doesn't slow the initial module
 * load. Returns `heal()`, which lets later requests through to the daemon.
 */
async function openWithFailingLoad(page: Page, route: string, pattern: RegExp): Promise<() => void> {
  await page.goto('/#/settings');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 120_000 });
  let broken = true;
  await page.route(pattern, (r) => {
    if (broken && r.request().method() === 'GET') {
      return r.fulfill({
        status: 500,
        contentType: 'application/json',
        // The daemon is cross-origin to the Vite-served UI in E2E.
        headers: { 'Access-Control-Allow-Origin': '*' },
        body: JSON.stringify({ code: 'internal', message: 'forced failure (e2e)' }),
      });
    }
    return r.fallback();
  });
  await page.evaluate((h) => {
    location.hash = h;
  }, `#/${route}`);
  return () => {
    broken = false;
  };
}

test('scheduled tasks: a failed list load shows Couldn’t load + Retry, not the empty state', async ({ page }) => {
  const heal = await openWithFailingLoad(page, 'scheduled-tasks', /\/api\/v1\/workspaces\/[^/]+\/scheduled-tasks(\?|$)/);

  const err = page.getByTestId('load-error');
  await expect(err).toBeVisible({ timeout: 30_000 });
  await expect(err).toContainText('Couldn\'t load scheduled tasks');
  await expect(err).toContainText('forced failure (e2e)');
  await expect(page.getByText('No scheduled tasks yet')).toHaveCount(0);

  heal();
  await err.getByRole('button', { name: 'Retry' }).click();
  await expect(page.getByText('Load-state probe')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('load-error')).toHaveCount(0);
});

test('MCP servers: a failed list load shows Couldn’t load + Retry, not "No external servers yet"', async ({ page }) => {
  const heal = await openWithFailingLoad(page, 'mcp/servers', /\/api\/v1\/workspaces\/[^/]+\/mcp\/servers(\?|$)/);

  const err = page.getByTestId('load-error');
  await expect(err).toBeVisible({ timeout: 30_000 });
  await expect(err).toContainText('Couldn\'t load MCP servers');
  await expect(page.getByText(/No external servers yet/)).toHaveCount(0);

  heal();
  await err.getByRole('button', { name: 'Retry' }).click();
  // The seeded workspace has no external servers: the REAL empty state now shows.
  await expect(page.getByText(/No external servers yet/)).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('load-error')).toHaveCount(0);
});
