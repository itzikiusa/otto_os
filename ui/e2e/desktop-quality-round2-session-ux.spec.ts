import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { rmSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { withUsage, writeChatTranscript } from './chat-fixture';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });
let ctx: APIRequestContext, base: string, workspace: string, session: string;
let fixture: ReturnType<typeof writeChatTranscript>;

test.beforeEach(async ({ page }) => {
  ({ ctx, base } = await apiCtx());
  workspace = await seedWorkspace(ctx, base);
  fixture = writeChatTranscript();
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Round 2 conversation', cwd: fixture.root,
      meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: fixture.path } },
  });
  expect(response.ok()).toBe(true);
  session = (await response.json()).id;
  await withUsage(page);
  await page.addInitScript(({ workspace, session }) => {
    localStorage.setItem('otto_workspace', workspace);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem(`otto_session_view:${session}`, 'chat');
  }, { workspace, session });
});
test.afterEach(async () => {
  await ctx?.dispose();
  if (fixture) rmSync(fixture.dir, { recursive: true, force: true });
});

async function openChat(page: Page) {
  await page.goto(`/#/agents/${session}`);
  await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible();
  await expect(page.locator('.composer textarea')).toBeVisible();
}

for (const outcome of ['success', 'failure'] as const) {
  test(`late send ${outcome} preserves conversation search focus and newer draft`, async ({ page }) => {
    let release!: () => void;
    const held = new Promise<void>(resolve => { release = resolve; });
    let started = false;
    await page.route(`**/sessions/${session}/input`, async route => {
      started = true;
      await held;
      await route.fulfill(outcome === 'success' ? { status: 204 } : {
        status: 503, json: { code: 'unavailable', message: 'Fixture send unavailable' },
      });
    });
    try {
      await openChat(page);
      const composer = page.locator('.composer textarea');
      await composer.fill('Please review the retry fix');
      await composer.press('Enter');
      await expect.poll(() => started).toBe(true);
      await composer.fill('Keep this next draft');
      await composer.press('Meta+f');
      const search = page.locator('.conv .search-in');
      await expect(search).toBeFocused();
      await search.fill('retry');
      const finished = page.waitForResponse(`**/sessions/${session}/input`);
      release();
      await finished;
      await expect(page.locator('.composer .send')).toBeEnabled();
      await expect(search).toBeFocused();
      await page.keyboard.type(' jitter');
      await expect(search).toHaveValue('retry jitter');
      await expect(composer).toHaveValue('Keep this next draft');
    } finally { release(); }
  });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`conversation load Retry and keyboard navigation preserve unsent draft ${scheme}`, async ({ page }, info) => {
    await page.addInitScript(scheme => {
      localStorage.setItem('otto_scheme', scheme);
      localStorage.setItem('otto_theme', 'native');
    }, scheme);
    await page.setViewportSize({ width: 1440, height: 900 });
    let failLoad = true;
    await page.route(`**/sessions/${session}/transcript?*`, route => failLoad
      ? route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Conversation is temporarily unavailable' } })
      : route.continue());
    await page.goto(`/#/agents/${session}`);
    await expect(page.getByText('Couldn’t load the conversation', { exact: true })).toBeVisible();
    const composer = page.locator('.composer textarea');
    await composer.fill('Unsent draft survives load recovery\nKeep the second line');
    await page.screenshot({ path: info.outputPath('conversation-error-desktop.png') });
    failLoad = false;
    await page.locator('.conv').getByRole('button', { name: 'Retry', exact: true }).press('Enter');
    await expect(page.locator('.conv .turn[data-role="assistant"]').first()).toBeVisible();
    await expect(composer).toHaveValue('Unsent draft survives load recovery\nKeep the second line');
    await composer.press('Meta+f');
    await expect(page.getByRole('search', { name: 'Find in page', exact: true })).toHaveCount(0);
    await page.locator('.conv .search-in').fill('retry');
    await expect(page.locator('.conv .search-n')).not.toHaveAttribute('data-search-hits', '0');
    await page.locator('.conv .search-in').press('Escape');
    await expect(page.locator('.conv .search-in')).toHaveCount(0);
    await expect(composer).toBeFocused();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath('conversation-populated-desktop.png') });
    await page.locator('.view-seg button', { hasText: 'Terminal' }).press('Enter');
    await expect(page.locator('.conv')).toHaveCount(0);
    await page.locator('.view-seg button', { hasText: 'Chat' }).press('Enter');
    await expect(composer).toHaveValue('Unsent draft survives load recovery\nKeep the second line');
    await page.setViewportSize({ width: 390, height: 844 });
    await expectFullyInViewport(page, composer);
    await expectFullyInViewport(page, page.locator('.composer .send'));
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath('conversation-draft-phone.png') });
    await page.reload();
    await expect(composer).toHaveValue('Unsent draft survives load recovery\nKeep the second line');
  });
}

test('late attachment upload preserves search focus and keeps the attachment with its draft', async ({ page }) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let started = false;
  await page.route(`**/sessions/${session}/inbox`, async route => {
    started = true;
    await held;
    await route.fulfill({ json: { path: '/tmp/round2-fixture-image.png' } });
  });
  try {
    await openChat(page);
    const composer = page.locator('.composer textarea');
    await composer.fill('Inspect this attachment');
    await page.locator('.composer input[type="file"]').setInputFiles({
      name: 'fixture.png', mimeType: 'image/png',
      buffer: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64'),
    });
    await expect.poll(() => started).toBe(true);
    await expect(page.locator('.composer .send')).toBeDisabled();
    await composer.press('Meta+f');
    const search = page.locator('.conv .search-in');
    await search.fill('retry');
    release();
    await expect(page.locator('.composer .thumb')).toHaveCount(1);
    await expect(page.locator('.composer .send')).toBeEnabled();
    await expect(search).toBeFocused();
    await expect(composer).toHaveValue('Inspect this attachment');
    await search.press('Escape');
    await page.locator('.composer').getByRole('button', { name: 'Remove fixture.png' }).press('Enter');
    await expect(page.locator('.composer .thumb')).toHaveCount(0);
    await expect(composer).toHaveValue('Inspect this attachment');
  } finally { release(); }
});

test('successful send returns keyboard focus when the composer still owns it', async ({ page }) => {
  await page.route(`**/sessions/${session}/input`, route => route.fulfill({ status: 204 }));
  await openChat(page);
  const composer = page.locator('.composer textarea');
  await composer.fill('A completed prompt');
  await page.locator('.composer .send').press('Enter');
  await expect(composer).toHaveValue('');
  await expect(composer).toBeFocused();
});

test('conversation search releases ownership when focus moves to the sidebar', async ({ page }) => {
  await openChat(page);
  const composer = page.locator('.composer textarea');
  await composer.fill('Keep this unsent prompt');
  await composer.press('Meta+f');
  const search = page.locator('.conv .search-in');
  await expect(search).toBeFocused();
  await search.press('Meta+f');
  await expect(page.getByRole('search')).toHaveCount(1);
  await search.press('Escape');
  await expect(composer).toBeFocused();
  await page.getByRole('textbox', { name: 'Filter sessions', exact: true }).press('Meta+f');
  await expect(page.getByRole('search', { name: 'Find in page', exact: true })).toBeVisible();
  await expect(search).toHaveCount(0);
  await expect(composer).toHaveValue('Keep this unsent prompt');
});

test('macOS Control F remains text movement in the message composer', async ({ page }) => {
  await page.addInitScript(() => Object.defineProperty(navigator, 'platform', { value: 'MacIntel' }));
  await openChat(page);
  const composer = page.locator('.composer textarea');
  await composer.fill('An unsent Mac draft');
  await composer.press('Control+f');
  await expect(page.getByRole('search')).toHaveCount(0);
  await expect(composer).toBeFocused();
  await expect(composer).toHaveValue('An unsent Mac draft');
});
