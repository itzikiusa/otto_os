import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// Project tree, GoLand-style, on a seeded copy of a Go repo's shape (sinatra-common-go):
// git-ignored files are dimmed and announced; go.sum nests under go.mod, the filter still
// finds it, and opening go.sum unfolds go.mod. Needs a daemon built from this tree
// (OTTO_E2E_BIN=<repo>/target/debug/ottod): the `ignored` mark is new.
// IDE_TREE_SHOTS=<dir> saves light/dark screenshots of the tree.

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });
test.describe.configure({ mode: 'serial' });

const SHOTS = process.env.IDE_TREE_SHOTS;
let root = '';

function git(cwd: string, ...args: string[]): void {
  execFileSync('git', ['-c', 'user.email=e2e@example.com', '-c', 'user.name=E2E', ...args], { cwd, stdio: 'ignore' });
}

async function bootIde(page: Page, file?: string): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.agentOpen', '0');
  });
  const q = `root=${encodeURIComponent(root)}${file ? `&file=${encodeURIComponent(join(root, file))}` : ''}`;
  await page.goto(`/#/ide?${q}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
}

const row = (page: Page, name: string) => page.locator(`[data-tree-key="${join(root, name)}"]`);

test.describe('Desktop IDE tree: ignored files + file nesting', () => {
  test.beforeAll(() => {
    root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-tree-')));
    mkdirSync(join(root, 'pkg'));
    mkdirSync(join(root, 'out'));
    writeFileSync(join(root, '.gitignore'), '*.iml\nout/\n');
    writeFileSync(join(root, 'go.mod'), 'module example.com/common\n\ngo 1.22\n');
    writeFileSync(join(root, 'go.sum'), 'example.com/dep v1.0.0 h1:abc=\n');
    writeFileSync(join(root, 'sinatra-common-go.iml'), '<module/>\n');
    writeFileSync(join(root, 'README.md'), '# common\n');
    writeFileSync(join(root, 'pkg', 'util.go'), 'package pkg\n');
    writeFileSync(join(root, 'out', 'build.log'), 'ok\n');
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

  test('ignored entries are dimmed and announced; tracked ones are not', async ({ page }) => {
    await bootIde(page);
    const iml = row(page, 'sinatra-common-go.iml');
    await expect(iml).toBeVisible({ timeout: 20_000 });
    await expect(iml).toHaveAttribute('data-ignored', 'true');
    await expect(iml.locator('.tree-name')).toHaveClass(/is-ignored/);
    await expect(iml).toHaveAccessibleName(/ignored by \.gitignore/);
    await expect(iml).toHaveAttribute('title', /ignored by \.gitignore/);
    // An ignored folder dims too; a tracked file does not.
    await expect(row(page, 'out')).toHaveAttribute('data-ignored', 'true');
    await expect(row(page, 'README.md')).not.toHaveAttribute('data-ignored', 'true');
    const dim = await iml.locator('.tree-name').evaluate((el) => getComputedStyle(el).color);
    const normal = await row(page, 'README.md').locator('.tree-name').evaluate((el) => getComputedStyle(el).color);
    expect(dim).not.toBe(normal);
  });

  test('go.sum nests under go.mod, expands, and the filter finds it', async ({ page }) => {
    await bootIde(page);
    const mod = row(page, 'go.mod');
    await expect(mod).toBeVisible({ timeout: 20_000 });
    await expect(mod).toHaveAttribute('aria-expanded', 'false');
    await expect(row(page, 'go.sum')).toHaveCount(0);

    await mod.getByTestId('ide-tree-nest-toggle').click();
    await expect(mod).toHaveAttribute('aria-expanded', 'true');
    const sum = row(page, 'go.sum');
    await expect(sum).toBeVisible();
    const level = Number(await mod.getAttribute('aria-level'));
    await expect(sum).toHaveAttribute('aria-level', String(level + 1));
    // Clicking the chevron folds; it does not open go.mod.
    await expect(mod).toHaveAttribute('aria-selected', 'false');

    if (SHOTS) {
      mkdirSync(SHOTS, { recursive: true });
      const tree = page.getByTestId('ide-project-tree');
      for (const scheme of ['light', 'dark'] as const) {
        await page.emulateMedia({ colorScheme: scheme });
        await tree.screenshot({ path: join(SHOTS, `ide-tree-${scheme}.png`) });
      }
      await page.emulateMedia({ colorScheme: 'light' });
    }

    // ← on go.mod folds it from the keyboard.
    await mod.focus();
    await page.keyboard.press('ArrowLeft');
    await expect(mod).toHaveAttribute('aria-expanded', 'false');
    await expect(row(page, 'go.sum')).toHaveCount(0);

    await page.getByTestId('ide-tree-filter').fill('go.sum');
    await expect(row(page, 'go.sum')).toBeVisible();
    await expect(row(page, 'go.mod')).toBeVisible(); // kept as its context
    await expect(row(page, 'README.md')).toHaveCount(0);
  });

  test('opening go.sum reveals it under go.mod', async ({ page }) => {
    await bootIde(page, 'go.sum');
    await expect(page.locator('.ide-tab').first()).toBeVisible({ timeout: 20_000 });
    const sum = row(page, 'go.sum');
    await expect(sum).toBeVisible();
    await expect(sum).toHaveAttribute('aria-selected', 'true');
    await expect(row(page, 'go.mod')).toHaveAttribute('aria-expanded', 'true');
  });
});
