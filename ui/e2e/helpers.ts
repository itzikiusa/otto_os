import { expect, type Locator, type Page, type Request } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import AxeBuilder from '@axe-core/playwright';

// The top-level routable pages (hash router: #/<module>). `share` is excluded
// (it needs a scoped token) and is covered separately.
export const PAGES = [
  'agents',
  'rooms',
  'rooms/recaps',
  'home',
  'assistant',
  'mission-control',
  'api',
  'aws',
  'brokers',
  'connections',
  'database',
  'git',
  'walkthroughs',
  'insights',
  'kubernetes',
  'settings/plugins',
  'product',
  'settings',
  'skills-eval',
  'swarm',
  'usage',
  'vault',
  'workflows',
  'history',
  'run-with-otto',
  'loops',
  'proof',
  'canvas',
  'design',
  'design/brand',
  'design/learned',
  'browser',
  'mcp',
  'scheduled-tasks',
  'personal-agents',
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
 * Open the New Session sheet with ⌘T, falling back to the TabBar + button only
 * if the shortcut never reaches the app. The sheet is lazy-loaded
 * (`{#await import(...)}` in App.svelte), so it appears a beat after the key
 * press: an instant `isVisible()` check reads false, and the fallback click then
 * lands on the sheet's own backdrop once it mounts. Wait for the sheet first.
 */
export async function openNewSessionSheet(page: Page): Promise<Locator> {
  const dialog = page.locator('.sheet[role="dialog"][aria-label="New session"]');
  await page.keyboard.press('Meta+t');
  const opened = await dialog
    .waitFor({ state: 'visible', timeout: 10_000 })
    .then(() => true, () => false);
  if (!opened) await page.getByTitle('New session', { exact: true }).click();
  await expect(dialog).toBeVisible();
  return dialog;
}

/**
 * Open the API page and make sure the request editor is showing. A workspace
 * with nothing in it (no saved requests, history or edited tab) opens on the
 * "Create your first request" onboarding state; this clicks through it.
 */
export async function openApiEditor(page: Page): Promise<void> {
  await openPage(page, 'api');
  // The editor shows (aria-busy) while the workspace's lists are in flight; an
  // untouched empty workspace swaps it for onboarding once they settle. Decide
  // only after that, or the onboarding check races the swap.
  const url = page.getByLabel('Request URL', { exact: true });
  const onboarding = page.getByText('Create your first request', { exact: true });
  await expect(url.or(onboarding).first()).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.api-page[aria-busy="true"]')).toHaveCount(0, { timeout: 15_000 });
  if (await onboarding.isVisible()) {
    await page.getByRole('button', { name: 'New request', exact: true }).first().click();
  }
  await expect(url).toBeVisible();
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
  // Measure where the element SETTLES: an entrance animation (a drawer sliding
  // in, a menu popping in) moves it through off-screen positions on the way.
  await locator.evaluate((el) =>
    Promise.all(
      document
        .getAnimations()
        .filter((a) => a.playState === 'running' && a.effect instanceof KeyframeEffect && a.effect.target instanceof Element && a.effect.target.contains(el))
        .map((a) => a.finished.catch(() => undefined)),
    ),
  );
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
 * The DB explorer opens Mongo results in Vertical view by default (and any
 * engine's result past the auto-Vertical column threshold); grid-shaped
 * assertions opt back in with this. Waits for the results toolbar (the view
 * switch only exists once a result with columns is on screen), so it is safe to
 * call straight after Run; a no-op when the switch never appears (an error /
 * empty result — the caller's own assertion reports that) or already shows Grid.
 * Picking Grid is remembered on the tab, so later runs in the same tab stay Grid.
 */
export async function ensureGridView(page: Page, timeout = 20_000): Promise<void> {
  const seg = page.locator('.view-seg');
  const shown = await seg.waitFor({ state: 'visible', timeout }).then(
    () => true,
    () => false,
  );
  if (!shown) return;
  const on = await seg.locator('.vs.on').textContent();
  if ((on ?? '').trim() !== 'Grid') await seg.locator('.vs', { hasText: 'Grid' }).click();
  await expect(seg.locator('.vs.on')).toHaveText('Grid');
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

/**
 * Run a ⌘K command by title. On desktop ⌘K focuses the floating bar (the one
 * command surface — FloatingBar.svelte); the row is picked by its visible
 * title and the bar closes once the command runs.
 */
export async function runBarCommand(page: Page, title: string): Promise<void> {
  const input = page.getByRole('combobox', { name: 'Ask Otto or search commands' });
  await page.keyboard.press('Meta+k');
  await expect(input).toBeFocused({ timeout: 10_000 });
  await page.keyboard.type(title);
  await page
    .getByTestId('floating-bar')
    .getByRole('option')
    .filter({ hasText: title })
    .filter({ hasNotText: 'Ask Otto' })
    .first()
    .click();
  // Raw locator: once a terminal takes focus the bar docks and its input
  // leaves the accessibility tree.
  await expect(page.getByTestId('floating-bar').locator('input.input-main')).not.toBeFocused({
    timeout: 10_000,
  });
}

/** Select a right-panel tab by name. A narrow panel folds the tabs that
 *  don't fit into its "More panels" menu, so pick from there when needed. */
export async function openRightPanelTab(page: Page, name: string): Promise<void> {
  const panel = page.locator('.rpanel');
  await expect(panel.locator('.rtab').first()).toBeVisible();
  const tab = panel.getByRole('tab', { name, exact: true });
  if (await tab.isVisible()) {
    await tab.click();
  } else {
    await panel.getByRole('button', { name: 'More panels' }).click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name, exact: true }).click();
  }
  await expect(tab).toHaveAttribute('aria-selected', 'true');
}

/** The flattened PNG a snip annotated-save POSTs, as base64. The editor sends
 *  a raw `image/png` body (docs/contracts/api.md, `POST /snips/{id}/annotated`),
 *  not the legacy `{data_b64}` JSON — assert that framing, then hand back the
 *  bytes in the form the specs decode. */
export function snipAnnotatedPng(request: Request): string {
  expect(request.headers()['content-type']).toBe('image/png');
  const body = request.postDataBuffer();
  expect(body, 'annotated save must carry a body').not.toBeNull();
  expect(body!.subarray(0, 8).toString('hex'), 'annotated save must be a PNG').toBe('89504e470d0a1a0a');
  return body!.toString('base64');
}

/** The tour film's chapter `title` as the Help page labels its button
 *  (`m:ss Title`, `timeLabel` in src/modules/help/guide.ts) plus its start in
 *  seconds — read from the bundled film.json so the playback specs follow a
 *  re-cut film instead of pinning the old timestamps. */
export function tourChapter(title: string): { label: string; start: number } {
  const film = JSON.parse(readFileSync(join(process.cwd(), 'src/lib/walkthroughs/film.json'), 'utf8')) as {
    chapters: { title: string; start: number }[];
  };
  const chapter = film.chapters.find((c) => c.title === title);
  if (!chapter) throw new Error(`film.json has no chapter titled ${JSON.stringify(title)}`);
  const s = Math.floor(chapter.start);
  return { label: `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')} ${title}`, start: chapter.start };
}

/** Path of a real tour MP4 (OTTO_E2E_TOUR_VIDEO — CI generates a synthetic
 *  one); playback specs skip without it. */
export const tourVideoPath = process.env.OTTO_E2E_TOUR_VIDEO;

/**
 * The select-all chord CodeMirror binds for THIS page. CodeMirror's `Mod` is
 * Meta on Mac or iOS — the same test as @codemirror/view's `browser.mac`
 * (`/Mac/` platform, or an Apple-vendor engine with a `Mobile/` UA or touch
 * points) — Control otherwise. Playwright's `ControlOrMeta` follows the HOST OS instead, so on a
 * Linux runner it sent Control+A to the iPhone (WebKit) project, which
 * CodeMirror ignores — 19 iPhone DB tests failed on that alone (S12-304).
 */
export async function editorSelectAll(page: Page): Promise<string> {
  const apple = await page.evaluate(() => {
    const ios =
      /Apple Computer/.test(navigator.vendor) &&
      (/Mobile\/\w+/.test(navigator.userAgent) || navigator.maxTouchPoints > 2);
    return ios || /Mac/.test(navigator.platform);
  });
  return apple ? 'Meta+A' : 'Control+A';
}
