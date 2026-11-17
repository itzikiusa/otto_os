import { mkdirSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Scratches are a built-in IDE project: the same per-user folder in every
// workspace. Open it from one workspace, create a scratch, and find it from a
// second workspace (reached through the old `#/workbench` route, which now
// forwards to the IDE on the Scratches project). Also covers Preview.

let wsA = '';
let wsB = '';
let projectName = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsA = await seedWorkspace(ctx, base, 'Scratches A');
  wsB = await seedWorkspace(ctx, base, 'Scratches B');
  const repo = join(mkdtempSync(join(tmpdir(), 'otto-e2e-proj-')), 'scratch-default-repo');
  mkdirSync(repo);
  projectName = `Default Order ${Date.now().toString(36)}`;
  const resp = await ctx.post(`${base}/api/v1/ide/projects`, {
    data: { name: projectName, primary_root: repo, folders: [repo], pinned: true },
  });
  expect(resp.ok()).toBe(true);
  await ctx.dispose();
});

async function useWorkspace(page: Page, id: string): Promise<void> {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, id);
}

const switcher = (page: Page) => page.getByTestId('ide-target-switcher');
const tree = (page: Page) => page.getByTestId('ide-project-tree');

test('a scratch made from one workspace shows in another', async ({ page, browser }) => {
  await useWorkspace(page, wsA);
  await page.goto('/#/ide');
  await switcher(page).click();
  const option = page.getByTestId('ide-scratches-option');
  await expect(option).toBeVisible();
  await option.click();
  await expect(switcher(page)).toContainText('Scratches');
  await expect(tree(page).getByTestId('ide-tree-root-node')).toContainText('Scratches');

  await tree(page).getByTestId('ide-tree-new-file').click();
  const dialog = page.getByRole('dialog', { name: 'New scratch file' });
  await dialog.getByRole('textbox').fill('e2e-note.md');
  await dialog.getByRole('button', { name: 'Create' }).click();
  await expect(tree(page).getByText('e2e-note.md')).toBeVisible();

  // Workspace B — via the old route.
  const other = await browser.newPage();
  await useWorkspace(other, wsB);
  await other.goto('/#/workbench');
  await expect(other).toHaveURL(/#\/ide$/);
  await expect(switcher(other)).toContainText('Scratches');
  await expect(tree(other).getByText('e2e-note.md')).toBeVisible();
  await other.close();
});

test('a Markdown scratch previews in the editor', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const put = await ctx.put(`${base}/api/v1/scratches/file`, {
    data: { path: 'preview-e2e.md', content: '# Hello scratches\n\nSome *text*.\n' },
  });
  expect(put.ok()).toBe(true);
  await ctx.dispose();

  await useWorkspace(page, wsA);
  await page.goto('/#/ide?scratches=1');
  await expect(switcher(page)).toContainText('Scratches');
  await tree(page).getByText('preview-e2e.md').dblclick();
  const toggle = page.getByTestId('ide-preview-toggle');
  await toggle.getByRole('button', { name: 'Preview', exact: true }).click();
  await expect(page.getByTestId('file-preview').getByRole('heading', { name: 'Hello scratches' })).toBeVisible({ timeout: 15_000 });
  await toggle.getByRole('button', { name: 'Editor', exact: true }).click();
  await expect(page.getByTestId('file-preview')).toHaveCount(0);
  // Send to… lives in the crumbs row's ⋯ (editor actions) now.
  await page.getByTestId('ide-editor-more-btn').click();
  await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'Send to' })).toBeVisible();
  await page.keyboard.press('Escape');
});

test('Scratches is the default project: first in the list, never renamed or deleted', async ({ page }) => {
  await useWorkspace(page, wsA);
  await page.goto('/#/ide');
  await switcher(page).click();
  const pop = page.getByTestId('ide-workspace-popover');
  const scratches = pop.getByTestId('ide-scratches-option');
  await expect(scratches).toContainText('Default');
  await expect(scratches).toContainText('the same in every workspace');
  // First row of the projects list, above every named project.
  const rows = pop.locator('[data-testid="ide-scratches-option"], [data-testid="ide-named-project-option"]');
  await expect(rows.first()).toHaveAttribute('data-testid', 'ide-scratches-option');
  await expect(pop.getByTestId('ide-named-project-option').filter({ hasText: projectName })).toBeVisible();
  // The search finds it by name; a named project's filter hides it.
  await page.getByTestId('ide-workspace-search').fill(projectName);
  await expect(scratches).toHaveCount(0);
  await page.getByTestId('ide-workspace-search').fill('scratch');
  await expect(scratches).toBeVisible();

  await scratches.click({ button: 'right' });
  for (const name of ['Rename project…', 'Delete project…']) {
    const item = page.getByRole('menuitem', { name });
    await expect(item).toHaveAttribute('aria-disabled', 'true');
    await item.hover(); // the reason shows as the row's tooltip
    await expect(item).toHaveAttribute('title', /default project/);
  }
});
