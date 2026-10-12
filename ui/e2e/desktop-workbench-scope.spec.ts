import { test, expect, type Page } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Workbench scope: a file shows either in every workspace (`global`, the
// default for new files) or only in the workspace it lives in. Reported bug:
// two files "disappeared" after switching to a tab of another workspace —
// they were workspace-scoped. Flow: New file in A → switch to B → still listed
// and editable → "This workspace" moves it into B → gone from A.
// ─────────────────────────────────────────────────────────────────────────────

const suffix = Math.random().toString(36).slice(2, 8);
let wsA = '';
let wsB = '';
let base = '';
let token = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  base = c.base;
  token = c.token;
  wsA = await seedWorkspace(c.ctx, c.base, `Scope A ${suffix}`);
  wsB = await seedWorkspace(c.ctx, c.base, `Scope B ${suffix}`);
  // A workspace-only file in A that B must never list.
  const r = await c.ctx.post(`${base}/api/v1/workspaces/${wsA}/workbench/docs`, {
    data: { name: `only-a-${suffix}.sql`, content: 'select 1', scope: 'workspace' },
  });
  expect(r.ok()).toBeTruthy();
  await c.ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'Desktop flow');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => localStorage.setItem('otto_rail_expanded', '0'));
});

async function openIn(page: Page, ws: string): Promise<void> {
  await page.evaluate((id) => localStorage.setItem('otto_workspace', id), ws);
  await page.goto('/#/workbench');
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

async function docOf(page: Page, ws: string, id: string): Promise<{ scope: string; workspace_id: string; content: string }> {
  const r = await page.request.get(`${base}/api/v1/workspaces/${ws}/workbench/docs/${id}`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(r.ok(), `GET doc in ${ws} → ${r.status()}`).toBeTruthy();
  return r.json();
}

test('a new file follows you across workspaces until scoped to one', async ({ page }) => {
  await page.goto('/');
  await openIn(page, wsA);
  const rows = page.getByTestId('wb-file-row');
  await expect(rows.filter({ hasText: `only-a-${suffix}.sql` })).toBeVisible();

  // New file in A — global by default; the toolbar says where it shows.
  await page.getByTestId('wb-new').first().click();
  const editor = page.getByTestId('wb-editor').locator('.cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.insertText(`-- dicasinos ${suffix}`);
  const scopeBtn = page.getByTestId('wb-scope-toggle');
  await expect(scopeBtn).toContainText('All workspaces');
  await expect(page.getByTestId('wb-save-state')).toContainText('Saved');
  // The new file is the open (active) row; other specs' global files may list too.
  const id = await page.locator('[data-testid="wb-file-row"].active').getAttribute('data-doc-id');
  expect(id).toBeTruthy();
  // Both scopes present → the list labels its groups.
  await expect(page.getByTestId('wb-files-global')).toBeVisible();
  await expect(page.getByTestId('wb-files-here')).toContainText(`only-a-${suffix}.sql`);
  if (process.env.OTTO_REVIEW_ARTIFACT_DIR) {
    await mkdir(process.env.OTTO_REVIEW_ARTIFACT_DIR, { recursive: true });
    for (const scheme of ['light', 'dark']) {
      await page.evaluate((s) => { localStorage.setItem('otto_scheme', s); document.documentElement.dataset.scheme = s; }, scheme);
      await page.screenshot({ path: `${process.env.OTTO_REVIEW_ARTIFACT_DIR}/workbench-scope-${scheme}.png`, animations: 'disabled' });
    }
  }

  // Switch to B: the global file is still there (and editable); A's local one is not.
  await openIn(page, wsB);
  const row = page.locator(`[data-testid="wb-file-row"][data-doc-id="${id}"]`);
  await expect(row).toBeVisible();
  await expect(rows.filter({ hasText: `only-a-${suffix}.sql` })).toHaveCount(0);
  await row.click();
  await expect(editor).toContainText(`-- dicasinos ${suffix}`);
  await editor.click();
  await page.keyboard.press('End');
  await page.keyboard.insertText(' (edited in B)');
  await expect(page.getByTestId('wb-save-state')).toContainText('Saved');
  await expect.poll(async () => (await docOf(page, wsA, id!)).content).toContain('(edited in B)');
  await expectNoHorizontalOverflow(page);

  // "This workspace" from B moves it into B …
  await scopeBtn.click();
  await expect(scopeBtn).toContainText('This workspace');
  await expect.poll(async () => (await docOf(page, wsB, id!)).workspace_id).toBe(wsB);
  // … so A no longer lists it.
  await openIn(page, wsA);
  await expect(rows.filter({ hasText: `only-a-${suffix}.sql` })).toBeVisible();
  await expect(page.locator(`[data-testid="wb-file-row"][data-doc-id="${id}"]`)).toHaveCount(0);
});

test('the ⋯ menu sets the scope of new files', async ({ page }) => {
  await page.goto('/');
  await openIn(page, wsB);
  await page.getByRole('button', { name: 'More actions', exact: true }).first().click();
  await page.getByRole('menuitemcheckbox', { name: 'New files in this workspace only' }).click();
  await page.getByTestId('wb-new').first().click();
  await expect(page.getByTestId('wb-scope-toggle')).toContainText('This workspace');
  // Restore the default for later specs sharing this browser profile.
  await page.getByRole('button', { name: 'More actions', exact: true }).first().click();
  await page.getByRole('menuitemcheckbox', { name: 'New files in all workspaces' }).click();
  expect(await page.evaluate(() => localStorage.getItem('otto.wb.newScope'))).toBe('"global"');
});
