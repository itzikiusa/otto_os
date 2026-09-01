import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// Live-tab element-picker overlay (Task 10): unit-level coverage of
// overlay.js's selector algorithm and pick/queue/highlight protocol, run
// against a bare fixture page — no Otto app, no daemon, no Tauri needed.
// `overlay.js` is injected into a live tab's own JS context via
// `browser_eval` (it can't be `eval`'d through Tauri IPC here, since a native
// webview is unavailable outside the packaged desktop app — see Task 9's
// report), but the SAME injection mechanism the real app uses —
// `page.addScriptTag({ content: <raw source> })`, then driving it purely
// through its public `window.__ottoOverlay` surface — is available in a
// plain Chromium page, and exercises the real, unmodified file.
//
// selector.ts (the TS twin used by reader-mode marks) shares the exact same
// priority — id > data-* test attribute > nth-of-type path — so this spec
// stands in for both; ReaderView's own marks are covered end-to-end (through
// the id/nth-of-type branches the sanitizer actually allows) by
// desktop-browser-reader.spec.ts.

test.describe.configure({ mode: 'serial' });

const OVERLAY_SRC = readFileSync(
  join(process.cwd(), 'src/modules/browser/overlay.js'),
  'utf8',
);

// An over-length id/data-testid — long enough to exceed overlay.js's
// MAX_SELECTOR_LEN (300) once wrapped as `#…`/`[data-testid="…"]` — attacker
// content a hostile page's own author fully controls.
const LONG_ID = 'a'.repeat(310);
const LONG_TESTID = 'b'.repeat(310);

const FIXTURE_HTML = `<!doctype html><html><body>
  <div id="with-id">A</div>
  <div data-testid="my-test-id">B</div>
  <div class="plain">
    <p>one</p>
    <p>two</p>
    <p>three</p>
  </div>
  <div id="${LONG_ID}" data-testid="short-ok">C</div>
  <div data-testid="${LONG_TESTID}">D</div>
</body></html>`;

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only — viewport-independent');
  await page.setContent(FIXTURE_HTML);
  await page.addScriptTag({ content: OVERLAY_SRC });
  expect(await page.evaluate(() => typeof (window as any).__ottoOverlay)).toBe('object');
});

test('clicks are ignored while picking is off', async ({ page }) => {
  await page.locator('#with-id').click();
  const drained = await page.evaluate(() => (window as any).__ottoOverlay.tick('[]'));
  expect(drained).toEqual([]);
});

test('id takes priority over everything else', async ({ page }) => {
  await page.evaluate(() => (window as any).__ottoOverlay.setPicking(true));
  await page.locator('#with-id').click();
  const [mark] = await page.evaluate(() => (window as any).__ottoOverlay.tick('[]'));
  expect(mark.selector).toBe('#with-id');
  expect(mark.text).toBe('A');
  expect(mark.rect).toMatchObject({ width: expect.any(Number), height: expect.any(Number) });
});

test('data-testid wins over the nth-of-type fallback when there is no id', async ({ page }) => {
  await page.evaluate(() => (window as any).__ottoOverlay.setPicking(true));
  await page.locator('[data-testid="my-test-id"]').click();
  const [mark] = await page.evaluate(() => (window as any).__ottoOverlay.tick('[]'));
  expect(mark.selector).toBe('[data-testid="my-test-id"]');
});

test('nth-of-type path from body is the fallback for a plain element', async ({ page }) => {
  await page.evaluate(() => (window as any).__ottoOverlay.setPicking(true));
  await page.getByText('two', { exact: true }).click();
  const [mark] = await page.evaluate(() => (window as any).__ottoOverlay.tick('[]'));
  // body's 3rd child div (.plain) > its 2nd <p> ("two").
  expect(mark.selector).toBe('div:nth-of-type(3) > p:nth-of-type(2)');
});

test('the queue drains and does not repeat marks on the next tick', async ({ page }) => {
  await page.evaluate(() => (window as any).__ottoOverlay.setPicking(true));
  await page.locator('#with-id').click();
  const first = await page.evaluate(() => (window as any).__ottoOverlay.tick('[]'));
  expect(first).toHaveLength(1);
  const second = await page.evaluate(() => (window as any).__ottoOverlay.tick('[]'));
  expect(second).toHaveLength(0);
});

test('an over-length id is skipped in favor of a short data-testid', async ({ page }) => {
  await page.evaluate(() => (window as any).__ottoOverlay.setPicking(true));
  await page.getByText('C', { exact: true }).click();
  const [mark] = await page.evaluate(() => (window as any).__ottoOverlay.tick('[]'));
  expect(mark.selector).toBe('[data-testid="short-ok"]');
});

test('an over-length data-testid with no id falls all the way through to nth-of-type', async ({ page }) => {
  await page.evaluate(() => (window as any).__ottoOverlay.setPicking(true));
  await page.getByText('D', { exact: true }).click();
  const [mark] = await page.evaluate(() => (window as any).__ottoOverlay.tick('[]'));
  expect(mark.selector).not.toContain('b'.repeat(310));
  expect(mark.selector.length).toBeLessThanOrEqual(300);
  // body's 5th child div.
  expect(mark.selector).toBe('div:nth-of-type(5)');
});

test('tick highlights existing marks and un-highlights ones no longer passed', async ({ page }) => {
  await page.evaluate((json) => (window as any).__ottoOverlay.tick(json), '[{"selector":"#with-id"}]');
  await expect(page.locator('.__otto_mark__')).toHaveCount(1);
  await page.evaluate((json) => (window as any).__ottoOverlay.tick(json), '[]');
  await expect(page.locator('.__otto_mark__')).toHaveCount(0);
});
