import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });

test('Slack directory failure offers Retry without preventing manual IDs', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  await page.route(`**/workspaces/${ws}/integrations`, r => r.fulfill({ json: [{ workspace_id: ws, channel: 'slack', enabled: false, allowed_users: 'U1', open_to_all: false, agent_reply: false, reply_instructions: '', channel_id: 'C1', preferred_cli: '', has_bot_token: true, has_app_token: true, updated_at: new Date().toISOString() }] }));
  await page.route(`**/workspaces/${ws}/integrations/status`, r => r.fulfill({ json: [] }));
  let fail = true;
  await page.route('**/integrations/slack/directory*', r => fail
    ? r.fulfill({ status: 503, json: { message: 'Slack directory unavailable' } })
    : r.fulfill({ json: { users: [], channels: [{ id: 'C1', name: 'general', is_private: false }], users_error: null, channels_error: null } }));
  await page.goto('/#/settings/channels');
  await page.getByRole('button', { name: 'Edit Slack integration', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  await expect(dialog.getByLabel('Default channel ID')).toBeEditable();
  fail = false;
  await dialog.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Choose channel…', exact: true })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Retry', exact: true })).not.toBeVisible();
});

for (const scheme of ['light', 'dark']) {
  test(`Sharing phone rows and saved sender explanation — ${scheme}`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript(scheme => localStorage.setItem('otto_scheme', scheme), scheme);
    await page.route('**/email-sender', r => r.fulfill({ json: { gmail_address: 'sender@example.test', verified: true } }));
    await page.route('**/auth/shares', r => r.fulfill({ json: [{ id: 'fixture-share', session_id: 'fixture-session', session_title: 'Long session name for testing phone layout and controls', role: 'viewer', token_prefix: 'fixture', label: 'Review with a colleague', created_at: '2026-10-18T08:00:00Z', expires_at: '2026-10-19T08:00:00Z' }] }));
    await page.goto('/#/settings/sharing');
    await expect(page.getByLabel('Gmail address')).toBeDisabled();
    await expect(page.getByText('Choose Replace… below to change the app password and sender address together.')).toBeVisible();
    const links = page.getByTestId('active-share-links');
    await links.scrollIntoViewIfNeeded();
    await expect(links.getByText('View only')).toBeVisible();
    await expect(links.getByRole('button', { name: 'Revoke', exact: true })).toBeVisible();
    const row = links.locator('tbody tr').first();
    expect(await row.evaluate(el => getComputedStyle(el).display)).toBe('grid');
    expect(await row.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  });
}
