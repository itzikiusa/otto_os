import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// The Agents inbox (spec A1): every session in one list beside the canvas —
// status words, needs you first, multi-select → Tile, and the app navigator
// folded to its icon rail while Agents is open (one click expands it back).

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const TITLES = ['Pirlo', 'Seedorf', 'Inzaghi'];
const ids: Record<string, string> = {};
const SHOTS = process.env.OTTO_E2E_SHOTS;

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  ({ ctx, base } = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
  for (const title of TITLES) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
    ids[title] = (await r.json()).id as string;
  }
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_agents_fold_nav', '1');
    localStorage.setItem('otto_rail_expanded', '1');
  }, wsId);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

/** Switch the scheme and wait until the app has applied it (a screenshot taken
 *  before html[data-scheme] flips shows light tokens under dark form controls). */
async function scheme(page: Page, s: 'light' | 'dark'): Promise<void> {
  await page.emulateMedia({ colorScheme: s });
  await expect(page.locator('html')).toHaveAttribute('data-scheme', s);
}

const rows = (page: Page) => page.locator(`[data-testid="agents-list"] .ai-row[data-workspace="${wsId}"]`);

test('one list with words; needs you first; the navigator folds to its rail and comes back', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/agents');
  await expect(rows(page)).toHaveCount(3, { timeout: 20_000 });
  // A status word on every row, never a bare dot.
  for (const t of TITLES) await expect(page.getByTestId(`agents-status-${ids[t]}`)).toHaveText(/\S/);
  // Folded: the expanded navigator is gone while Agents is open.
  await expect(page.locator('.navigator')).toHaveCount(0);

  // A session that needs you rises to the top and says so (not the one on
  // screen: arrival opened the latest, and a raise you are watching is seen).
  await page.evaluate(async (id) => {
    const path = '/src/lib/stores/workspace.svelte.ts';
    const { ws } = await import(/* @vite-ignore */ path);
    ws.markNeedsYou(id);
  }, ids.Pirlo);
  await expect(rows(page).first()).toContainText('Pirlo');
  await expect(page.getByTestId(`agents-status-${ids.Pirlo}`)).toHaveText('Needs you');

  for (const s of ['light', 'dark'] as const) {
    await scheme(page, s);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-inbox-1440-${s}.png`, animations: 'disabled' });
  }

  // Leaving Agents brings the navigator back.
  await page.goto('/#/settings');
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 15_000 });
});

test('select mode: Tile lays the checked sessions out side by side', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(rows(page)).toHaveCount(3, { timeout: 20_000 });
  await page.getByTestId('agents-select-toggle').click();
  await page.getByLabel('Select Pirlo').check();
  await page.getByLabel('Select Seedorf').check();
  // The count never runs under Tile (design review 2, M3), and is said once.
  const tools = page.getByTestId('agents-select-tools');
  const count = (await tools.locator('.ai-count span').boundingBox())!;
  const tile = (await page.getByTestId('agents-tile-selected').boundingBox())!;
  expect(count.x + count.width).toBeLessThanOrEqual(tile.x + 0.5);
  await expect(tools.locator('.ai-count span')).toHaveText('2');
  await expect(page.getByTestId('agents-delete-selected')).toHaveText('Delete');
  if (SHOTS) for (const s of ['light', 'dark'] as const) { await scheme(page, s); await page.screenshot({ path: `${SHOTS}/agents-select-mode-1440-${s}.png`, animations: 'disabled' }); }
  await page.getByTestId('agents-tile-selected').click();
  await expect(page.locator(`[data-tile-id="${ids.Pirlo}"]`)).toBeVisible({ timeout: 15_000 });
  await expect(page.locator(`[data-tile-id="${ids.Seedorf}"]`)).toBeVisible();
  await expect(page.getByTestId('agents-select-tools')).toHaveCount(0);
});

test('a row opens its session; rename from the row menu; hide and show the list', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(rows(page)).toHaveCount(3, { timeout: 20_000 });
  await page.locator(`[data-testid="agents-row-${ids.Seedorf}"] .tr-main`).click();
  await expect(page.locator(`[data-pane-key][data-session="${ids.Seedorf}"]`).first()).toBeVisible({ timeout: 15_000 });
  await expect(page.locator(`[data-testid="agents-row-${ids.Seedorf}"]`)).toHaveClass(/selected/);

  await page.locator(`[data-testid="agents-row-${ids.Pirlo}"] .tr-main`).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Rename…' }).click();
  await page.locator('.cf-input').fill('Pirlo il Maestro');
  await page.getByRole('button', { name: 'Rename' }).click();
  await expect(page.locator(`[data-testid="agents-row-${ids.Pirlo}"]`)).toContainText('Pirlo il Maestro');

  await page.getByTestId('agents-inbox-hide').click();
  await expect(page.getByTestId('agents-inbox')).toHaveCount(0);
  await page.getByTestId('agents-inbox-show').click();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
});

test('the rarely used dialogs load on demand from the pane ⋯', async ({ page }) => {
  await page.goto(`/#/agents/${ids.Seedorf}`);
  const pane = page.locator(`[data-pane-key][data-session="${ids.Seedorf}"]`).first();
  await expect(pane).toBeVisible({ timeout: 15_000 });
  for (const [item, dialog] of [
    ['Additional directories…', 'Additional directories'],
    ['Hand over to…', /Hand over/],
    ['Attach issue…', /issue/i],
  ] as const) {
    await pane.locator('button[title="More…"]').click();
    await page.getByRole('menuitem', { name: item }).click();
    await expect(page.getByRole('dialog', { name: dialog })).toBeVisible({ timeout: 10_000 });
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
  }
});

test('per-session usage is read only when someone looks (perf A7a)', async ({ page }) => {
  const usageReads: string[] = [];
  page.on('request', (r) => {
    if (/\/api\/v1\/sessions\/[^/]+\/usage/.test(new URL(r.url()).pathname)) usageReads.push(r.url());
  });
  await page.goto(`/#/agents/${ids.Seedorf}`);
  await expect(page.locator(`[data-pane-key][data-session="${ids.Seedorf}"]`).first()).toBeVisible({ timeout: 15_000 });
  // Switching sessions (each focus used to read) reads nothing.
  for (const t of ['Pirlo', 'Inzaghi', 'Seedorf']) {
    await page.locator(`[data-testid="agents-row-${ids[t]}"] .tr-main`).click();
    await expect(page.locator(`[data-pane-key][data-session="${ids[t]}"]`).first()).toBeVisible({ timeout: 15_000 });
  }
  await page.waitForLoadState('networkidle');
  expect(usageReads).toEqual([]);
  // Looking at the details (⋯) reads once.
  await page.locator(`[data-pane-key][data-session="${ids.Seedorf}"]`).first().locator('button[title="More…"]').click();
  await expect.poll(() => usageReads.length).toBe(1);
});

test('Terminal ⇄ Chat: an agent pane shows its conversation and remembers the choice (A2)', async ({ page }) => {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: {
      kind: 'agent', provider: 'shell', title: 'Shevchenko', cwd: '/tmp',
      meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: `${process.cwd()}/../crates/otto-transcript/fixtures/claude/01-basic-tools.jsonl` },
    },
  });
  const id = (await r.json()).id as string;
  await page.goto(`/#/agents/${id}`);
  const pane = page.locator(`[data-pane-key][data-session="${id}"]`).first();
  await expect(pane.locator('.term-host')).toBeVisible({ timeout: 30_000 });
  await pane.getByTestId('pane-view').getByRole('button', { name: 'Chat' }).click();
  await expect(pane.locator('.pane-body[data-view="chat"]')).toBeVisible();
  await expect(pane.locator('.term-host')).toHaveCount(0);
  // Never a blank pane: the conversation (or its own empty line) shows.
  await expect(pane.locator('.conv[data-loaded="true"]')).toBeVisible({ timeout: 15_000 });
  await expect(pane.locator('.conv')).toContainText(/\S/);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-pane-chat-1440-light.png`, animations: 'disabled' });
  await page.reload();
  await expect(page.locator(`[data-pane-key][data-session="${id}"] .pane-body[data-view="chat"]`).first()).toBeVisible({ timeout: 20_000 });
  await page.locator(`[data-pane-key][data-session="${id}"]`).first().getByTestId('pane-view').getByRole('button', { name: 'Terminal' }).click();
  await expect(page.locator(`[data-pane-key][data-session="${id}"] .term-host`).first()).toBeVisible({ timeout: 20_000 });
  // A plain shell has no chat.
  await page.goto(`/#/agents/${ids.Pirlo}`);
  await expect(page.locator(`[data-pane-key][data-session="${ids.Pirlo}"] .term-host`).first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator(`[data-pane-key][data-session="${ids.Pirlo}"]`).first().getByTestId('pane-view')).toHaveCount(0);
});

test('New session is a stage: tiles, Terminal | Chat, a first message, Start; the rest behind Options (A4)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/agents');
  await expect(rows(page)).toHaveCount(3, { timeout: 20_000 });
  await page.keyboard.press('Meta+t');
  const dialog = page.getByRole('dialog', { name: 'New session' });
  await expect(dialog.getByTestId('ns-kind')).toBeVisible({ timeout: 10_000 });
  // Tiles only for providers this machine can start; the rest behind one row.
  const tiles = dialog.locator('.provider-card .card-main');
  for (const t of await tiles.all()) await expect(t).toBeEnabled();
  // Sized to its content: no band of empty sheet above the footer.
  const optsBottom = (await dialog.getByTestId('ns-options').boundingBox())!;
  const footTop = (await dialog.locator('footer').boundingBox())!.y;
  expect(footTop - (optsBottom.y + optsBottom.height)).toBeLessThan(80);
  // Defaults cost no rows: title, folder, workspace are behind Options.
  await expect(dialog.locator('#ns-title')).toHaveCount(0);
  await expect(dialog.getByRole('radio', { name: 'No workspace' })).toHaveCount(0);
  for (const s of ['light', 'dark'] as const) {
    await scheme(page, s);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-new-session-1440-${s}.png`, animations: 'disabled' });
  }
  // Dark is really dark: the tiles and Cancel sit on dark surfaces with light text.
  await scheme(page, 'dark');
  const lum = (c: string) => { const [r, g, b] = c.match(/\d+/g)!.map(Number); return (r + g + b) / 3; };
  // Polled: the surfaces ease into the new scheme (150 ms transitions).
  for (const [sel, el] of [['tile', dialog.locator('.provider-card').first()], ['Cancel', dialog.getByRole('button', { name: 'Cancel' })]] as const) {
    const read = () => el.evaluate((n) => { const s = getComputedStyle(n); return { bg: s.backgroundColor, fg: s.color }; });
    await expect.poll(async () => lum((await read()).fg), { message: `${sel} text is light in dark` }).toBeGreaterThan(180);
    await expect.poll(async () => { const c = await read(); return c.bg.startsWith('rgba(0, 0, 0, 0)') ? 0 : lum(c.bg); }, { message: `${sel} surface is dark in dark` }).toBeLessThan(90);
  }
  await scheme(page, 'light');
  // A changed option shows as a chip once Options folds.
  await dialog.getByTestId('ns-options').click();
  await dialog.locator('#ns-title').fill('Kaladze');
  await dialog.getByTestId('ns-options').click();
  await expect(dialog.locator('.opt-chip', { hasText: 'Title: Kaladze' })).toBeVisible();
  // Enter on the shell tile starts it.
  await dialog.locator('.provider-card', { hasText: 'shell' }).locator('.card-main').click();
  await page.keyboard.press('Enter');
  await expect(dialog).toHaveCount(0, { timeout: 10_000 });
  await expect(page.locator('[data-testid^="agents-row-"]', { hasText: 'Kaladze' })).toBeVisible({ timeout: 15_000 });
});

test('the right panel opens on the session details card; nothing loads while it is closed (A5)', async ({ page }) => {
  const cardChunks: string[] = [];
  page.on('request', (r) => { if (r.url().includes('ThreadDetailsCard')) cardChunks.push(r.url()); });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`/#/agents/${ids.Seedorf}`);
  await expect(page.locator(`[data-pane-key][data-session="${ids.Seedorf}"]`).first()).toBeVisible({ timeout: 15_000 });
  await page.waitForLoadState('networkidle');
  expect(cardChunks).toEqual([]);
  await page.keyboard.press('Meta+j');
  const card = page.getByTestId('agents-details-card');
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card).toContainText('tmp');
  for (const s of ['light', 'dark'] as const) {
    await scheme(page, s);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-details-card-1440-${s}.png`, animations: 'disabled' });
  }
});

test('Chat from the stage starts the chosen provider as a chat pane here (R2, A2)', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(rows(page)).toHaveCount(3, { timeout: 20_000 });
  await page.getByTestId('agents-inbox-new').click();
  const dialog = page.getByRole('dialog', { name: 'New session' });
  await dialog.getByTestId('ns-kind').getByRole('radio', { name: 'Chat' }).click();
  await dialog.locator('.provider-card', { hasText: 'codex' }).locator('.card-main').click();
  await dialog.locator('#ns-prompt').fill('hello there');
  await dialog.getByRole('button', { name: 'Start chat' }).click();
  const pane = page.locator('[data-testid^="thread-pane-"]');
  await expect(pane).toBeVisible({ timeout: 20_000 });
  await expect(pane.getByText('echo: hello there')).toBeVisible({ timeout: 30_000 });
  const started = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string; provider: string }[];
  expect(started.map((r) => r.provider)).toContain('codex');
  for (const r of started) await ctx.post(`${base}/api/v1/runtime-sessions/${r.id}/close`).catch(() => {});
});

// Threads that need you are needs-you here exactly as on Threads: a pending
// approval survives "Only what needs you", and an Otto MCP approval card on a
// thread makes its row say Approval (threadNav `threadClass`, one derivation).
test('a thread waiting on an approval or an MCP card needs you, and "Only what needs you" keeps it', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const mk = async (title: string): Promise<string> => {
    const t = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: '/tmp', title } });
    expect(t.ok(), await t.text()).toBeTruthy();
    return (await t.json()).id as string;
  };
  const asking = await mk('Gattuso asks');
  const carded = await mk('Nesta card');
  await page.route('**/api/v1/mcp/approvals?status=pending*', (r) => r.fulfill({ json: [{
    id: 'apr_e2e', workspace_id: wsId, kind: 'tool_call', server_id: null, server_name: 'otto', tool: 'otto.create_pr',
    title: 'otto MCP server → create_pr', detail: 'Opens a pull request.', args_redacted_json: '{}', risk_label: 'dangerous',
    status: 'pending', requested_by: 'u1', requested_by_kind: 'mcp_server', requested_by_session_id: carded,
    decided_by: null, decision_note: null, created_at: '2026-11-23T10:00:00Z', decided_at: null, consumed_at: null, expires_at: null,
  }] }));
  try {
    const p = await ctx.post(`${base}/api/v1/runtime-sessions/${asking}/prompt`, { data: { text: 'ask' } });
    expect(p.ok(), await p.text()).toBeTruthy();
    await page.goto('/#/agents');
    await expect(page.getByTestId(`agents-status-${asking}`)).toHaveText(/Needs you|Approval/, { timeout: 30_000 });
    await expect(page.getByTestId(`agents-status-${carded}`)).toHaveText(/Needs you|Approval/, { timeout: 20_000 });
    await page.getByTestId('agents-sort-toggle').click();
    await page.getByRole('menuitemcheckbox', { name: 'Only what needs you' }).click();
    await expect(page.getByTestId(`agents-row-${asking}`)).toBeVisible();
    await expect(page.getByTestId(`agents-row-${carded}`)).toBeVisible();
    await expect(rows(page)).toHaveCount(2);
  } finally {
    for (const id of [asking, carded]) await ctx.post(`${base}/api/v1/runtime-sessions/${id}/close`).catch(() => {});
  }
});

// With every session working the live sessions ARE the list: no "Working"
// fold with its own scroll under an empty list (design review B1).
test('working sessions stay in the one list, needs you first', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/agents');
  await expect(rows(page)).toHaveCount(3, { timeout: 20_000 });
  await page.evaluate(async (ids) => {
    const path = '/src/lib/stores/workspace.svelte.ts';
    const { ws } = await import(/* @vite-ignore */ path);
    for (const id of ids) ws.statusMap[id] = 'working';
  }, Object.values(ids));
  for (const t of TITLES) await expect(page.getByTestId(`agents-status-${ids[t]}`)).toHaveText(/^Working/);
  await expect(rows(page)).toHaveCount(3);
  await expect(page.getByTestId('agents-working-toggle')).toHaveCount(0);
  await page.evaluate(async (id) => {
    const path = '/src/lib/stores/workspace.svelte.ts';
    const { ws } = await import(/* @vite-ignore */ path);
    ws.markNeedsYou(id);
  }, ids.Pirlo);
  await expect(rows(page).first()).toContainText('Pirlo');
});
