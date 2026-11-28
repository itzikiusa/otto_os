import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFingerSized, expectNoHorizontalOverflow } from './helpers';

// Phone (spec A1 + A6): #/agents is the inbox; a tap pushes the session
// full-height; "Sessions" (or the system Back) returns to the list.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const SHOTS = process.env.OTTO_E2E_SHOTS;

test.beforeEach(async ({ page }, info) => {
  test.skip(!['iphone-portrait', 'android-phone'].includes(info.project.name), 'phone projects');
  ({ ctx, base } = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
  for (const title of ['Gullit', 'Rijkaard']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
  }
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

test('a long session title shrinks before the Back label does: "‹ Sessions" stays whole', async ({ page }) => {
  const title = 'A very long session title that cannot fit beside the Back on any phone at all';
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
  });
  expect(r.ok()).toBeTruthy();
  await page.goto('/#/agents');
  const row = page.locator(`.ai-row[data-workspace="${wsId}"] .tr-main`, { hasText: 'A very long' });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.tap();
  const back = page.getByTestId('agents-back-to-list');
  await expect(back).toHaveText('Sessions');
  // The laid-out label is as wide as its first word's text (nothing ellipsized).
  const { shown, needed } = await back.evaluate((b) => {
    const label = b.querySelector('.pb-label') as HTMLElement;
    const range = document.createRange();
    range.selectNodeContents(label);
    return { shown: label.getBoundingClientRect().width, needed: range.getBoundingClientRect().width };
  });
  expect(shown).toBeGreaterThanOrEqual(needed - 1);
});

test('the list is the page; a tap pushes the terminal; Back returns', async ({ page }, info) => {
  await page.goto('/#/agents');
  const row = page.locator(`.ai-row[data-workspace="${wsId}"] .tr-main`, { hasText: 'Gullit' });
  await expect(row).toBeVisible({ timeout: 20_000 });
  const box = await row.boundingBox();
  expect(box!.height).toBeGreaterThanOrEqual(44);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-inbox-${info.project.name}.png` });

  // ONE header row: the list's header is the page's (☰ · Sessions N · New · ⋯ · Side panel).
  await expect(page.locator('.mtopbar')).toHaveCount(0);
  const head = page.locator('.page-header-bar').first();
  await expect(head.locator('.ph-title')).toHaveText('Sessions');
  await expect(head.getByTestId('open-navigator')).toBeVisible();
  await expect(head.getByTestId('agents-inbox-new')).toBeVisible();
  await expect(head.getByTestId('open-side-panel')).toBeVisible();

  await row.tap();
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('agents-inbox')).toBeHidden();
  // …and the pushed session's pane header IS the row: ‹ Sessions · word · title · ⋯ · Side panel.
  await expect(page.locator('.mtopbar')).toHaveCount(0);
  await expect(page.getByTestId('agents-back-to-list')).toHaveText('Sessions');
  await expect(page.locator('.pane-head [data-testid="open-side-panel"]')).toBeVisible();
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-session-${info.project.name}.png` });

  await page.getByTestId('agents-back-to-list').tap();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
  // Back keeps the session marked in the list.
  await expect(row).toHaveAttribute('aria-current', 'true');
  await row.tap();
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  await page.goBack();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
});

// The terminal's ‹ steps back too: the system Back from the list leaves Agents.
test('a terminal\'s ‹ steps back to the list; the system Back then leaves Agents', async ({ page }) => {
  await page.goto('/#/settings');
  await page.evaluate(() => (location.hash = '#/agents'));
  const row = page.locator(`.ai-row[data-workspace="${wsId}"] .tr-main`, { hasText: 'Gullit' });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.tap();
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  await page.getByTestId('agents-back-to-list').tap();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/agents');
  await page.goBack();
  await expect.poll(() => page.evaluate(() => location.hash)).toMatch(/^#\/settings/);
});

// Phone targets are 44 px: the list's Select / Sort, its New session, and the
// app bar's controls (design review H8).
test('the list and the app bar give 44 px targets', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.getByTestId('agents-inbox')).toBeVisible({ timeout: 20_000 });
  // …and the workspace switch (design review 2, H8). With no usage numbers the
  // footer says nothing and is not shown (decision 13); Usage is in the ⋯.
  const targets = [page.getByTestId('agents-select-toggle'), page.getByTestId('agents-sort-toggle'), page.getByTestId('agents-inbox-new'), page.getByTestId('agents-current-ws')];
  await expect(page.getByTestId('threads-open-usage')).toHaveCount(0);
  for (const t of targets) {
    const box = (await t.boundingBox())!;
    expect(Math.round(box.height), await t.getAttribute('aria-label') ?? await t.innerText()).toBeGreaterThanOrEqual(44);
    expect(Math.round(box.width), await t.getAttribute('aria-label') ?? await t.innerText()).toBeGreaterThanOrEqual(44);
  }
  // The one header row's buttons: 44 tall, 38 wide (components sheet "Phone header row").
  for (const b of await page.locator('.page-header-bar .ph-row button:visible').all()) {
    const box = (await b.boundingBox())!;
    expect(Math.round(box.height), (await b.getAttribute('aria-label')) ?? '').toBeGreaterThanOrEqual(44);
    expect(Math.round(box.width), (await b.getAttribute('aria-label')) ?? '').toBeGreaterThanOrEqual(38);
  }
});

test('workspace groups fit the phone list and fold on tap', async ({ page }) => {
  const name = 'Another workspace with a long name for the phone sidebar';
  const other = await seedWorkspace(ctx, base, name);
  const r = await ctx.post(`${base}/api/v1/workspaces/${other}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Other session', cwd: '/tmp', meta: { origin: 'e2e' } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();

  await page.goto('/#/agents');
  await expect(page.getByTestId('agents-inbox')).toBeVisible({ timeout: 20_000 });
  await page.getByTestId('agents-sort-toggle').tap();
  await page.getByRole('menuitemcheckbox', { name: 'All workspaces' }).tap();
  const group = page.getByRole('region', { name, exact: true });
  const fold = group.getByRole('button', { name: `${name} 1` });
  await expect(group.getByText('Other session', { exact: true })).toBeVisible();
  await expectFingerSized(fold);
  await expectNoHorizontalOverflow(page);
  await fold.tap();
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  await expect(group.getByText('Other session', { exact: true })).toHaveCount(0);
});

// New session on a phone: the Terminal | Chat switch and each card's − / +
// take a finger (design review 2, H8).
test('the New session sheet gives 44 px targets', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.getByTestId('agents-inbox')).toBeVisible({ timeout: 20_000 });
  await page.getByTestId('agents-inbox-new').tap();
  const dialog = page.getByRole('dialog', { name: 'New session' });
  const kinds = dialog.getByTestId('ns-kind').getByRole('radio');
  await expect(kinds.first()).toBeVisible({ timeout: 10_000 });
  await expectFingerSized(...(await kinds.all()), ...(await dialog.locator('.count-ctl:visible .cbtn').all()));
});
