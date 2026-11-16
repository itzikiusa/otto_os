import { test, expect, type Page, type Request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// IDE (Beta) never loses or corrupts a person's edits (round 4 review G1/G5/G6):
// 1) ⌘S while the file is still loading, or after its read failed, leaves the file on disk
//    untouched (no PUT at all — the placeholder is not the file);
// 2) "Discard & switch" really discards: reopening the file shows the disk text, no draft;
// 3) ⌃D typed in a text field is forward-delete there, and starts no Run / Debug.

test.use({ serviceWorkers: 'block', viewport: { width: 1440, height: 900 } });
test.describe.configure({ mode: 'serial' });

const A_TEXT = 'export const a = 1;\n';
let rootA = '';
let rootB = '';

function git(cwd: string, ...args: string[]): void {
  execFileSync('git', ['-c', 'user.email=e2e@example.com', '-c', 'user.name=E2E', ...args], { cwd, stdio: 'ignore' });
}

async function bootIde(page: Page, hashQuery: string): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.agentOpen', '0');
    // Autosave off, so an unsaved edit stays unsaved and the switch has to ask.
    const key = 'otto.ide.preferences.v1';
    let cur: Record<string, unknown> = {};
    try { cur = JSON.parse(localStorage.getItem(key) || '{}'); } catch { /* fresh */ }
    localStorage.setItem(key, JSON.stringify({ ...cur, autoSave: false }));
  });
  await page.goto(`/#/ide?${hashQuery}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
}

test.describe('Desktop IDE edit safety', () => {
  test.beforeAll(() => {
    rootA = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-safe-a-')));
    rootB = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-safe-b-')));
    writeFileSync(join(rootA, 'a.ts'), A_TEXT);
    writeFileSync(join(rootB, 'b.ts'), 'export const b = 2;\n');
    git(rootA, 'init', '-q');
    git(rootA, 'add', '.');
    git(rootA, 'commit', '-q', '-m', 'init');
  });

  test.afterAll(() => {
    for (const r of [rootA, rootB]) if (r) rmSync(r, { recursive: true, force: true });
  });

  test.beforeEach(({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  });

  test('G1: ⌘S while the file is still loading writes nothing', async ({ page }) => {
    const puts: Request[] = [];
    page.on('request', (r) => { if (r.method() === 'PUT' && r.url().includes('/ide/fs/file')) puts.push(r); });
    let release: () => void = () => {};
    const held = new Promise<void>((r) => { release = r; });
    await page.route(/\/api\/v1\/ide\/fs\/file\?/, async (route) => {
      if (route.request().method() !== 'GET') return route.continue();
      await held;
      return route.continue();
    });
    await bootIde(page, `root=${encodeURIComponent(rootA)}&file=${encodeURIComponent(join(rootA, 'a.ts'))}`);
    await expect(page.locator('.ide-tab').first()).toBeVisible({ timeout: 20_000 });
    await page.keyboard.press('Meta+s');
    await page.waitForTimeout(400); // ui-guards: allow — proves an absence (no write / no launch / no resurrected draft)
    expect(puts, 'no PUT while loading').toHaveLength(0);
    expect(readFileSync(join(rootA, 'a.ts'), 'utf8')).toBe(A_TEXT);

    release();
  });

  test('G1: ⌘S after the file read failed writes nothing', async ({ page }) => {
    const puts: Request[] = [];
    page.on('request', (r) => { if (r.method() === 'PUT' && r.url().includes('/ide/fs/file')) puts.push(r); });
    await page.route(/\/api\/v1\/ide\/fs\/file\?/, (route) =>
      route.request().method() === 'GET'
        ? route.fulfill({ status: 404, contentType: 'application/json', body: '{"error":"gone for this test"}' })
        : route.continue());
    await bootIde(page, `root=${encodeURIComponent(rootA)}&file=${encodeURIComponent(join(rootA, 'a.ts'))}`);
    await expect(page.locator('.ide-tab').first()).toBeVisible({ timeout: 20_000 });
    await expect(page.getByRole('button', { name: /Retry/i }).first()).toBeVisible({ timeout: 20_000 });
    await page.keyboard.press('Meta+s');
    await page.waitForTimeout(400); // ui-guards: allow — proves an absence (no write / no launch / no resurrected draft)
    expect(puts, 'no PUT after a failed read').toHaveLength(0);
    expect(readFileSync(join(rootA, 'a.ts'), 'utf8')).toBe(A_TEXT);
  });

  test('G5: "Discard & switch" really discards — reopening shows the disk text', async ({ page }) => {
    const fileA = join(rootA, 'a.ts');
    await bootIde(page, `root=${encodeURIComponent(rootA)}&file=${encodeURIComponent(fileA)}`);
    const content = page.locator('.cm-editor .cm-content').first();
    await expect(content).toContainText('export const a = 1', { timeout: 20_000 });
    await content.click();
    await page.keyboard.press('Meta+ArrowDown');
    await page.keyboard.type('// DISCARD_ME');
    // Past the 2 s idle draft write: the pre-fix code had a draft on disk to resurrect.
    await page.waitForTimeout(2_600); // ui-guards: allow — the pre-fix draft write lands after 2 s idle

    await page.goto(`/#/ide?root=${encodeURIComponent(rootB)}`);
    const dialog = page.getByRole('dialog').filter({ hasText: /unsaved changes/i });
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await dialog.getByRole('button', { name: /Discard & switch/i }).click();
    await expect(page.locator('.ide-tab', { hasText: 'b.ts' }).first()).toBeVisible({ timeout: 20_000 });

    await page.goto(`/#/ide?root=${encodeURIComponent(rootA)}&file=${encodeURIComponent(fileA)}`);
    await expect(content).toContainText('export const a = 1', { timeout: 20_000 });
    await page.waitForTimeout(500); // ui-guards: allow — proves an absence (no write / no launch / no resurrected draft)
    await expect(content).not.toContainText('DISCARD_ME');
    expect(readFileSync(fileA, 'utf8')).toBe(A_TEXT);
  });

  test('G6: ⌃D / ⌃R in a text field edit text and launch nothing; outside one, ⌃R runs', async ({ page }) => {
    // A run config, so that without the guard ⌃R / ⌃D really would start a run / debug session.
    const c = await apiCtx();
    try {
      await daemonCall(c.ctx, c.base, c.token, 'POST', '/ide/run-configs', {
        target: { kind: 'local', workspace_id: null, root: rootA, connection_id: null },
        id: 'safety-noop', name: 'Safety no-op', runner: 'custom', command: 'true', args: [], cwd: rootA, env_overrides: {},
      });
    } finally {
      await c.ctx.dispose();
    }
    const launched: string[] = [];
    page.on('request', (r) => { if (r.method() === 'POST' && /\/ide\/(debug\/sessions|runs)(\?|$)/.test(r.url())) launched.push(r.url()); });
    await bootIde(page, `root=${encodeURIComponent(rootA)}`);
    await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByText('Safety no-op').first()).toBeAttached({ timeout: 20_000 });
    // The same guard covers every text field outside the code editor (agent composer,
    // commit box, find fields); the tree filter is the one always on screen.
    const box = page.getByTestId('ide-tree-filter');
    await box.click();
    await box.fill('abcd');
    await page.keyboard.press('ArrowLeft');
    await page.keyboard.press('ArrowLeft');
    await page.keyboard.press('Control+d');
    await page.keyboard.press('Control+r');
    await page.waitForTimeout(600); // ui-guards: allow — proves an absence: nothing launched
    await expect(box).toHaveValue('abd');
    expect(launched, 'no Run / Debug launched from a text field').toEqual([]);

    // Control: with focus outside a text field the same ⌃R does start the run, so the
    // assertion above is not vacuous.
    await box.blur();
    const run = page.waitForRequest((r) => r.method() === 'POST' && /\/ide\/runs(\?|$)/.test(r.url()), { timeout: 10_000 });
    await page.keyboard.press('Control+r');
    await run;
  });

  test('G15: the project tree is one Tab stop with arrow-key navigation', async ({ page }) => {
    await bootIde(page, `root=${encodeURIComponent(rootA)}`);
    const tree = page.getByRole('tree', { name: 'Project files' });
    await expect(tree.getByRole('treeitem', { name: /a\.ts/ })).toBeVisible({ timeout: 20_000 });
    await expect(tree.locator('[role="treeitem"][tabindex="0"]'), 'exactly one row is in the Tab order').toHaveCount(1);
    await expect(tree.locator('.tree-more[tabindex="-1"]').first()).toBeAttached();
    const rootRow = page.getByTestId('ide-tree-root-node').getByRole('treeitem');
    await rootRow.focus();
    await page.keyboard.press('ArrowDown');
    await expect(tree.getByRole('treeitem', { name: /a\.ts/ })).toBeFocused();
    await expect(tree.locator('[role="treeitem"][tabindex="0"]')).toHaveAttribute('data-tree-key', join(rootA, 'a.ts'));
    await page.keyboard.press('ArrowLeft');
    await expect(rootRow).toBeFocused();
  });
});
