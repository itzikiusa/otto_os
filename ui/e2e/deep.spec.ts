import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';

// Deep, data-dependent specs: seed a real shell session into the isolated test
// daemon, then verify behaviors the empty-state baseline can't — the terminal
// renders with height (collapse fix) and focuses on tap (mobile keyboard fix),
// and the desktop sidebar collapse toggle actually changes width.
// Run with --workers=1.

let workspaceId = '';
let sessionId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  sessionId = await seedShellSession(ctx, base, workspaceId);
  await ctx.dispose();
});

// Make the seeded workspace the active one deterministically (the store reads
// this key on boot), before any page script runs.
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
  }, workspaceId);
});

test('terminal: renders with height (collapse fix)', async ({ page }) => {
  await page.goto(`/#/agents/${sessionId}`);
  const host = page.locator('.term-host');
  await expect(host).toBeVisible({ timeout: 30_000 });
  const box = await host.boundingBox();
  expect(box, 'terminal host present').not.toBeNull();
  // The collapse bug made this ~0 on the mobile shell; verify it now has real
  // height on every device profile.
  expect(box!.height, 'terminal height').toBeGreaterThan(100);
  expect(box!.width, 'terminal width').toBeGreaterThan(0);
});

test('terminal: focuses on tap', async ({ page }, testInfo) => {
  // The mobile soft-keyboard focus path (onTouchPointerDown → term.focus()) is a
  // real-device behavior that WebKit's touch EMULATION doesn't faithfully
  // synthesize (it doesn't raise a keyboard or fire the same pointer sequence),
  // so we assert tap-to-focus on the pointer-driven profiles (iPad/desktop) where
  // it's reproducible. Phone soft-keyboard focus is verified on a real device.
  test.skip(
    testInfo.project.name.startsWith('iphone'),
    'soft-keyboard focus not reproducible under touch emulation; verified on device',
  );
  await page.goto(`/#/agents/${sessionId}`);
  const host = page.locator('.term-host');
  await expect(host).toBeVisible({ timeout: 30_000 });
  await host.click();
  await expect
    .poll(() =>
      page.evaluate(() => document.activeElement?.classList.contains('xterm-helper-textarea')),
    )
    .toBe(true);
});

test('sidebar: collapse toggle changes width', async ({ page }, testInfo) => {
  test.skip(
    !testInfo.project.name.includes('ipad-landscape'),
    'desktop inline sidebar only on >1024px profile',
  );
  await page.goto('/#/agents');
  const sidebar = page.locator('.sidebar').first();
  await expect(sidebar).toBeVisible();
  const before = (await sidebar.boundingBox())!.width;

  await page.locator('button[aria-label="Collapse sidebar"]').first().click();
  await expect.poll(async () => (await sidebar.boundingBox())!.width).not.toBe(before);
});
