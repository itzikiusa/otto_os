import { expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

// The top-level routable pages (hash router: #/<module>). `share` is excluded
// (it needs a scoped token) and is covered separately.
export const PAGES = [
  'agents',
  'mission-control',
  'api',
  'brokers',
  'connections',
  'database',
  'git',
  'help',
  'insights',
  'plugins',
  'product',
  'settings',
  'skills-eval',
  'swarm',
  'usage',
  'vault',
  'workflows',
] as const;

export type PageId = (typeof PAGES)[number];

/** Navigate to a hash route and wait for the shell + content to settle. */
export async function openPage(page: Page, id: string): Promise<void> {
  await page.goto(`/#/${id}`);
  // The app shell is always present once booted.
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  // Let layout/reflow + first data fetch settle.
  await page.waitForLoadState('networkidle').catch(() => {});
}

/**
 * Assert the page does not overflow the viewport horizontally. A small
 * tolerance absorbs sub-pixel rounding. This is the core check for the "content
 * runs past the right edge / is clipped" class of bugs.
 */
export async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  const overflow = await page.evaluate(() => {
    const el = document.documentElement;
    return el.scrollWidth - el.clientWidth;
  });
  expect(overflow, 'horizontal overflow (px) past the viewport').toBeLessThanOrEqual(2);
}

/**
 * Assert the main content pane actually has height — this is what catches the
 * "lower pane collapses to ~0" class of bugs (blank terminal, missing DB
 * results). `.center` is the content pane in both desktop and mobile shells.
 */
export async function expectContentHasHeight(page: Page, min = 120): Promise<void> {
  const box = await page.locator('.center').first().boundingBox();
  expect(box, 'content pane (.center) should be present').not.toBeNull();
  expect(box!.height, 'content pane height').toBeGreaterThan(min);
  expect(box!.width, 'content pane width').toBeGreaterThan(0);
}

/**
 * Assert a floating element (menu / dropdown / popover / typeahead) is FULLY
 * inside the viewport. This is the core check for the "popup rendered
 * off-screen / clipped" class of bugs: flip-style positioning (`top = y - h`)
 * goes negative when the popup is taller than the window, leaving the whole
 * thing unreachable. Popups with data-driven lists must be tested with enough
 * items to overflow the window (they should clamp + scroll internally) — see
 * desktop-git-add-menu.spec.ts.
 */
export async function expectFullyInViewport(
  page: Page,
  locator: import('@playwright/test').Locator,
  what = 'floating element',
): Promise<void> {
  await expect(locator).toBeVisible();
  const box = await locator.boundingBox();
  const viewport = page.viewportSize()!;
  expect(box, `${what} should render`).not.toBeNull();
  expect(box!.y, `${what} top must be inside the viewport`).toBeGreaterThanOrEqual(0);
  expect(box!.x, `${what} left must be inside the viewport`).toBeGreaterThanOrEqual(0);
  expect(box!.y + box!.height, `${what} bottom must be inside the viewport`).toBeLessThanOrEqual(
    viewport.height,
  );
  expect(box!.x + box!.width, `${what} right must be inside the viewport`).toBeLessThanOrEqual(
    viewport.width,
  );
}

/**
 * Run an axe-core accessibility scan. Fails on any `critical` violation; returns
 * the full violation list so callers can additionally inspect `serious` ones.
 */
export async function expectAccessible(
  page: Page,
): Promise<Awaited<ReturnType<AxeBuilder['analyze']>>['violations']> {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'])
    .analyze();
  const critical = results.violations.filter((v) => v.impact === 'critical');
  expect(
    critical,
    `critical a11y violations: ${critical.map((v) => v.id).join(', ')}`,
  ).toEqual([]);
  return results.violations;
}
