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

// ─────────────────────────────────────────────────────────────────────────────
// Redesign: the panel reads as the bell's own popover (anchored + caret), unread
// survives opening, session notices group per session, long failures clamp
// behind "Show details", and Clear lives behind ⋯ + a confirm.
// ─────────────────────────────────────────────────────────────────────────────

type MockNotice = {
  id: string;
  created_at: string;
  read: boolean;
  kind: 'credential' | 'session' | 'system';
  severity: 'info' | 'warn' | 'error';
  title: string;
  body: string;
  source_key: string | null;
  action: { type: 'open_session'; session_id: string } | null;
};

/** Serve exactly `notices`; every mutation succeeds. Returns the read-all hit log. */
async function mockList(page: Page, notices: MockNotice[]): Promise<{ readAll: number }> {
  const hits = { readAll: 0 };
  await page.route(/\/api\/v1\/notifications(\?.*)?$/, async (route) => {
    if (route.request().method() === 'GET') await route.fulfill({ json: notices });
    else await route.fulfill({ status: 204, body: '' });
  });
  await page.route(/\/api\/v1\/notifications\/(?!settings).+$/, async (route) => {
    if (route.request().url().endsWith('/read-all')) hits.readAll++;
    await route.fulfill({ status: 204, body: '' });
  });
  return hits;
}

const ago = (mins: number): string => new Date(Date.now() - mins * 60_000).toISOString();
const navBell = (page: Page) =>
  page.locator('.navigator .nav-head').getByRole('button', { name: /^Notifications/ });

test('panel is anchored to the bell with a caret, and unread survives opening', async ({ page }) => {
  const hits = await mockList(page, [
    { id: 'a', created_at: ago(1), read: false, kind: 'system', severity: 'info', title: 'Update ready', body: 'Otto 1.2 is available.', source_key: null, action: null },
    { id: 'b', created_at: ago(5), read: true, kind: 'system', severity: 'info', title: 'Old news', body: 'Already seen.', source_key: null, action: null },
  ]);
  await setRail(page, true);
  await openPage(page, 'git');

  const bell = navBell(page);
  await expect(bell.locator('.badge')).toHaveText('1');
  await bell.click();
  const panel = page.getByRole('dialog', { name: 'Notifications' });
  await expectFullyInViewport(page, panel, 'notification panel');

  // Anchored: the panel starts right past the sidebar edge, a short hop from
  // the bell (which sits at the header's inline-end), and the bell's centre
  // falls inside the panel's vertical span.
  const b = (await bell.boundingBox())!;
  const p = (await panel.boundingBox())!;
  expect(p.x - (b.x + b.width)).toBeGreaterThan(0);
  expect(p.x - (b.x + b.width)).toBeLessThan(64);
  const bellMidY = b.y + b.height / 2;
  expect(bellMidY).toBeGreaterThan(p.y);
  expect(bellMidY).toBeLessThan(p.y + p.height);
  // The caret points back at the bell: vertically centred on it, poking out
  // of the panel's inline-start edge.
  const caret = (await page.locator('.nb-caret').boundingBox())!;
  expect(Math.abs(caret.y + caret.height / 2 - bellMidY)).toBeLessThan(3);
  expect(caret.x).toBeLessThan(p.x);

  // Opening does NOT mark read: the unread row keeps its dot, the badge stays
  // and "Mark all read" is live. Focus lands on the first row.
  await expect(panel.locator('.nb-item.unread')).toHaveCount(1);
  await expect(bell.locator('.badge')).toHaveText('1');
  await expect(panel.getByRole('button', { name: 'Mark all read' })).toBeEnabled();
  await expect(panel.locator('.nb-row').first()).toBeFocused();
  expect(hits.readAll).toBe(0);

  // Closing marks what you saw as read, and focus returns to the bell.
  await page.keyboard.press('Escape');
  await expect(panel).toHaveCount(0);
  await expect(bell.locator('.badge')).toHaveCount(0);
  await expect(bell).toBeFocused();
  await expect.poll(() => hits.readAll).toBe(1);
});

test('session notices group per session; long failures clamp; Clear asks first', async ({ page }) => {
  const sk = (s: string): string => `session:s1:${s}`;
  const long = 'Codex update failed: ' + 'npm ERR! code ETIMEDOUT at registry.npmjs.org '.repeat(12);
  const go = { type: 'open_session' as const, session_id: 's1' };
  await mockList(page, [
    { id: 's3', created_at: ago(1), read: false, kind: 'session', severity: 'info', title: 'Session awaiting input', body: 'fix-login (claude) is idle and may be waiting for your input · was on: tests', source_key: sk('waiting'), action: go },
    { id: 's2', created_at: ago(8), read: false, kind: 'session', severity: 'info', title: 'Session awaiting input', body: 'fix-login (claude) is idle and may be waiting for your input.', source_key: sk('waiting'), action: go },
    { id: 's1', created_at: ago(20), read: true, kind: 'session', severity: 'info', title: 'Session awaiting input', body: 'fix-login (claude) is idle and may be waiting for your input.', source_key: sk('idle'), action: go },
    { id: 'e1', created_at: ago(3), read: false, kind: 'system', severity: 'error', title: 'CLI update failed', body: long, source_key: null, action: null },
  ]);
  await setRail(page, true);
  await openPage(page, 'git');

  const bell = navBell(page);
  // Two unread ROWS (one session group + one alert), coloured by the error.
  await expect(bell.locator('.badge')).toHaveText('2');
  await expect(bell.locator('.badge')).toHaveClass(/sev-error/);
  await bell.click();
  const panel = page.getByRole('dialog', { name: 'Notifications' });

  // One row for the session: its title, the short state line, and ×3.
  const rows = panel.locator('.nb-item');
  await expect(rows).toHaveCount(2);
  const session = rows.first();
  await expect(session.locator('.nb-title')).toHaveText('fix-login');
  await expect(session.locator('.nb-text')).toContainText('Waiting for your input');
  await expect(session.locator('.nb-count')).toHaveText('×3');
  await expect(session.getByRole('button', { name: /Go to session/ })).toBeVisible();
  // Compact: no per-row action button, the whole row is the target.
  expect((await session.boundingBox())!.height).toBeLessThan(64);

  // The long failure is clamped until "Show details" opens the full text.
  const alert = rows.nth(1);
  expect((await alert.locator('.nb-text').boundingBox())!.height).toBeLessThan(40);
  await alert.getByRole('button', { name: 'Show details' }).click();
  const detail = alert.locator('.nb-detail pre');
  await expect(detail).toBeVisible();
  expect((await detail.boundingBox())!.height).toBeLessThanOrEqual(161);
  await expect(alert.getByRole('button', { name: /Copy/ })).toBeVisible();

  // Clear is behind ⋯ and confirms before deleting.
  await panel.getByRole('button', { name: 'More notification actions' }).click();
  await page.getByText('Clear all notifications…').click();
  const confirm = page.getByRole('dialog', { name: 'Clear all notifications' });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  await expect(confirm).toHaveCount(0);
});

test('a failed load shows an error with Retry, not "all caught up"', async ({ page }) => {
  await page.route(/\/api\/v1\/notifications(\?.*)?$/, (route) =>
    route.fulfill({ status: 500, json: { error: 'boom' } }),
  );
  await setRail(page, true);
  await openPage(page, 'git');
  await navBell(page).click();
  const panel = page.getByRole('dialog', { name: 'Notifications' });
  await expect(panel.getByText("Couldn't load notifications")).toBeVisible();
  await expect(panel.getByRole('button', { name: /Retry/ })).toBeVisible();
  await expect(panel.getByText("You're all caught up")).toHaveCount(0);
});
