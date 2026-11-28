import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall, seedPolyglotFiles } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

test.use({ serviceWorkers: 'block' });

let seed: { workspaceId: string; root: string } | null = null;
async function getSeed(): Promise<{ workspaceId: string; root: string }> {
  if (seed) return seed;
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-side-ide-')));
  seedPolyglotFiles(root);
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `Side IDE ${Date.now()}`, root_path: root });
    seed = { workspaceId: ws.id, root };
    return seed;
  } finally {
    await c.ctx.dispose();
  }
}

async function bootIde(page: Page, pane = 'files'): Promise<void> {
  const s = await getSeed();
  await page.addInitScript(
    ({ wsId, root, pane }) => {
      if (sessionStorage.getItem('__side_ide_init')) return;
      sessionStorage.setItem('__side_ide_init', '1');
      sessionStorage.setItem('otto.ide.stackPane', pane);
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.agent.provider', 'claude');
      localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-side', name: 'otto-web', folders: [{ path: root, name: 'otto-web' }], updatedAt: Date.now() }]));
    },
    { wsId: s.workspaceId, root: s.root, pane },
  );
  await page.goto('/#/ide');
  await expect(page.getByTestId('ide-page')).toBeVisible({ timeout: 20_000 });
}

async function shot(page: Page, name: string): Promise<void> {
  const out = process.env.OTTO_E2E_SHOTS;
  if (!out) return;
  mkdirSync(out, { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
    await page.screenshot({ path: join(out, `${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

const tree = (page: Page) => page.getByTestId('ide-project-tree');
const row = (page: Page, rel: string) => tree(page).locator(`button.tree-item[title="${rel}"]`).first();

// ─────────────────────────────────────────────────────────────────────────────
// The IDE on the shared side surfaces (design.md §2, §7; sidebars/MIGRATION.md),
// with REAL taps:
//   phone   the explorer IS the page — ONE header row: ☰ · Files | Structure |
//           Breakpoints · + · ⋯ · Side panel; tap a file → it is pushed with
//           "‹ Files"; Back returns with the file still marked; tap another.
//           The agent is a Detail pane (full-width sheet); the build tools too.
//           A tool window (⋯) fills the content; its chevron goes back.
//   tablet  the explorer is a column beside the editor (ListDetail clamp).
// Screenshots (OTTO_E2E_SHOTS) for the visual review.
// ─────────────────────────────────────────────────────────────────────────────

test('phone: Files list → file → ‹ Files → another file; agent sheet; tools; ☰ and Side panel', async ({ page }, info) => {
  test.skip(!['iphone-portrait', 'android-phone'].includes(info.project.name), 'phone portrait');
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 390, height: 844 });
  await bootIde(page, 'files');
  const header = page.getByTestId('page-header').first();
  // The list page: the explorer's views ARE the one header row; no second row, no switcher.
  await expect(tree(page)).toBeVisible({ timeout: 20_000 });
  await expect(header.getByTestId('ide-tree-tab-files')).toBeVisible();
  await expect(header.getByTestId('open-navigator')).toBeVisible();
  await expect(header.getByTestId('open-side-panel')).toBeVisible();
  await expect(page.getByTestId('ide-stack-switcher')).toHaveCount(0);
  await expect(page.locator('.ph-tabs-below')).toHaveCount(0);
  const dir = row(page, 'app');
  await expect(dir).toBeVisible({ timeout: 15_000 });
  if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.tap();
  await shot(page, 'ide-phone-list');

  // Tap a file: pushed, "‹ Files" in the header.
  await row(page, 'app/main.go').tap();
  const back = header.getByTestId('phone-back');
  await expect(back).toHaveText(/Files/);
  await expect(page.locator('.ide-editor-area .cm-editor').first()).toBeVisible();
  await expect(tree(page)).toBeHidden();
  await shot(page, 'ide-phone-content');

  // Back: the list, the file still marked.
  await back.tap();
  await expect(tree(page)).toBeVisible();
  await expect(row(page, 'app/main.go')).toHaveAttribute('aria-selected', 'true');
  // Another file.
  await row(page, 'go.mod').tap();
  await expect(header.getByTestId('phone-back')).toBeVisible();
  await expect(header).toContainText('go.mod');

  // The agent: a full-width Detail pane sheet; Close returns to the file.
  await header.getByTestId('ide-toggle-agent').tap();
  const agent = page.getByTestId('ide-agent');
  await expect(agent).toBeVisible();
  const box = await agent.boundingBox();
  expect(box?.width ?? 0).toBeGreaterThanOrEqual(388);
  await shot(page, 'ide-phone-agent');
  await page.getByTestId('ide-agent-close').tap();
  await expect(agent).toBeHidden();

  // A tool window from ⋯ fills the content; its chevron goes back to the editor.
  // ONE ⋯ in the row.
  await expect(header.getByRole('button', { name: 'More actions' }).filter({ visible: true })).toHaveCount(1);
  await header.getByTestId('ide-more-btn').tap();
  await page.getByRole('menuitem', { name: /Git changes/ }).tap();
  await expect(page.getByTestId('ide-bottom-dock')).toBeVisible();
  await expect(page.getByTestId('ide-editor-area')).toBeHidden();
  await shot(page, 'ide-phone-tools');
  await page.getByTestId('ide-dock-close-tools').tap();
  await expect(page.locator('.ide-editor-area .cm-editor').first()).toBeVisible();

  // ☰ from the list page; the Side panel from the content page.
  await header.getByTestId('phone-back').tap();
  await header.getByTestId('open-navigator').tap();
  await expect(page.getByRole('dialog', { name: /Navigator/ })).toBeVisible();
  await page.keyboard.press('Escape');
  await row(page, 'go.mod').tap();
  await header.getByTestId('open-side-panel').tap();
  await expect(page.getByRole('dialog', { name: /Side panel/ })).toBeVisible();
});

test('phone: Structure and Breakpoints are the same list page; a symbol pushes the editor', async ({ page }, info) => {
  test.skip(!['iphone-portrait', 'android-phone'].includes(info.project.name), 'phone portrait');
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 390, height: 844 });
  await bootIde(page, 'files');
  const header = page.getByTestId('page-header').first();
  const dir = row(page, 'app');
  await expect(dir).toBeVisible({ timeout: 20_000 });
  if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.tap();
  await row(page, 'app/main.go').tap();
  await header.getByTestId('phone-back').tap();
  await header.getByTestId('ide-tree-tab-structure').tap();
  // A phone drops the counts from the views.
  await expect(header.getByTestId('ide-tree-tab-structure')).toHaveText('Structure');
  const sym = page.locator('.struct-row').first();
  await expect(sym).toBeVisible({ timeout: 20_000 });
  await shot(page, 'ide-phone-structure');
  await sym.tap();
  await expect(header.getByTestId('phone-back')).toHaveText(/Structure/);
  await header.getByTestId('phone-back').tap();
  await header.getByTestId('ide-tree-tab-breakpoints').tap();
  await expect(page.getByTestId('ide-breakpoints-list')).toBeVisible();
});

test('tablet: the explorer is a column beside the editor, Detail pane inline or a sheet', async ({ page }, info) => {
  test.skip(info.project.name !== 'ipad-portrait', 'tablet portrait');
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 820, height: 1180 });
  await bootIde(page, 'editor');
  await expect(tree(page)).toBeVisible({ timeout: 20_000 });
  const dir = row(page, 'app');
  await expect(dir).toBeVisible({ timeout: 15_000 });
  if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.tap();
  await row(page, 'app/main.go').tap();
  await expect(tree(page)).toBeVisible();
  await expect(page.locator('.ide-editor-area .cm-editor').first()).toBeVisible();
  await shot(page, 'ide-tablet');
  await page.getByTestId('ide-toggle-agent').tap();
  await expect(page.getByTestId('ide-agent')).toBeVisible();
  await shot(page, 'ide-tablet-agent');
});
