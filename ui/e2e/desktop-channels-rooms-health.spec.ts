import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// Settings → Channels shows each enabled listener's live state (was: only
// "Enabled", while a revoked token or a hung Socket Mode dial left the bot
// silently dead), and the agent-rooms list shows each room's last activity.

test.use({ serviceWorkers: 'block' });

async function setup(page: Page) {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  return { ctx, base, ws };
}

test('channels: an enabled Slack card shows why it is not connected', async ({ page }) => {
  const { ctx, ws } = await setup(page);
  await ctx.dispose();
  const now = new Date().toISOString();
  // A real enabled Slack integration would dial Slack from the isolated
  // daemon; serve the config + health instead.
  await page.route(`**/api/v1/workspaces/${ws}/integrations`, r => r.request().method() === 'GET'
    ? r.fulfill({ json: [{
        // Blank allow-list + the explicit opt-in: a blank list alone admits
        // nobody (the daemon fails closed), so "open" needs `open_to_all`.
        workspace_id: ws, channel: 'slack', enabled: true, allowed_users: '', open_to_all: true, agent_reply: false,
        reply_instructions: '', channel_id: 'C0123', preferred_cli: '', has_bot_token: true,
        has_app_token: true, updated_at: now,
      }] })
    : r.continue());
  await page.route(`**/api/v1/workspaces/${ws}/integrations/status`, r => r.fulfill({ json: [{
    workspace_id: ws, channel: 'slack', state: 'failing',
    detail: 'Slack rejected the app token (invalid_auth). Check that Socket Mode is enabled for the app and paste a current xapp-… token.',
    since: now, failures: 3, last_error: 'invalid_auth', last_error_at: now,
  }] }));
  await page.goto('/#/settings/channels');

  const badge = page.getByTestId('channel-health-slack');
  await expect(badge).toHaveText('Not connected');
  await expect(page.getByText(/Slack rejected the app token \(invalid_auth\)/)).toBeVisible();
  // Opened to everyone is flagged on the card…
  await expect(page.getByText('Open to everyone')).toBeVisible();
  // …and explained in the editor.
  await page.getByRole('button', { name: 'Edit Slack integration' }).click();
  await expect(page.getByText(/can run an agent on this Mac/)).toBeVisible();
  await page.getByLabel('Allowed users').fill('U0123ABC');
  await expect(page.getByText(/can run an agent on this Mac/)).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
});

test('rooms: the list shows when each room was last active', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  for (const name of ['Busy room', 'Quiet room']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/agent-rooms`, { data: { name } });
    expect(r.ok()).toBeTruthy();
    if (name === 'Busy room') {
      const id = (await r.json()).id;
      const m = await ctx.post(`${base}/api/v1/agent-rooms/${id}/messages`, { data: { text: 'Kick-off' } });
      expect(m.ok()).toBeTruthy();
    }
  }
  // Over-long names are refused up front.
  const long = await ctx.post(`${base}/api/v1/workspaces/${ws}/agent-rooms`, { data: { name: 'x'.repeat(121) } });
  expect(long.status()).toBe(400);
  await ctx.dispose();

  await page.goto('/#/personal-agents/rooms');
  const quiet = page.getByRole('button', { name: /Quiet room 0 agents/ });
  await expect(quiet).toContainText('no messages yet');
  const busy = page.getByRole('button', { name: /Busy room 0 agents/ });
  await expect(busy).not.toContainText('no messages yet');
  await expect(busy.locator('time')).toHaveText(/^(now|\d+s ago|\d+m ago)$/);
});
