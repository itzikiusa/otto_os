import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';

// Durable mobile-terminal suite. Seeds a REAL shell session with output into the
// isolated test daemon, then asserts the things that made the phone terminal
// feel broken ("big black void, can't see or type"):
//
//   1. The terminal host actually has height (the lower pane used to collapse to
//      ~0 on the mobile shell → a blank black box).
//   2. On phone the terminal uses xterm's DOM renderer, NOT a WebGL canvas. This
//      is the core robustness fix: mobile WKWebView/Safari frequently fails to
//      create the GL context or loses it right after first paint, leaving a
//      permanently black canvas. The DOM renderer has no GPU dependency, so
//      output is always visible. Asserting "canvas absent + .xterm-rows present"
//      proves the fallback is active AND that rendered text lives in the DOM.
//   3. The phone readability font floor is applied (rendered rows ≥ 15px) so the
//      grid isn't a cramped 13px on a high-DPI handset.
//   4. Best-effort: input reaches the terminal. WebKit's touch EMULATION does not
//      faithfully synthesize the iOS soft-keyboard focus path (it doesn't raise a
//      keyboard or fire the real pointer sequence), so we can't assert soft-kb
//      focus here — that's verified on a real device. We instead assert the phone
//      key accessory bar mounts + is usable (its buttons share the exact WS input
//      path the physical keyboard uses), which is the on-screen typing affordance.
//
// Run with --workers=1 (shared seeded session).

let workspaceId = '';
let sessionId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  sessionId = await seedShellSession(ctx, base, workspaceId);
  // Real output so the renderer has something to paint.
  await ctx.post(`${base}/api/v1/sessions/${sessionId}/input`, {
    data: { text: 'for i in $(seq 1 40); do echo "OTTO-LINE-$i"; done', submit: true },
  });
  await new Promise((r) => setTimeout(r, 1000));
  await ctx.dispose();
});

// Activate the seeded workspace + close the nav drawer so it doesn't cover the
// terminal in portrait.
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

// Phone-mode is decided by the LIVE viewport width (≤640px), not the device
// profile name: an iPhone held in LANDSCAPE is 932px wide → the app treats it as
// "tablet" (desktop chrome + WebGL), so the phone-only renderer/keybar behavior
// does NOT apply there. We must gate phone-only assertions on the real width.
async function liveIsPhone(page: import('@playwright/test').Page): Promise<boolean> {
  return page.evaluate(() => window.matchMedia('(max-width: 640px)').matches);
}

test('terminal host has real height', async ({ page }) => {
  await page.goto(`/#/agents/${sessionId}`);
  const host = page.locator('.term-host');
  await expect(host).toBeVisible({ timeout: 30_000 });
  const box = await host.boundingBox();
  expect(box, 'terminal host present').not.toBeNull();
  // The collapse bug rendered this ~0; require real height + width.
  expect(box!.height, 'terminal height').toBeGreaterThan(120);
  expect(box!.width, 'terminal width').toBeGreaterThan(0);
});

test('phone: DOM renderer is active (no WebGL canvas) + readable font', async ({ page }) => {
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.term-host')).toBeVisible({ timeout: 30_000 });
  test.skip(!(await liveIsPhone(page)), 'phone-mode only (≤640px live viewport)');
  // Let the WS attach + the renderer paint its first frame.
  await page.waitForTimeout(2000);

  const r = await page.evaluate(() => {
    const rows = document.querySelector('.xterm-rows') as HTMLElement | null;
    return {
      hasCanvas: !!document.querySelector('.xterm canvas'),
      hasRowsDom: !!rows,
      rowsChildren: rows?.childElementCount ?? 0,
      rowsFontPx: rows ? parseFloat(getComputedStyle(rows).fontSize) : 0,
    };
  });

  // DOM renderer: spans in .xterm-rows, no GPU canvas → can't go black.
  expect(r.hasCanvas, 'phone must NOT use a WebGL canvas (it can go black)').toBe(false);
  expect(r.hasRowsDom, 'phone must use the xterm DOM renderer (.xterm-rows)').toBe(true);
  expect(r.rowsChildren, 'DOM renderer should have rendered rows').toBeGreaterThan(0);
  // Readability floor (PHONE_MIN_FONT = 15) applied — not a cramped 13px.
  expect(r.rowsFontPx, 'phone terminal font should hit the readability floor').toBeGreaterThanOrEqual(15);
});

test('phone: key accessory bar mounts + is usable', async ({ page }) => {
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.term-host')).toBeVisible({ timeout: 30_000 });
  test.skip(!(await liveIsPhone(page)), 'phone-mode only (≤640px live viewport)');

  // The key bar is toggled by the floating ⌨ button (a real user gesture is
  // required on-device to raise the soft keyboard; the bar itself mounts here).
  const kbToggle = page.locator('.phone-btn[aria-label="Toggle keyboard"]');
  await expect(kbToggle).toBeVisible();
  await kbToggle.click();

  const bar = page.locator('.keys-bar');
  await expect(bar).toBeVisible();
  // The bar must expose the keys a soft keyboard can't produce (Esc/Ctrl/arrows).
  // Use exact role names so "Ctrl" doesn't also match "Ctrl-C".
  await expect(bar.getByRole('button', { name: 'Esc', exact: true })).toBeVisible();
  await expect(bar.getByRole('button', { name: 'Ctrl', exact: true })).toBeVisible();
  await expect(bar.getByRole('button', { name: '↑', exact: true })).toBeVisible();
  // Tap targets meet the ≥44px guideline.
  const escBox = await bar.getByRole('button', { name: 'Esc', exact: true }).boundingBox();
  expect(escBox!.height, 'key tap target height').toBeGreaterThanOrEqual(44);
});

test('terminal: focuses the xterm textarea on tap (pointer profiles)', async ({ page }) => {
  await page.goto(`/#/agents/${sessionId}`);
  const host = page.locator('.term-host');
  await expect(host).toBeVisible({ timeout: 30_000 });
  // WebKit touch EMULATION doesn't synthesize the iOS soft-keyboard focus path,
  // so assert tap-to-focus only on the pointer-driven (non-phone) viewports where
  // it's reproducible. Phone soft-keyboard focus is verified on a real device.
  test.skip(
    await liveIsPhone(page),
    'soft-keyboard focus not reproducible under touch emulation; verified on device',
  );
  await host.click();
  await expect
    .poll(() =>
      page.evaluate(() => document.activeElement?.classList.contains('xterm-helper-textarea')),
    )
    .toBe(true);
});
