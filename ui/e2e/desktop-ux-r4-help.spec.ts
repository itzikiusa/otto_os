import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });
test.setTimeout(120_000);

async function emptyCoach(page: Page) {
  await page.addInitScript(() => localStorage.removeItem('otto_firstrun_dismissed'));
  await page.route('**/api/v1/workspaces', (r) => r.fulfill({ json: [] }));
  await page.route('**/api/v1/workspaces/*/sessions', (r) => r.fulfill({ json: [] }));
  await page.route('**/api/v1/library/bundled', (r) => r.fulfill({ json: [] }));
}

test('provider recheck announces pending and failed detection, then recovers without duplicate requests', async ({ page }, info) => {
  await emptyCoach(page);
  await page.setViewportSize({ width: 375, height: 812 });
  let phase: 'initial' | 'fail' | 'found' = 'initial';
  let requests = 0;
  let release!: () => void;
  const held = new Promise<void>((resolve) => { release = resolve; });
  await page.route('**/api/v1/meta', async (r) => {
    if (phase === 'fail') {
      requests++;
      await held;
      return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic provider discovery unavailable' } });
    }
    const response = await r.fetch();
    const meta = await response.json();
    await r.fulfill({ json: { ...meta, tools: meta.tools.map((tool: { name: string }) => ['claude', 'codex', 'agy'].includes(tool.name) ? { ...tool, found: phase === 'found', version: 'Synthetic 1.0' } : tool) } });
  });
  await page.goto('/#/agents', { waitUntil: 'domcontentloaded' });
  const coach = page.locator('.coach');
  const recheck = coach.getByRole('button', { name: 'Re-check', exact: true });
  await expect(recheck).toBeVisible({ timeout: 60_000 });
  phase = 'fail';
  await recheck.focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => requests).toBe(1);
  // Release even on a red assertion, so fixture teardown never leaves a pending request.
  try { await expect.soft(coach.getByRole('button', { name: 'Checking…', exact: true })).toBeDisabled(); }
  finally { release(); }
  await expect(coach.getByText('Could not check agent CLIs. Try again.')).toBeVisible();
  await expect(recheck).toBeEnabled();
  expect(requests).toBe(1);
  await page.screenshot({ path: info.outputPath('provider-recheck-error-phone.png'), animations: 'disabled' });
  phase = 'found';
  await recheck.click();
  await expect(coach.getByText('claude Synthetic 1.0')).toBeVisible();
  await expect(coach.getByText('Could not check agent CLIs. Try again.')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
});

test('folder picker keeps a deliberate workspace name through error retry and keyboard selection', async ({ page }, info) => {
  await emptyCoach(page);
  await page.setViewportSize({ width: 375, height: 812 });
  await page.addInitScript(() => { localStorage.setItem('otto_theme', 'warm'); localStorage.setItem('otto_scheme', 'dark'); localStorage.setItem('otto_direction', 'rtl'); });
  let fail = true;
  await page.route('**/api/v1/fs/browse**', (r) => fail
    ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic folder unavailable' } })
    : r.fulfill({ json: { path: '/synthetic/chosen-directory', parent: '/synthetic', entries: [], is_git_repo: false } }));
  await page.goto('/#/agents', { waitUntil: 'domcontentloaded' });
  const coach = page.locator('.coach');
  const name = coach.getByRole('textbox', { name: 'Workspace name', exact: true });
  await name.fill('Deliberate project title');
  const browse = coach.getByRole('button', { name: 'Browse…', exact: true });
  await browse.click();
  const dialog = page.getByRole('dialog', { name: 'Choose project directory' });
  await expect(dialog.getByText('Synthetic folder unavailable')).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Use this folder', exact: true })).toBeDisabled();
  fail = false;
  await dialog.getByRole('button', { name: 'Retry', exact: true }).click();
  const pick = dialog.getByRole('button', { name: 'Use this folder', exact: true });
  await expect(pick).toBeEnabled();
  await pick.focus();
  await page.keyboard.press('Enter');
  await expect(dialog).toHaveCount(0);
  await expect(name).toHaveValue('Deliberate project title');
  await expect(coach.getByRole('textbox', { name: 'Workspace folder', exact: true })).toHaveValue('/synthetic/chosen-directory');
  await expect(browse).toBeFocused();
  await page.screenshot({ path: info.outputPath('chosen-folder-preserved-name-rtl.png'), animations: 'disabled' });
});

async function realMedia(page: Page) {
  const bytes = readFileSync(process.env.OTTO_E2E_TOUR_VIDEO!);
  await page.route('**/walkthroughs/resolve**', (r) => r.fulfill({ json: { url: `${new URL(page.url()).origin}/__r4tour.mp4` } }));
  await page.route('**/otto-tour-poster.jpg', (r) => r.abort());
  await page.route('**/__r4tour.mp4', (r) => {
    const range = r.request().headers().range?.match(/bytes=(\d+)-(\d*)/);
    const start = range ? Number(range[1]) : 0;
    const end = range?.[2] ? Number(range[2]) : bytes.length - 1;
    return r.fulfill({ status: range ? 206 : 200, contentType: 'video/mp4', headers: {
      'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1),
      ...(range ? { 'Content-Range': `bytes ${start}-${end}/${bytes.length}` } : {}),
    }, body: bytes.subarray(start, end + 1) });
  });
}

test('tour pause, end and keyboard chapter replay preserve caption preference', async ({ page }, info) => {
  await realMedia(page);
  await page.goto('/#/walkthroughs', { waitUntil: 'domcontentloaded' });
  const video = page.getByTestId('tour-film-video');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
  const chapter = page.getByRole('button', { name: '2:45 Everywhere', exact: true });
  await chapter.focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => !el.paused && el.currentTime >= 165.2)).toBe(true);
  await video.evaluate((el: HTMLVideoElement) => el.pause());
  const pausedAt = await video.evaluate((el: HTMLVideoElement) => el.currentTime);
  await page.getByRole('button', { name: 'CC on', exact: true }).click();
  expect(await video.evaluate((el: HTMLVideoElement) => el.paused)).toBe(true);
  expect(await video.evaluate((el: HTMLVideoElement) => el.currentTime)).toBeCloseTo(pausedAt, 1);
  await video.evaluate((el: HTMLVideoElement) => { el.currentTime = el.duration - 0.3; return el.play(); });
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.ended)).toBe(true);
  expect(await video.evaluate((el: HTMLVideoElement) => el.paused)).toBe(true);
  const first = page.getByRole('button', { name: '0:00 Meet Otto', exact: true });
  await first.focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => !el.ended && !el.paused && el.currentTime < 7.5)).toBe(true);
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0]?.mode)).toBe('hidden');
  await page.screenshot({ path: info.outputPath('tour-replayed.png'), animations: 'disabled' });
});

test('phone tour recovery and guide search have reachable touch targets', async ({ page }, info) => {
  await realMedia(page);
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto('/#/walkthroughs', { waitUntil: 'domcontentloaded' });
  const video = page.getByTestId('tour-film-video');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
  await video.evaluate((el) => el.dispatchEvent(new Event('error')));
  const error = page.getByTestId('tour-film-unavailable');
  await expect(error).toBeVisible();
  for (const control of [error.getByRole('button', { name: 'Retry', exact: true }), error.getByRole('link', { name: 'Open in browser' }), page.getByTestId('guide-search')]) {
    expect.soft((await control.boundingBox())!.height).toBeGreaterThanOrEqual(36);
  }
  await page.screenshot({ path: info.outputPath('tour-recovery-touch-targets.png'), animations: 'disabled' });
});
