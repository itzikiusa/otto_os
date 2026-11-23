import { test, expect, type Page } from '@playwright/test';
import { mkdtempSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openPage, expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';
import { apiCtx } from './seed';
import { daemonCall, seedPolyglotFiles } from './ide-polyglot-fixtures';

// End-to-end verification of Otto's new Browser, IDE, and Mobile Simulator
// features across all 8 touch form factors (iOS iPhone/iPad + Android
// Phone/Tablet, Portrait & Landscape) in both Full and Lite modes.

test.use({ serviceWorkers: 'block' });

interface IdeSeed {
  workspaceId: string;
  root: string;
}
let ideSeed: IdeSeed | null = null;

async function ensureIdeSeed(): Promise<IdeSeed> {
  if (ideSeed) return ideSeed;
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-mobile-sim-ide-')));
  seedPolyglotFiles(root);
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', {
      name: `Mobile Sim IDE ${Date.now()}`,
      root_path: root,
    });
    ideSeed = { workspaceId: ws.id, root };
    return ideSeed;
  } finally {
    await c.ctx.dispose();
  }
}

async function assertZeroCutOff(page: Page, step: string): Promise<void> {
  await expectNoHorizontalOverflow(page);
  await expect.poll(async () => page.evaluate(() => {
    const content = document.getElementById('app-content');
    return (
      Math.round(window.scrollX) === 0 &&
      Math.round(window.scrollY) === 0 &&
      Math.round(document.documentElement.scrollLeft) === 0 &&
      Math.round(document.body.scrollLeft) === 0 &&
      Math.round(content?.scrollLeft ?? 0) === 0
    );
  }), { message: `${step}: scroll offsets must stay zero` }).toBe(true);
}

/** Toggle Full ↔ Lite using the real visible control for the current device & orientation. */
async function toggleMode(page: Page): Promise<void> {
  const { width: vw, height: vh } = page.viewportSize()!;
  if (vw <= 640) {
    await page.locator('nav.bottomnav').getByRole('button', { name: /^More/ }).click();
    const sheet = page.getByRole('dialog', { name: 'More' });
    await expectFullyInViewport(page, sheet, 'More drawer');
    await sheet.getByTestId('light-toggle').click();
    await expect(sheet).toBeHidden();
    return;
  }
  if (vw <= 1024 && vh <= 500) {
    await page.getByRole('button', { name: 'Open navigator' }).first().click();
    const drawer = page.locator('.drawer[aria-label="Navigator"]');
    await expectFullyInViewport(page, drawer, 'Landscape Navigator drawer');
    await drawer.getByTestId('light-toggle').click();
    await expect(drawer).toBeHidden();
    return;
  }
  const navToggle = page.getByTestId('nav-light-toggle');
  const railToggle = page.getByTestId('rail-light-toggle');
  await expect(navToggle.or(railToggle).first()).toBeVisible();
  if (await navToggle.isVisible()) await navToggle.click();
  else await railToggle.click();
}

test('GET /api/v1/simulators/status returns structured readiness without 500s', async () => {
  const { ctx, base } = await apiCtx();
  try {
    const resp = await ctx.get(`${base}/api/v1/simulators/status`);
    expect(resp.status(), 'simulators status endpoint must succeed').toBe(200);
    const body = (await resp.json()) as {
      ios?: { platform?: string; state?: string; summary?: string; setup_steps?: unknown[] };
      android?: { platform?: string; state?: string; summary?: string; setup_steps?: unknown[] };
    };
    expect(body.ios?.platform).toBe('ios');
    expect(['ready', 'degraded', 'unavailable']).toContain(body.ios?.state);
    expect(typeof body.ios?.summary).toBe('string');
    expect(Array.isArray(body.ios?.setup_steps)).toBe(true);
    expect(body.android?.platform).toBe('android');
    expect(['ready', 'degraded', 'unavailable']).toContain(body.android?.state);
    expect(typeof body.android?.summary).toBe('string');
    expect(Array.isArray(body.android?.setup_steps)).toBe(true);
  } finally {
    await ctx.dispose();
  }
});

test('Browser opens cleanly, renders Simulators diagnostics, and handles mobile deep-links', async ({ page }) => {
  test.setTimeout(90_000);
  await openPage(page, 'browser');
  await assertZeroCutOff(page, 'browser initial');

  // Single-row URL bar on phone and landscape viewports.
  const urlBar = page.locator('.urlbar').first();
  await expect(urlBar).toBeVisible();
  const barBox = (await urlBar.boundingBox())!;
  expect(barBox.height, 'URL bar should stay single-row without wrapping').toBeLessThanOrEqual(64);
  expect(barBox.x + barBox.width, 'URL bar must fit within viewport').toBeLessThanOrEqual(page.viewportSize()!.width + 2);

  // Open iOS Simulators & Android Emulators modal via the real Browser UI control.
  const directSimBtn = page.getByRole('button', { name: 'iOS Simulators and Android Emulators' });
  const compactMoreBtn = page.getByRole('button', { name: 'More browser tools' });
  if (await directSimBtn.isVisible()) {
    await directSimBtn.click();
  } else {
    await expect(compactMoreBtn).toBeVisible();
    await compactMoreBtn.click();
    await page.getByRole('menuitem', { name: /Simulators/ }).click();
  }

  const simDialog = page.getByRole('dialog', { name: 'iOS Simulators & Android Emulators' });
  await expectFullyInViewport(page, simDialog, 'Browser Simulators modal');
  await expect(simDialog.getByRole('heading', { name: 'iOS Simulator', exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(simDialog.getByRole('heading', { name: 'Android Emulator', exact: true })).toBeVisible();

  // Open and validate the shared create-device form inside Browser's Simulators modal.
  const newDevBtn = simDialog.getByRole('button', { name: /New device/i });
  await expect(newDevBtn).toBeVisible();
  await newDevBtn.click();
  const browserCreateForm = simDialog.getByRole('form', { name: 'Create new simulator or emulator' });
  await expect(browserCreateForm).toBeVisible();
  await expect(browserCreateForm.getByRole('button', { name: 'iOS Simulator' })).toBeVisible();
  await expect(browserCreateForm.getByRole('button', { name: 'Android AVD' })).toBeVisible();
  await expect(browserCreateForm.getByRole('button', { name: 'Create device' })).toBeDisabled();
  await browserCreateForm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(browserCreateForm).toHaveCount(0);

  await page.keyboard.press('Escape');
  await expect(simDialog).toHaveCount(0);
  await assertZeroCutOff(page, 'after closing simulators modal');

  // Deep-link from IDE: `#/browser?url=...&mode=live&device=iphone-16-pro`.
  await page.goto('/#/browser?url=https%3A%2F%2Fexample.com&mode=live&device=iphone-16-pro');
  await expect(page.locator('.shell')).toBeVisible();
  await expect.poll(
    () => page.evaluate(() => window.location.hash),
    { message: 'deep-link query must be consumed and stripped to #/browser', timeout: 15_000 },
  ).toBe('#/browser');
  await expect(page.locator('.urlbar input').first()).toHaveValue(/example\.com/, { timeout: 15_000 });
  const liveModeBtn = page.getByRole('button', { name: 'Live mode' });
  await expect(liveModeBtn).toBeVisible({ timeout: 15_000 });
  await expect(liveModeBtn).toHaveAttribute('aria-pressed', 'true');
  const devBar = page.getByRole('toolbar', { name: 'Device emulation' });
  const engineSetup = page.getByTestId('live-engine-setup');
  await expect(devBar.or(engineSetup).first()).toBeVisible({ timeout: 15_000 });
  if (await devBar.isVisible()) {
    await expect(devBar.getByLabel('Device preset')).toHaveValue('iphone-16-pro');
  } else {
    await expect(engineSetup).toBeVisible();
  }
  await assertZeroCutOff(page, 'browser deep-link');
});

test('IDE Devices panel, Toolchains modal (iOS/Android/Flutter), and Browser↔IDE↔Threads↔Settings Full/Lite hops', async ({ page }) => {
  test.setTimeout(120_000);
  const s = await ensureIdeSeed();
  await page.addInitScript(({ workspaceId, root }) => {
    localStorage.setItem('otto_workspace', workspaceId);
    localStorage.setItem(
      'otto.ide.namedProjects.v1',
      JSON.stringify([{ id: 'sim-proj', name: 'Sim Mobile', folders: [{ path: root, name: 'sim-mobile' }], updatedAt: Date.now() }]),
    );
  }, s);

  await openPage(page, 'ide');
  await assertZeroCutOff(page, 'ide initial');

  // On stacked phone/landscape viewports, switch to the Tools pane first.
  const toolsTab = page.getByTestId('ide-stack-tab-tools');
  const dock = page.getByTestId('ide-bottom-dock');
  await expect(toolsTab.or(dock).first()).toBeVisible({ timeout: 20_000 });
  if (await toolsTab.isVisible()) {
    await expect(async () => {
      await toolsTab.click();
      await expect(toolsTab).toHaveAttribute('aria-pressed', 'true');
      await expect(dock).toBeVisible({ timeout: 2_000 });
    }).toPass({ timeout: 15_000 });
  }

  // Open the on-demand Devices tool window via dock view menu (or directly if already visible).
  const devicesTab = page.getByTestId('ide-tab-devices');
  if (!(await devicesTab.isVisible())) {
    await page.getByTestId('ide-dock-view-menu').click();
    await page.getByRole('menuitemcheckbox', { name: 'Devices' }).or(page.getByRole('menuitem', { name: 'Devices' })).first().click();
  }
  await expect(devicesTab).toBeVisible();
  await devicesTab.click();
  await expect(page.locator('[aria-label="Simulators and emulators"]')).toBeVisible({ timeout: 15_000 });
  await assertZeroCutOff(page, 'ide devices tab');

  // Open and validate the IDE New Simulator / AVD modal (fits in viewport, platform toggle, submit validation).
  const ideNewDevBtn = page.getByTestId('ide-sim-new-device');
  await expect(ideNewDevBtn).toBeVisible();
  await ideNewDevBtn.click();
  const ideCreateDialog = page.getByRole('dialog', { name: 'New Simulator / AVD' });
  await expectFullyInViewport(page, ideCreateDialog, 'IDE New Simulator / AVD modal');
  await expect(ideCreateDialog.getByRole('button', { name: 'iOS Simulator' })).toBeVisible();
  await expect(ideCreateDialog.getByRole('button', { name: 'Android AVD' })).toBeVisible();
  await expect(ideCreateDialog.getByRole('button', { name: /Create device/i })).toBeDisabled();
  await page.keyboard.press('Escape');
  await expect(ideCreateDialog).toHaveCount(0);
  await assertZeroCutOff(page, 'after closing ide create device modal');

  // Open Toolchains modal (Environments → Environment tab → Download / Manage SDKs…) and verify mobile ecosystems.
  const headerMore = page.locator('[data-testid="page-header"]').getByRole('button', { name: 'More actions' }).first();
  await expect(headerMore).toBeVisible();
  await headerMore.click();
  await page.getByRole('menuitem', { name: /Edit run configurations/ }).click();
  const envDialog = page.getByRole('dialog', { name: /Environments/i });
  await expectFullyInViewport(page, envDialog, 'IDE Environments modal');
  const envTab = envDialog.locator('#ide-env-tab-env');
  if (await envTab.isVisible()) await envTab.click();
  await envDialog.getByTestId('ide-env-open-toolchains').click();

  const tcDialog = page.getByRole('dialog', { name: 'SDKs, toolchains & debuggers' });
  await expectFullyInViewport(page, tcDialog, 'IDE Toolchains modal');
  for (const label of ['iOS / Xcode', 'Android SDK', 'Flutter / Dart']) {
    await expect(tcDialog.getByRole('tab', { name: label })).toBeVisible({ timeout: 15_000 });
  }
  await page.keyboard.press('Escape');
  await expect(tcDialog).toHaveCount(0);
  if (await envDialog.isVisible()) {
    await page.keyboard.press('Escape');
    await expect(envDialog).toHaveCount(0);
  }
  await assertZeroCutOff(page, 'after closing toolchains modal');

  // Cross-module navigation: Browser → IDE → Threads → Settings → Browser + Full ↔ Lite toggle.
  for (const route of ['browser', 'ide', 'threads', 'settings/appearance', 'browser'] as const) {
    await openPage(page, route);
    await assertZeroCutOff(page, `cross-hop -> ${route}`);
  }

  await toggleMode(page);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto_phone_light'))).toBe('1');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/threads');
  await assertZeroCutOff(page, 'lite mode after cross-hop');

  await toggleMode(page);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto_phone_light'))).toBe('0');
  await openPage(page, 'browser');
  await assertZeroCutOff(page, 'back to full mode browser');
});
