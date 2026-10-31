import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';

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
  const composer = page.getByLabel('Message to the channel');
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
  const composer = page.getByLabel('Message to the channel');
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

test('rooms: live messages append from the WS event without a refetch (perf R2/R3)', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  const room = await (await ctx.post(`${base}/api/v1/workspaces/${ws}/agent-rooms`, { data: { name: 'Live room' } })).json();
  const gets: string[] = [];
  page.on('request', r => {
    if (r.method() === 'GET' && r.url().includes(`/agent-rooms/${room.id}/messages`)) gets.push(r.url());
  });
  await page.goto('/#/personal-agents/rooms');
  await page.getByRole('button', { name: 'Live room 0 agents' }).click();
  await expect.poll(() => gets.length).toBeGreaterThan(0);
  await expect(page.getByRole('log', { name: 'Channel messages' })).toBeVisible();
  const before = gets.length;
  // Another author posts (REST as the same user stands in for an agent).
  await ctx.post(`${base}/api/v1/agent-rooms/${room.id}/messages`, { data: { text: 'Pushed over the event stream' } });
  await expect(page.getByText('Pushed over the event stream', { exact: true })).toBeVisible();
  // Our own post appends the POST's returned row, deduped against its echo.
  await page.getByLabel('Message to the channel').fill('Sent from the composer');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByText('Sent from the composer', { exact: true })).toHaveCount(1);
  await page.waitForTimeout(300);
  await expect(page.getByText('Sent from the composer', { exact: true })).toHaveCount(1);
  expect(gets.length).toBe(before);
  await ctx.dispose();
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
  await expect(dialog.locator('.err')).toContainText('Agent storage temporarily unavailable');
  await expectFullyInViewport(page, dialog.locator('.err'));
  await expect(dialog.getByLabel('Name', { exact: true })).toHaveValue('Offline review assistant');
  await page.screenshot({ path: '/tmp/otto-ux-r3-automation-screens/phone-agent-save-retry.png' });
  fail = false;
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText('Offline review assistant', { exact: true }).first()).toBeVisible();
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
  await page.getByLabel('Message to the channel').fill('Current room draft');
  await expect(page.getByRole('log')).not.toContainText('Old room reply');
  await expect(page.getByLabel('Message to the channel')).toHaveValue('Current room draft');
});

test('rooms: opens the latest 200 messages and pages earlier evidence', async ({ page }) => {
  const { ctx, base, ws } = await setup(page);
  const room = await (await ctx.post(`${base}/api/v1/workspaces/${ws}/agent-rooms`, { data: { name: 'Long evidence review' } })).json();
  await ctx.dispose();
  const messages = Array.from({ length: 201 }, (_, i) => ({ id: String(i).padStart(26, '0'), room_id: room.id, author_kind: 'user', author_id: 'synthetic-user', text: `Evidence note ${i + 1}`, created_at: '2026-09-25T12:00:00Z' }));
  const requests: { after: string | null; before: string | null; tail: boolean; limit: number }[] = [];
  await page.route(`**/api/v1/agent-rooms/${room.id}/messages*`, r => {
    const query = new URL(r.request().url()).searchParams;
    const after = query.get('after'), before = query.get('before');
    const tail = query.get('tail') === 'true';
    const limit = Math.min(500, Math.max(1, Number(query.get('limit') ?? 100)));
    requests.push({ after, before, tail, limit });
    // Match the contract: after takes precedence; tail/before select the
    // newest matching page, still returned in chronological order.
    const eligible = after
      ? messages.slice(messages.findIndex(m => m.id === after) + 1)
      : before ? messages.slice(0, Math.max(0, messages.findIndex(m => m.id === before))) : messages;
    return r.fulfill({ json: !after && (before || tail) ? eligible.slice(-limit) : eligible.slice(0, limit) });
  });
  await page.goto('/#/personal-agents/rooms');
  await expect(page.getByText('Evidence note 201', { exact: true })).toBeVisible();
  await expect(page.getByText('Evidence note 1', { exact: true })).toHaveCount(0);
  await expect(page.locator('.msg-text')).toHaveCount(200);
  expect(requests).toEqual([{ after: null, before: null, tail: true, limit: 200 }]);
  await page.getByRole('button', { name: 'Show earlier messages', exact: true }).click();
  await expect(page.locator('.msg-text')).toHaveCount(201);
  await expect(page.getByText('Evidence note 1', { exact: true })).toBeVisible();
  expect(requests.at(-1)).toEqual({ after: null, before: messages[1].id, tail: false, limit: 200 });
});
