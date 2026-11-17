import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// Several projects in the IDE (review I3): the discard confirm is on top of the
// switcher and works on the FIRST click; two folders named `api` are told apart;
// a folder opened by path is still listed after you leave it and after a reload;
// the tree comes back expanded; the agent draft does not follow you into another
// project; a deleted folder shows a designed state.
//
// The harness daemon is the INSTALLED build: the multi-repo "open one member repo
// attaches only that repo" fix (I3-02) is daemon-side and covered by Rust tests
// (`crates/otto-ide/src/projects.rs` `primary_root_tests`).

let base = '';
let wsAlpha = '';
let rootAlpha = '';
let rootApiA = '';
let rootApiB = '';
let rootLoose = '';
let rootGone = '';

async function postJson(path: string, data: unknown): Promise<{ id: string }> {
  const { ctx, base: b } = await apiCtx();
  const r = await ctx.post(`${b}${path}`, { data });
  expect(r.ok(), `${path}: ${r.status()}`).toBe(true);
  const body = (await r.json()) as { id: string };
  await ctx.dispose();
  return body;
}

test.beforeAll(async () => {
  base = mkdtempSync(join(tmpdir(), 'otto-e2e-projects-'));
  rootAlpha = join(base, 'alpha-web');
  rootApiA = join(base, 'team-a', 'api');
  rootApiB = join(base, 'team-b', 'api');
  rootLoose = join(base, 'loose-folder');
  rootGone = join(base, 'deleted-later');
  for (const d of [join(rootAlpha, 'src', 'lib'), rootApiA, rootApiB, rootLoose, rootGone]) mkdirSync(d, { recursive: true });
  writeFileSync(join(rootAlpha, 'util.ts'), 'export const x = 1;\n');
  writeFileSync(join(rootAlpha, 'src', 'lib', 'deep.ts'), 'export const deep = 2;\n');
  writeFileSync(join(rootApiA, 'index.js'), 'console.log("a");\n');
  writeFileSync(join(rootApiB, 'index.js'), 'console.log("b");\n');
  writeFileSync(join(rootLoose, 'notes.md'), '# loose\n');
  writeFileSync(join(rootGone, 'a.txt'), 'soon gone\n');
  wsAlpha = (await postJson('/api/v1/workspaces', { name: 'alpha-web', root_path: rootAlpha })).id;
  await postJson('/api/v1/workspaces', { name: 'api', root_path: rootApiA });
  await postJson('/api/v1/workspaces', { name: 'api', root_path: rootApiB });
  await postJson('/api/v1/workspaces', { name: 'deleted-later', root_path: rootGone });
});

test.afterAll(() => {
  if (base) rmSync(base, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop IDE');
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
    // Unsaved edits must stay unsaved so the switch asks (autosave is on by default).
    const key = 'otto.ide.preferences.v1';
    let cur: Record<string, unknown> = {};
    try { cur = JSON.parse(localStorage.getItem(key) || '{}'); } catch { /* fresh */ }
    localStorage.setItem(key, JSON.stringify({ ...cur, autoSave: false }));
  }, wsAlpha);
});

const switcher = (page: Page) => page.getByTestId('ide-target-switcher');
const popover = (page: Page) => page.getByTestId('ide-workspace-popover');
const tree = (page: Page) => page.getByTestId('ide-project-tree');

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(dir, 'fix-IDE-PROJECTS', `${name}-${scheme}.png`) });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

async function openAlpha(page: Page): Promise<void> {
  await page.goto('/#/ide');
  await expect(switcher(page)).toContainText('alpha-web', { timeout: 20_000 });
  await expect(tree(page).getByText('util.ts')).toBeVisible();
}

async function pickFolder(page: Page, label: string): Promise<void> {
  if (!(await popover(page).isVisible())) await switcher(page).click();
  await popover(page).getByTestId('ide-workspace-option').filter({ hasText: label }).first().click();
}

test('same-name folders are told apart; the discard confirm is on top and switches on the first click', async ({ page }) => {
  await openAlpha(page);
  await switcher(page).click();
  const options = popover(page).getByTestId('ide-workspace-option');
  await expect(options.filter({ hasText: 'api — team-a' })).toHaveCount(1);
  await expect(options.filter({ hasText: 'api — team-b' })).toHaveCount(1);
  await shot(page, 'switcher-same-name');
  await page.keyboard.press('Escape');

  // A dirty file, then switch.
  await tree(page).getByText('util.ts').dblclick();
  const content = page.locator('.ide-editor-area .cm-content').first();
  await expect(content).toContainText('export const x');
  await content.click();
  await page.keyboard.press('End');
  await page.keyboard.type(' // dirty');
  await pickFolder(page, 'api — team-b');

  const discard = page.getByRole('button', { name: 'Discard & switch' });
  await expect(discard).toBeVisible();
  await expect(popover(page)).toHaveCount(0);
  const onTop = await discard.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return Boolean(hit && (hit === el || el.contains(hit)));
  });
  expect(onTop, 'the confirm button is the top element at its centre').toBe(true);
  await shot(page, 'discard-on-top');
  await discard.click();
  await expect(switcher(page)).toContainText('api — team-b');
  await expect(tree(page).getByText('index.js')).toBeVisible();
});

test('a folder opened by path stays listed after leaving it and after a reload; the tree comes back expanded', async ({ page }) => {
  await openAlpha(page);
  // Expand src/lib in alpha.
  await tree(page).getByText('src', { exact: true }).click();
  await tree(page).getByText('lib', { exact: true }).click();
  await expect(tree(page).getByText('deep.ts')).toBeVisible();

  // Open the unregistered folder by path, then go elsewhere.
  await page.goto(`/#/ide?root=${encodeURIComponent(rootLoose)}`);
  await expect(switcher(page)).toContainText('loose-folder', { timeout: 20_000 });
  await pickFolder(page, 'alpha-web');
  await expect(switcher(page)).toContainText('alpha-web');
  await expect(tree(page).getByText('deep.ts'), 'A → loose → A keeps the tree expanded').toBeVisible();

  await switcher(page).click();
  await popover(page).getByTestId('ide-workspace-search').fill('loose');
  await expect(popover(page).getByTestId('ide-workspace-option').filter({ hasText: 'loose-folder' })).toHaveCount(1);
  await page.keyboard.press('Escape');

  await page.reload();
  await expect(switcher(page)).toContainText('alpha-web', { timeout: 20_000 });
  await switcher(page).click();
  await popover(page).getByTestId('ide-workspace-search').fill('loose');
  const loose = popover(page).getByTestId('ide-workspace-option').filter({ hasText: 'loose-folder' });
  await expect(loose).toHaveCount(1);
  await loose.click();
  await expect(tree(page).getByText('notes.md')).toBeVisible();
});

test('the agent draft stays with its project', async ({ page }) => {
  await openAlpha(page);
  const composer = page.getByTestId('ide-agent-composer');
  if ((await composer.count()) === 0) await page.getByTestId('ide-toggle-agent').click();
  await expect(composer).toBeVisible({ timeout: 15_000 });
  await composer.fill('about alpha only');
  await pickFolder(page, 'api — team-a');
  await expect(switcher(page)).toContainText('api — team-a');
  await expect(composer).toHaveValue('');
  await pickFolder(page, 'alpha-web');
  await expect(switcher(page)).toContainText('alpha-web');
  await expect(composer).toHaveValue('about alpha only');
});

test('a deleted project folder shows what is wrong and next steps that can work', async ({ page }) => {
  await openAlpha(page);
  rmSync(rootGone, { recursive: true, force: true });
  await pickFolder(page, 'deleted-later');
  const missing = page.getByTestId('ide-project-missing');
  await expect(missing).toBeVisible({ timeout: 15_000 });
  await expect(missing).toContainText('This folder no longer exists');
  await expect(page.getByTestId('ide-project-missing-path')).toHaveText(rootGone);
  await expect(missing.getByRole('button', { name: 'Retry' })).toHaveCount(0);
  await shot(page, 'deleted-folder');
  await missing.getByRole('button', { name: 'Choose another folder…' }).click();
  await expect(popover(page)).toBeVisible();
});
