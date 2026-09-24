import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The floating "Type or speak… ⌘K" bar, in-app host (desktop-browser only).
//
// ⌘K focuses the bar (it is the command surface on desktop); typing lists
// commands and an "Ask Otto" row; Enter runs the selection; Esc clears then
// closes; ⌃1–⌃4 switch spaces (persisted per device); a focused terminal
// docks the bar into the status bar and keeps every key; the results panel
// stays inside the window however short it is.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let sessionId = '';

const bar = (page: Page) => page.getByTestId('floating-bar');
const input = (page: Page) => page.getByRole('combobox', { name: 'Ask Otto or search commands' });
const options = (page: Page) => bar(page).getByRole('option');
/** The input even while docked (hidden from the a11y tree then). */
const rawInput = (page: Page) => bar(page).locator('input.input-main');

// Generous: parallel slots share one machine and a cold Vite compile.
test.setTimeout(180_000);

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Kaka', cwd: '/tmp', meta: { origin: 'e2e' } },
  });
  if (!r.ok()) throw new Error(`seed session → ${r.status()} ${await r.text()}`);
  sessionId = (await r.json()).id as string;
  // One-shot reset (init scripts re-run on reload): fresh spaces, AI fallback
  // off so a free-form ask resolves deterministically without an LLM.
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    if (!sessionStorage.getItem('fb-reset')) {
      sessionStorage.setItem('fb-reset', '1');
      localStorage.removeItem('otto_bar_spaces');
      localStorage.removeItem('otto_bar_pref');
      localStorage.setItem('otto_orch_fallback', '0');
    }
  }, wsId);
  await page.goto('/#/git', { waitUntil: 'domcontentloaded' });
  // The first load of a run compiles the app in a fresh Vite server.
  await expect(page.locator('.shell')).toBeVisible({ timeout: 150_000 });
  await expect(bar(page)).toHaveAttribute('data-presence', 'dock');
});

test.afterEach(async () => {
  await ctx?.dispose();
});

test('⌘K focuses the bar and a command runs with Enter', async ({ page }) => {
  await page.keyboard.press('Meta+k');
  await expect(input(page)).toBeFocused();
  await expect(bar(page)).toHaveAttribute('data-presence', 'full');
  await expect(page.locator('.palette')).toHaveCount(0); // one surface, not two

  await page.keyboard.type('go to vault');
  const first = options(page).first();
  await expect(first).toContainText('Go to Vault');
  await expect(first).toHaveAttribute('aria-selected', 'true');
  await expect(input(page)).toHaveAttribute('aria-activedescendant', 'fb-opt-0');
  // The Ask Otto row is always offered for typed text.
  await expect(options(page).filter({ hasText: 'Ask Otto' })).toHaveCount(1);

  await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(() => window.location.hash)).toBe('#/vault');
  await expect(input(page)).not.toBeFocused();
  await expect(input(page)).toHaveValue('');

  // ⌘K again focuses; ⌘K while focused closes.
  await page.keyboard.press('Meta+k');
  await expect(input(page)).toBeFocused();
  await page.keyboard.press('Meta+k');
  await expect(input(page)).not.toBeFocused();
});

test('free text defaults to Ask Otto and the answer lands in the thread', async ({ page }) => {
  await page.keyboard.press('Meta+k');
  await page.keyboard.type('what is on my plate today');
  const first = options(page).first();
  await expect(first).toContainText('Ask Otto');
  await expect(first).toContainText('what is on my plate today');
  await page.keyboard.press('Enter');

  const turn = bar(page).locator('.turn').last();
  await expect(turn.locator('.q')).toHaveText('what is on my plate today');
  // AI fallback is off → the engine says so honestly (no fake answer).
  await expect(turn.locator('.a')).toContainText('couldn’t turn that into an action', { timeout: 15_000 });
  await expect(turn.locator('.src')).toContainText('Ask Otto');
  await expect(input(page)).toHaveValue('');

  // A deterministic request runs; the main window foregrounds the new
  // session (like ⌘I), and the thread keeps the result with an Open link.
  await page.keyboard.type('open 1 shell session');
  await page.keyboard.press('Meta+Enter'); // ⌘↵ asks whatever is selected
  await expect.poll(() => page.evaluate(() => window.location.hash), { timeout: 20_000 }).toMatch(/^#\/agents\/.+/);
  if (!(await rawInput(page).evaluate((el) => el === document.activeElement))) {
    await page.keyboard.press('Meta+k');
  }
  const done = bar(page).locator('.turn').last();
  await expect(done.locator('.q')).toHaveText('open 1 shell session');
  await expect(done.locator('.a')).toContainText('Done');
  await expect(done.getByRole('button', { name: /Open/ })).toBeVisible();
});

test('Esc clears the query first, then closes the bar', async ({ page }) => {
  await page.keyboard.press('Meta+k');
  await page.keyboard.type('git');
  await expect(options(page).first()).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(input(page)).toHaveValue('');
  await expect(input(page)).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(input(page)).not.toBeFocused();
  await expect(bar(page)).toHaveAttribute('data-presence', 'dock');
});

test('⌃1–⌃4 switch spaces inside the bar, and spaces persist per device', async ({ page }) => {
  await page.keyboard.press('Meta+k');
  const spaces = bar(page).getByRole('radio');
  await expect(spaces).toHaveCount(4);
  await expect(spaces.nth(0)).toHaveAttribute('aria-checked', 'true');

  await page.keyboard.press('Control+3');
  await expect(spaces.nth(2)).toHaveAttribute('aria-checked', 'true');
  await expect(input(page)).toBeFocused(); // the global ⌃3 "jump to session" didn't fire

  // Clicking the active space opens its settings: rename it.
  await spaces.nth(2).click();
  const name = bar(page).getByLabel('Name');
  await expect(name).toBeVisible();
  await name.fill('Casino');
  await name.press('Tab');
  await expect(spaces.nth(2)).toHaveAttribute('aria-label', 'Space 3: Casino');

  await page.reload({ waitUntil: 'domcontentloaded' });
  await expect(page.locator('.shell')).toBeVisible({ timeout: 60_000 });
  await page.keyboard.press('Meta+k');
  await expect(bar(page).getByRole('radio').nth(2)).toHaveAttribute('aria-checked', 'true');
  await expect(bar(page).getByRole('radio').nth(2)).toHaveAttribute('aria-label', 'Space 3: Casino');
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('otto_bar_spaces') ?? '{}'));
  expect(saved.active).toBe(2);
  expect(saved.spaces[2].name).toBe('Casino');
});

test('a focused terminal keeps its keys; the bar docks into the status bar', async ({ page }) => {
  await page.goto(`/#/agents/${sessionId}`);
  const term = page.locator(`[data-session="${sessionId}"] .xterm`).first();
  await expect(term).toBeVisible({ timeout: 20_000 });
  await term.click();
  await expect(bar(page)).toHaveAttribute('data-presence', 'dock', { timeout: 10_000 });

  await page.keyboard.type('echo fbmarker');
  await expect(rawInput(page)).toHaveValue('');
  await expect(rawInput(page)).not.toBeFocused();
  await expect(page.locator(`[data-session="${sessionId}"] .xterm-rows`)).toContainText('echo fbmarker', {
    timeout: 10_000,
  });

  // The docked chip sits inside the status bar, over no content.
  const chip = bar(page).getByRole('button', { name: 'Open the Otto bar' });
  const chipBox = await chip.boundingBox();
  const status = await page.locator('.statusbar').boundingBox();
  expect(chipBox && status).toBeTruthy();
  expect(chipBox!.y).toBeGreaterThanOrEqual(status!.y - 1);
  expect(chipBox!.y + chipBox!.height).toBeLessThanOrEqual(status!.y + status!.height + 1);

  // ⌘K still reaches the bar from the terminal.
  await page.keyboard.press('Meta+k');
  await expect(input(page)).toBeFocused();
  await expect(bar(page)).toHaveAttribute('data-presence', 'full');
});

test('the results panel stays inside a short window and scrolls', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 440 });
  await page.keyboard.press('Meta+k');
  await page.keyboard.type('go');
  await expect(options(page).first()).toBeVisible();
  await page.waitForTimeout(800); // let the debounced workspace search land
  const panel = bar(page).locator('.panel');
  await expectFullyInViewport(page, panel, 'floating bar results panel');
  await expectFullyInViewport(page, bar(page).locator('.surface'), 'floating bar');
  // Every row is reachable: the last one scrolls into view with the keyboard.
  const count = await options(page).count();
  expect(count).toBeGreaterThan(6);
  for (let i = 0; i < count - 1; i++) await page.keyboard.press('ArrowDown');
  const last = options(page).nth(count - 1);
  await expect(last).toHaveAttribute('aria-selected', 'true');
  await expectFullyInViewport(page, last, 'last result row');
  // Arrow keys wrap back to the top.
  await page.keyboard.press('ArrowDown');
  await expect(options(page).first()).toHaveAttribute('aria-selected', 'true');
});

test('hidden in Settings → ⌘K falls back to the palette sheet', async ({ page }) => {
  await page.evaluate(() => localStorage.setItem('otto_bar_pref', 'hidden'));
  await page.reload({ waitUntil: 'domcontentloaded' });
  await expect(page.locator('.shell')).toBeVisible({ timeout: 60_000 });
  await expect(bar(page)).toHaveCount(0);
  await page.keyboard.press('Meta+k');
  await expect(page.locator('.palette')).toBeVisible();
});
