import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import AxeBuilder from '@axe-core/playwright';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });
test.setTimeout(60_000);
const skill = { name: 'grill', category: 'Review', version: 1, description: 'Synthetic review skill', installed_version: null, state: 'not_installed', update_available: false };
const themes = [
  { name: 'native-light', theme: 'native', scheme: 'light', dir: 'ltr', width: 1440 },
  { name: 'native-dark', theme: 'native', scheme: 'dark', dir: 'ltr', width: 375 },
  { name: 'warm-light', theme: 'warm', scheme: 'light', dir: 'rtl', width: 834 },
  { name: 'warm-dark', theme: 'warm', scheme: 'dark', dir: 'rtl', width: 375 },
  { name: 'pro-dark', theme: 'pro-dark', scheme: 'dark', dir: 'ltr', width: 1440 },
];
async function emptyWorkspace(page: Page) {
  await page.addInitScript(() => localStorage.removeItem('otto_firstrun_dismissed'));
  await page.route('**/api/v1/workspaces', (r) => r.fulfill({ json: [] }));
  await page.route('**/api/v1/workspaces/*/sessions', (r) => r.fulfill({ json: [] }));
  await page.route('**/api/v1/library/bundled', (r) => r.fulfill({ json: [skill] }));
}
for (const variant of themes) {
  test(`coach ${variant.name}: readable controls and dismiss persistence`, async ({ page }, info) => {
    await emptyWorkspace(page);
    await page.setViewportSize({ width: variant.width, height: 900 });
    await page.addInitScript((v) => {
      localStorage.setItem('otto_theme', v.theme);
      localStorage.setItem('otto_scheme', v.scheme);
      localStorage.setItem('otto_direction', v.dir);
    }, variant);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/#/agents');
    const coach = page.locator('.coach');
    await expect(coach.getByText('grill', { exact: true })).toBeVisible();
    const result = await new AxeBuilder({ page }).include('.coach').withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    await page.screenshot({ path: info.outputPath(`${variant.name}-coach.png`) });
    if (variant.width === 375) {
      for (const control of [coach.locator('.coach-close'), coach.locator('.coach-launch .btn')]) {
        expect((await control.boundingBox())!.height).toBeGreaterThanOrEqual(36);
      }
    }
    expect(result.violations).toEqual([]);
    await expectNoHorizontalOverflow(page);
    await expect(coach.getByRole('button', { name: 'Start your first agent' })).toBeDisabled();
    await coach.getByRole('button', { name: 'Dismiss the getting-started guide' }).click();
    await expect(coach).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem('otto_firstrun_dismissed'))).toBe('1');
  });
}

test('coach failed skill discovery has inline retry and skill installation recovers', async ({ page }, info) => {
  await emptyWorkspace(page);
  let discoveryFailed = true;
  let installFailed = true;
  let installed = false;
  await page.route('**/api/v1/library/bundled', (r) => discoveryFailed
    ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Skill catalog temporarily unavailable' } })
    : r.fulfill({ json: [{ ...skill, state: installed ? 'up_to_date' : 'not_installed' }] }));
  await page.route('**/api/v1/library/bundled/grill/install?backup=true', (r) => {
    expect(r.request().method()).toBe('POST');
    if (installFailed) return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic install failure' } });
    installed = true;
    return r.fulfill({ json: { name: 'grill', installed: true, backed_up: false, backup_path: null } });
  });
  await page.goto('/#/agents');
  const coach = page.locator('.coach');
  await expect(coach.getByText('Could not load recommended skills.')).toBeVisible();
  await coach.getByRole('button', { name: 'Retry skills' }).scrollIntoViewIfNeeded();
  await expect(coach.getByRole('button', { name: 'Retry skills' })).toBeInViewport();
  await page.screenshot({ path: info.outputPath('coach-skill-error.png'), animations: 'disabled' });
  discoveryFailed = false;
  await coach.getByRole('button', { name: 'Retry skills' }).click();
  await coach.getByRole('button', { name: 'Install', exact: true }).click();
  await expect(page.getByText('Synthetic install failure')).toBeVisible();
  await expect(coach.getByRole('button', { name: 'Install', exact: true })).toBeEnabled();
  installFailed = false;
  await coach.getByRole('button', { name: 'Install', exact: true }).click();
  await expect(coach.getByText('installed', { exact: true })).toBeVisible();
});

async function realMedia(page: Page) {
  const bytes = readFileSync(process.env.OTTO_E2E_TOUR_VIDEO!);
  await page.route('**/walkthroughs/resolve**', (r) => r.fulfill({ json: { url: `${new URL(page.url()).origin}/__r3tour.mp4` } }));
  await page.route('**/otto-tour-poster.jpg', (r) => r.abort());
  await page.route('**/__r3tour.mp4', (r) => {
    const range = r.request().headers().range?.match(/bytes=(\d+)-(\d*)/);
    const start = range ? Number(range[1]) : 0;
    const end = range?.[2] ? Number(range[2]) : bytes.length - 1;
    return r.fulfill({ status: range ? 206 : 200, contentType: 'video/mp4', headers: {
      'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1),
      ...(range ? { 'Content-Range': `bytes ${start}-${end}/${bytes.length}` } : {}),
    }, body: bytes.subarray(start, end + 1) });
  });
}

test('native caption changes stay in sync with the Help caption button', async ({ page }) => {
  await realMedia(page);
  await page.goto('/#/walkthroughs');
  const video = page.getByTestId('tour-film-video');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0]?.mode)).toBe('showing');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
  // Native video controls change the TextTrack directly, including in fullscreen.
  await video.evaluate((el: HTMLVideoElement) => { el.textTracks[0].mode = 'disabled'; });
  await expect(page.getByRole('button', { name: 'CC off', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await page.getByRole('button', { name: 'CC off', exact: true }).click();
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0]?.mode)).toBe('showing');
  await page.getByRole('button', { name: 'CC on', exact: true }).click();
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0].mode)).toBe('hidden');
  await video.evaluate((el) => el.dispatchEvent(new Event('error')));
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.readyState)).toBeGreaterThanOrEqual(1);
  await expect(page.getByRole('button', { name: 'CC off', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0].mode)).toBe('hidden');
});

test('Watch the tour respects reduced motion while revealing the player', async ({ page }) => {
  await realMedia(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/#/walkthroughs/keyboard-shortcuts');
  await page.getByTestId('guide-main').evaluate((el) => {
    el.scrollTop = el.scrollHeight;
    const original = el.scrollTo.bind(el);
    el.scrollTo = ((options: ScrollToOptions) => {
      el.dataset.requestedBehavior = options.behavior ?? 'auto';
      original(options);
    }) as typeof el.scrollTo;
  });
  await page.getByRole('button', { name: 'Watch the tour', exact: true }).click();
  await expect(page.getByTestId('guide-main')).not.toHaveAttribute('data-requested-behavior', 'smooth');
  await expect(page.getByTestId('tour-film-video')).toBeInViewport();
});

test('coach workspace and first-agent flow retries safely and sends the starter prompt', async ({ page }, info) => {
  await emptyWorkspace(page);
  await page.setViewportSize({ width: 375, height: 812 });
  const workspace = { id: '01ARZ3NDEKTSV4RRFFQ69G5FAV', name: 'Synthetic project', root_path: '/synthetic/project', settings: {}, archived: false, created_at: new Date().toISOString(), my_role: 'admin' };
  let workspaceFailed = true;
  let launchFailed = true;
  let created = false;
  const now = new Date().toISOString();
  const session = { id: '01ARZ3NDEKTSV4RRFFQ69G5FAW', workspace_id: workspace.id, kind: 'agent', provider: 'claude', title: 'First session', status: 'exited', cwd: workspace.root_path, provider_session_id: null, connection_id: null, created_by: 'synthetic-root', created_at: now, last_active_at: now, archived: false, meta: { source: 'onboarding' } };
  const launches: Record<string, unknown>[] = [];
  const inputs: Record<string, unknown>[] = [];
  await page.route('**/api/v1/workspaces', (r) => {
    if (r.request().method() !== 'POST') return r.fulfill({ json: created ? [workspace] : [] });
    expect(r.request().postDataJSON()).toEqual({ name: 'Synthetic project', root_path: '/synthetic/project' });
    if (workspaceFailed) return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic workspace failure' } });
    created = true;
    return r.fulfill({ json: workspace });
  });
  await page.route(`**/api/v1/workspaces/${workspace.id}/sessions**`, (r) => {
    if (r.request().method() !== 'POST') return r.fulfill({ json: [] });
    launches.push(r.request().postDataJSON());
    return launchFailed ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic launch failure' } }) : r.fulfill({ json: session });
  });
  await page.route(`**/api/v1/sessions/${session.id}/input`, (r) => { inputs.push(r.request().postDataJSON()); return r.fulfill({ json: {} }); });
  await page.route(`**/api/v1/sessions/${session.id}`, (r) => r.fulfill({ json: session }));
  const sessionLoad = page.waitForResponse((response) => /\/workspaces\/[^/]+\/sessions$/.test(new URL(response.url()).pathname));
  await page.goto('/#/agents');
  await sessionLoad;
  await page.evaluate(() => new Promise(requestAnimationFrame));
  const coach = page.locator('.coach');
  await expect(coach.getByText('grill', { exact: true })).toBeVisible();
  await coach.getByRole('textbox', { name: 'Workspace name', exact: true }).fill('Synthetic project');
  await expect(coach.getByRole('textbox', { name: 'Workspace name', exact: true })).toHaveValue('Synthetic project');
  await coach.getByRole('textbox', { name: 'Workspace folder', exact: true }).fill('/synthetic/project');
  await expect(coach.getByRole('textbox', { name: 'Workspace name', exact: true })).toHaveValue('Synthetic project');
  await coach.getByRole('button', { name: 'Create workspace', exact: true }).click();
  await expect(page.getByText('Synthetic workspace failure')).toBeVisible();
  await expect(coach.getByRole('textbox', { name: 'Workspace name', exact: true })).toHaveValue('Synthetic project');
  workspaceFailed = false;
  await coach.getByRole('button', { name: 'Create workspace', exact: true }).click();
  const launch = coach.getByRole('button', { name: 'Start your first agent', exact: true });
  await expect(launch).toBeEnabled();
  await launch.click();
  await expect(page.getByText('Synthetic launch failure')).toBeVisible();
  await expect(launch).toBeEnabled();
  launchFailed = false;
  await launch.click();
  await expect(coach).toHaveCount(0);
  await expect.poll(() => inputs.length).toBe(1);
  expect(launches).toHaveLength(2);
  expect(launches[1]).toMatchObject({ kind: 'agent', title: 'First session', cwd: '/synthetic/project', meta: { source: 'onboarding' } });
  expect(inputs[0]).toMatchObject({ submit: true });
  expect(inputs[0].text).toContain('summarise what this project is');
  expect(await page.evaluate(() => localStorage.getItem('otto_firstrun_dismissed'))).toBe('1');
  await page.screenshot({ path: info.outputPath('first-agent-opened.png') });
});

test('queued chapter requests use the latest selection and survive fullscreen return', async ({ page, browserName }, info) => {
  await realMedia(page);
  let release!: () => void;
  const held = new Promise<void>((resolve) => { release = resolve; });
  await page.route('**/walkthroughs/resolve**', async (r) => {
    await held;
    await r.fulfill({ json: { url: `${new URL(page.url()).origin}/__r3tour.mp4` } });
  });
  await page.goto('/#/walkthroughs');
  await expect(page.getByText('Loading the tour…')).toBeVisible();
  await page.getByRole('button', { name: '1:30 Git, reviews and proof' }).click();
  await page.getByRole('button', { name: '2:37 Insights and Usage' }).click();
  release();
  const video = page.getByTestId('tour-film-video');
  await expect.poll(() => video.evaluate((el: HTMLVideoElement) => !el.seeking && el.currentTime > 157 && !el.paused)).toBe(true);
  await expect(page.getByRole('button', { name: '2:37 Insights and Usage' })).toHaveAttribute('aria-current', 'step');
  if (browserName === 'chromium') {
    await page.getByRole('button', { name: 'CC on', exact: true }).evaluate((button) => {
      button.addEventListener('click', () => { void document.querySelector('video')!.requestFullscreen(); }, { once: true });
    });
    await page.getByRole('button', { name: 'CC on', exact: true }).click();
    await expect.poll(() => page.evaluate(() => document.fullscreenElement?.tagName)).toBe('VIDEO');
    await page.screenshot({ path: info.outputPath('tour-fullscreen.png') });
    await page.evaluate(() => document.exitFullscreen());
    await expect.poll(() => page.evaluate(() => document.fullscreenElement === null)).toBe(true);
    await expect(page.getByRole('button', { name: 'CC off', exact: true })).toBeVisible();
    await expect.poll(() => video.evaluate((el: HTMLVideoElement) => el.textTracks[0].mode)).toBe('hidden');
  }
  await page.locator('.chapter').filter({ hasText: 'Insights and Usage' }).getByRole('button', { name: 'Read the guide' }).click();
  await expect(page.getByTestId('tour-film-video')).toHaveCount(0);
  await expect(page.getByTestId('guide-article')).toHaveAttribute('data-guide-id', 'insights');
});

test('RTL shortcut sheet keeps modifier-first chords and keyboard dismissal', async ({ page }, info) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.addInitScript(() => {
    localStorage.setItem('otto_theme', 'warm');
    localStorage.setItem('otto_scheme', 'dark');
    localStorage.setItem('otto_direction', 'rtl');
  });
  await page.goto('/#/walkthroughs/keyboard-shortcuts');
  await expect(page.getByTestId('guide-article')).toBeVisible();
  await page.keyboard.press('?');
  const dialog = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(dialog).toBeVisible();
  const chord = dialog.locator('.sc-row').filter({ hasText: 'Floating bar — commands' }).locator('.sc-keys');
  const keys = await chord.locator('kbd').evaluateAll((els) => els.map((el) => ({ text: el.textContent, x: el.getBoundingClientRect().x })));
  await page.screenshot({ path: info.outputPath('shortcut-sheet-warm-dark-rtl.png'), animations: 'disabled' });
  expect(keys.map((key) => key.text)).toEqual(['⌘', 'K']);
  expect(keys[0].x).toBeLessThan(keys[1].x);
  const axe = await new AxeBuilder({ page }).include('[role="dialog"]').withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  expect(axe.violations).toEqual([]);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
});

test('coach unavailable provider recheck enables launch only after detection', async ({ page }) => {
  await emptyWorkspace(page);
  let found = false;
  await page.route('**/api/v1/meta', async (r) => {
    const response = await r.fetch();
    const meta = await response.json();
    await r.fulfill({ json: { ...meta, tools: meta.tools.map((tool: { name: string }) => ['claude', 'codex', 'agy'].includes(tool.name) ? { name: tool.name, found, version: found ? 'Synthetic CLI 1.0' : null } : tool) } });
  });
  await page.goto('/#/agents');
  const coach = page.locator('.coach');
  await expect(coach.getByText('No coding-agent CLI found on your')).toBeVisible();
  await expect(coach.getByRole('button', { name: 'Start your first agent' })).toBeDisabled();
  found = true;
  await coach.getByRole('button', { name: 'Re-check', exact: true }).click();
  await expect(coach.getByText('claude Synthetic CLI 1.0', { exact: false })).toBeVisible();
  // Detection is only one prerequisite: no workspace still keeps launch disabled.
  await expect(coach.getByRole('button', { name: 'Start your first agent' })).toBeDisabled();
});

test('slow initial workspace discovery never discards an accepted coach draft', async ({ page }) => {
  await emptyWorkspace(page);
  let release!: () => void;
  const held = new Promise<void>((resolve) => { release = resolve; });
  let requested!: () => void;
  const started = new Promise<void>((resolve) => { requested = resolve; });
  await page.route('**/api/v1/workspaces', async (r) => { requested(); await held; await r.fulfill({ json: [] }); });
  const sessionsLoaded = page.waitForResponse((r) => /\/workspaces\/[^/]+\/sessions$/.test(new URL(r.url()).pathname));
  await page.goto('/#/agents');
  await started;
  await expect(page.locator('.agents')).toBeVisible();
  const coach = page.locator('.coach');
  const acceptedEarlyInput = await coach.isVisible();
  // Either keep the initial view loading, or preserve any draft it accepts.
  if (acceptedEarlyInput) {
    await coach.getByRole('textbox', { name: 'Workspace folder', exact: true }).fill('/synthetic/draft');
    await coach.getByRole('textbox', { name: 'Workspace name', exact: true }).fill('My deliberate draft');
    await expect(coach.getByRole('textbox', { name: 'Workspace name', exact: true })).toHaveValue('My deliberate draft');
  }
  release();
  await sessionsLoaded;
  await page.evaluate(() => new Promise(requestAnimationFrame));
  await expect(coach).toBeVisible();
  if (acceptedEarlyInput) {
    await expect(coach.getByRole('textbox', { name: 'Workspace name', exact: true })).toHaveValue('My deliberate draft');
    await expect(coach.getByRole('textbox', { name: 'Workspace folder', exact: true })).toHaveValue('/synthetic/draft');
  } else {
    await coach.getByRole('textbox', { name: 'Workspace name', exact: true }).fill('My deliberate draft');
    await coach.getByRole('textbox', { name: 'Workspace folder', exact: true }).fill('/synthetic/draft');
    await expect(coach.getByRole('textbox', { name: 'Workspace name', exact: true })).toHaveValue('My deliberate draft');
  }
});
