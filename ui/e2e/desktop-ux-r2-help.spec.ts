import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import AxeBuilder from '@axe-core/playwright';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });
test.setTimeout(90_000);
const videoPath = process.env.OTTO_E2E_TOUR_VIDEO;
let workspace = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
});
test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), workspace);
  await page.route('**/walkthroughs/resolve**', (route) => route.fulfill({ json: { url: `${new URL(page.url()).origin}/__r2tour.mp4` } }));
  // Keep screenshots independent of release-host availability.
  await page.route('**/otto-tour-poster.jpg', (route) => route.abort());
});
async function media(page: Page) {
  test.skip(!videoPath, 'Set OTTO_E2E_TOUR_VIDEO to test actual playback');
  const bytes = readFileSync(videoPath!);
  await page.route('**/__r2tour.mp4', async (route) => {
    const range = route.request().headers().range?.match(/bytes=(\d+)-(\d*)/);
    const start = range ? Number(range[1]) : 0;
    const end = range?.[2] ? Number(range[2]) : bytes.length - 1;
    await route.fulfill({ status: range ? 206 : 200, contentType: 'video/mp4', headers: {
      'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1),
      ...(range ? { 'Content-Range': `bytes ${start}-${end}/${bytes.length}` } : {}),
    }, body: bytes.subarray(start, end + 1) });
  });
}

test('chapter activation reveals the player; passive chapter changes preserve reading position', async ({ page }, info) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await media(page);
  await page.goto('/#/walkthroughs');
  const video = page.getByTestId('tour-film-video');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
  const chapter = page.getByRole('button', { name: '2:37 Insights and Usage', exact: true });
  await chapter.scrollIntoViewIfNeeded();
  expect((await video.boundingBox())!.y).toBeLessThan(0);
  await chapter.click();
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => !el.seeking && el.currentTime > 158)).toBe(true);
  await expectFullyInViewport(page, video);
  await page.screenshot({ path: info.outputPath('chapter-revealed.png') });
  await chapter.scrollIntoViewIfNeeded();
  const rail = page.locator('.help-layout .rail');
  const before = await rail.evaluate((el) => el.scrollTop);
  await video.evaluate((el: HTMLVideoElement) => { el.pause(); el.currentTime = 166; el.dispatchEvent(new Event('timeupdate')); });
  await expect(page.getByRole('button', { name: '2:45 Everywhere', exact: true })).toHaveAttribute('aria-current', 'step');
  expect(await rail.evaluate((el) => el.scrollTop)).toBe(before);
  await chapter.focus();
  await chapter.press('Enter');
  await expectFullyInViewport(page, video);
});

const variants = [
  { name: 'native-light-desktop', theme: 'native', scheme: 'light', width: 1440, height: 900, dir: 'ltr' },
  { name: 'native-dark-phone', theme: 'native', scheme: 'dark', width: 375, height: 812, dir: 'ltr' },
  { name: 'warm-light-tablet-rtl', theme: 'warm', scheme: 'light', width: 834, height: 1194, dir: 'rtl' },
  { name: 'warm-dark-phone-rtl', theme: 'warm', scheme: 'dark', width: 375, height: 812, dir: 'rtl' },
  { name: 'pro-dark-desktop', theme: 'pro-dark', scheme: 'dark', width: 1440, height: 900, dir: 'ltr' },
] as const;
for (const variant of variants) {
  test(`${variant.name}: loaded tour, captions, keyboard guide search and readable long guide`, async ({ page }, info) => {
    await page.setViewportSize({ width: variant.width, height: variant.height });
    await page.addInitScript((v) => {
      localStorage.setItem('otto_theme', v.theme);
      localStorage.setItem('otto_scheme', v.scheme);
      localStorage.setItem('otto_direction', v.dir);
    }, variant);
    await media(page);
    await page.goto('/#/walkthroughs');
    const video = page.getByTestId('tour-film-video');
    await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
    expect(await video.evaluate((el: HTMLVideoElement) => el.paused)).toBe(true);
    await page.locator('.chapter-play').first().click();
    await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.currentTime)).toBeGreaterThan(0.25);
    await page.getByRole('button', { name: 'CC on', exact: true }).click();
    await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0]?.mode)).toBe('hidden');
    await page.getByRole('button', { name: 'CC off', exact: true }).press('Enter');
    await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0]?.mode)).toBe('showing');
    await video.evaluate((el: HTMLVideoElement) => el.pause());
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`${variant.name}-film.png`) });
    await video.evaluate((el) => el.dispatchEvent(new Event('error')));
    await expect(page.getByTestId('tour-film-unavailable')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`${variant.name}-error.png`) });
    await page.getByRole('button', { name: 'Retry', exact: true }).click();
    await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
    expect(await video.evaluate((el: HTMLVideoElement) => el.paused)).toBe(true);
    const search = page.getByTestId('guide-search');
    await search.fill('keyboard shortcuts');
    await search.press('ArrowDown');
    await expect(page.locator('[data-guide="keyboard-shortcuts"]')).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page.getByTestId('guide-article')).toHaveAttribute('data-guide-id', 'keyboard-shortcuts');
    await expect(page.locator('.guide-body table').first()).toBeVisible();
    await expectNoHorizontalOverflow(page);
    const axe = await new AxeBuilder({ page }).include('.help-page').withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    expect(axe.violations).toEqual([]);
    await page.screenshot({ path: info.outputPath(`${variant.name}-guide.png`) });
  });
}

test('pending chapter selection survives media recovery and ordinary Retry never autoplays', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.route('**/__r2tour.mp4', (route) => route.abort());
  await page.goto('/#/walkthroughs');
  await expect(page.getByTestId('tour-film-unavailable')).toBeVisible();
  await page.unroute('**/__r2tour.mp4');
  await media(page);
  await page.getByRole('button', { name: '2:37 Insights and Usage', exact: true }).click();
  const video = page.getByTestId('tour-film-video');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => !el.seeking && el.currentTime > 158)).toBe(true);
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.paused)).toBe(false);
  await expectFullyInViewport(page, video);
  await video.evaluate((el) => el.dispatchEvent(new Event('error')));
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
  expect(await video.evaluate((el: HTMLVideoElement) => el.paused)).toBe(true);
});

test('RTL keyboard chords preserve modifier-first reading order', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.addInitScript(() => localStorage.setItem('otto_direction', 'rtl'));
  await page.goto('/#/walkthroughs/keyboard-shortcuts');
  const chord = page.locator('.guide-body .keys').filter({ hasText: '⌘K' }).first();
  await expect(chord).toBeVisible();
  const keys = await chord.locator('kbd').evaluateAll((els) => els.map((el) => ({ text: el.textContent, x: el.getBoundingClientRect().x })));
  expect(keys.map((key) => key.text)).toEqual(['⌘', 'K']);
  expect(keys[0].x).toBeLessThan(keys[1].x);
});

test('long chapter names remain readable in desktop columns', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.route('**/__r2tour.mp4', (route) => route.abort());
  await page.goto('/#/walkthroughs');
  const title = page.locator('.chapter-title').filter({ hasText: 'Run with Otto and Mission Control' });
  await expect(title).toBeVisible();
  expect(await title.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
});
