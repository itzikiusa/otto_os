import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// Optional artifact: static UX checks never depend on a generated video.
test.setTimeout(90_000);
test.use({ serviceWorkers: 'block' });
const videoPath = process.env.OTTO_E2E_TOUR_VIDEO;
let wsId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});
test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), wsId);
  await page.route('**/walkthroughs/resolve**', (r) => r.fulfill({ json: { url: `${new URL(page.url()).origin}/__tour.mp4` } }));
  await page.route('**/__tour.mp4', (r) => r.abort());
});
async function openHelp(page: Page, id = '') {
  await page.goto(`/#/walkthroughs${id ? `/${id}` : ''}`, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('page-header').locator('h1')).toHaveText('Help');
}

test('Getting started opens from the list and the first chapter', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openHelp(page);
  await page.locator('[data-guide="getting-started"]').click();
  await expect(page.getByTestId('guide-article')).toHaveAttribute('data-guide-id', 'getting-started');
  await page.getByRole('button', { name: 'Back to all guides' }).click();
  await page.locator('.chapter').first().getByRole('button', { name: 'Read the guide' }).click();
  await expect(page.getByTestId('guide-article')).toHaveAttribute('data-guide-id', 'getting-started');
  await expectNoHorizontalOverflow(page);
});

test('search, no results, keyboard selection and unknown guide recovery', async ({ page }) => {
  await openHelp(page);
  await page.getByTestId('guide-search').fill('zzzznothing');
  await expect(page.getByText('No guides match')).toBeVisible();
  await page.getByRole('button', { name: 'Clear search' }).click();
  await page.getByTestId('guide-search').fill('cmd k');
  await expect(page.locator('[data-guide="command-bar"]')).toBeVisible();
  await page.getByTestId('guide-search').press('Enter');
  await expect(page.getByTestId('guide-article')).toBeVisible();
  await openHelp(page, 'missing-guide');
  await page.getByRole('button', { name: 'Open Getting started' }).click();
  await expect(page.getByTestId('guide-article')).toHaveAttribute('data-guide-id', 'getting-started');
});

test('phone tour controls do not trigger guide-list keyboard navigation', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openHelp(page);
  const video = page.locator('.chapter-play').first();
  await video.focus();
  const canceled = await video.evaluate((el) => !el.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })));
  expect(canceled).toBe(false);
  await expect(video).toBeFocused();
});

for (const scheme of ['light', 'dark'] as const) {
  test(`phone RTL ${scheme}: failed tour remains readable and retryable`, async ({ page }, info) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.addInitScript((value) => {
      localStorage.setItem('otto_scheme', value);
      localStorage.setItem('otto_direction', 'rtl');
    }, scheme);
    await openHelp(page);
    const fallback = page.getByTestId('tour-film-unavailable');
    await expect(fallback).toBeVisible();
    await fallback.getByRole('button', { name: 'Retry', exact: true }).click();
    await expect(fallback).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: `/tmp/otto-ux-help-${info.project.name}-${scheme}-rtl.png` });
    await page.getByTestId('guide-search').fill('git');
    await page.locator('[data-guide="git"]').click();
    await expect(page.getByTestId('guide-article')).toHaveAttribute('data-guide-id', 'git');
    await page.getByTestId('guide-open-module').click();
    await expect(page).toHaveURL(/#\/git$/);
  });
}

test('real instrumental tour: playback, audio, seek, captions, guide and retry', async ({ page }, info) => {
  test.skip(!videoPath, 'Set OTTO_E2E_TOUR_VIDEO to verify the generated media artifact');
  test.setTimeout(90_000);
  const bytes = readFileSync(videoPath!);
  await page.unroute('**/__tour.mp4');
  await page.route('**/__tour.mp4', async (route) => {
    const range = route.request().headers().range?.match(/bytes=(\d+)-(\d*)/);
    const start = range ? Number(range[1]) : 0;
    const end = range?.[2] ? Number(range[2]) : bytes.length - 1;
    await route.fulfill({ status: range ? 206 : 200, contentType: 'video/mp4', headers: {
      'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1),
      ...(range ? { 'Content-Range': `bytes ${start}-${end}/${bytes.length}` } : {}),
    }, body: bytes.subarray(start, end + 1) });
  });
  await openHelp(page);
  let video = page.getByTestId('tour-film-video');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
  expect(await video.evaluate((el: HTMLVideoElement) => el.paused)).toBe(true);
  await page.locator('.chapter-play').first().click();
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.currentTime)).toBeGreaterThan(0.5);
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0]?.cues?.length ?? 0)).toBeGreaterThan(0);
  expect(await video.evaluate((el: HTMLVideoElement) => !el.muted && el.volume > 0)).toBe(true);
  const audio = await video.evaluate((el) => {
    const media = el as HTMLVideoElement & { audioTracks?: { length: number }; webkitAudioDecodedByteCount?: number };
    return { tracks: media.audioTracks?.length, decoded: media.webkitAudioDecodedByteCount };
  });
  if (audio.tracks !== undefined) expect(audio.tracks).toBeGreaterThan(0);
  else expect(audio.decoded).toBeGreaterThan(0);
  await page.getByRole('button', { name: 'CC on', exact: true }).click();
  expect(await video.evaluate((el: HTMLVideoElement) => el.textTracks[0].mode)).toBe('hidden');
  await page.getByRole('button', { name: 'CC off', exact: true }).click();
  expect(await video.evaluate((el: HTMLVideoElement) => el.textTracks[0].mode)).toBe('showing');
  await page.getByRole('button', { name: '1:30 Git, reviews and proof' }).click();
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.currentTime)).toBeGreaterThanOrEqual(90.9);
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0].activeCues?.length ?? 0)).toBeGreaterThan(0);
  await page.screenshot({ path: `/tmp/otto-ux-help-${info.project.name}-playing.png` });
  await page.locator('.chapter').filter({ hasText: 'Git, reviews and proof' }).getByRole('button', { name: 'Read the guide' }).click();
  await expect(page.getByTestId('guide-article')).toHaveAttribute('data-guide-id', 'git');
  await page.getByTestId('guide-watch-part').click();
  video = page.getByTestId('tour-film-video');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.currentTime)).toBeGreaterThanOrEqual(90.9);
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.paused)).toBe(false);
  await video.evaluate((el) => el.dispatchEvent(new Event('error')));
  await page.getByTestId('tour-film-unavailable').getByRole('button', { name: 'Retry' }).click();
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
  expect(await video.evaluate((el: HTMLVideoElement) => el.paused)).toBe(true);
  await expectNoHorizontalOverflow(page);
});
