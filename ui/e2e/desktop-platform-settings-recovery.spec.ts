import { test, expect } from '@playwright/test';

test.use({ serviceWorkers: 'block' });

test('Insights Retry reflects debounce and pending persistence after a failed model save', async ({ page }) => {
  const config = { daily: false, weekly: false, monthly: false, provider: 'claude', model: 'original-model' };
  let attempts = 0;
  let releaseSecond!: () => void;
  await page.route('**/api/v1/providers/models', route => route.fulfill({ json: {} }));
  await page.route('**/api/v1/insights/config', async route => {
    if (route.request().method() === 'GET') return route.fulfill({ json: config });
    attempts++;
    if (attempts === 2) await new Promise<void>(resolve => { releaseSecond = resolve; });
    return route.fulfill({ status: 503, json: { code: 'upstream', message: `Synthetic model save failed ${attempts}` } });
  });
  await page.goto('/#/settings/insights');
  const model = page.getByRole('textbox', { name: /Model/ });
  await expect(model).toHaveValue('original-model');
  await page.clock.install();
  await model.fill('first-draft');
  await page.clock.runFor(510);
  const retry = page.getByRole('button', { name: 'Retry saving report agent', exact: true });
  await expect(retry).toBeEnabled();
  await expect(model).toHaveValue('first-draft');
  await model.fill('replacement-draft');
  await expect(retry).toBeDisabled();
  expect(attempts).toBe(1);
  await page.clock.runFor(510);
  await expect.poll(() => !!releaseSecond).toBe(true);
  await expect(retry).toBeDisabled();
  releaseSecond();
  await expect(retry).toBeEnabled();
  await expect(model).toHaveValue('replacement-draft');
  await expect(page.getByRole('alert').filter({ has: retry, hasText: 'Synthetic model save failed 2' })).toBeVisible();
});
