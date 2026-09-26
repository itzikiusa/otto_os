import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedSwarm } from './seed';
import { expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';

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
async function rooms(page: Page) {
  const { ctx, base, ws } = await setup(page);
  const ids: string[] = [];
  for (const name of ['Design room', 'Release room']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/agent-rooms`, { data: { name } });
    expect(r.ok()).toBeTruthy(); ids.push((await r.json()).id);
  }
  await ctx.dispose();
  await page.goto('/#/personal-agents/rooms');
  await page.getByRole('button', { name: 'Design room 0 agents' }).click();
  return ids;
}

test('rooms: drafts belong to their conversation', async ({ page }) => {
  await rooms(page);
  const composer = page.getByLabel('Message to the room');
  await composer.fill('Design-only draft');
  await page.getByRole('button', { name: 'Release room 0 agents' }).click();
  await expect(composer).toHaveValue('');
  await composer.fill('Release-only draft');
  await page.getByRole('button', { name: 'Design room 0 agents' }).click();
  await expect(composer).toHaveValue('Design-only draft');
});

test('rooms: a pending send neither duplicates nor clears newer text', async ({ page }) => {
  const [id] = await rooms(page);
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  let posts = 0;
  await page.route(`**/api/v1/agent-rooms/${id}/messages`, async r => {
    if (r.request().method() !== 'POST') return r.continue();
    posts++; await gate; await r.fulfill({ json: { id: 'synthetic-message' } });
  });
  const composer = page.getByLabel('Message to the room');
  await composer.fill('Submitted message');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => posts).toBe(1);
  await composer.fill('New text typed while sending');
  await composer.press('Control+Enter');
  try { await expect.poll(() => posts).toBe(1); }
  finally { release(); }
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
  await expect(composer).toHaveValue('New text typed while sending');
});

test('swarm feed: failed refresh retains posts and offers Retry', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  const { swarmId } = await seedSwarm(ctx, base, ws);
  await ctx.dispose();
  await page.goto('/#/swarm');
  await page.getByRole('tab', { name: 'Feed', exact: true }).click();
  await expect(page.getByText('Decided to ship behind a flag first.')).toBeVisible();
  let fail = true;
  await page.route(`**/api/v1/swarm/swarms/${swarmId}/board*`, r => fail
    ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Board temporarily unavailable' } })
    : r.continue());
  await page.getByRole('button', { name: 'Refresh board' }).click();
  await expect(page.getByText(/Board temporarily unavailable/)).toBeVisible();
  await expect(page.getByText('Decided to ship behind a flag first.')).toBeVisible();
  fail = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByText(/Board temporarily unavailable/)).toHaveCount(0);
});

test('swarm tabs: RTL arrows follow the visible direction', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  await seedSwarm(ctx, base, ws); await ctx.dispose();
  await page.goto('/#/swarm');
  await page.evaluate(() => document.documentElement.dir = 'rtl');
  await page.getByRole('tab', { name: 'Org', exact: true }).focus();
  await page.keyboard.press('ArrowLeft');
  await expect(page.getByRole('tab', { name: 'Graph', exact: true })).toBeFocused();
});

async function workflows(page: Page) {
  const { ctx, base, ws } = await setup(page);
  const ids: string[] = [];
  for (const name of ['Alpha review', 'Beta review']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/workflows`, { data: {
      name, graph: { nodes: [{ id: 'start', kind: 'manual_trigger', name: `${name} start`, x: 0, y: 0, params: {} }], edges: [] },
    } });
    expect(r.ok()).toBeTruthy(); ids.push((await r.json()).id);
  }
  await ctx.dispose(); await page.goto('/#/workflows');
  await page.getByTestId(`wf-row-${ids[0]}`).locator('.row-main').click();
  await page.getByTestId('page-header').getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitemcheckbox', { name: 'Instructions', exact: true }).click();
  return ids;
}

test('workflow instructions: navigation guards unsaved standing rules', async ({ page }) => {
  const [, second] = await workflows(page);
  const input = page.getByPlaceholder('Standing rules every step follows by the letter (markdown)');
  await input.fill('Never publish without approval');
  await page.getByTestId(`wf-row-${second}`).locator('.row-main').click();
  await expect(page.getByRole('dialog', { name: 'Discard unsaved changes' })).toBeVisible();
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(input).toHaveValue('Never publish without approval');
});

test('workflow instructions: late save cannot switch the active workflow', async ({ page }) => {
  const [first, second] = await workflows(page);
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  let requested = false;
  await page.route(`**/api/v1/workflows/${first}`, async r => {
    if (r.request().method() !== 'PATCH') return r.continue();
    const response = await r.fetch(); requested = true; await gate; await r.fulfill({ response });
  });
  await page.getByPlaceholder('Standing rules every step follows by the letter (markdown)').fill('Alpha standing rules');
  await page.getByRole('button', { name: 'Save instructions', exact: true }).click();
  await expect.poll(() => requested).toBe(true);
  await page.getByTestId(`wf-row-${second}`).locator('.row-main').click();
  const discard = page.getByRole('button', { name: 'Discard changes', exact: true });
  if (await discard.isVisible()) await discard.click();
  await expect(page.getByTestId(`wf-row-${second}`)).toHaveClass(/active/);
  release();
  await expect(page.getByText('Instructions saved', { exact: true })).toBeVisible();
  await expect(page.getByTestId(`wf-row-${second}`)).toHaveClass(/active/);
});

test('swarm feed: phone composer has usable message width', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  await seedSwarm(ctx, base, ws); await ctx.dispose();
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto('/#/swarm');
  await page.locator('.swarm-item', { hasText: 'E2E Swarm' }).click();
  await page.getByRole('tab', { name: 'Feed', exact: true }).click();
  const composer = page.getByRole('textbox', { name: 'Message', exact: true });
  await expectFullyInViewport(page, composer);
  expect((await composer.boundingBox())!.width).toBeGreaterThan(240);
  await composer.fill('Review the complete release evidence before approving');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: '/tmp/otto-ux-r3-automation-screens/phone-swarm-composer.png' });
});

test('personal agent: failed save preserves form and a retry creates the paused agent', async ({ page }) => {
  const { ctx, ws } = await setup(page); await ctx.dispose();
  let fail = true;
  await page.route(`**/api/v1/workspaces/${ws}/personal-agents`, r => r.request().method() === 'POST' && fail
    ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Agent storage temporarily unavailable' } }) : r.continue());
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto('/#/personal-agents');
  await page.getByRole('button', { name: 'New agent', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Name', { exact: true }).fill('Offline review assistant');
  await dialog.getByLabel('Enabled', { exact: false }).uncheck();
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('Agent storage temporarily unavailable');
  await expectFullyInViewport(page, dialog.getByRole('alert'));
  await expect(dialog.getByLabel('Name', { exact: true })).toHaveValue('Offline review assistant');
  await page.screenshot({ path: '/tmp/otto-ux-r3-automation-screens/phone-agent-save-retry.png' });
  fail = false;
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText('Offline review assistant', { exact: true }).first()).toBeVisible();
});

test('proof waiver: reason is required, denied approval preserves it, retry records waiver', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/proof-packs`, { data: { title: 'Release exception review', work_item_kind: 'manual', work_item_id: `r3-proof-${Date.now()}` } });
  expect(r.ok()).toBeTruthy(); const pack = await r.json(); await ctx.dispose();
  let fail = true;
  await page.route(`**/api/v1/proof-packs/${pack.id}/waive`, route => fail
    ? route.fulfill({ status: 403, json: { code: 'forbidden', message: 'Approval access temporarily unavailable' } }) : route.continue());
  await page.goto('/#/proof');
  await page.getByRole('button', { name: 'Waive', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Waive proof gate' });
  await expect(dialog).toContainText('records you as the approver');
  await dialog.getByLabel('Reason', { exact: true }).fill('Short');
  await expect(dialog.getByRole('button', { name: 'Waive', exact: true })).toBeDisabled();
  await dialog.getByLabel('Reason', { exact: true }).fill('Verified manually with the release evidence.');
  await dialog.getByRole('button', { name: 'Waive', exact: true }).click();
  await expect(page.getByText("Couldn't waive the proof gate", { exact: true })).toBeVisible();
  await expect(dialog.getByLabel('Reason', { exact: true })).toHaveValue('Verified manually with the release evidence.');
  fail = false;
  await dialog.getByRole('button', { name: 'Waive', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator('.waived-note')).toContainText('Verified manually with the release evidence.');
  await page.screenshot({ path: '/tmp/otto-ux-r3-automation-screens/proof-waiver-recorded.png' });
});

test('swarm feed: finishing a post keeps the next message draft', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  const { swarmId } = await seedSwarm(ctx, base, ws); await ctx.dispose();
  await page.goto('/#/swarm');
  await page.getByRole('tab', { name: 'Feed', exact: true }).click();
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  let requested = false;
  await page.route(`**/api/v1/swarm/swarms/${swarmId}/board`, async r => {
    if (r.request().method() !== 'POST') return r.continue();
    requested = true; await gate; await r.fulfill({ json: { id: 'synthetic-post' } });
  });
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await input.fill('Sent message');
  await page.getByRole('button', { name: 'Post', exact: true }).click();
  await expect.poll(() => requested).toBe(true);
  await input.fill('Next message still being written');
  release();
  await expect(page.getByRole('button', { name: 'Post', exact: true })).toBeEnabled();
  await expect(input).toHaveValue('Next message still being written');
});

test('workflow graph: completing Save retains edits made while saving', async ({ page }) => {
  const [first] = await workflows(page);
  await page.locator('.node', { hasText: 'Alpha review start' }).click();
  const message = page.getByLabel('Message / prompt', { exact: true });
  await message.fill('Submitted input');
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  let requested = false;
  await page.route(`**/api/v1/workflows/${first}`, async r => {
    if (r.request().method() !== 'PATCH') return r.continue();
    const response = await r.fetch(); requested = true; await gate; await r.fulfill({ response });
  });
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => requested).toBe(true);
  await message.fill('Newer unsaved input');
  release();
  await expect(page.getByText('Saved', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeEnabled();
  await expect(message).toHaveValue('Newer unsaved input');
});

test('rooms: a late previous-room reply cannot replace the selected conversation', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  const ids: string[] = [];
  for (const name of ['Slow room', 'Current room']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/agent-rooms`, { data: { name } });
    ids.push((await r.json()).id);
  }
  await ctx.dispose();
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const msg = (id: string, text: string) => ({ id: `message-${id}`, room_id: id, author_kind: 'user', author_id: 'synthetic-user', text, created_at: '2026-09-25T12:00:00Z' });
  await page.route(`**/api/v1/agent-rooms/${ids[0]}/messages*`, async r => { await gate; await r.fulfill({ json: [msg(ids[0], 'Old room reply')] }); });
  await page.route(`**/api/v1/agent-rooms/${ids[1]}/messages*`, r => r.fulfill({ json: [msg(ids[1], 'Current room reply')] }));
  await page.goto('/#/personal-agents/rooms');
  await page.getByRole('button', { name: 'Slow room 0 agents' }).click();
  await page.getByRole('button', { name: 'Current room 0 agents' }).click();
  await expect(page.getByRole('log')).toContainText('Current room reply');
  release();
  await page.getByLabel('Message to the room').fill('Current room draft');
  await expect(page.getByRole('log')).not.toContainText('Old room reply');
  await expect(page.getByLabel('Message to the room')).toHaveValue('Current room draft');
});

test('rooms: a full 200-message page continues to the latest evidence', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  const room = await (await ctx.post(`${base}/api/v1/workspaces/${ws}/agent-rooms`, { data: { name: 'Long evidence review' } })).json();
  await ctx.dispose();
  const messages = Array.from({ length: 201 }, (_, i) => ({ id: String(i).padStart(26, '0'), room_id: room.id, author_kind: 'user', author_id: 'synthetic-user', text: `Evidence note ${i + 1}`, created_at: '2026-09-25T12:00:00Z' }));
  const cursors: (string | null)[] = [];
  await page.route(`**/api/v1/agent-rooms/${room.id}/messages*`, r => {
    const after = new URL(r.request().url()).searchParams.get('after'); cursors.push(after);
    return r.fulfill({ json: after ? messages.slice(200) : messages.slice(0, 200) });
  });
  await page.goto('/#/personal-agents/rooms');
  await expect(page.getByText('Evidence note 201', { exact: true })).toBeVisible();
  expect(cursors).toContain(messages[199].id);
  await expect(page.locator('.msg-text')).toHaveCount(201);
});

test.describe('Swarm organization keyboard flow in five themes', () => {
  let workspaceId = '';
  test.beforeAll(async () => {
    const { ctx, base } = await apiCtx();
    workspaceId = await seedWorkspace(ctx, base);
    await seedSwarm(ctx, base, workspaceId);
    await ctx.dispose();
  });
  for (const [theme, scheme, width, height, rtl] of [
    ['native', 'light', 1440, 900, false], ['native', 'dark', 1440, 900, false],
    ['warm', 'light', 375, 812, false], ['warm', 'dark', 834, 1112, true],
    ['pro-dark', 'dark', 1440, 900, false],
  ] as const) {
    test(`org expansion and editor focus: ${theme} ${scheme} ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await page.addInitScript(({ workspaceId, theme, scheme }) => {
        localStorage.setItem('otto_workspace', workspaceId);
        localStorage.setItem('otto_rail_expanded', '0');
        localStorage.setItem('otto_theme', theme);
        localStorage.setItem('otto_scheme', scheme);
      }, { workspaceId, theme, scheme });
      await page.goto('/#/swarm');
      if (width <= 640 || width > 1024) await page.locator('.swarm-item', { hasText: 'E2E Swarm' }).click();
      await expect(page.getByRole('tab', { name: 'Org', exact: true })).toBeVisible();
      if (rtl) await page.evaluate(() => document.documentElement.dir = 'rtl');
      if (width === 834) {
        await expect(page.locator('.switcher')).toHaveCSS('overflow-x', 'auto');
        await page.getByRole('spinbutton', { name: 'Max parallel' }).focus();
        await expectFullyInViewport(page, page.getByRole('spinbutton', { name: 'Max parallel' }));
        await page.getByRole('tab', { name: 'Org', exact: true }).focus();
        await expectFullyInViewport(page, page.getByRole('tab', { name: 'Org', exact: true }));
      }
      const toggle = page.locator('.org-row .twist').first();
      await expect(toggle).toBeVisible();
      const rows = await page.locator('.org-row').count();
      await toggle.focus(); await page.keyboard.press('Enter');
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');
      expect(await page.locator('.org-row').count()).toBeLessThan(rows);
      await page.keyboard.press('Enter');
      await expect(page.locator('.org-row')).toHaveCount(rows);
      await expectNoHorizontalOverflow(page);
      await page.screenshot({ path: `/tmp/otto-ux-r3-automation-screens/${theme}-${scheme}-${width}-org.png` });
      const editorTrigger = page.locator('.org-row .who').first();
      await editorTrigger.focus(); await page.keyboard.press('Enter');
      const dialog = page.getByRole('dialog');
      await expect(dialog.getByLabel('Name', { exact: true })).not.toHaveValue('');
      await expectFullyInViewport(page, dialog);
      await page.screenshot({ animations: 'disabled', path: `/tmp/otto-ux-r3-automation-screens/${theme}-${scheme}-${width}-swarm-agent.png` });
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      await expect(editorTrigger).toBeFocused();
    });
  }
});
