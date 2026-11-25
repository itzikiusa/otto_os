import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test, expect, type Page } from '@playwright/test';
import { apiCtx } from './seed';

// A NAMED project (the owner only uses named, pinned projects) gets its tab strip back after
// a reload and after a fresh page load (an app restart): the same tabs in the same order, the
// same active tab, the pinned tab, the split, the unsaved draft's dot, and the active file is
// read before the hidden tabs. (Before the fix the strip came back as README.md only.)
// The tab state lives in the browser's storage, so a fresh document load stands in for an
// app restart; the harness daemon is shared by the run, so it is not restarted here.

test.use({ serviceWorkers: 'block', viewport: { width: 1280, height: 800 } });

const FILES = [
  'AGENTS.md', 'CLAUDE.md', 'Cargo.toml', 'ui/package.json', 'ui/vite.config.ts', 'crates/ottod/src/main.rs',
  'docs/RELEASE.md', 'ui/src/App.svelte', 'scripts/check.sh', 'rust-toolchain.toml', 'deny.toml',
];
let root = '';
let projectId = '';
let otherRoot = '';
// beforeAll runs once per worker against the one daemon: names are unique per worker.
const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
const OTTO = `Otto clone ${stamp}`;
const OTHER = `Other clone ${stamp}`;

test.beforeAll(async () => {
  root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-named-tabs-')));
  execFileSync('git', ['clone', '--quiet', '--depth', '1', `file://${fileURLToPath(new URL('../..', import.meta.url))}`, join(root, 'otto')]);
  root = join(root, 'otto');
  const { ctx, base } = await apiCtx();
  projectId = `proj-e2e-tabs-${stamp}`;
  const resp = await ctx.post(`${base}/api/v1/ide/projects`, { data: { id: projectId, name: OTTO, primary_root: root, folders: [root], pinned: true } });
  expect(resp.ok()).toBe(true);
  otherRoot = join(root, '..', 'other');
  mkdirSync(otherRoot);
  writeFileSync(join(otherRoot, 'README.md'), '# other\n');
  const other = await ctx.post(`${base}/api/v1/ide/projects`, { data: { name: OTHER, primary_root: otherRoot, folders: [otherRoot], pinned: true } });
  expect(other.ok()).toBe(true);
  await ctx.dispose();
});

test.afterAll(() => {
  if (root) rmSync(resolve(root, '..'), { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop IDE');
  await page.addInitScript(() => {
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.agentOpen', '0');
    const key = 'otto.ide.preferences.v1'; // the draft must stay unsaved
    let cur: Record<string, unknown> = {};
    try { cur = JSON.parse(localStorage.getItem(key) || '{}'); } catch { /* fresh */ }
    localStorage.setItem(key, JSON.stringify({ ...cur, autoSave: false }));
  });
});

const tabs = (page: Page) => page.getByTestId('ide-tabs-strip').getByTestId('ide-tab-item');
const tab = (page: Page, name: string) => tabs(page).filter({ has: page.getByText(name, { exact: true }) });
const editor = (page: Page) => page.locator('.ide-editor-area .cm-content').first();
const firstLine = (rel: string) => readFileSync(join(root, rel), 'utf8').split('\n').find((l) => l.trim().length > 8)!.trim().slice(0, 30);

interface Strip { labels: string[]; active: string; pinned: string[]; dirty: string[] }
async function strip(page: Page): Promise<Strip> {
  return tabs(page).evaluateAll((els) => {
    const label = (el: Element) => el.querySelector('.ide-tab-label')?.textContent?.trim() ?? '';
    return {
      labels: els.map(label),
      active: els.filter((el) => el.classList.contains('active')).map(label).join(),
      pinned: els.filter((el) => el.querySelector('[aria-label^="Unpin"]')).map(label),
      dirty: els.filter((el) => el.querySelector('.ide-dirty-dot')).map(label),
    };
  });
}

async function expectRestored(page: Page, want: Strip, reads: string[]): Promise<void> {
  await expect.poll(() => strip(page), { timeout: 20_000 }).toEqual(want);
  await expect(editor(page)).toContainText(firstLine('crates/ottod/src/main.rs'));
  await expect(page.getByTestId('ide-split-pane')).toContainText(firstLine('ui/package.json'));
  // The active file is read before any hidden tab (the split starts its own read first).
  const hidden = reads.filter((p) => !p.endsWith('package.json'));
  expect(hidden[0] ?? '').toMatch(/main\.rs$/);
}

/** A tree row (the IDE shows `/private/var` as `/var`). */
const row = (page: Page, path: string) => page.locator(`[data-tree-key="${path.replace(/^\/private(?=\/var\/)/, '')}"]`);

async function pickProject(page: Page, name: string): Promise<void> {
  await page.getByTestId('ide-target-switcher').click();
  await page.getByTestId('ide-named-project-option').filter({ hasText: name }).click();
}

test('a named project gets its tabs back after a reload and a restart', async ({ page }) => {
  test.setTimeout(120_000);
  const reads: string[] = [];
  page.on('request', (r) => { if (r.url().includes('/ide/fs/file?')) reads.push(new URL(r.url()).searchParams.get('path') ?? ''); });
  await page.goto('/#/ide');
  await pickProject(page, OTTO);
  await expect(tab(page, 'README.md')).toBeVisible({ timeout: 30_000 });
  for (const rel of FILES) {
    await page.evaluate((p) => { location.hash = `#/ide?file=${encodeURIComponent(p)}`; }, join(root, rel));
    await expect(tab(page, rel.split('/').pop()!)).toBeVisible({ timeout: 15_000 });
  }
  // Pin one, leave an unsaved draft in another, split a third, end on a tab that is not the first.
  await tab(page, 'AGENTS.md').click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Pin tab' }).click();
  await tab(page, 'Cargo.toml').getByRole('tab').click();
  await expect(editor(page)).toContainText('[workspace]');
  await editor(page).click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('# a draft line');
  await expect(tab(page, 'Cargo.toml').locator('.ide-dirty-dot')).toBeVisible();
  await tab(page, 'package.json').getByRole('tab').click();
  await expect(editor(page)).toContainText(firstLine('ui/package.json'));
  await page.getByTestId('ide-split-right-btn').click();
  await expect(page.getByTestId('ide-split-pane')).toContainText(firstLine('ui/package.json'));
  await tab(page, 'main.rs').getByRole('tab').click();
  await expect(editor(page)).toContainText(firstLine('crates/ottod/src/main.rs'));
  await page.waitForTimeout(2_500); // ui-guards: allow — the draft's idle persist
  const want = await strip(page);
  expect(want.labels).toHaveLength(FILES.length + 1);
  expect(want).toMatchObject({ active: 'main.rs', pinned: ['AGENTS.md'], dirty: ['Cargo.toml'] });

  reads.length = 0;
  await page.reload();
  await expectRestored(page, want, reads);

  // A fresh document load (the app restarting) restores it again.
  await page.goto('about:blank');
  reads.length = 0;
  await page.goto('/#/ide');
  await expectRestored(page, want, reads);
  await tab(page, 'Cargo.toml').getByRole('tab').click();
  await expect(editor(page)).toContainText('# a draft line');
});

test('the tree keeps its scroll after a switch back and a reload (no auto-reveal of the restored tab)', async ({ page }) => {
  await page.goto('/#/ide');
  await pickProject(page, OTTO);
  await expect(tab(page, 'README.md')).toBeVisible({ timeout: 30_000 });
  const crates = row(page, join(root, 'crates'));
  await page.evaluate((p) => { location.hash = `#/ide?file=${encodeURIComponent(p)}`; }, join(root, 'crates/ottod/src/main.rs'));
  await expect(tab(page, 'main.rs')).toHaveClass(/active/, { timeout: 15_000 });
  await expect(editor(page)).toContainText(firstLine('crates/ottod/src/main.rs'));
  const ottod = row(page, join(root, 'crates/ottod'));
  // The new tab's reveal may still be opening folders: retry until crates/ is open, ottod/ shut.
  await expect(async () => {
    if ((await crates.getAttribute('aria-expanded')) !== 'true') await crates.click();
    await expect(ottod).toBeVisible({ timeout: 2_000 });
    if ((await ottod.getAttribute('aria-expanded')) === 'true') await ottod.click();
    await expect(ottod).toHaveAttribute('aria-expanded', 'false', { timeout: 2_000 });
    await page.waitForTimeout(800); // ui-guards: allow — and it stays so
    await expect(ottod).toHaveAttribute('aria-expanded', 'false', { timeout: 100 });
  }).toPass({ timeout: 20_000 });
  const scroller = page.getByTestId('ide-project-tree').locator('.tree-scroll');
  await scroller.evaluate((el) => { el.scrollTop = 240; });
  await expect.poll(() => scroller.evaluate((el) => el.scrollTop)).toBe(240);
  await page.waitForTimeout(500); // ui-guards: allow — let the scroll offset be recorded

  await pickProject(page, OTHER);
  await expect(row(page, join(otherRoot, 'README.md'))).toBeVisible({ timeout: 15_000 });
  await pickProject(page, OTTO);
  await expect(tab(page, 'main.rs')).toHaveClass(/active/, { timeout: 15_000 });
  await page.waitForTimeout(1_200); // ui-guards: allow — absence: nothing scrolls the tree by itself
  expect(await scroller.evaluate((el) => el.scrollTop), 'switch back').toBe(240);
  await expect(ottod).toHaveAttribute('aria-expanded', 'false');

  await page.reload();
  await expect(tab(page, 'main.rs')).toHaveClass(/active/, { timeout: 20_000 });
  await page.waitForTimeout(1_200); // ui-guards: allow — absence: nothing scrolls the tree by itself
  expect(await scroller.evaluate((el) => el.scrollTop), 'reload').toBe(240);
  await expect(ottod).toHaveAttribute('aria-expanded', 'false');
});
