import { test, expect, type Page } from '@playwright/test';
import { openPage, expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';
import { apiCtx, seedWorkspace } from './seed';

// Mobile & tablet UX end-to-end verification for Otto's 3D Studio & Voice/TTS
// Studio across all 9 touch form factors (iPhone SE, iPhone Portrait/Landscape,
// iPad Portrait/Landscape, Android Phone Portrait/Landscape, Android Tablet
// Portrait/Landscape) in Light & Dark themes and Full & Lite modes.

test.use({ serviceWorkers: 'block' });

const V1 = '/api/v1';

const SAMPLE_SCENE = {
  type: 'otto-scene3d',
  version: 2,
  background: '#eceaf3',
  grid: true,
  environment: { preset: 'studio-soft', intensity: 1, background: true },
  camera: { position: [3.4, 2.1, 5.2], target: [0, 0.8, 0], fov: 30 },
  lights: [
    {
      id: 'key',
      name: 'Key light',
      type: 'directional',
      position: [4, 6, 4],
      target: [0, 0.8, 0],
      intensity: 1.6,
      shadow: true,
    },
  ],
  objects: [
    {
      id: 'lantern',
      name: 'Lantern',
      type: 'box',
      radius: 0.05,
      position: [0, 0.95, 0],
      rotation: [0, 20, 0],
      scale: [1.2, 1.4, 0.9],
      material: { preset: 'glossy-plastic', color: '#5b3df5' },
    },
  ],
  groups: [],
  states: [
    { id: 'idle', name: 'Idle' },
    { id: 'lit', name: 'Lit', duration_ms: 400, overrides: { lantern: { rotation: [0, 180, 0] } } },
  ],
  default_state: 'idle',
};

let wsId = '';
let sceneId = '';

async function ensureSeeded(): Promise<{ wsId: string; sceneId: string }> {
  if (wsId && sceneId) return { wsId, sceneId };
  const { ctx, base } = await apiCtx();
  try {
    wsId = await seedWorkspace(ctx, base);
    const stamp = Date.now();
    const projResp = await ctx.post(`${base}${V1}/design/projects`, {
      data: { workspace_id: wsId, name: `Mobile 3D Studio ${stamp}` },
    });
    expect(projResp.ok(), `create design project failed: ${projResp.status()}`).toBeTruthy();
    const proj = (await projResp.json()) as { id: string };

    const artResp = await ctx.post(`${base}${V1}/design/artifacts`, {
      data: {
        workspace_id: wsId,
        project_id: proj.id,
        format: 'scene3d',
        studio: '3d',
        title: `Mobile Lantern 3D ${stamp}`,
        content: JSON.stringify(SAMPLE_SCENE, null, 2),
      },
    });
    expect(artResp.ok(), `create 3d artifact failed: ${artResp.status()}`).toBeTruthy();
    const art = (await artResp.json()) as { artifact: { id: string } };
    sceneId = art.artifact.id;
    return { wsId, sceneId };
  } finally {
    await ctx.dispose();
  }
}

async function assertZeroCutOff(page: Page, step: string): Promise<void> {
  await expectNoHorizontalOverflow(page);
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const content = document.getElementById('app-content');
          return (
            Math.round(window.scrollX) === 0 &&
            Math.round(window.scrollY) === 0 &&
            Math.round(document.documentElement.scrollLeft) === 0 &&
            Math.round(document.body.scrollLeft) === 0 &&
            Math.round(content?.scrollLeft ?? 0) === 0
          );
        }),
      { message: `${step}: scroll offsets must stay zero` },
    )
    .toBe(true);
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
  const toggle = navToggle.or(railToggle).first();
  await expect(toggle).toBeVisible();
  await toggle.click();
}

test('Live 3D & TTS provider endpoints return otto-forge, trellis-2, hunyuan3d-omni (with license_warning), and ready macos-say', async () => {
  const { ctx, base } = await apiCtx();
  try {
    const p3dResp = await ctx.get(`${base}${V1}/design/3d/providers`);
    expect(p3dResp.status(), 'GET /design/3d/providers must succeed').toBe(200);
    const p3dBody = (await p3dResp.json()) as {
      providers?: Array<{ id: string; name: string; license_warning?: string | null }>;
    };
    const p3d = Array.isArray(p3dBody) ? p3dBody : (p3dBody.providers ?? []);
    const byId3d = new Map(p3d.map((p) => [p.id, p]));
    expect(byId3d.has('otto-forge'), 'otto-forge built-in provider must be registered').toBe(true);
    expect(byId3d.has('trellis-2'), 'trellis-2 built-in provider must be registered').toBe(true);
    const hunyuan = byId3d.get('hunyuan3d-omni');
    expect(hunyuan, 'hunyuan3d-omni built-in provider must be registered').toBeTruthy();
    expect(
      (hunyuan?.license_warning ?? '').length,
      'hunyuan3d-omni must expose an explicit territory license_warning',
    ).toBeGreaterThan(10);

    const ttsResp = await ctx.get(`${base}${V1}/product/tts/providers`);
    expect(ttsResp.status(), 'GET /product/tts/providers must succeed').toBe(200);
    const ttsBody = (await ttsResp.json()) as {
      providers?: Array<{ id: string; name: string; ready?: boolean; enabled?: boolean }>;
    };
    const tts = Array.isArray(ttsBody) ? ttsBody : (ttsBody.providers ?? []);
    const macosSay = tts.find((p) => p.id === 'macos-say');
    expect(macosSay, 'macos-say built-in provider must be registered').toBeTruthy();
    expect(macosSay?.enabled).toBe(true);
    expect(macosSay?.ready, 'macos-say must be ready on macOS').toBe(true);
  } finally {
    await ctx.dispose();
  }
});

test('Voice / TTS Studio renders cleanly in Light & Dark themes, meets coarse-pointer ergonomics, opens Provider Manager, and synthesizes via macos-say', async ({ page }) => {
  test.setTimeout(90_000);
  const seed = await ensureSeeded();
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_scheme', 'dark');
  }, seed.wsId);

  // Open via Product mode tabs first, then verify direct `#/design/tts`.
  await openPage(page, 'product');
  const voiceTab = page.getByRole('tab', { name: 'Voice' });
  await expect(voiceTab).toBeVisible({ timeout: 15_000 });
  await voiceTab.click();
  await expect(page.getByTestId('tts-studio-page')).toBeVisible({ timeout: 15_000 });
  await expect.poll(() => page.evaluate(() => window.location.hash)).toBe('#/design/tts');
  await assertZeroCutOff(page, 'voice studio dark theme via Product tab');

  // Verify coarse-pointer input font-size (>=16px avoids iOS Safari zoom) and >=40px tap targets.
  const scriptInput = page.getByTestId('tts-script-input');
  const generateBtn = page.getByTestId('tts-generate-btn');
  await expect(scriptInput).toBeVisible();
  await expect(generateBtn).toBeVisible();
  const ergonomics = await page.evaluate(() => {
    const coarse = window.matchMedia('(pointer: coarse)').matches;
    const ta = document.querySelector('[data-testid="tts-script-input"]') as HTMLElement | null;
    const sel = document.querySelector('[data-testid="tts-provider-select"]') as HTMLElement | null;
    const btn = document.querySelector('[data-testid="tts-generate-btn"]') as HTMLElement | null;
    return {
      coarse,
      taFont: ta ? parseFloat(getComputedStyle(ta).fontSize) : 0,
      selFont: sel ? parseFloat(getComputedStyle(sel).fontSize) : 0,
      btnHeight: btn ? btn.getBoundingClientRect().height : 0,
    };
  });
  if (ergonomics.coarse) {
    expect(ergonomics.taFont, 'script textarea font-size must be >=16px on coarse pointer').toBeGreaterThanOrEqual(16);
    expect(ergonomics.selFont, 'provider select font-size must be >=16px on coarse pointer').toBeGreaterThanOrEqual(16);
    expect(ergonomics.btnHeight, 'Generate speech button must be >=40px tall on coarse pointer').toBeGreaterThanOrEqual(40);
  }

  // Open Provider Manager modal via direct header button or PageHeader overflow menu.
  const directProvidersBtn = page.getByTestId('tts-manage-providers');
  if (await directProvidersBtn.isVisible()) {
    await directProvidersBtn.click();
  } else {
    const headerMore = page
      .locator('[data-testid="page-header"]')
      .getByRole('button', { name: 'More actions' })
      .first();
    await expect(headerMore).toBeVisible();
    await headerMore.click();
    await page.getByRole('menuitem', { name: /Providers/ }).click();
  }
  const providerDialog = page.getByRole('dialog', { name: 'TTS voice providers' });
  await expectFullyInViewport(page, providerDialog, 'TTS Provider Manager modal');
  await expect(providerDialog.getByTestId('tts-provider-manager')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(providerDialog).toHaveCount(0);
  await assertZeroCutOff(page, 'voice studio after closing provider modal');

  // Synthesize a short test line via built-in macos-say and verify clip renders cleanly.
  const providerSelect = page.getByTestId('tts-provider-select');
  await expect(providerSelect).toBeVisible();
  await providerSelect.selectOption('macos-say');
  await scriptInput.fill('Hello from Otto mobile studio.');
  await expect(generateBtn).toBeEnabled();
  await generateBtn.click();
  await expect(page.getByTestId('tts-clip-history')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('tts-clip-history')).toContainText('Hello from Otto mobile studio.');
  await assertZeroCutOff(page, 'voice studio after macos-say synthesis');

  // Switch to Light theme and verify direct `#/design/tts` renders cleanly without overflow.
  await page.evaluate(() => localStorage.setItem('otto_scheme', 'light'));
  await page.reload();
  await expect(page.getByTestId('tts-studio-page')).toBeVisible({ timeout: 15_000 });
  await assertZeroCutOff(page, 'voice studio light theme');
});

/** Geometry of the Voice Studio form: rows of fields that sit side by side,
 *  with label/control boxes and whether each control's text fits. */
async function voiceFormGeometry(page: Page) {
  return page.evaluate(() => {
    const ctx = document.createElement('canvas').getContext('2d')!;
    const textFits = (el: HTMLInputElement | HTMLSelectElement): boolean => {
      const s = getComputedStyle(el);
      ctx.font = `${s.fontWeight} ${s.fontSize} ${s.fontFamily}`;
      const text = el instanceof HTMLSelectElement ? el.selectedOptions[0]?.text ?? '' : el.value || el.placeholder;
      const room = el.clientWidth - parseFloat(s.paddingInlineStart) - parseFloat(s.paddingInlineEnd);
      return ctx.measureText(text).width <= room + 0.5;
    };
    const form = document.querySelector('.tts-main') as HTMLElement;
    const formWidth = form.clientWidth;
    const fields = [...document.querySelectorAll<HTMLElement>('.tts-main .tts-field')].map((f) => {
      const label = f.querySelector('label')!.getBoundingClientRect();
      const control = f.querySelector<HTMLInputElement | HTMLSelectElement>('select, input')!;
      const box = control.getBoundingClientRect();
      return {
        id: control.id,
        top: Math.round(f.getBoundingClientRect().top),
        width: f.getBoundingClientRect().width,
        labelTop: label.top,
        controlHeight: box.height,
        kind: control instanceof HTMLSelectElement ? 'select' : control.type,
        fits: control.type === 'range' ? true : textFits(control),
      };
    });
    const rows = new Map<number, typeof fields>();
    for (const f of fields) rows.set(f.top, [...(rows.get(f.top) ?? []), f]);
    return { formWidth, rows: [...rows.values()] };
  });
}

test('Voice Studio form, voice list, clip count and loading/error states stay correct', async ({ page }, info) => {
  test.setTimeout(90_000);
  const seed = await ensureSeeded();
  const shots = process.env.OTTO_E2E_SHOTS;
  const shot = async (state: string) => {
    if (shots) await page.screenshot({ path: `${shots}/${info.project.name}-${state}.png` });
  };
  // A synthesized clip so the rail lists one (its header count must match).
  const { ctx, base } = await apiCtx();
  try {
    const synth = await ctx.post(`${base}${V1}/product/tts/synthesize`, {
      data: { workspace_id: seed.wsId, provider_id: 'macos-say', voice_id: 'Samantha', text: 'Count me.', format: 'wav', title: 'Count me' },
    });
    expect(synth.ok(), `synthesize failed: ${synth.status()}`).toBeTruthy();
  } finally {
    await ctx.dispose();
  }
  const pageErrors: string[] = [];
  // Render errors only (WebKit also reports fetches a reload cancels).
  page.on('pageerror', (e) => {
    if (/svelte/i.test(String(e))) pageErrors.push(String(e));
  });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    // Dark first; the light pass below switches it and must survive reloads.
    if (!localStorage.getItem('otto_scheme')) localStorage.setItem('otto_scheme', 'dark');
  }, seed.wsId);

  // macOS `say -v ?` lists some voices twice; a repeated id once aborted the
  // render and left "Voice (loading…)" and a stale clip count on screen.
  const voices = ['Albert', 'Samantha (English (US))', 'Samantha (English (US))', 'Daniel', 'Moira', 'Fred'].map((id) => ({
    id,
    name: id,
    locale: id === 'Daniel' ? 'en_GB' : 'en_US',
    gender: null,
    preview_text: null,
    provider_id: 'macos-say',
  }));
  await page.route('**/api/v1/product/tts/providers/*/voices', (r) => r.fulfill({ json: voices }));
  await openPage(page, 'design/tts');
  await expect(page.getByTestId('tts-studio-page')).toBeVisible({ timeout: 15_000 });
  // Soft assertions: each defect reports on its own (they also run on the
  // pre-fix code to prove they catch it).
  const voiceLabel = page.locator('label[for="tts-voice"]');
  const voiceSelect = page.locator('select#tts-voice');
  const form = page.locator('.tts-controls').first();
  await expect(form).toBeVisible({ timeout: 15_000 });
  await expect.soft(voiceLabel).toHaveText('Voice (5)', { timeout: 10_000 });
  await expect.soft(voiceSelect.locator('option')).toHaveCount(5);
  // The Narrator default "Samantha" resolves to the listed variant, not a blank select.
  await expect.soft(voiceSelect).toHaveValue('Samantha (English (US))');
  const clipList = page.getByTestId('tts-clip-history').locator('li');
  await expect(clipList.first()).toBeVisible();
  await expect.soft(page.locator('.tts-history .tts-shead .k')).toHaveText(`Generated clips (${await clipList.count()})`);
  expect.soft(pageErrors, 'no render error on the Voice Studio').toEqual([]);

  // Form: fields beside each other line up (labels and control heights), no
  // field is orphaned on a short row, and no control cuts its text.
  for (const scheme of ['dark', 'light'] as const) {
    if (scheme === 'light') {
      await page.evaluate(() => localStorage.setItem('otto_scheme', 'light'));
      await page.reload();
      await expect(form).toBeVisible({ timeout: 15_000 });
      await expect(page.locator('html')).toHaveAttribute('data-scheme', 'light');
    }
    const geo = await voiceFormGeometry(page);
    for (const row of geo.rows) {
      const ids = row.map((f) => f.id).join('+');
      if (row.length === 1) {
        expect.soft(row[0].width, `${ids} must not sit alone on a short row`).toBeGreaterThan(geo.formWidth * 0.75);
      }
      for (const f of row) {
        expect.soft(Math.abs(f.labelTop - row[0].labelTop), `${ids}: labels must line up`).toBeLessThanOrEqual(1);
        expect.soft(f.fits, `${f.id} must show its whole text`).toBe(true);
      }
    }
    const heights = geo.rows.flat().filter((f) => f.kind !== 'range').map((f) => f.controlHeight);
    expect.soft(Math.max(...heights) - Math.min(...heights), 'selects and inputs share one height').toBeLessThanOrEqual(1);
    if (await page.evaluate(() => matchMedia('(pointer: coarse)').matches)) {
      expect.soft(Math.min(...heights), 'form controls are touch-sized').toBeGreaterThanOrEqual(40);
    }
    // The speed slider draws its own track (the native one was near-invisible in dark).
    expect.soft(await page.locator('#tts-speed').evaluate((el) => getComputedStyle(el).appearance)).toBe('none');
    await assertZeroCutOff(page, `voice form ${scheme}`);
    await shot(`tts-${scheme}-loaded`);
  }

  // Touch browsers: the shell chrome is opaque (a see-through sidebar layer
  // overprinted its rows on the Android tablet).
  const chrome = await page.evaluate(() => {
    if (!matchMedia('(pointer: coarse)').matches) return null;
    const probe = document.body.appendChild(Object.assign(document.createElement('div'), { className: 'glass-raised' }));
    const raised = [getComputedStyle(probe).backgroundColor, getComputedStyle(probe).backdropFilter];
    probe.remove();
    const nav = document.querySelector('.navigator');
    const navStyle = nav && nav.getClientRects().length ? getComputedStyle(nav) : null;
    return {
      raised,
      nav: navStyle ? [navStyle.backgroundColor, navStyle.backdropFilter, getComputedStyle(nav!.querySelector('.nav-scroll')!).backgroundColor] : null,
    };
  });
  const opaque = /^rgb\(/;
  if (chrome) expect.soft(chrome.raised[0], 'menus and popovers are opaque on touch').toMatch(opaque);
  if (chrome) expect.soft(chrome.raised[1]).toBe('none');
  if (chrome?.nav) {
    expect.soft(chrome.nav[1]).toBe('none');
    for (const bg of [chrome.nav[0], chrome.nav[2]]) expect.soft(bg, 'sidebar background is opaque').toMatch(opaque);
  }

  // A failed voice list shows inline with Retry (it used to fall back silently).
  await page.unroute('**/api/v1/product/tts/providers/*/voices');
  await page.route('**/api/v1/product/tts/providers/*/voices', (r) => r.fulfill({ status: 500, json: { code: 'internal', message: 'say crashed' } }));
  await page.reload();
  const voicesError = page.getByTestId('tts-voices-error');
  await expect(form).toBeVisible({ timeout: 15_000 });
  await expect.soft(voicesError).toBeVisible({ timeout: 10_000 });
  await expect.soft(voiceLabel).toHaveText('Voice');
  await assertZeroCutOff(page, 'voice list error');
  await shot('tts-light-voices-error');
  await page.unroute('**/api/v1/product/tts/providers/*/voices');
  await page.route('**/api/v1/product/tts/providers/*/voices', (r) => r.fulfill({ json: voices }));
  if (await voicesError.isVisible()) {
    await voicesError.getByRole('button', { name: 'Retry' }).click();
    await expect(voiceSelect).toBeVisible();
    await expect(voicesError).toHaveCount(0);
  }

  // First load: a skeleton, not an empty form; a failed first load: inline error + Retry.
  await page.route('**/api/v1/product/tts/providers', async (r) => {
    await new Promise((res) => setTimeout(res, 2_500));
    await r.fallback();
  });
  await page.reload();
  await expect(page.getByTestId('tts-studio-page')).toBeVisible({ timeout: 15_000 });
  await expect.soft(page.getByRole('status', { name: 'Loading Voice Studio' })).toBeVisible({ timeout: 2_000 });
  await expect.soft(page.locator('.tts-controls')).toHaveCount(0, { timeout: 1_000 });
  await expect(page.locator('.ls-loading .sr-only')).toBeAttached(); // past LoadState's skeleton grace
  await shot('tts-light-loading');
  await expect(form).toBeVisible({ timeout: 15_000 });
  await page.unroute('**/api/v1/product/tts/providers');
  await page.route('**/api/v1/product/tts/providers', (r) => r.fulfill({ status: 500, json: { code: 'internal', message: 'daemon unavailable' } }));
  await page.reload();
  const loadError = page.getByTestId('load-error');
  await expect(page.getByTestId('tts-studio-page')).toBeVisible({ timeout: 15_000 });
  await expect.soft(loadError).toContainText('Couldn’t load Voice Studio', { timeout: 10_000 });
  await expect.soft(page.locator('.tts-controls')).toHaveCount(0);
  await assertZeroCutOff(page, 'voice studio load error');
  await shot('tts-light-load-error');
  await page.unroute('**/api/v1/product/tts/providers');
  if (await loadError.isVisible()) {
    await loadError.getByRole('button', { name: 'Retry' }).click();
    await expect(voiceSelect).toBeVisible({ timeout: 15_000 });
  }
  expect.soft(pageErrors, 'no render error across the states').toEqual([]);
});

test('3D Studio mobile panels, Generate modal (Otto Forge / TRELLIS.2 / Hunyuan3D-Omni warning), and cross-module Full/Lite hops', async ({ page }) => {
  test.setTimeout(120_000);
  const seed = await ensureSeeded();
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
  }, seed.wsId);

  // Open the seeded 3D scene artifact in Design Hall / Studio3D.
  await openPage(page, `design/a/${encodeURIComponent(seed.sceneId)}`);
  await expect(page.getByTestId('s3d-generate')).toBeVisible({ timeout: 20_000 });
  await assertZeroCutOff(page, 'studio3d initial');

  // On narrow/short mobile viewports, verify collapsible Scene and Details panels toggle cleanly.
  const sceneToggle = page.locator('.studio3d .scene-toggle');
  if (await sceneToggle.isVisible()) {
    await expect(sceneToggle).toHaveAttribute('aria-expanded', 'false');
    await sceneToggle.click();
    await expect(sceneToggle).toHaveAttribute('aria-expanded', 'true');
    await assertZeroCutOff(page, 'studio3d scene panel open');
    await sceneToggle.click();
    await expect(sceneToggle).toHaveAttribute('aria-expanded', 'false');
  }
  const detailsToggle = page.locator('.studio3d .details-toggle');
  if (await detailsToggle.isVisible()) {
    await expect(detailsToggle).toHaveAttribute('aria-expanded', 'false');
    await detailsToggle.click();
    await expect(detailsToggle).toHaveAttribute('aria-expanded', 'true');
    await assertZeroCutOff(page, 'studio3d details panel open');
    await detailsToggle.click();
    await expect(detailsToggle).toHaveAttribute('aria-expanded', 'false');
  }

  // Open Generate modal and verify Otto Forge (local), TRELLIS.2, and Hunyuan3D-Omni territory warning.
  await page.getByTestId('s3d-generate').click();
  await page.getByRole('menuitem', { name: /Generate 3D asset/ }).click();
  const genModalContent = page.getByTestId('gen3d-modal');
  await expect(genModalContent).toBeVisible({ timeout: 15_000 });
  const genDialog = genModalContent.locator('xpath=ancestor::*[@role="dialog"][1]');
  await expectFullyInViewport(page, genDialog, '3D Generate modal');
  // Otto Forge is the generator; TRELLIS.2 and Hunyuan3D-Omni wait under a closed Advanced.
  await expect(genModalContent.getByText('TRELLIS.2')).toBeHidden();
  await genModalContent.getByText('Advanced', { exact: true }).click();
  await expect(genModalContent.locator('input[type="radio"][value="otto-forge"]')).toBeVisible();
  await expect(genModalContent).toContainText('TRELLIS.2');
  await expect(genModalContent).toContainText('Hunyuan3D-Omni');
  await expectFullyInViewport(page, genDialog, '3D Generate modal with Advanced open');

  // Selecting Hunyuan3D-Omni surfaces the explicit territory license restriction notice.
  const hunyuanRadio = genModalContent.locator('input[type="radio"][value="hunyuan3d-omni"]');
  await expect(hunyuanRadio).toBeVisible();
  await hunyuanRadio.check();
  const licenseNotice = genModalContent.getByTestId('gen3d-license-notice');
  await expect(licenseNotice).toBeVisible();
  await expect(licenseNotice).toContainText(/EU|UK|South Korea|Territory restriction/i);

  await page.keyboard.press('Escape');
  await expect(genModalContent).toHaveCount(0);
  await assertZeroCutOff(page, 'studio3d after closing generate modal');

  // Cross-page navigation: Voice → Design → Browser → IDE → Settings → Voice + Full ↔ Lite toggle.
  for (const route of ['design/tts', 'design', 'browser', 'ide', 'settings/appearance', 'design/tts'] as const) {
    await openPage(page, route);
    await assertZeroCutOff(page, `cross-hop -> ${route}`);
  }

  await toggleMode(page);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto_phone_light'))).toBe('1');
  await expect.poll(() => page.evaluate(() => window.location.hash)).toBe('#/threads');
  await assertZeroCutOff(page, 'lite mode after voice/3d cross-hop');

  await toggleMode(page);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto_phone_light'))).toBe('0');
  await openPage(page, 'design/tts');
  await expect(page.getByTestId('tts-studio-page')).toBeVisible({ timeout: 15_000 });
  await assertZeroCutOff(page, 'back to full mode voice studio');
});

/** Stand in for the on-screen keyboard: Playwright has none, so this replaces
 *  `window.visualViewport` with one whose height drops by `kb` px and which is
 *  panned `pan` px down inside the layout viewport — what Android Chrome
 *  (interactive-widget=resizes-visual) did on a real Pixel when the Voice script
 *  field took focus: the app sat above a ≈195 px blank band. */
async function installKeyboardViewport(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const et = new EventTarget();
    let kb = 0;
    let pan = 0;
    const vv = {
      get height() { return innerHeight - kb; },
      get width() { return innerWidth; },
      get offsetTop() { return pan; },
      offsetLeft: 0,
      pageTop: 0,
      pageLeft: 0,
      scale: 1,
      addEventListener: et.addEventListener.bind(et),
      removeEventListener: et.removeEventListener.bind(et),
      dispatchEvent: et.dispatchEvent.bind(et),
    };
    Object.defineProperty(window, 'visualViewport', { configurable: true, get: () => vv });
    (window as unknown as { __kb: (k: number, p: number) => void }).__kb = (k, p) => {
      kb = k;
      pan = p;
      et.dispatchEvent(new Event('resize'));
    };
  });
}
/** Is the element inside the part of the page the keyboard leaves visible? */
async function inVisibleBand(page: Page, sel: string): Promise<{ top: number; bottom: number; bandTop: number; bandBottom: number; ok: boolean }> {
  return page.locator(sel).last().evaluate((el) => {
    const r = el.getBoundingClientRect();
    const vv = window.visualViewport!;
    let [bandTop, bandBottom] = [vv.offsetTop, vv.offsetTop + vv.height];
    // Ancestors that clip (the app, a scroll container) narrow what can be seen — for a
    // fixed overlay only from its containing block (a transformed ancestor) up.
    let fixed = getComputedStyle(el).position === 'fixed';
    for (let n = el.parentElement; n; n = n.parentElement) {
      const cs = getComputedStyle(n);
      if (fixed && cs.transform !== 'none') fixed = false;
      if (!fixed && cs.overflowY !== 'visible') {
        const c = n.getBoundingClientRect();
        [bandTop, bandBottom] = [Math.max(bandTop, c.top), Math.min(bandBottom, c.bottom)];
      }
      if (cs.position === 'fixed') fixed = true;
    }
    return { top: Math.round(r.top), bottom: Math.round(r.bottom), bandTop, bandBottom, ok: r.top >= bandTop - 1 && r.bottom <= bandBottom + 1 };
  });
}

test('keyboard open: the Voice script field and Generate speech stay visible; a 3D dialog stays above the keyboard', async ({ page }, info) => {
  test.setTimeout(90_000);
  const seed = await ensureSeeded();
  await installKeyboardViewport(page);
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), seed.wsId);
  const shots = process.env.OTTO_E2E_SHOTS;
  await openPage(page, 'design/tts');
  const script = page.getByTestId('tts-script-input');
  await expect(script).toBeVisible({ timeout: 15_000 });
  await script.click();
  await script.fill('Narrator: Beyond the northern ridge lies the sunken forge.');
  const h = page.viewportSize()!.height;
  // A phone keyboard is ≈ 40 % of a portrait screen; Chrome panned the page to reveal the field.
  // On its side (a ≤ 500 px page) a real Pixel keeps 67 px above the keyboard, panned 77 px.
  const short = h < 500;
  const kb = short ? h - 70 : Math.round(h * 0.4);
  await page.evaluate(([k, p]) => (window as unknown as { __kb: (k: number, p: number) => void }).__kb(k, p), [kb, Math.round(h * (short ? 0.25 : 0.23))]);
  await expect(page.locator('html')).toHaveAttribute('data-kb', 'open');
  // The shell's top row and the page header step aside only when the strip is short.
  const chrome = page.locator('.mtopbar, [data-testid="page-header"]');
  if (h - kb < 400) for (const c of await chrome.all()) await expect.soft(c, 'shell chrome steps aside over a short keyboard strip').toBeHidden();
  else await expect.soft(page.locator('[data-testid="page-header"]'), 'a portrait page with room keeps its header').toBeVisible();
  await expect.poll(async () => (await inVisibleBand(page, '[data-testid="tts-script-input"]')).ok, { message: 'the script field is in the visible part' }).toBe(true);
  await expect.poll(async () => (await inVisibleBand(page, '[data-testid="tts-generate-btn"]')).ok, { message: 'Generate speech is in the visible part' }).toBe(true);
  expect.soft((await inVisibleBand(page, '.tts-ebar .tts-dim')).ok, 'the character counter is visible').toBe(true);
  if (!short) {
    const g = await inVisibleBand(page, '[data-testid="tts-generate-btn"]');
    expect.soft(g.bandBottom - g.bottom, 'Generate speech keeps a small gap above the keyboard').toBeGreaterThanOrEqual(4);
  }
  if (shots) await page.screenshot({ path: `${shots}/${info.project.name}-tts-keyboard-open.png` });
  await page.evaluate(() => (window as unknown as { __kb: (k: number, p: number) => void }).__kb(0, 0));
  await page.locator('body').click({ position: { x: 2, y: 2 } }).catch(() => {});

  // 3D Generate dialog with its prompt focused: the whole sheet sits above the keyboard.
  await openPage(page, `design/a/${encodeURIComponent(seed.sceneId)}`);
  await expect(page.getByTestId('s3d-generate')).toBeVisible({ timeout: 20_000 });
  await page.getByTestId('s3d-generate').click();
  await page.getByRole('menuitem', { name: /Generate 3D asset/ }).click();
  const prompt = page.getByTestId('gen3d-modal').locator('textarea').first();
  await prompt.click();
  await page.evaluate(([k]) => (window as unknown as { __kb: (k: number, p: number) => void }).__kb(k, 0), [kb]);
  await expect(page.locator('html')).toHaveAttribute('data-kb', 'open');
  await expect.poll(async () => {
    const f = await inVisibleBand(page, '[data-testid="gen3d-modal"] textarea');
    return Math.min(f.bottom, f.bandBottom) - Math.max(f.top, f.bandTop);
  }, { message: 'the focused prompt shows above the keyboard' }).toBeGreaterThanOrEqual(20);
  await expect.poll(async () => (await inVisibleBand(page, '.sheet[role="dialog"]')).ok, { message: 'the Generate dialog fits above the keyboard' }).toBe(true);
  await expect.poll(async () => (await inVisibleBand(page, '.sheet[role="dialog"] footer')).ok, { message: 'its footer (Cancel / Generate 3D) is above the keyboard' }).toBe(true);
  if (shots) await page.screenshot({ path: `${shots}/${info.project.name}-3d-generate-modal-keyboard-open.png` });
});

test('Voice Studio: leaving while a clip’s audio loads leaks no blob URL and raises no stale toast', async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-portrait', 'page lifecycle, one project is enough');
  const seed = await ensureSeeded();
  const { ctx, base } = await apiCtx();
  try {
    const synth = await ctx.post(`${base}${V1}/product/tts/synthesize`, {
      data: { workspace_id: seed.wsId, provider_id: 'macos-say', voice_id: 'Samantha', text: 'Hold on.', format: 'wav', title: 'Held clip' },
    });
    expect(synth.ok(), `synthesize failed: ${synth.status()}`).toBeTruthy();
  } finally {
    await ctx.dispose();
  }
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    const w = window as unknown as { __blobs: number };
    w.__blobs = 0;
    const make = URL.createObjectURL.bind(URL);
    // Audio blobs only: the page navigated to may make its own (thumbnails).
    URL.createObjectURL = (o: Blob | MediaSource) => {
      if (o instanceof Blob && o.type.startsWith('audio/')) w.__blobs++;
      return make(o);
    };
  }, seed.wsId);
  for (const answer of ['audio', 'error'] as const) {
    // Hold the first clip's audio until the page is gone, then answer it.
    let release!: (route: import('@playwright/test').Route) => void;
    const held = new Promise<import('@playwright/test').Route>((r) => (release = r));
    await page.route('**/api/v1/product/tts/clips/*/audio', (route) => release(route), { times: 1 });
    // Not openPage: its network-idle wait would sit on the held request.
    await page.goto('/#/design/tts');
    await expect(page.getByTestId('tts-studio-page')).toBeVisible({ timeout: 15_000 });
    const route = await held;
    await page.evaluate(() => (location.hash = '#/design'));
    await expect(page.getByTestId('tts-studio-page')).toHaveCount(0);
    await expect(page.getByTestId('design-lobby')).toBeVisible({ timeout: 15_000 });
    const before = await page.evaluate(() => (window as unknown as { __blobs: number }).__blobs);
    if (answer === 'audio') await route.fulfill({ status: 200, contentType: 'audio/wav', body: Buffer.alloc(64) });
    else await route.fulfill({ status: 500, json: { code: 'internal', message: 'boom' } });
    await page.waitForTimeout(800); // ui-guards: allow — proves nothing happens after the answer lands
    if (answer === 'audio') {
      const after = await page.evaluate(() => (window as unknown as { __blobs: number }).__blobs);
      expect.soft(after - before, 'no blob URL is made for a page that is gone').toBe(0);
    } else {
      await expect.soft(page.getByText('Couldn’t load clip audio'), 'no error toast from a page that is gone').toHaveCount(0);
    }
  }
});
