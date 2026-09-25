import { test, expect, type Locator, type Page } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Sidebar Favorites + reorderable sections (desktop-browser only).
//
//   • a module row's context menu favorites it; Favorites is the FIRST section,
//     holds the favorites in the order added, and a favorite is listed ONLY
//     there (it leaves its own section) — no empty Favorites section otherwise;
//   • favorites reorder by drag (outside edit mode too) and ⌥↑ / ⌥↓, persisted
//     per device across a reload; the collapsed Rail lists them first;
//   • unfavoriting puts a module back in its own section;
//   • edit mode: a star toggle (aria-pressed) per row, arrows inside Favorites;
//     a saved favorite the user can't see (unknown id) is skipped;
//   • sections reorder (edit-mode arrows + the header context menu), persist,
//     and "Reset to default" restores the order and clears favorites;
//   • ⌘K toggles the current page in Favorites.
//
// GET /plugins is stubbed per test (see desktop-sidebar-groups.spec.ts: other
// specs leave plugins installed on the shared daemon, which would add a
// Plugins section to every count here).
// ─────────────────────────────────────────────────────────────────────────────

const DEFAULT_SECTIONS = ['work', 'automate', 'build', 'infra', 'insight'];

test.use({ serviceWorkers: 'block' });

/** Fresh sidebar config + expanded Navigator, ONCE per test (init scripts
 *  re-run on reload; the sessionStorage flag keeps the reset one-shot so a
 *  reload inside a test sees what the test persisted). `seed` pre-sets keys. */
async function boot(page: Page, route: string, seed: Record<string, string> = {}): Promise<void> {
  await page.context().route(/\/api\/v1\/plugins(\?|$)/, (r) =>
    r.request().method() === 'GET' ? r.fulfill({ json: [] }) : r.fallback(),
  );
  await page.addInitScript((s) => {
    if (sessionStorage.getItem('fav-reset')) return;
    sessionStorage.setItem('fav-reset', '1');
    for (const k of [
      'otto_sidebar_order',
      'otto_sidebar_hidden',
      'otto_sidebar_groups_collapsed',
      'otto_sidebar_favorites',
      'otto_sidebar_group_order',
    ]) {
      localStorage.removeItem(k);
    }
    localStorage.setItem('otto_rail_expanded', '1');
    for (const [k, v] of Object.entries(s)) localStorage.setItem(k, v);
  }, seed);
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 15_000 });
}

const modules = (page: Page) => page.locator('.navigator [data-testid="sidebar-modules"]');
const group = (page: Page, id: string) => page.getByTestId(`sidebar-group-${id}`);
const head = (page: Page, id: string) => page.getByTestId(`sidebar-group-head-${id}`);
/** A module row by id — count chips (working sessions, running workflows)
 *  make the accessible name unstable on a shared daemon. */
const rowIn = (scope: Locator, id: string) => scope.locator(`[data-nav-id="${id}"]`);
const navRow = (page: Page, id: string) => rowIn(modules(page), id);
const menu = (page: Page) => page.locator('.ctx-menu');

const favIds = (page: Page) =>
  group(page, 'favorites')
    .locator('[data-nav-id]')
    .evaluateAll((els) => els.map((e) => e.getAttribute('data-nav-id')!));
const sectionIds = (page: Page) =>
  page
    .locator('[data-testid^="sidebar-group-"][data-open]')
    .evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.replace('sidebar-group-', '')));
const ls = (page: Page, key: string) =>
  page.evaluate((k) => JSON.parse(localStorage.getItem(k) || '[]') as string[], key);

async function favoriteFromMenu(page: Page, id: string): Promise<void> {
  await navRow(page, id).click({ button: 'right' });
  await menu(page).getByRole('menuitem', { name: 'Add to Favorites' }).click();
  await expect(menu(page)).toHaveCount(0);
}

async function unfavoriteFromMenu(page: Page, id: string): Promise<void> {
  await navRow(page, id).click({ button: 'right' });
  await menu(page).getByRole('menuitem', { name: 'Remove from Favorites' }).click();
  await expect(menu(page)).toHaveCount(0);
}

/** HTML5 DnD through explicit events sharing one DataTransfer (see
 *  desktop-pane-reorder.spec.ts — `locator.dragTo` is flaky for these). */
async function drag(page: Page, source: Locator, target: Locator): Promise<void> {
  const dt = await page.evaluateHandle(() => new DataTransfer());
  await source.dispatchEvent('dragstart', { dataTransfer: dt });
  await target.dispatchEvent('dragover', { dataTransfer: dt });
  await target.dispatchEvent('drop', { dataTransfer: dt });
  await source.dispatchEvent('dragend', { dataTransfer: dt });
  await dt.dispose();
}

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop Navigator/Rail surface');
});

test('favorite from the row menu → first section, only there; reorder persists; unfavorite restores', async ({ page }) => {
  await boot(page, 'home');
  // No favorites → no (empty) Favorites section.
  await expect(group(page, 'favorites')).toHaveCount(0);
  expect(await sectionIds(page)).toEqual(DEFAULT_SECTIONS);

  const FAVS = ['agents', 'workflows', 'git', 'connections'];
  for (const id of FAVS) await favoriteFromMenu(page, id);

  // Favorites is first, labelled, and holds them in the order added.
  expect(await sectionIds(page)).toEqual(['favorites', ...DEFAULT_SECTIONS]);
  await expect(page.getByRole('group', { name: 'Favorites' })).toBeVisible();
  await expect(head(page, 'favorites')).toContainText('Favorites');
  expect(await favIds(page)).toEqual(FAVS);
  expect(await ls(page, 'otto_sidebar_favorites')).toEqual(FAVS);

  // …and each is listed ONLY there: gone from its own section.
  for (const [id, sec] of [
    ['agents', 'work'],
    ['workflows', 'automate'],
    ['git', 'build'],
    ['connections', 'infra'],
  ] as const) {
    await expect(rowIn(group(page, sec), id)).toHaveCount(0);
    await expect(navRow(page, id)).toHaveCount(1);
  }
  // A favorite still navigates and highlights.
  await navRow(page, 'git').click();
  await expect.poll(() => page.evaluate(() => window.location.hash)).toBe('#/git');
  await expect(navRow(page, 'git')).toHaveClass(/active/);

  // Reorder by drag (outside edit mode, like Finder's Favorites):
  // Connections onto Agents → lands before it.
  await drag(page, navRow(page, 'connections'), navRow(page, 'agents'));
  expect(await favIds(page)).toEqual(['connections', 'agents', 'workflows', 'git']);

  // …and from the keyboard: ⌥↑ on Git; focus stays on the moved row.
  await navRow(page, 'git').focus();
  await page.keyboard.press('Alt+ArrowUp');
  expect(await favIds(page)).toEqual(['connections', 'agents', 'git', 'workflows']);
  await expect(navRow(page, 'git')).toBeFocused();

  // The row menu moves too (and bounds at the ends).
  await navRow(page, 'connections').click({ button: 'right' });
  await expect(menu(page).getByRole('menuitem', { name: 'Move up' })).toBeDisabled();
  await menu(page).getByRole('menuitem', { name: 'Move down' }).click();
  const ORDERED = ['agents', 'connections', 'git', 'workflows'];
  expect(await favIds(page)).toEqual(ORDERED);

  // Persisted per device.
  await page.reload();
  await expect(page.locator('.navigator')).toBeVisible();
  expect(await favIds(page)).toEqual(ORDERED);
  expect(await ls(page, 'otto_sidebar_favorites')).toEqual(ORDERED);

  // The collapsed Rail lists the favorites first, then a separator.
  await page.locator('.navigator').getByRole('button', { name: 'Collapse sidebar' }).click();
  const rail = page.locator('.rail');
  await expect(rail).toBeVisible();
  const railLabels = await rail
    .locator('.rail-modules > *')
    .evaluateAll((els) => els.slice(0, 5).map((e) => (e.matches('[data-testid="rail-sep"]') ? '|' : e.getAttribute('aria-label'))));
  expect(railLabels).toEqual(['Agents', 'Connections', 'Git', 'Workflows', '|']);
  await expect(page.getByTestId('rail-sep')).toHaveCount(DEFAULT_SECTIONS.length);
  await rail.getByRole('button', { name: 'Expand sidebar' }).click();
  await expect(page.locator('.navigator')).toBeVisible();

  // Unfavorite → back in its own section; the last one removes the section.
  await unfavoriteFromMenu(page, 'git');
  await expect(rowIn(group(page, 'build'), 'git')).toHaveCount(1);
  expect(await favIds(page)).toEqual(['agents', 'connections', 'workflows']);
  for (const id of ['agents', 'connections', 'workflows']) await unfavoriteFromMenu(page, id);
  await expect(group(page, 'favorites')).toHaveCount(0);
  await expect(rowIn(group(page, 'work'), 'agents')).toHaveCount(1);
  expect(await ls(page, 'otto_sidebar_favorites')).toEqual([]);
});

test('edit mode: star toggles, arrows inside Favorites, unknown saved ids skipped', async ({ page }) => {
  await boot(page, 'home', { otto_sidebar_favorites: JSON.stringify(['no-such-module', 'vault']) });
  // The unknown id is skipped, never rendered.
  expect(await favIds(page)).toEqual(['vault']);

  await page.getByTestId('sidebar-edit-toggle').click();
  const star = (id: string) => page.getByTestId(`sidebar-fav-${id}`);
  await expect(star('vault')).toHaveAttribute('aria-pressed', 'true');
  await expect(star('usage')).toHaveAttribute('aria-pressed', 'false');

  await star('usage').click();
  await expect(star('usage')).toHaveAttribute('aria-pressed', 'true');
  await expect(star('usage')).toBeFocused(); // focus follows the row into Favorites
  const favEditRows = () =>
    group(page, 'favorites')
      .locator('[data-testid^="sidebar-edit-row-"]')
      .evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.replace('sidebar-edit-row-', '')));
  expect(await favEditRows()).toEqual(['vault', 'usage']);
  await expect(page.getByTestId('sidebar-edit-row-usage')).toHaveCount(1); // only once
  await expect(group(page, 'insight').getByTestId('sidebar-edit-row-usage')).toHaveCount(0);

  await expect(page.getByRole('button', { name: 'Move Vault up' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Move Usage down' })).toBeDisabled();
  await page.getByRole('button', { name: 'Move Usage up' }).click();
  expect(await favEditRows()).toEqual(['usage', 'vault']);
  // The unknown id kept its saved slot (it returns if it ever becomes available).
  expect(await ls(page, 'otto_sidebar_favorites')).toContain('no-such-module');

  // Favorites never gets section arrows; it's always first.
  await expect(page.getByRole('button', { name: 'Move Favorites section up' })).toHaveCount(0);

  await star('vault').click();
  await page.getByTestId('sidebar-edit-toggle').click();
  expect(await favIds(page)).toEqual(['usage']);
  await expect(rowIn(group(page, 'build'), 'vault')).toHaveCount(1);
});

test('sections reorder from edit mode and the header menu; persist; reset restores', async ({ page }) => {
  await boot(page, 'home');
  await page.getByTestId('sidebar-edit-toggle').click();
  await expect(page.getByRole('button', { name: 'Move Work section up' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Move Insight section down' })).toBeDisabled();

  await page.getByRole('button', { name: 'Move Insight section up' }).click();
  expect(await sectionIds(page)).toEqual(['work', 'automate', 'build', 'insight', 'infra']);
  await expect(page.getByRole('button', { name: 'Move Insight section up' })).toBeFocused();
  await page.getByTestId('sidebar-edit-toggle').click();

  // Header context menu (outside edit mode).
  await head(page, 'work').click({ button: 'right' });
  await expect(menu(page).getByRole('menuitem', { name: 'Move section up' })).toBeDisabled();
  await menu(page).getByRole('menuitem', { name: 'Move section down' }).click();
  const MOVED = ['automate', 'work', 'build', 'insight', 'infra'];
  expect(await sectionIds(page)).toEqual(MOVED);

  // Favorites stays first whatever the section order.
  await favoriteFromMenu(page, 'git');
  expect(await sectionIds(page)).toEqual(['favorites', ...MOVED]);
  await head(page, 'favorites').click({ button: 'right' });
  await expect(menu(page).getByRole('menuitem', { name: 'Move section up' })).toHaveCount(0);
  await page.keyboard.press('Escape');

  // Persisted.
  await page.reload();
  await expect(page.locator('.navigator')).toBeVisible();
  expect(await sectionIds(page)).toEqual(['favorites', ...MOVED]);
  expect((await ls(page, 'otto_sidebar_group_order')).slice(0, 5)).toEqual(MOVED);

  // Reset restores the shipped section order and clears favorites.
  await page.getByTestId('sidebar-edit-toggle').click();
  await page.getByTestId('sidebar-reset').click();
  await page.getByTestId('sidebar-edit-toggle').click();
  expect(await sectionIds(page)).toEqual(DEFAULT_SECTIONS);
  expect(await ls(page, 'otto_sidebar_favorites')).toEqual([]);
  expect(await ls(page, 'otto_sidebar_group_order')).toEqual([]);
});

test('⌘K adds / removes the current page from Favorites', async ({ page }) => {
  await boot(page, 'vault');
  await page.keyboard.press('Meta+k');
  await page.keyboard.type('add vault to favorites');
  await page.getByTestId('floating-bar').getByRole('option').filter({ hasText: 'Add Vault to Favorites' }).first().click();
  expect(await favIds(page)).toEqual(['vault']);
  await expect(navRow(page, 'vault')).toHaveClass(/active/);

  await page.keyboard.press('Meta+k');
  await page.keyboard.type('remove vault from favorites');
  await page.getByTestId('floating-bar').getByRole('option').filter({ hasText: 'Remove Vault from Favorites' }).first().click();
  await expect(group(page, 'favorites')).toHaveCount(0);
});
