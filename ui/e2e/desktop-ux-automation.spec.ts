import { apiCtx, seedWorkspace } from './seed';
import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';

for (const route of ['workflows', 'proof', 'run-with-otto']) {
  test(`${route}: no workspace offers an actionable setup state`, async ({ page }) => {
    await page.route('**/api/v1/workspaces', r => r.fulfill({ json: [] }));
    await page.setViewportSize({ width: 375, height: 812 });
    await page.addInitScript(() => localStorage.setItem('otto_scheme', 'light'));
    await page.goto(`/#/${route}`);
    await expect(page.getByRole('heading', { name: 'Add a workspace to get started' })).toBeVisible();
    const add = page.getByTestId('page-empty').getByRole('button', { name: 'Add workspace', exact: true });
    await expectFullyInViewport(page, add);
    await expectNoHorizontalOverflow(page);
    await add.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.screenshot({ path: `/tmp/otto-ux-screenshots/automation-after-phone-${route}.png` });
  });
}

test('personal agents: keyboard tab selection moves focus to the selected tab', async ({ page }) => {
  await page.route('**/api/v1/workspaces', r => r.fulfill({ json: [] }));
  await page.goto('/#/personal-agents');
  const agents = page.getByRole('tab', { name: 'Agents', exact: true });
  const rooms = page.getByRole('tab', { name: 'Rooms', exact: true });
  await agents.focus();
  await page.keyboard.press('ArrowRight');
  await expect(rooms).toHaveAttribute('aria-selected', 'true');
  await expect(rooms).toBeFocused();
  await page.keyboard.press('Home');
  await expect(agents).toHaveAttribute('aria-selected', 'true');
  await expect(agents).toBeFocused();
});


for (const [route, endpoint] of [
  ['mission-control', 'workgraph/items'], ['run-with-otto', 'runs'],
  ['swarm', 'swarm/swarms'], ['loops', 'goal-loops'],
  ['workflows', 'workflows'], ['scheduled-tasks', 'scheduled-tasks'],
  ['personal-agents', 'personal-agents'], ['proof', 'proof-packs'],
]) {
  test(`${route}: a failed load recovers through inline Retry`, async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const wsId = await seedWorkspace(ctx, base);
    await ctx.dispose();
    await page.addInitScript(id => localStorage.setItem('otto_workspace', id), wsId);
    let failing = true;
    const attempts: number[] = [];
    await page.route(`**/api/v1/workspaces/${wsId}/${endpoint}*`, async r => {
      attempts.push(Date.now());
      if (failing) await r.fulfill({ status: 503, json: { code: 'unavailable', message: 'Service temporarily unavailable' } });
      else await r.continue();
    });
    await page.goto(`/#/${route}`);
    const retry = page.getByRole('button', { name: 'Retry', exact: true }).first();
    await expect(retry).toBeVisible({ timeout: 20000 });
    const priorAttempts = attempts.length;
    failing = false;
    await retry.click();
    await expect.poll(() => attempts.length).toBeGreaterThan(priorAttempts);
    await expect(retry).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
  });
}

test('proof: failed media fetch shows an inline error and retries the image', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const pack = await ctx.post(`${base}/api/v1/workspaces/${wsId}/proof-packs`, { data: {
    work_item_kind: 'manual', work_item_id: `ux-media-${Date.now()}`, title: 'Media recovery evidence',
  }});
  expect(pack.ok(), await pack.text()).toBeTruthy();
  const packId = (await pack.json()).id;
  const media = await ctx.post(`${base}/api/v1/proof-packs/${packId}/media`, { data: {
    kind: 'screenshot', title: 'Recovered screenshot', mime: 'image/png',
    data_base64: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
  }});
  expect(media.ok(), await media.text()).toBeTruthy();
  await ctx.dispose();
  await page.addInitScript(id => localStorage.setItem('otto_workspace', id), wsId);
  let failing = true;
  await page.route('**/api/v1/proof-artifacts/*/blob', async r => {
    if (failing) await r.fulfill({ status: 503, body: 'temporarily unavailable' });
    else await r.continue();
  });
  await page.goto('/#/proof');
  await expect(page.getByText('Couldn’t load the media')).toBeVisible();
  await expect(page.getByText('Loading media…')).toHaveCount(0);
  failing = false;
  await page.getByRole('button', { name: 'Retry media' }).click();
  const img = page.getByRole('img', { name: 'Recovered screenshot' });
  await expect(img).toBeVisible();
  await expect.poll(() => img.evaluate(el => (el as HTMLImageElement).naturalWidth)).toBe(1);
});

for (const [theme, scheme, width, height, rtl] of [
  ['native', 'light', 1440, 900, false], ['native', 'dark', 1440, 900, false],
  ['warm', 'dark', 1440, 900, false], ['native', 'dark', 1024, 768, true],
] as const) {
  test(`setup states: ${theme} ${scheme} ${width} RTL=${rtl}`, async ({ page }) => {
    await page.route('**/api/v1/workspaces', r => r.fulfill({ json: [] }));
    await page.setViewportSize({ width, height });
    await page.addInitScript(({ theme, scheme }) => {
      localStorage.setItem('otto_theme', theme);
      localStorage.setItem('otto_scheme', scheme);
    }, { theme, scheme });
    for (const route of ['workflows', 'proof', 'run-with-otto']) {
      await page.goto(`/#/${route}`);
      if (rtl) await page.evaluate(() => document.documentElement.dir = 'rtl');
      await expect(page.getByRole('heading', { name: 'Add a workspace to get started' })).toBeVisible();
      await expectFullyInViewport(page, page.getByTestId('page-empty').getByRole('button', { name: 'Add workspace', exact: true }));
      await expectNoHorizontalOverflow(page);
      await page.screenshot({ path: `/tmp/otto-ux-screenshots/automation-after-${theme}-${scheme}-${width}-${route}.png` });
    }
  });
}
