import { test, expect, devices, type Page } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { setup } from './threadsReviewNotesFixture';

// Lazy pieces on a phone at 4× CPU throttle: the first open of the model
// picker and of a review note box, measured from the tap to the piece on
// screen (a cold chunk vs. a warm one). OTTO_THREADS_RT_PERF=1 runs it;
// OTTO_THREADS_RT_PERF_OUT=<file> keeps the numbers.
test.skip(!process.env.OTTO_THREADS_RT_PERF, 'perf suite off');

async function tapToVisible(page: Page, tap: () => Promise<void>, sel: string): Promise<number> {
  const t0 = await page.evaluate(() => performance.now());
  await tap();
  await expect(page.locator(sel).first()).toBeVisible({ timeout: 30_000 });
  return Math.round((await page.evaluate(() => performance.now())) - t0);
}

test('phone, 4× throttle: first model picker and first review note', async ({ browser }) => {
  // Chromium with the phone's viewport and touch (CDP throttling needs Chromium).
  const { defaultBrowserType: _d, ...iphone } = devices['iPhone 14 Pro Max'];
  const context = await browser.newContext({ ...iphone, storageState: test.info().project.use.storageState as string });
  const page = await context.newPage();
  await setup(page, { inspector: true });
  await page.goto('/#/threads/notes-alpha');
  await expect(page.getByTestId('runtime-composer')).toBeVisible({ timeout: 30_000 });
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setCPUThrottlingRate', { rate: 4 });
  const out: Record<string, number> = {};
  const model = page.getByTestId('runtime-model');
  out.modelPickerFirstMs = await tapToVisible(page, () => model.tap(), '[data-testid^="model-picker-provider-"]');
  await page.keyboard.press('Escape');
  out.modelPickerAgainMs = await tapToVisible(page, () => model.tap(), '[data-testid^="model-picker-provider-"]');
  await page.keyboard.press('Escape');
  await page.getByTestId('threads-toggle-inspector').tap();
  const changesTab = page.getByTestId('threads-tab-changes');
  if (await changesTab.isVisible()) await changesTab.tap();
  const line = page.getByTestId('inspector-changes-tab').getByRole('button', { name: 'Note on src/a.ts new line 9', exact: true });
  await expect(line).toBeVisible({ timeout: 30_000 });
  await line.scrollIntoViewIfNeeded();
  out.reviewNoteFirstMs = await tapToVisible(page, () => line.tap(), '[data-testid="diff-note-input"]');
  console.log(JSON.stringify({ lazyFirstUse: out }));
  if (process.env.OTTO_THREADS_RT_PERF_OUT) writeFileSync(process.env.OTTO_THREADS_RT_PERF_OUT + '.lazy.json', JSON.stringify(out, null, 2));
  await context.close();
});
