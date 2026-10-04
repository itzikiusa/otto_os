import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });

test('automation navigation preserves canceled and failed saves, then saves before leaving', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  try {
    const workspaceId = await seedWorkspace(ctx, base);
    const api = `${base}/api/v1/workspaces/${workspaceId}/api-client`;
    const requestResponse = await ctx.post(`${api}/requests`, { data: { name: 'Leave guard request', method: 'GET', url: 'https://example.test/never-sent' } });
    expect(requestResponse.ok()).toBeTruthy();
    const request = await requestResponse.json();
    const automationResponse = await ctx.post(`${api}/automations`, { data: { name: 'Leave guard flow', steps: [{ request_id: request.id, assertions: [], extract: [] }] } });
    expect(automationResponse.ok()).toBeTruthy();
    const automation = await automationResponse.json();
    await page.addInitScript(id => {
      localStorage.setItem('otto_workspace', id);
      localStorage.setItem('otto_api_side', 'automations');
      localStorage.setItem('otto_rail_expanded', '0');
    }, workspaceId);
    await page.goto('/#/api');
    await page.locator('.auto-pick', { hasText: 'Leave guard flow' }).click();
    await page.getByRole('button', { name: 'Add check', exact: true }).click();
    await page.getByLabel('Expected value').fill('201');
    const navigation = page.getByRole('navigation', { name: /^(Navigator|Modules)$/ });
    const home = navigation.getByRole('button', { name: 'Home', exact: true });
    const dialog = page.getByRole('dialog');
    await home.click();
    await expect(dialog).toContainText('unsaved changes');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(page.getByLabel('Expected value')).toHaveValue('201');
    await expect(page).toHaveURL(/#\/api$/);

    let failSave = true;
    let attempts = 0;
    await page.route(`**/api-client/automations/${automation.id}`, route => {
      if (route.request().method() !== 'PATCH' && route.request().method() !== 'PUT') return route.fallback();
      attempts++;
      return failSave ? route.fulfill({ status: 503, json: { error: { code: 'unavailable', message: 'Save fixture unavailable' } } }) : route.fallback();
    });
    await home.click();
    await dialog.getByRole('button', { name: 'Save', exact: true }).click();
    await expect.poll(() => attempts).toBe(1);
    await expect(page.getByLabel('Expected value')).toHaveValue('201');
    await expect(page).toHaveURL(/#\/api$/);

    failSave = false;
    await home.click();
    await dialog.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page).toHaveURL(/#\/home$/);
    const saved = await (await ctx.get(`${api}/automations`)).json();
    expect(saved.find((row: { id: string }) => row.id === automation.id).steps[0].assertions[0].value).toBe('201');

    await navigation.getByRole('button', { name: 'API', exact: true }).click();
    await page.locator('.auto-pick', { hasText: 'Leave guard flow' }).click();
    await page.getByLabel('Expected value').fill('202');
    await home.click();
    await dialog.getByRole('button', { name: 'Discard', exact: true }).click();
    await expect(page).toHaveURL(/#\/home$/);
    const unchanged = await (await ctx.get(`${api}/automations`)).json();
    expect(unchanged.find((row: { id: string }) => row.id === automation.id).steps[0].assertions[0].value).toBe('201');
    expect(attempts).toBe(2);
  } finally {
    await ctx.dispose();
  }
});
