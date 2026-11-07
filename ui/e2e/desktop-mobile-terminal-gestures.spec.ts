import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';

test.use({ hasTouch: true, viewport: { width: 390, height: 844 }, serviceWorkers: 'block' });
type Probe = { disposed: boolean; text(): string };
async function text(page: Page): Promise<string> {
  return page.evaluate(() => (window as unknown as { __ottoTermProbe: Probe[] }).__ottoTermProbe.find(p => !p.disposed)?.text() ?? '');
}
for (const [name, width, height] of [['portrait', 390, 844], ['landscape', 844, 390]] as const) {
  test(`touch ${name}: drag reads older output without focusing the keyboard`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    const { ctx, base } = await apiCtx();
    const workspace = await seedWorkspace(ctx, base, 'Touch gesture test');
    const session = await seedShellSession(ctx, base, workspace);
    await ctx.post(`${base}/api/v1/sessions/${session}/input`, { data: { text: 'for i in $(seq 1 250); do echo "SCROLL-LINE-$i"; done', submit: true } });
    await ctx.dispose();
    await page.addInitScript(id => {
      localStorage.setItem('otto_workspace', id);
      localStorage.setItem('otto_right_open', '0');
      (window as unknown as { __ottoTermProbe: Probe[] }).__ottoTermProbe = [];
    }, workspace);
    await page.setViewportSize({ width, height });
    await page.goto(`/#/agents/${session}`);
    const host = page.locator('.term-host');
    await expect(host).toBeVisible();
    await expect.poll(() => text(page)).toContain('SCROLL-LINE-250');
    await page.evaluate(() => (document.activeElement as HTMLElement)?.blur());
    const before = await text(page);
    const box = (await host.boundingBox())!;
    const client = await page.context().newCDPSession(page);
    const x = box.x + box.width * 0.5;
    const start = box.y + box.height * 0.2;
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y: start }] });
    for (let n = 1; n <= 8; n++) {
      await client.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x, y: start + box.height * 0.6 * n / 8 }] });
      await page.waitForTimeout(20); // ui-guards: allow — real gesture velocity needs elapsed touch frames
    }
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await expect.poll(() => text(page)).not.toBe(before);
    expect(await page.evaluate(() => document.activeElement?.classList.contains('xterm-helper-textarea'))).toBe(false);
    // A completed tap still focuses the keyboard; a swipe did not.
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y: start }] });
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await expect.poll(() => page.evaluate(() => document.activeElement?.classList.contains('xterm-helper-textarea'))).toBe(true);
    expect(errors).toEqual([]);
    await client.detach();
  });
}
