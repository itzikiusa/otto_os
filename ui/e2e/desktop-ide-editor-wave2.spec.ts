import { test, expect, type Page } from '@playwright/test';
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// IDE (Beta) editor bug hunt, wave 2 — the UI-only fixes:
// E4) a binary / non-UTF-8 file is labelled, not just "Read-only";
// E5) a Replace in Path preview is dropped as soon as the replacement or an option changes;
// E8) Local History opens on the newest entry that differs from the file, and Revert is off
//     on an entry identical to it.

test.use({ serviceWorkers: 'block', viewport: { width: 1440, height: 900 } });
test.describe.configure({ mode: 'serial' });

let root = '';
const target = () => ({ kind: 'local', workspace_id: null, root, connection_id: null });

async function bootIde(page: Page, file: string): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.agentOpen', '0');
  });
  await page.goto(`/#/ide?root=${encodeURIComponent(root)}&file=${encodeURIComponent(join(root, file))}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.ide-tab', { hasText: file }).first()).toBeVisible({ timeout: 20_000 });
}

test.describe('Desktop IDE editor wave 2', () => {
  test.beforeAll(() => {
    root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-wave2-')));
    writeFileSync(join(root, 'flags.ts'), 'export const FLAG_ONE = 1;\nexport const flag_two = 2;\n');
    writeFileSync(join(root, 'blob.bin'), Buffer.from([0x89, 0x50, 0x00, 0x01, 0x02, 0x00, 0xff]));
  });

  test.afterAll(() => {
    if (root) rmSync(root, { recursive: true, force: true });
  });

  test.beforeEach(({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  });

  test('E4: a binary file says so in the editor bar', async ({ page }) => {
    await bootIde(page, 'blob.bin');
    await expect(page.getByTestId('ide-binary-badge')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId('ide-binary-badge')).toContainText('Binary or not UTF-8 text');
  });

  test('E5: changing the replacement or an option drops the stale replace preview', async ({ page }) => {
    await bootIde(page, 'flags.ts');
    await page.keyboard.press('Shift');
    await page.keyboard.press('Shift');
    const dialog = page.locator('.sheet[role="dialog"]').last();
    await expect(dialog).toBeVisible({ timeout: 5_000 });
    await dialog.getByTestId('ide-se-tab-text').click();
    await dialog.locator('input.se-input').first().fill('FLAG_ONE');
    await dialog.getByTestId('ide-se-replace-toggle').click();
    const replacement = dialog.getByTestId('ide-replace-input');
    await replacement.fill('FLAG_1');
    const preview = dialog.locator('.se-preview-box');
    await dialog.getByRole('button', { name: /^Preview$/ }).click();
    await expect(preview).toContainText('1 occurrence', { timeout: 10_000 });
    await expect(preview).toContainText('(1 match)');

    await replacement.fill('FLAG_ONE_RENAMED');
    await expect(preview).toHaveCount(0);

    await dialog.getByRole('button', { name: /^Preview$/ }).click();
    await expect(preview).toBeVisible({ timeout: 10_000 });
    await dialog.getByRole('button', { name: 'Match case' }).click();
    await expect(preview).toHaveCount(0);
  });

  test('E8: Local History opens on the newest entry that differs; Revert is off on an identical one', async ({ page }) => {
    const c = await apiCtx();
    try {
      for (const v of ['v1', 'v2']) {
        await daemonCall(c.ctx, c.base, c.token, 'PUT', '/ide/fs/file', { target: target(), path: 'notes.txt', content: `notes ${v}\n` });
      }
    } finally {
      await c.ctx.dispose();
    }
    await bootIde(page, 'notes.txt');
    const item = page.getByTestId('ide-project-tree').locator('button.tree-item[title$="notes.txt"]').first();
    await item.click({ button: 'right' });
    const menu = page.locator('.ctx-menu');
    await menu.getByRole('menuitem', { name: 'Local History' }).click();
    await menu.getByRole('menuitem', { name: 'Show History…' }).click();
    const modal = page.getByTestId('ide-comparison-modal');
    await expect(modal).toBeVisible({ timeout: 15_000 });
    const rows = modal.getByTestId('ide-local-history-row');
    const revert = page.getByTestId('ide-local-history-restore');
    // The newest entry (v2) is the file itself: the modal moves on to v1.
    await expect(rows.nth(1)).toHaveClass(/active/, { timeout: 10_000 });
    await expect(page.getByTestId('ide-compare-identical-banner')).toHaveCount(0);
    await expect(revert).toBeEnabled();

    await rows.nth(0).click();
    await expect(page.getByTestId('ide-compare-identical-banner')).toBeVisible({ timeout: 10_000 });
    await expect(revert).toBeDisabled();
  });
});
