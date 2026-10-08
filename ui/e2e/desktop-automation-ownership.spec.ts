import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';
import { mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

function loop(id: string) {
  return { id, name: `Loop ${id}`, workspace_id: 'ws', status: 'paused', phase: 'done', current_iteration: 1,
    iterations_started: 1, progress_pct: 50, elapsed_secs: 10, run_started_at: null,
    definition: { title: 'Review', acceptance_criteria: [] }, config: { mode: 'research', executors: [] },
    limits: { max_iterations: 3, max_runtime_secs: 300 }, ledger: { questions: [], verifications: [] },
  };
}

test('goal-loop Stop remains bound to the loop confirmed before a route change', async ({ page }) => {
  const mutations: string[] = [];
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith('/stop')) { mutations.push(path); return route.fulfill({ json: { ...loop('A'), status: 'stopped' } }); }
    if (/\/goal-loops\/[AB]$/.test(path)) return route.fulfill({ json: { loop: loop(path.slice(-1)), iterations: [] } });
    return route.fulfill({ json: [] });
  });
  await page.goto('/e2e/fixtures/automation-ownership.html');
  await expect(page.getByRole('heading', { name: 'Loop A', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'More actions', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Stop loop…', exact: true }).click();
  const sheet = page.getByRole('dialog'); await expect(sheet).toBeVisible();
  await page.evaluate(() => window.dispatchEvent(new Event('review-target-change')));
  await sheet.getByRole('button', { name: 'Stop loop', exact: true }).click();
  await expect.poll(() => mutations).toEqual(['/api/v1/goal-loops/A/stop']);
  await expect(page.getByRole('heading', { name: 'Loop B', exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

test('Open PR rechecks a partial snapshot and keeps the confirmed target across navigation', async ({ page }) => {
  const mutations: string[] = [];
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (route.request().method() === 'POST') mutations.push(path);
    return route.fulfill({ json: path.endsWith('/events') ? [] : { id: 'A', workspace_id: 'ws' } });
  });
  await page.goto('/e2e/fixtures/automation-ownership.html?kind=run&proof=partial');
  await expect(page.getByRole('button', { name: 'Open PR', exact: true })).toBeEnabled({ timeout: 1500 });
  await page.getByRole('button', { name: 'Open PR', exact: true }).click();
  const sheet = page.getByRole('dialog'); await expect(sheet).toContainText('Run A');
  await page.evaluate(() => window.dispatchEvent(new Event('review-target-change')));
  await sheet.getByRole('button', { name: 'Open PR', exact: true }).click();
  await expect.poll(() => mutations).toEqual(['/api/v1/runs/A/open-pr']);
  await expect(page.getByRole('heading', { name: 'Run B', exact: true })).toBeVisible();
});

for (const scheme of ['light', 'dark'] as const) {
  test(`goal-loop loaded state ${scheme} remains usable at desktop and phone widths`, async ({ page }) => {
    await page.addInitScript((scheme) => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); localStorage.setItem('otto_scheme', scheme); localStorage.setItem('otto_theme', 'native'); }, scheme);
    await page.route('**/api/v1/**', (route) => route.fulfill({ json: { loop: loop('A'), iterations: [] } }));
    await page.goto('/e2e/fixtures/automation-ownership.html');
    await expect(page.getByRole('heading', { name: 'Loop A', exact: true })).toBeVisible();
    await page.evaluate((scheme) => { document.documentElement.dataset.scheme = scheme; document.documentElement.dataset.theme = 'native'; }, scheme);
    await expect(page.locator('html')).toHaveAttribute('data-scheme', scheme);
    const dir = fileURLToPath(new URL('../../docs/reviews/quality-20261008/evidence/', import.meta.url)); await mkdir(dir, { recursive: true });
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 }); await expectNoHorizontalOverflow(page);
      await page.screenshot({ path: `${dir}/r08-loop-${width}-${scheme}.png`, fullPage: true });
    }
  });
}
