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

  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-inbox-1440-${scheme}.png` });
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
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-pane-chat-1440-light.png` });
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
  // Defaults cost no rows: title, folder, workspace are behind Options.
  await expect(dialog.locator('#ns-title')).toHaveCount(0);
  await expect(dialog.getByRole('radio', { name: 'No workspace' })).toHaveCount(0);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-new-session-1440-${scheme}.png` });
  }
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
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-details-card-1440-${scheme}.png` });
  }
});
