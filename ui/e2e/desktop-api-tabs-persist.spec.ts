import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// API client — open request tabs persist across reloads (per workspace):
//   • Edited tabs (method/url/headers) + the active index survive a reload,
//     so reopening the app restores the calls you had open — not only History.
//   • A closed tab stays closed after reload.
//   • Tab sets are keyed per workspace: switching workspaces swaps tab sets
//     without leaking drafts between them.
//   • A corrupt persisted payload is discarded gracefully (fresh blank tab,
//     page still functional and re-persisting).
//
// Desktop-browser project only.
// ─────────────────────────────────────────────────────────────────────────────

let wsA = '';
let wsB = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsA = await seedWorkspace(ctx, base);
  wsB = await seedWorkspace(ctx, base);
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    if (!localStorage.getItem('otto_workspace')) localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsA);
});

const urlInput = (page: Page) => page.getByLabel('Request URL');
const methodSelect = (page: Page) => page.getByLabel('HTTP method');
const tabs = (page: Page) => page.locator('.req-tab');

/** Give the store's debounced localStorage write time to land. */
async function settleWrite(page: Page): Promise<void> {
  await page.waitForTimeout(450);
}

async function switchWorkspace(page: Page, wsId: string): Promise<void> {
  await page.evaluate((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.reload();
  await openPage(page, 'api');
}

test('open tabs + active index survive a reload', async ({ page }) => {
  await openPage(page, 'api');
  await expect(tabs(page)).toHaveCount(1);

  // Tab 1: POST https://example.com/one with a custom header.
  await methodSelect(page).selectOption('POST');
  await urlInput(page).fill('https://example.com/one');
  await page.getByRole('tab', { name: 'Headers', exact: true }).click();
  await page.getByRole('button', { name: 'Add header' }).click();
  await page.locator('.kv-row .kv-key').last().fill('X-Persist');
  await page.locator('.kv-row .kv-val').last().fill('yes');

  // Tab 2: GET https://example.com/two (stays active).
  await page.locator('.req-tab-new').click();
  await expect(tabs(page)).toHaveCount(2);
  await urlInput(page).fill('https://example.com/two');

  await settleWrite(page);
  await page.reload();
  await openPage(page, 'api');

  // Both tabs restored; the second one still active with its URL.
  await expect(tabs(page)).toHaveCount(2);
  await expect(tabs(page).nth(1)).toHaveClass(/active/);
  await expect(urlInput(page)).toHaveValue('https://example.com/two');

  // Tab 1 restored wholesale: method, URL and the nested header row.
  await tabs(page).first().click();
  await expect(methodSelect(page)).toHaveValue('POST');
  await expect(urlInput(page)).toHaveValue('https://example.com/one');
  await page.getByRole('tab', { name: 'Headers', exact: true }).click();
  await expect(page.locator('.kv-row .kv-key').last()).toHaveValue('X-Persist');
  await expect(page.locator('.kv-row .kv-val').last()).toHaveValue('yes');
});

test('a closed tab stays closed after reload', async ({ page }) => {
  await openPage(page, 'api');
  await urlInput(page).fill('https://example.com/keep-1');
  await page.locator('.req-tab-new').click();
  await urlInput(page).fill('https://example.com/drop');
  await page.locator('.req-tab-new').click();
  await urlInput(page).fill('https://example.com/keep-2');
  await expect(tabs(page)).toHaveCount(3);

  // Close the middle tab, then reload.
  await tabs(page).nth(1).locator('.req-tab-close').click();
  await expect(tabs(page)).toHaveCount(2);
  await settleWrite(page);
  await page.reload();
  await openPage(page, 'api');

  await expect(tabs(page)).toHaveCount(2);
  await expect(tabs(page).first()).toContainText('keep-1');
  await expect(tabs(page).nth(1)).toContainText('keep-2');
});

test('tab sets are per-workspace', async ({ page }) => {
  await openPage(page, 'api');
  await urlInput(page).fill('https://a.example.com/only-in-A');
  await settleWrite(page);

  // Workspace B starts blank; give it its own tab.
  await switchWorkspace(page, wsB);
  await expect(tabs(page)).toHaveCount(1);
  await expect(urlInput(page)).toHaveValue('');
  await urlInput(page).fill('https://b.example.com/only-in-B');
  await settleWrite(page);

  // Back to A: A's draft is intact, B's did not leak in.
  await switchWorkspace(page, wsA);
  await expect(tabs(page)).toHaveCount(1);
  await expect(urlInput(page)).toHaveValue('https://a.example.com/only-in-A');

  // And B still holds its own.
  await switchWorkspace(page, wsB);
  await expect(urlInput(page)).toHaveValue('https://b.example.com/only-in-B');
});

test('corrupt persisted payload is discarded gracefully', async ({ page }) => {
  // Inject the corrupt payload ONCE (init scripts rerun on reload; a second
  // injection would clobber the recovered state this test re-persists).
  await page.addInitScript((wsId) => {
    if (localStorage.getItem('e2e_corrupt_injected')) return;
    localStorage.setItem('e2e_corrupt_injected', '1');
    localStorage.setItem(`otto_api_tabs_v1:${wsId}`, '{not json!!');
  }, wsA);
  await openPage(page, 'api');

  // Fresh blank tab, page functional.
  await expect(tabs(page)).toHaveCount(1);
  await expect(urlInput(page)).toHaveValue('');

  // And persistence works again from here on.
  await urlInput(page).fill('https://example.com/recovered');
  await settleWrite(page);
  await page.reload();
  await openPage(page, 'api');
  await expect(urlInput(page)).toHaveValue('https://example.com/recovered');
});
