import { test, expect, type Page } from '@playwright/test';
import { openPage, expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';

// Real mobile & tablet UX verification across all 8 touch form factors:
//   • iOS:     iPhone Portrait, iPhone Landscape, iPad Portrait, iPad Landscape
//   • Android: Phone Portrait, Phone Landscape, Tablet Portrait, Tablet Landscape
// Exercises multi-hop navigation, drawer/modal/input focus, Full ↔ Lite mode
// live switching, tablet sidebar collapse, and strict anti-"half cut off"
// geometry invariants (scrollLeft, scrollX, .shell bounds, usable .center area,
// and >=16px touch input font-size).

test.use({ serviceWorkers: 'block' });

const LITE_MODULES = ['threads', 'connections', 'git', 'api', 'usage'] as const;
const FULL_HOP_SEQUENCE = ['threads', 'git', 'connections', 'api', 'settings/appearance', 'vault', 'usage', 'agents'] as const;

interface LayoutHealth {
  vw: number;
  vh: number;
  overflowX: number;
  scrollX: number;
  scrollY: number;
  docScrollLeft: number;
  bodyScrollLeft: number;
  contentScrollLeft: number;
  contentScrollTop: number;
  shellRect: { x: number; y: number; width: number; height: number } | null;
  centerRect: { x: number; y: number; width: number; height: number } | null;
}

async function readLayoutHealth(page: Page): Promise<LayoutHealth> {
  return page.evaluate(() => {
    const docEl = document.documentElement;
    const content = document.getElementById('app-content');
    const shell = document.querySelector<HTMLElement>('.shell');
    const center = document.querySelector<HTMLElement>('.center');
    const rect = (el: HTMLElement | null) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) };
    };
    return {
      vw: window.innerWidth,
      vh: window.innerHeight,
      overflowX: docEl.scrollWidth - docEl.clientWidth,
      scrollX: Math.round(window.scrollX),
      scrollY: Math.round(window.scrollY),
      docScrollLeft: Math.round(docEl.scrollLeft),
      bodyScrollLeft: Math.round(document.body.scrollLeft),
      contentScrollLeft: Math.round(content?.scrollLeft ?? 0),
      contentScrollTop: Math.round(content?.scrollTop ?? 0),
      shellRect: rect(shell),
      centerRect: rect(center),
    };
  });
}

async function assertViewportHealthy(page: Page, step: string): Promise<void> {
  await expectNoHorizontalOverflow(page);
  await expect.poll(async () => {
    const h = await readLayoutHealth(page);
    return h.scrollX === 0 && h.scrollY === 0 && h.docScrollLeft === 0 && h.bodyScrollLeft === 0 && h.contentScrollLeft === 0;
  }, { message: `${step}: scroll offsets must stay zeroed (no half-cut-off drift)` }).toBe(true);

  const h = await readLayoutHealth(page);
  expect(h.shellRect, `${step}: .shell must render`).not.toBeNull();
  expect(Math.abs(h.shellRect!.x), `${step}: .shell x must align to viewport start`).toBeLessThanOrEqual(2);
  expect(Math.abs(h.shellRect!.width - h.vw), `${step}: .shell width must match viewport`).toBeLessThanOrEqual(4);
  expect(Math.abs(h.shellRect!.height - h.vh), `${step}: .shell height must fill viewport`).toBeLessThanOrEqual(4);

  expect(h.centerRect, `${step}: .center must render`).not.toBeNull();
  expect(h.centerRect!.x, `${step}: .center must not shift off left edge`).toBeGreaterThanOrEqual(-1);
  expect(h.centerRect!.x + h.centerRect!.width, `${step}: .center must not overflow right edge`).toBeLessThanOrEqual(h.vw + 2);
  const minCenterW = h.vw <= 640 ? h.vw - 8 : Math.min(320, Math.floor(h.vw * 0.5));
  const minCenterH = h.vh <= 500 ? 140 : 200;
  expect(h.centerRect!.width, `${step}: .center usable width`).toBeGreaterThanOrEqual(minCenterW);
  expect(h.centerRect!.height, `${step}: .center usable height`).toBeGreaterThanOrEqual(minCenterH);
}

/** Open the visible Full ↔ Lite toggle control for the current device/orientation and click it. */
async function toggleLightModeViaUi(page: Page): Promise<void> {
  const vw = page.viewportSize()!.width;
  const vh = page.viewportSize()!.height;
  const isPhonePortrait = vw <= 640;
  const isPhoneLandscape = vw > 640 && vw <= 1024 && vh <= 500;

  if (isPhonePortrait) {
    const moreBtn = page.locator('nav.bottomnav').getByRole('button', { name: /^More/ });
    await expect(moreBtn).toBeVisible();
    await moreBtn.click();
    const sheet = page.getByRole('dialog', { name: 'More' });
    await expectFullyInViewport(page, sheet, 'More drawer');
    await sheet.getByTestId('light-toggle').click();
    await expect(sheet).toBeHidden();
    return;
  }

  if (isPhoneLandscape) {
    const navBtn = page.getByRole('button', { name: 'Open navigator' }).first();
    await expect(navBtn).toBeVisible();
    await navBtn.click();
    const drawer = page.locator('.drawer[aria-label="Navigator"]');
    await expectFullyInViewport(page, drawer, 'Landscape Navigator drawer');
    await drawer.getByTestId('light-toggle').click();
    await expect(drawer).toBeHidden();
    return;
  }

  // Tablet portrait / tablet landscape: toggle sits directly in Navigator footer (or Rail bottom if folded).
  const navToggle = page.getByTestId('nav-light-toggle');
  const railToggle = page.getByTestId('rail-light-toggle');
  await expect(navToggle.or(railToggle).first()).toBeVisible();
  if (await navToggle.isVisible()) await navToggle.click();
  else await railToggle.click();
}

test('multi-hop navigation + drawers + input focus never cuts off the screen', async ({ page }) => {
  test.setTimeout(120_000);
  await openPage(page, 'threads');
  await assertViewportHealthy(page, 'initial threads');

  // Navigate across 8 diverse modules, checking zero horizontal/vertical drift on every hop.
  for (const route of FULL_HOP_SEQUENCE) {
    await openPage(page, route);
    await assertViewportHealthy(page, `full hop -> ${route}`);
  }

  // Open & focus a text input (Settings filter) and verify coarse-pointer >=16px anti-zoom rule.
  await openPage(page, 'settings');
  const filter = page.getByPlaceholder('Filter settings…');
  await expect(filter).toBeVisible();
  await filter.click();
  await filter.fill('appearance');
  const info = await filter.evaluate((el) => ({
    coarse: window.matchMedia('(pointer: coarse)').matches,
    fontSize: parseFloat(getComputedStyle(el).fontSize),
  }));
  if (info.coarse) {
    expect(info.fontSize, 'touch text inputs must be >=16px so iOS never auto-zooms').toBeGreaterThanOrEqual(16);
  }
  await assertViewportHealthy(page, 'after settings input focus');

  // Open and dismiss Right Panel drawer on mobile/tablet, then hop back to Git + Threads.
  const sidePanelBtn = page.locator('.mtopbar').getByRole('button', { name: 'Side panel' });
  if (await sidePanelBtn.isVisible()) {
    await sidePanelBtn.click();
    const rightDrawer = page.locator('.drawer.right:not(.inline)');
    await expectFullyInViewport(page, rightDrawer, 'Right panel drawer');
    await page.keyboard.press('Escape');
    await expect(rightDrawer).toBeHidden();
  }

  await openPage(page, 'git');
  await assertViewportHealthy(page, 'post-drawer git');
  await openPage(page, 'threads');
  await assertViewportHealthy(page, 'post-drawer threads');
});

test('Full ↔ Lite live switching works across phone & tablet (portrait and landscape)', async ({ page }) => {
  test.setTimeout(120_000);
  await openPage(page, 'home');
  await assertViewportHealthy(page, 'full mode home');

  // Full mode is the default everywhere.
  expect(await page.evaluate(() => localStorage.getItem('otto_phone_light'))).not.toBe('1');

  // Switch Full → Lite via the real visible UI toggle on this form factor.
  await toggleLightModeViaUi(page);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto_phone_light'))).toBe('1');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/threads');
  await assertViewportHealthy(page, 'after switch to lite');

  // Walk every Lite module and verify clean layout + zero drift.
  for (const mod of LITE_MODULES) {
    await openPage(page, mod);
    await expect(page.getByText('isn’t in the light version')).toHaveCount(0);
    await assertViewportHealthy(page, `lite mode -> ${mod}`);
  }

  // Switch Lite → Full via the same real UI affordance.
  await toggleLightModeViaUi(page);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto_phone_light'))).toBe('0');
  await assertViewportHealthy(page, 'after switch back to full');

  // Full-only pages render cleanly immediately after switching back.
  for (const mod of ['home', 'vault', 'workflows'] as const) {
    await openPage(page, mod);
    await expect(page.getByText('isn’t in the light version')).toHaveCount(0);
    await assertViewportHealthy(page, `restored full mode -> ${mod}`);
  }
});

test('tablet collapses/expands Navigator to Rail; phone portrait Full mode keeps Navigator on Threads', async ({ page }) => {
  const vw = page.viewportSize()!.width;
  const vh = page.viewportSize()!.height;

  if (vw <= 640) {
    // Phone portrait Full mode: Threads hides shell topbar, so Threads header must expose Navigator.
    await openPage(page, 'threads');
    const navTrigger = page.getByTestId('threads-open-navigator');
    await expect(navTrigger).toBeVisible();
    await navTrigger.click();
    const drawer = page.locator('.drawer[aria-label="Navigator"]');
    await expectFullyInViewport(page, drawer, 'Threads Full-mode Navigator drawer');
    await drawer.getByRole('button', { name: 'Vault', exact: true }).click();
    await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/vault');
    await expect(drawer).toHaveCount(0);
    await assertViewportHealthy(page, 'phone portrait threads -> vault via navigator');
    return;
  }

  if (vw > 640 && vw <= 1024 && vh > 500) {
    // Tablet portrait: Navigator starts expanded (220px), can fold to 44px Rail and expand back.
    await openPage(page, 'git');
    const before = (await page.locator('.center').first().boundingBox())!;
    const collapseBtn = page.locator('.navigator').getByRole('button', { name: 'Collapse sidebar' });
    await expect(collapseBtn).toBeVisible();
    await collapseBtn.click();
    await expect(page.locator('.rail')).toBeVisible();
    await expect.poll(async () => (await page.locator('.center').first().boundingBox())!.width).toBeGreaterThan(before.width + 140);
    await assertViewportHealthy(page, 'tablet folded to rail');

    const expandBtn = page.locator('.rail').getByRole('button', { name: 'Expand sidebar' });
    await expandBtn.click();
    await expect(page.locator('.navigator')).toBeVisible();
    await assertViewportHealthy(page, 'tablet expanded back to navigator');
    return;
  }

  // Phone landscape or wide tablet landscape: verify clean layout on Settings (landscape two-level / split).
  await openPage(page, 'settings/appearance');
  await expect(page.getByTestId('appearance-light-version')).toBeVisible();
  await assertViewportHealthy(page, 'landscape settings appearance');
});
