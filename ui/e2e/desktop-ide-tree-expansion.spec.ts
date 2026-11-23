import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// The project tree comes back exactly as the person left it: leaving the IDE and returning,
// a reload, hiding and showing the window, switching project and back, or typing then
// clearing the filter never opens a folder by itself — not even the folders of the open file.
// Collapsing a folder folds everything under it: opening it again shows its direct children only.
// Needs a daemon built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod).

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });

let root = '';
let other = '';

async function bootIde(page: Page, at = root): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.agentOpen', '0');
  });
  await page.goto(`/#/ide?root=${encodeURIComponent(at)}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(row(page, 'README.md', at)).toBeVisible({ timeout: 20_000 });
}

const row = (page: Page, name: string, at = root) => page.locator(`[data-tree-key="${join(at, name)}"]`);

async function toggle(page: Page, name: string, open: boolean): Promise<void> {
  const r = row(page, name);
  if ((await r.getAttribute('aria-expanded')) !== String(open)) await r.click();
  await expect(r).toHaveAttribute('aria-expanded', String(open));
}

/** The folders shown open, root excluded, as paths relative to the project. */
async function openFolders(page: Page): Promise<string[]> {
  const keys = await page.getByTestId('ide-project-tree').locator('[role="treeitem"][aria-expanded="true"]').evaluateAll((els) => els.map((e) => e.getAttribute('data-tree-key') ?? ''));
  return keys.filter((k) => !k.startsWith('root:')).map((k) => k.slice(root.length + 1)).sort();
}

/** Open a deep file (the active tab), then fold its folders away and leave `c` open. */
async function leaveTreeFolded(page: Page): Promise<void> {
  await toggle(page, 'a', true);
  await toggle(page, 'a/a1', true);
  await toggle(page, 'a/a1/a1x', true);
  await row(page, 'a/a1/a1x/deep.ts').click();
  await expect(page.locator('.cm-content').first()).toContainText('deep', { timeout: 10_000 });
  await toggle(page, 'b', true);
  await toggle(page, 'b/b1', true);
  await toggle(page, 'c', true);
  await toggle(page, 'a', false);
  await toggle(page, 'b', false);
  expect(await openFolders(page)).toEqual(['c']);
}

/** Give anything that would open folders by itself (reveal, restore, live refresh) time to. */
async function settle(page: Page): Promise<void> {
  await expect(row(page, 'README.md')).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(1_200); // ui-guards: allow — absence: no folder opens by itself
}

test.describe('Desktop IDE tree: folders stay as the person left them', () => {
  test.beforeAll(() => {
    root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-exp-')));
    for (const d of ['a/a1/a1x', 'a/a2', 'b/b1/b1x', 'c/c1']) mkdirSync(join(root, d), { recursive: true });
    writeFileSync(join(root, 'README.md'), '# expansion\n');
    writeFileSync(join(root, 'a', 'a1', 'a1x', 'deep.ts'), 'export const deep = 1;\n');
    writeFileSync(join(root, 'a', 'a2', 'x.ts'), 'export {};\n');
    writeFileSync(join(root, 'b', 'b1', 'b1x', 'y.ts'), 'export {};\n');
    writeFileSync(join(root, 'c', 'c1', 'z.ts'), 'export {};\n');
    other = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-exp-other-')));
    writeFileSync(join(other, 'README.md'), '# other\n');
  });

  test.afterAll(() => {
    for (const d of [root, other]) if (d) rmSync(d, { recursive: true, force: true });
  });

  test.beforeEach(({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  });

  test('collapsing a folder folds everything under it', async ({ page }) => {
    await bootIde(page);
    await toggle(page, 'a', true);
    await toggle(page, 'a/a1', true);
    await toggle(page, 'a/a1/a1x', true);
    await toggle(page, 'a', false);
    await toggle(page, 'a', true);
    await expect(row(page, 'a/a1')).toHaveAttribute('aria-expanded', 'false');
    await expect(row(page, 'a/a1/a1x')).toHaveCount(0);
    expect(await openFolders(page)).toEqual(['a']);
  });

  test('leaving the IDE and coming back', async ({ page }) => {
    await bootIde(page);
    await leaveTreeFolded(page);
    await page.evaluate(() => { location.hash = '#/home'; });
    await expect(page.getByTestId('ide-project-tree')).toHaveCount(0, { timeout: 10_000 });
    await page.evaluate(() => { location.hash = '#/ide'; });
    await settle(page);
    expect(await openFolders(page)).toEqual(['c']);
  });

  test('a reload', async ({ page }) => {
    await bootIde(page);
    await leaveTreeFolded(page);
    await page.reload();
    // The reload lands on the default project; opening this one again restores its tree.
    await page.evaluate((p) => { location.hash = `#/ide?root=${encodeURIComponent(p)}`; }, root);
    await settle(page);
    expect(await openFolders(page)).toEqual(['c']);
  });

  test('hiding and showing the window', async ({ page }) => {
    await bootIde(page);
    await leaveTreeFolded(page);
    for (const state of ['hidden', 'visible']) {
      await page.evaluate((s) => {
        Object.defineProperty(document, 'visibilityState', { value: s, configurable: true });
        document.dispatchEvent(new Event('visibilitychange'));
      }, state);
    }
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await settle(page);
    expect(await openFolders(page)).toEqual(['c']);
  });

  test('switching project and back', async ({ page }) => {
    await bootIde(page);
    await leaveTreeFolded(page);
    await page.evaluate((p) => { location.hash = `#/ide?root=${encodeURIComponent(p)}`; }, other);
    await expect(row(page, 'README.md', other)).toBeVisible({ timeout: 20_000 });
    await page.evaluate((p) => { location.hash = `#/ide?root=${encodeURIComponent(p)}`; }, root);
    await settle(page);
    expect(await openFolders(page)).toEqual(['c']);
  });

  test('typing then clearing the filter', async ({ page }) => {
    await bootIde(page);
    await leaveTreeFolded(page);
    const filter = page.getByTestId('ide-tree-filter');
    await filter.fill('deep');
    await expect(row(page, 'a/a1/a1x/deep.ts')).toBeVisible();
    await filter.fill('');
    await settle(page);
    expect(await openFolders(page)).toEqual(['c']);
  });
});
