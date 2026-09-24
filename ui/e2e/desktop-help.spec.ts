import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow, runBarCommand } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Help (#/walkthroughs) — section guides + the tour film (desktop-browser).
//
// The default view is the film + Getting started (never an empty pane); an
// unreachable film shows its inline "can't load" state with Retry (network is
// stubbed so this never depends on GitHub); the rail search finds a guide by a
// shortcut; ↑/↓ walk the rail; #/walkthroughs/<id> deep-links; "Open <module>"
// routes; ⌘K has one "Guide: …" command per README.
// ─────────────────────────────────────────────────────────────────────────────

test.setTimeout(120_000);

let wsId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  // The film streams from the internet: make it unreachable, deterministically.
  await page.route('**/walkthroughs/resolve**', (r) => r.fulfill({ status: 502, body: '{}' }));
  // Vite's dev-mode `?import&raw` fetch of the captions module is app code,
  // not the film — blocking it blanks the whole page.
  await page.route(/otto-tour[^/]*\.(mp4|jpg|vtt)(\?(?!import)|$)/, (r) => r.abort('internetdisconnected'));
});

function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  return errors;
}

async function openHelp(page: Page, sub = ''): Promise<void> {
  await page.goto(`/#/walkthroughs${sub ? `/${sub}` : ''}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('[data-testid="page-header"] h1')).toHaveText('Help', { timeout: 20_000 });
}

const article = (page: Page) => page.getByTestId('guide-article');
const rail = (page: Page) => page.getByRole('navigation', { name: 'Guides' });

test('opens on the film + Getting started; an unreachable film shows Retry, not an error', async ({ page }) => {
  const errors = collectErrors(page);
  await openHelp(page);

  // Never an empty pane: Getting started is open under the film.
  await expect(article(page)).toHaveAttribute('data-guide-id', 'getting-started');
  await expect(article(page).locator('h2').first()).toHaveText('Getting started');
  await expect(rail(page).locator('[data-guide="getting-started"]')).toHaveAttribute('aria-current', 'page');

  // Rail grouped like the sidebar, Basics first.
  const labels = await rail(page).locator('.rail-group-label').allTextContents();
  expect(labels[0]).toBe('Basics');
  for (const g of ['Work', 'Automate', 'Build']) expect(labels).toContain(g);

  // The film area is there and lands on its inline unavailable state.
  const film = page.getByTestId('tour-film');
  await expect(film).toBeVisible();
  const unavailable = page.getByTestId('tour-film-unavailable');
  await expect(unavailable).toBeVisible({ timeout: 20_000 });
  await expect(unavailable).toContainText("can't load");
  await unavailable.getByRole('button', { name: 'Retry' }).click();
  await expect(page.getByTestId('tour-film-unavailable')).toBeVisible({ timeout: 20_000 });

  await expectNoHorizontalOverflow(page);
  expect(errors, errors.join('\n')).toEqual([]);
});

test('search finds a guide by a shortcut, and arrow keys walk the rail', async ({ page }) => {
  await openHelp(page, 'git');
  const search = page.getByTestId('guide-search');
  await search.fill('⌘K');
  const bar = rail(page).locator('[data-guide="command-bar"]');
  await expect(bar).toBeVisible();
  await expect(bar.locator('kbd').first()).toHaveText('⌘');
  // Words work too.
  await search.fill('cmd k');
  await expect(rail(page).locator('[data-guide="command-bar"]')).toBeVisible();
  // Enter opens the top hit.
  const top = await rail(page).locator('[data-guide]').first().getAttribute('data-guide');
  await search.press('Enter');
  await expect(page).toHaveURL(new RegExp(`#/walkthroughs/${top}$`));
  await expect(article(page)).toHaveAttribute('data-guide-id', top!);

  // Clearing shows the grouped rail again; ↓ from the search moves into it.
  await search.fill('');
  await search.press('ArrowDown');
  await expect(page).toHaveURL(/#\/walkthroughs\/getting-started$/);
  await page.keyboard.press('ArrowDown');
  await expect(page).toHaveURL(/#\/walkthroughs\/command-bar$/);
  await expect(article(page)).toHaveAttribute('data-guide-id', 'command-bar');
  await expect(rail(page).locator('[data-guide="command-bar"]')).toBeFocused();

  // A query with no match says so and offers to clear.
  await search.fill('zzqqxx nothing');
  await expect(rail(page)).toContainText('No guides match');
});

test('deep link opens a guide with kbd shortcut chips; Open <module> routes there', async ({ page }) => {
  const errors = collectErrors(page);
  await openHelp(page, 'git');
  await expect(article(page)).toHaveAttribute('data-guide-id', 'git');
  await expect(article(page).locator('h2').first()).toHaveText('Git');
  await expect(rail(page).locator('[data-guide="git"]')).toHaveAttribute('aria-current', 'page');
  // No film on a guide page until asked for.
  await expect(page.getByTestId('tour-film')).toHaveCount(0);

  // Related links are in-app deep links.
  await openHelp(page, 'keyboard-shortcuts');
  await expect(article(page).locator('.keys-table kbd').first()).toBeVisible();

  await openHelp(page, 'git');
  await page.getByTestId('guide-open-module').click();
  await expect(page).toHaveURL(/#\/git$/);
  await expect(page.locator('[data-testid="page-header"] h1')).not.toHaveText('Help');

  // An unknown id is an inline "not found" with a way back.
  await openHelp(page, 'no-such-guide');
  await expect(page.getByText('There\'s no guide called')).toBeVisible();
  await page.getByRole('button', { name: 'Open Getting started' }).click();
  await expect(article(page)).toHaveAttribute('data-guide-id', 'getting-started');
  expect(errors, errors.join('\n')).toEqual([]);
});

test('⌘K lists a Guide: command per README', async ({ page }) => {
  await openHelp(page, 'git');
  await runBarCommand(page, 'Guide: Vault');
  await expect(page).toHaveURL(/#\/walkthroughs\/vault$/);
  await expect(article(page)).toHaveAttribute('data-guide-id', 'vault');
});
