import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, renameSync, rmSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// The project tree follows the folder on disk by itself: whatever creates, deletes or
// renames a file — the IDE's own New File, a terminal, an agent, git — every EXPANDED
// folder shows it within about a second, without collapsing anything, and the selection
// stays put. Needs a daemon built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod).
// IDE_TREE_SHOTS=<dir> saves before/after screenshots of the tree.

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });

const SHOTS = process.env.IDE_TREE_SHOTS;
// "About a second": the watcher's debounce plus one listing, with CI headroom.
const LIVE = { timeout: 4_000 };
let root = '';

function git(cwd: string, ...args: string[]): void {
  execFileSync('git', ['-c', 'user.email=e2e@example.com', '-c', 'user.name=E2E', ...args], { cwd, stdio: 'ignore' });
}

async function bootIde(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.agentOpen', '0');
  });
  await page.goto(`/#/ide?root=${encodeURIComponent(root)}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(row(page, 'README.md')).toBeVisible({ timeout: 20_000 });
}

const row = (page: Page, name: string) => page.locator(`[data-tree-key="${join(root, name)}"]`);
const tree = (page: Page) => page.getByTestId('ide-project-tree');

async function expand(page: Page, name: string): Promise<void> {
  const r = row(page, name);
  if ((await r.getAttribute('aria-expanded')) !== 'true') await r.click();
  await expect(r).toHaveAttribute('aria-expanded', 'true');
}

/** Expand `name` and wait until the daemon's watcher knows the folder is shown. */
async function expandWatched(page: Page, name: string): Promise<void> {
  const dir = join(root, name);
  const leased = page.waitForResponse((r) => r.url().includes('/ide/fs/watch') && (r.request().postDataJSON()?.dirs ?? []).includes(dir));
  await expand(page, name);
  await leased;
}

async function shot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await tree(page).screenshot({ path: join(SHOTS, name) });
}

test.describe('Desktop IDE tree: live refresh from disk', () => {
  test.beforeAll(() => {
    root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-live-')));
    mkdirSync(join(root, 'src', 'nested'), { recursive: true });
    mkdirSync(join(root, 'later'));
    writeFileSync(join(root, 'README.md'), '# live\n');
    writeFileSync(join(root, 'src', 'a.ts'), 'export const a = 1;\n');
    writeFileSync(join(root, 'src', 'b.ts'), 'export const b = 2;\n');
    writeFileSync(join(root, 'src', 'nested', 'n.ts'), 'export const n = 3;\n');
    writeFileSync(join(root, 'later', 'l.ts'), 'export const l = 4;\n');
    mkdirSync(join(root, 'many'));
    for (let i = 10; i < 90; i++) writeFileSync(join(root, 'many', `m${i}.ts`), 'export {};\n');
    git(root, 'init', '-q');
    git(root, 'add', '.');
    git(root, 'commit', '-q', '-m', 'init');
  });

  test.afterAll(() => {
    if (root) rmSync(root, { recursive: true, force: true });
  });

  test.beforeEach(({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  });

  test('the tree loads and follows the disk on a plain-http page (no crypto.randomUUID)', async ({ page }) => {
    // A phone reaching Otto over http:// is not a secure context: randomUUID does not exist there.
    await page.addInitScript(() => {
      Object.defineProperty(crypto, 'randomUUID', { value: undefined, configurable: true });
    });
    await bootIde(page);
    await expandWatched(page, 'src');
    writeFileSync(join(root, 'src', 'plain-http.ts'), 'export {};\n');
    await expect(row(page, 'src/plain-http.ts')).toBeVisible(LIVE);
  });

  test("the IDE's own New File shows in the expanded folder", async ({ page }) => {
    await bootIde(page);
    await expand(page, 'src'); // also selects it: New File lands inside
    await tree(page).getByTestId('ide-tree-new-file').click();
    const dialog = page.getByRole('dialog', { name: 'New file' });
    await dialog.getByRole('textbox').fill('own.ts');
    await dialog.getByRole('button', { name: 'Create' }).click();
    await expect(row(page, 'src/own.ts')).toBeVisible(LIVE);
  });

  test('a file created, deleted or renamed outside the IDE shows in expanded folders', async ({ page }) => {
    await bootIde(page);
    await expandWatched(page, 'src');
    await expect(row(page, 'src/a.ts')).toBeVisible();
    // The selection must survive a refresh.
    await row(page, 'README.md').click();
    await expect(row(page, 'README.md')).toHaveAttribute('aria-selected', 'true');
    await shot(page, 'tree-before.png');

    writeFileSync(join(root, 'src', 'outside.ts'), 'export {};\n');
    await expect(row(page, 'src/outside.ts')).toBeVisible(LIVE);
    writeFileSync(join(root, 'top-level.md'), 'x\n');
    await expect(row(page, 'top-level.md')).toBeVisible(LIVE);

    unlinkSync(join(root, 'src', 'a.ts'));
    await expect(row(page, 'src/a.ts')).toHaveCount(0, LIVE);
    renameSync(join(root, 'src', 'b.ts'), join(root, 'src', 'b-renamed.ts'));
    await expect(row(page, 'src/b-renamed.ts')).toBeVisible(LIVE);
    await expect(row(page, 'src/b.ts')).toHaveCount(0, LIVE);

    await expect(row(page, 'src')).toHaveAttribute('aria-expanded', 'true');
    await expect(row(page, 'README.md')).toHaveAttribute('aria-selected', 'true');
    await shot(page, 'tree-after.png');
  });

  test('nested expanded folders refresh; a collapsed one is fresh on expand', async ({ page }) => {
    await bootIde(page);
    await expand(page, 'src');
    await expandWatched(page, 'src/nested');
    mkdirSync(join(root, 'src', 'nested', 'deeper'));
    writeFileSync(join(root, 'src', 'nested', 'deep.ts'), 'export {};\n');
    await expect(row(page, 'src/nested/deep.ts')).toBeVisible(LIVE);
    await expect(row(page, 'src/nested/deeper')).toBeVisible(LIVE);

    // Seen once, then collapsed: the change lands while it is closed.
    await expand(page, 'later');
    await expect(row(page, 'later/l.ts')).toBeVisible();
    await row(page, 'later').click();
    await expect(row(page, 'later')).toHaveAttribute('aria-expanded', 'false');
    writeFileSync(join(root, 'later', 'while-closed.ts'), 'export {};\n');
    await row(page, 'later').click();
    await expect(row(page, 'later/while-closed.ts')).toBeVisible(LIVE);
  });

  test('a refresh keeps the scroll position', async ({ page }) => {
    await bootIde(page);
    await expandWatched(page, 'many');
    const scroller = tree(page).locator('.tree-scroll');
    await scroller.evaluate((el) => { el.scrollTop = 600; });
    await expect.poll(() => scroller.evaluate((el) => el.scrollTop)).toBe(600);
    const anchor = row(page, 'many/m50.ts');
    const before = (await anchor.boundingBox())!.y;
    writeFileSync(join(root, 'many', 'm00-first.ts'), 'export {};\n');
    writeFileSync(join(root, 'many', 'm99-last.ts'), 'export {};\n');
    await expect(row(page, 'many/m00-first.ts')).toHaveCount(1, LIVE);
    await expect(row(page, 'many/m99-last.ts')).toHaveCount(1, LIVE);
    // Not back at the top: the rows in view stay in view (a row inserted above may nudge them
    // by at most its own height).
    expect(await scroller.evaluate((el) => el.scrollTop)).toBeGreaterThanOrEqual(600);
    expect(Math.abs((await anchor.boundingBox())!.y - before)).toBeLessThanOrEqual(30);
    await expect(row(page, 'many')).toHaveAttribute('aria-expanded', 'true');
  });

  test('git checkout of another branch updates the tree', async ({ page }) => {
    git(root, 'add', '-A');
    git(root, 'commit', '-q', '--allow-empty', '-m', 'state');
    git(root, 'checkout', '-q', '-b', 'feature');
    writeFileSync(join(root, 'src', 'feature-only.ts'), 'export {};\n');
    git(root, 'add', '-A');
    git(root, 'commit', '-q', '-m', 'feature');
    git(root, 'checkout', '-q', '-');
    await bootIde(page);
    await expandWatched(page, 'src');
    await expect(row(page, 'src/feature-only.ts')).toHaveCount(0);
    git(root, 'checkout', '-q', 'feature');
    await expect(row(page, 'src/feature-only.ts')).toBeVisible(LIVE);
    git(root, 'checkout', '-q', '-');
    await expect(row(page, 'src/feature-only.ts')).toHaveCount(0, LIVE);
  });
});
