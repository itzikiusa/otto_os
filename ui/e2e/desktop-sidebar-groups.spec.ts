import { test, expect, type Page } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Sidebar sections (desktop-browser only): the Navigator groups its modules
// into macOS source-list sections (Work / Automate / Build / Infrastructure /
// Insight), each foldable from its header with the fold state persisted per
// device. Covered here:
//   • sections render in order and every module sits in exactly one;
//   • folding persists across a reload;
//   • the section holding the current page is never shown folded (navigating
//     into a folded section unfolds it; its header can't fold it);
//   • the active row is scrolled into view when the list overflows;
//   • "Customize sidebar" interplay — moves stay in-section, hidden stays
//     hidden, a section whose modules are all hidden disappears;
//   • the collapsed Rail draws a separator between sections and no two
//     modules share a glyph;
//   • ⌘K "Go to …" is derived from the registry (Vault / Workflows / Mission
//     Control reachable, with the section as secondary text).
// ─────────────────────────────────────────────────────────────────────────────

const ORDER = ['work', 'automate', 'build', 'infra', 'insight'];

/** Fresh sidebar config + expanded Navigator, ONCE per test (init scripts
 *  re-run on reload; the sessionStorage flag keeps the reset one-shot so a
 *  reload inside a test sees what the test persisted). */
async function boot(page: Page, route: string, rail: '1' | '0' = '1'): Promise<void> {
  await page.addInitScript((r) => {
    if (sessionStorage.getItem('sg-reset')) return;
    sessionStorage.setItem('sg-reset', '1');
    for (const k of ['otto_sidebar_order', 'otto_sidebar_hidden', 'otto_sidebar_groups_collapsed']) {
      localStorage.removeItem(k);
    }
    localStorage.setItem('otto_rail_expanded', r);
  }, rail);
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  if (rail === '1') await expect(page.locator('.navigator')).toBeVisible({ timeout: 15_000 });
}

const group = (page: Page, id: string) => page.getByTestId(`sidebar-group-${id}`);
const head = (page: Page, id: string) => page.getByTestId(`sidebar-group-head-${id}`);
const row = (page: Page, label: string) =>
  page.locator('.navigator [data-testid="sidebar-modules"]').getByRole('button', { name: label, exact: true });
const collapsedLs = (page: Page) =>
  page.evaluate(() => JSON.parse(localStorage.getItem('otto_sidebar_groups_collapsed') || '[]') as string[]);

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop Navigator/Rail surface');
});

test('sections render in order, each module in exactly one', async ({ page }) => {
  await boot(page, 'home');
  const ids = await page
    .locator('[data-testid^="sidebar-group-"][data-open]')
    .evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.replace('sidebar-group-', '')));
  expect(ids).toEqual(ORDER);
  await expect(head(page, 'infra')).toContainText('Infrastructure');

  await expect(group(page, 'work').getByRole('button', { name: 'Mission Control', exact: true })).toBeVisible();
  await expect(group(page, 'automate').getByRole('button', { name: 'Workflows', exact: true })).toBeVisible();
  await expect(group(page, 'build').getByRole('button', { name: 'Vault', exact: true })).toBeVisible();
  await expect(group(page, 'infra').getByRole('button', { name: 'Connections', exact: true })).toBeVisible();
  await expect(group(page, 'insight').getByRole('button', { name: 'Usage', exact: true })).toBeVisible();

  // No module label appears in two sections.
  const labels = await page
    .locator('[data-testid="sidebar-modules"] .nav-group > .nav-item, [data-testid="sidebar-modules"] .nav-group > .nav-item-row > .nav-item')
    .evaluateAll((els) => els.map((e) => e.textContent!.trim()));
  expect(new Set(labels).size).toBe(labels.length);

  // Selection is the accent tint, never the old hard-coded lime.
  await row(page, 'Home').waitFor();
  const bg = await row(page, 'Home').evaluate((el) => getComputedStyle(el).backgroundColor);
  expect(bg).not.toBe('rgb(126, 231, 135)');
});

test('folding a section persists across reload', async ({ page }) => {
  await boot(page, 'home');
  await expect(row(page, 'Usage')).toBeVisible();
  await head(page, 'insight').click();
  await expect(row(page, 'Usage')).toHaveCount(0);
  await expect(group(page, 'insight')).toHaveAttribute('data-open', 'false');
  await expect(head(page, 'insight')).toHaveAttribute('aria-expanded', 'false');
  expect(await collapsedLs(page)).toContain('insight');

  await page.reload();
  await expect(page.locator('.navigator')).toBeVisible();
  await expect(group(page, 'insight')).toHaveAttribute('data-open', 'false');
  await expect(row(page, 'Usage')).toHaveCount(0);

  await head(page, 'insight').click();
  await expect(row(page, 'Usage')).toBeVisible();
  expect(await collapsedLs(page)).not.toContain('insight');
});

test('the active section is never folded; navigating into a folded one opens it', async ({ page }) => {
  await boot(page, 'home');
  await head(page, 'build').click();
  await expect(row(page, 'Vault')).toHaveCount(0);

  await page.goto('/#/vault');
  await expect(group(page, 'build')).toHaveAttribute('data-open', 'true');
  await expect(row(page, 'Vault')).toHaveClass(/active/);
  // The unfold is persisted, not just displayed.
  await expect.poll(() => collapsedLs(page)).not.toContain('build');

  // Its header can't fold it while it holds the current page.
  await head(page, 'build').click();
  await expect(group(page, 'build')).toHaveAttribute('data-open', 'true');
  await expect(row(page, 'Vault')).toBeVisible();
  expect(await collapsedLs(page)).not.toContain('build');
});

test('the active row is scrolled into view when the list overflows', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 560 });
  await boot(page, 'home');
  const scroller = page.locator('.navigator .nav-scroll');
  const inView = async (label: string) => {
    const s = (await scroller.boundingBox())!;
    const r = (await row(page, label).boundingBox())!;
    return r.y >= s.y - 1 && r.y + r.height <= s.y + s.height + 1;
  };
  // The list is taller than the scroller at this height.
  expect(await scroller.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
  expect(await inView('Usage')).toBe(false);

  // Navigate from outside the sidebar (hash change) → Usage scrolls into view.
  await page.evaluate(() => (window.location.hash = '#/usage'));
  await expect(row(page, 'Usage')).toHaveClass(/active/);
  await expect.poll(() => inView('Usage')).toBe(true);

  // …and back to the top of the list.
  await page.evaluate(() => (window.location.hash = '#/home'));
  await expect.poll(() => inView('Home')).toBe(true);
});

test('customize: moves stay in-section, hidden stays hidden, empty sections vanish', async ({ page }) => {
  await boot(page, 'home');
  // A folded section still lists every row while editing.
  await head(page, 'automate').click();
  await expect(row(page, 'Swarm')).toHaveCount(0);
  await page.getByTestId('sidebar-edit-toggle').click();
  await expect(page.getByTestId('sidebar-edit-row-swarm')).toBeVisible();

  // First row of a section can't move up (no crossing into the previous one).
  await expect(page.getByRole('button', { name: 'Move Swarm up' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Move Home up' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Move Usage down' })).toBeDisabled();

  // In-section move: Workflows above Goal Loops.
  const automateRows = () =>
    group(page, 'automate')
      .locator('[data-testid^="sidebar-edit-row-"]')
      .evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.replace('sidebar-edit-row-', '')));
  expect((await automateRows()).slice(0, 3)).toEqual(['swarm', 'loops', 'workflows']);
  await page.getByRole('button', { name: 'Move Workflows up' }).click();
  expect((await automateRows()).slice(0, 3)).toEqual(['swarm', 'workflows', 'loops']);

  // Hide both Insight modules → that section disappears outside edit mode.
  await page.getByTestId('sidebar-hide-insights').click();
  await page.getByTestId('sidebar-hide-usage').click();
  await expect(group(page, 'insight')).toBeVisible(); // still listed while editing
  await page.getByTestId('sidebar-edit-toggle').click();
  await expect(group(page, 'insight')).toHaveCount(0);
  // The fold survived edit mode.
  await expect(group(page, 'automate')).toHaveAttribute('data-open', 'false');

  // Persisted: survives a reload, and the Rail agrees (no Insight icons).
  await page.reload();
  await expect(page.locator('.navigator')).toBeVisible();
  await expect(group(page, 'insight')).toHaveCount(0);
  await head(page, 'automate').click();
  const order = await group(page, 'automate')
    .locator('.nav-item')
    .evaluateAll((els) => els.map((e) => e.textContent!.trim()));
  expect(order.slice(0, 3)).toEqual(['Swarm', 'Workflows', 'Goal Loops']);
  await page.locator('.navigator').getByRole('button', { name: 'Collapse sidebar' }).click();
  await expect(page.locator('.rail')).toBeVisible();
  await expect(page.locator('.rail').getByRole('button', { name: 'Usage', exact: true })).toHaveCount(0);
  await expect(page.getByTestId('rail-sep')).toHaveCount(3); // 4 sections left

  // Reset restores everything (and unfolds).
  await page.locator('.rail').getByRole('button', { name: 'Expand sidebar' }).click();
  await page.getByTestId('sidebar-edit-toggle').click();
  await page.getByTestId('sidebar-reset').click();
  await page.getByTestId('sidebar-edit-toggle').click();
  await expect(row(page, 'Usage')).toBeVisible();
  expect(await collapsedLs(page)).toEqual([]);
});

test('collapsed Rail: section separators, unique glyphs, accent selection', async ({ page }) => {
  await boot(page, 'vault', '0');
  const rail = page.locator('.rail');
  await expect(rail).toBeVisible();
  await expect(page.getByTestId('rail-sep')).toHaveCount(ORDER.length - 1);
  const paths = await rail
    .locator('.rail-modules .rail-btn svg path')
    .evaluateAll((els) => els.map((e) => e.getAttribute('d')));
  expect(paths.length).toBeGreaterThan(20);
  expect(new Set(paths).size).toBe(paths.length);
  const vault = rail.getByRole('button', { name: 'Vault', exact: true });
  await expect(vault).toHaveClass(/active/);
  // Settings + account stay on screen below the (scrolling) module column.
  const gear = rail.getByRole('button', { name: 'Settings' });
  const box = (await gear.boundingBox())!;
  expect(box.y + box.height).toBeLessThanOrEqual(page.viewportSize()!.height);
});

test('⌘K reaches Vault, Workflows and Mission Control (derived from the registry)', async ({ page }) => {
  await boot(page, 'home');
  for (const [label, route, section] of [
    ['Vault', 'vault', 'Build'],
    ['Workflows', 'workflows', 'Automate'],
    ['Mission Control', 'mission-control', 'Work'],
  ] as const) {
    await page.keyboard.press('Meta+k');
    await page.keyboard.type(`go to ${label}`);
    const item = page.locator('.pal-item', { hasText: `Go to ${label}` }).first();
    await expect(item).toBeVisible();
    await expect(item.locator('.pal-detail')).toHaveText(section);
    await item.click();
    await expect.poll(() => page.evaluate(() => window.location.hash)).toBe(`#/${route}`);
    await expect(row(page, label)).toHaveClass(/active/);
  }
  // Tools no longer masquerade as session commands.
  await page.keyboard.press('Meta+k');
  await page.keyboard.type('update all clis');
  await expect(page.locator('.pal-item', { hasText: 'Update all CLIs' }).first().locator('.pal-group')).toHaveText('Tools');
});
