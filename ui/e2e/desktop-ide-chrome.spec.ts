import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { bootPolyglotIde, daemonCall, editorContent, openPolyglotFile, type PolyglotWorkspaceSeed } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// IDE editor chrome (round 5, review I1): ONE path crumbs row above the code (no second
// breadcrumb bar under it), one ⋯ instead of a strip of unlabelled icons, a status strip
// under the dock (line/column, problems, language, line ending, language server), tabs
// that survive 16 files (the active one scrolled into view, never under the end
// controls), deep paths that keep the file name, and actions that respect the file kind.

const DEEP_DIRS = ['services', 'payments', 'internal', 'settlement', 'reconciliation', 'adapters'];
const DEEP_FILE = 'very_long_descriptive_settlement_reconciliation_adapter_test.go';
const NOTES = Array.from({ length: 16 }, (_, i) => `note-${String(i + 1).padStart(2, '0')}-meeting-summary.md`);

let seed: { workspaceId: string; workspaceRoot: string } | null = null;

async function getSeed(): Promise<{ workspaceId: string; workspaceRoot: string }> {
  if (seed) return seed;
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-chrome-')));
  mkdirSync(join(root, 'app'), { recursive: true });
  mkdirSync(join(root, 'notes'), { recursive: true });
  mkdirSync(join(root, ...DEEP_DIRS), { recursive: true });
  writeFileSync(join(root, 'app', 'main.go'), ['package main', '', 'import "fmt"', '', 'func helper() int {', '\treturn 1', '}', '', 'func main() {', '\tfmt.Println(helper())', '}', ''].join('\n'));
  for (const n of NOTES) writeFileSync(join(root, 'notes', n), `# ${n}\n\nSome text.\n`);
  writeFileSync(join(root, ...DEEP_DIRS, DEEP_FILE), 'package adapters\n\nfunc X() int { return 1 }\n');
  writeFileSync(join(root, 'data.csv'), 'id,name,amount\n1,alpha,10\n2,beta,20\n');
  writeFileSync(join(root, 'README.md'), '# Chrome\n');
  writeFileSync(join(root, 'app', 'long.go'), ['package main', '', ...Array.from({ length: 400 }, (_, i) => `var v${i} = ${i}`), ''].join('\n'));
  for (const argv of [['init', '-b', 'main'], ['config', 'user.email', 'chrome@otto.local'], ['config', 'user.name', 'Otto Chrome'], ['config', 'commit.gpgsign', 'false'], ['add', '.'], ['commit', '-m', 'seed']]) {
    execFileSync('git', argv, { cwd: root, stdio: 'ignore' });
  }
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE Chrome ${Date.now()}`, root_path: root });
    await daemonCall(c.ctx, c.base, c.token, 'POST', '/repos', { path: root, workspace_id: ws.id }).catch(() => undefined);
    seed = { workspaceId: ws.id, workspaceRoot: root };
    return seed;
  } finally {
    await c.ctx.dispose();
  }
}

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-IDE-CHROME'), { recursive: true });
  await page.screenshot({ path: join(dir, 'fix-IDE-CHROME', `${name}.png`) });
}

async function boot(page: Page, scheme: 'dark' | 'light'): Promise<void> {
  const s = await getSeed();
  await bootPolyglotIde(page, s as unknown as PolyglotWorkspaceSeed, scheme);
}

const activeTab = (page: Page) => page.getByTestId('ide-tabs-strip').locator('.ide-tab.active');

/** The active tab is the element on top at its centre and near its end edge. */
async function expectActiveTabOnTop(page: Page): Promise<void> {
  const ok = await activeTab(page).evaluate((el) => {
    const r = el.getBoundingClientRect();
    const y = r.top + r.height / 2;
    return [r.left + r.width / 2, r.right - 3].every((x) => document.elementFromPoint(x, y)?.closest('.ide-tab') === el);
  });
  expect(ok, 'active tab is not covered by the tab-row end controls').toBe(true);
}

test.describe('IDE editor chrome', () => {
  test.describe.configure({ mode: 'serial' });
  test.use({ viewport: { width: 1280, height: 800 }, isMobile: false, hasTouch: false });

  test.beforeEach(({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop chrome');
  });

  test.afterAll(() => {
    if (seed) rmSync(seed.workspaceRoot, { recursive: true, force: true });
  });

  test('one crumbs row, a status strip that follows the cursor, 16 tabs, deep paths, CSV actions', async ({ page }) => {
    test.setTimeout(150_000);
    await boot(page, 'dark');
    await openPolyglotFile(page, 'app', 'main.go');
    await expect(editorContent(page)).toContainText('func main()', { timeout: 15_000 });

    // I1-01: exactly one path row, no bottom breadcrumb bar.
    await expect(page.getByTestId('ide-path-crumbs')).toHaveCount(1);
    await expect(page.locator('[data-testid="ide-breadcrumbs-bar"]')).toHaveCount(0);
    await expect(page.getByTestId('ide-crumb-file')).toHaveText('main.go');
    // I1-02: one ⋯ in the row; the old icon strip is gone.
    const subbar = page.getByTestId('ide-editor-subbar');
    await expect(subbar.getByTestId('ide-editor-more-btn')).toHaveCount(1);
    for (const gone of ['ide-structure-btn', 'ide-blame-btn', 'ide-quickfix-btn', 'ide-scheme-picker-btn', 'ide-inspection-pill', 'ide-send-to']) {
      await expect(page.getByTestId(gone), `${gone} left the crumbs row`).toHaveCount(0);
    }

    // The status strip: line/column follows the cursor; problems and language are buttons.
    const strip = page.getByTestId('ide-status-strip');
    await expect(strip).toBeVisible();
    const stripBox = await strip.boundingBox();
    expect(stripBox!.height).toBeLessThanOrEqual(24);
    await editorContent(page).click();
    await page.keyboard.press('Meta+ArrowUp');
    await expect(page.getByTestId('ide-status-cursor')).toContainText('Ln 1, Col 1');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('ArrowDown');
    await expect(page.getByTestId('ide-status-cursor')).toContainText('Ln 3, Col 1');
    await expect(page.getByTestId('ide-status-language')).toHaveText('Go');
    await expect(page.getByTestId('ide-status-eol')).toHaveText('LF');
    await expect(page.getByTestId('ide-status-problems')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    const census = await page.evaluate(() => {
      const vis = (el: Element) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
      const pageEl = document.querySelector('[data-testid="ide-page"]')!;
      const code = document.querySelector('.ide-editor-area .cm-editor')!.getBoundingClientRect();
      return {
        buttons: [...pageEl.querySelectorAll('button')].filter(vis).length,
        subbarButtons: [...document.querySelectorAll('[data-testid="ide-editor-subbar"] button')].filter(vis).length,
        codeTop: Math.round(code.top),
        codeBottom: Math.round(code.bottom),
      };
    });
    console.log(`chrome census: ${JSON.stringify(census)}`);
    await shot(page, '01-main-go-dark');

    // ⌥-less double-click keeps a tab and never pins it (I1-07).
    await page.getByTestId('ide-tabs-strip').locator('.ide-tab-btn', { hasText: 'main.go' }).dblclick();
    await expect(activeTab(page).locator('.ide-tab-close')).toHaveAttribute('aria-label', /^Close /);

    // I1-05: 16 more files — the active (last) tab is scrolled into view, on top.
    for (const n of NOTES) await openPolyglotFile(page, 'notes', n);
    await expect(activeTab(page)).toContainText('note-16');
    await expectFullyInViewport(page, activeTab(page), 'active tab');
    await expectActiveTabOnTop(page);
    const [tabR, stripR] = await Promise.all([activeTab(page).boundingBox(), page.getByTestId('ide-tabs-strip').boundingBox()]);
    expect(tabR!.x + tabR!.width).toBeLessThanOrEqual(stripR!.x + stripR!.width + 1);
    await shot(page, '02-tabs-16');

    // I1-30: a tablist with one tab stop; ← selects the previous tab, Delete closes it.
    await expect(page.getByTestId('ide-tabs-strip')).toHaveAttribute('role', 'tablist');
    await expect(page.locator('.ide-tab-btn[tabindex="0"]')).toHaveCount(1);
    await expect(page.locator('.ide-tab-btn[tabindex="0"]')).toHaveAttribute('aria-selected', 'true');
    const before = await page.getByTestId('ide-tab-item').count();
    await page.locator('.ide-tab-btn[tabindex="0"]').focus();
    await page.keyboard.press('ArrowLeft');
    await expect(activeTab(page)).toContainText('note-15');
    await page.keyboard.press('Delete');
    await expect(page.getByTestId('ide-tab-item')).toHaveCount(before - 1);
    await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab', { hasText: 'note-15' })).toHaveCount(0);

    // I1-06: "All open tabs" — names readable, parent folder as the hint, no mode rows.
    await page.getByTestId('ide-tabs-overflow-btn').click();
    const menu = page.locator('.ctx-menu').first();
    await expectFullyInViewport(page, menu, 'all open tabs menu');
    const row = menu.getByRole('menuitem').filter({ hasText: 'note-01' }).first();
    const labelW = await row.locator('.ctx-label').evaluate((el) => el.getBoundingClientRect().width);
    expect(labelW, 'a long tab name is not squeezed to a stub').toBeGreaterThan(90);
    await expect(row.locator('.ctx-hint')).toHaveText('notes');
    await expect(menu.getByText('Editor and preview')).toHaveCount(0);
    await shot(page, '03-all-open-tabs-menu');
    await page.keyboard.press('Escape');

    // I1-19: a deep path keeps the whole file name; the middle folders fold into "…".
    await openPolyglotFile(page, ...DEEP_DIRS, DEEP_FILE);
    const fileCrumb = page.getByTestId('ide-crumb-file');
    await expect(fileCrumb).toHaveText(DEEP_FILE);
    const clipped = await fileCrumb.evaluate((el) => el.scrollWidth > el.clientWidth + 1);
    expect(clipped, 'the file crumb is not ellipsised').toBe(false);
    await expectFullyInViewport(page, fileCrumb, 'file crumb');
    await page.getByTestId('ide-crumb-more').click();
    await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'payments' })).toBeVisible();
    await shot(page, '04-deep-path');
    await page.keyboard.press('Escape');

    // A CSV gets no function navigation, reformat or quick fix.
    await openPolyglotFile(page, 'data.csv');
    await expect(page.getByTestId('ide-crumb-file')).toHaveText('data.csv');
    await expect(page.getByTestId('ide-status-language')).toHaveText('CSV');
    await page.getByTestId('ide-editor-more-btn').click();
    const more = page.locator('.ctx-menu').first();
    await expect(more.getByRole('menuitem', { name: 'Send to' })).toBeVisible();
    for (const name of ['Previous function', 'Next function', 'Reformat code', 'Context actions…']) {
      await expect(more.getByRole('menuitem', { name }), `${name} on a CSV`).toHaveCount(0);
    }
    await shot(page, '05-csv-more-menu');
    await page.keyboard.press('Escape');

    // A Go file's ⋯ has them.
    await page.getByTestId('ide-tabs-strip').locator('.ide-tab-btn', { hasText: 'main.go' }).click();
    await page.getByTestId('ide-editor-more-btn').click();
    for (const name of ['File structure…', 'Previous function', 'Next function', 'Reformat code']) {
      await expect(page.locator('.ctx-menu').first().getByRole('menuitem', { name })).toBeVisible();
    }
    await shot(page, '06-go-more-menu');
    await page.keyboard.press('Escape');
  });

  test('leaving the IDE and coming back keeps the file scrolled where it was', async ({ page }) => {
    await boot(page, 'dark');
    await openPolyglotFile(page, 'app', 'long.go');
    const scroller = page.locator('.ide-editor-area .cm-scroller').first();
    await expect(editorContent(page)).toContainText('var v0 = 0');
    await scroller.evaluate((el) => { el.scrollTop = 3000; });
    await expect.poll(() => scroller.evaluate((el) => el.scrollTop)).toBeGreaterThan(2500);
    await page.evaluate(() => { location.hash = '#/agents'; });
    await expect(page.getByTestId('ide-project-tree')).toHaveCount(0);
    await page.evaluate(() => { location.hash = '#/ide'; });
    await expect(editorContent(page)).toBeVisible({ timeout: 15_000 });
    await expect.poll(() => scroller.evaluate((el) => el.scrollTop), { timeout: 5_000 }).toBeGreaterThan(2500);
  });

  test('light theme: crumbs row and status strip', async ({ page }) => {
    test.setTimeout(90_000);
    await boot(page, 'light');
    await openPolyglotFile(page, 'app', 'main.go');
    await expect(editorContent(page)).toContainText('func main()', { timeout: 15_000 });
    await expect(page.getByTestId('ide-status-strip')).toBeVisible();
    await expect(page.locator('[data-testid="ide-breadcrumbs-bar"]')).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
    await shot(page, '07-main-go-light');
  });
});
