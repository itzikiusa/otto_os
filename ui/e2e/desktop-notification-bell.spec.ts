import { test, expect, type Page } from '@playwright/test';
import { openPage, expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The notification bell lives in the sidebar (Navigator header / collapsed
// Rail) — one spot on every page (desktop-browser only).
//
// It used to float over the top-right corner of the center column, which forced
// every non-Agents page to reserve a full-height 42px right gutter (dead space)
// and put the bell in a different place on Agents (inside the tab bar). Now:
//   • no floating anchor, and module content reaches the right edge;
//   • the bell is in the Navigator, and in the Rail when collapsed;
//   • its panel opens beside the sidebar, clamped into the viewport, and a list
//     long enough to overflow the window scrolls inside the panel.
// ─────────────────────────────────────────────────────────────────────────────

/** Serve N unread notices so the badge shows and the panel list overflows. */
async function mockNotices(page: Page, n: number): Promise<void> {
  const notices = Array.from({ length: n }, (_, i) => ({
    id: `n${i}`,
    created_at: new Date(Date.now() - i * 60_000).toISOString(),
    read: false,
    kind: 'info',
    severity: i % 5 === 0 ? 'warn' : 'info',
    title: `Notice ${i}`,
    body: 'A notice body long enough to wrap onto a second line in the panel.',
    source_key: null,
    action: null,
  }));
  await page.route(/\/api\/v1\/notifications(\?.*)?$/, async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({ json: notices });
    } else {
      await route.fulfill({ status: 204, body: '' });
    }
  });
  await page.route(/\/api\/v1\/notifications\/read-all$/, (route) =>
    route.fulfill({ status: 204, body: '' }),
  );
}

async function setRail(page: Page, expanded: boolean): Promise<void> {
  await page.addInitScript((v) => {
    try {
      localStorage.setItem('otto_rail_expanded', v);
    } catch {
      /* storage blocked — the default (expanded) applies */
    }
  }, expanded ? '1' : '0');
}

test('bell sits in the Navigator and module pages keep no right gutter', async ({ page }) => {
  await mockNotices(page, 60);
  await setRail(page, true);
  await openPage(page, 'git');

  await expect(page.locator('.bell-anchor')).toHaveCount(0);
  const bell = page.locator('.navigator .nav-head').getByRole('button', { name: 'Notifications' });
  await expect(bell).toBeVisible();
  await expect(bell.locator('.badge')).toHaveText('60');

  // The content pane is not padded on the right for a bell anymore.
  const padEnd = await page
    .locator('.center > .content')
    .evaluate((el) => getComputedStyle(el).paddingInlineEnd);
  expect(padEnd).toBe('0px');

  // The panel opens beside the sidebar and stays fully on screen even though
  // 60 notices are far taller than the window — the list scrolls inside it.
  await bell.click();
  const panel = page.getByRole('dialog', { name: 'Notifications' });
  await expectFullyInViewport(page, panel, 'notification panel');
  const navBox = (await page.locator('.navigator').boundingBox())!;
  const panelBox = (await panel.boundingBox())!;
  expect(panelBox.x).toBeGreaterThanOrEqual(navBox.x + navBox.width);
  const list = panel.locator('.panel-list');
  expect(await list.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);

  // Escape closes it.
  await page.keyboard.press('Escape');
  await expect(panel).toHaveCount(0);
});

test('collapsed Rail carries the bell and its panel stays in the viewport', async ({ page }) => {
  await mockNotices(page, 3);
  await setRail(page, false);
  // >1024 wide: 641–1024 is the tablet shell, which always shows the Navigator.
  await page.setViewportSize({ width: 1180, height: 520 });
  await openPage(page, 'mcp');

  const bell = page.locator('.rail').getByRole('button', { name: 'Notifications' });
  await expect(bell).toBeVisible();
  await bell.click();
  await expectFullyInViewport(
    page,
    page.getByRole('dialog', { name: 'Notifications' }),
    'notification panel',
  );
});
