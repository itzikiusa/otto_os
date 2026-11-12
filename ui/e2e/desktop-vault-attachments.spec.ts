// Vault editor attachments: a pasted screenshot / dropped file is saved under
// the vault's attachments folder (never over an existing file) and its embed
// lands at the cursor; oversized files are refused before upload; a failed
// save stays inline with Retry. Plus ⋯ → Rebuild search index.
import { expect, test, type Page } from '@playwright/test';
import { existsSync, readFileSync, readdirSync, writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { openPage } from './helpers';

// A 1×1 PNG.
const PNG_B64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

let workspaceId = '';
let vaultId = 0;
let vaultDir = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  ({ vaultId, dir: vaultDir } = await seedVaultDir(ctx, base, workspaceId));
  // A user file the paste must never overwrite.
  mkdirSync(join(vaultDir, 'attachments'), { recursive: true });
  writeFileSync(join(vaultDir, 'attachments', 'diagram.png'), 'user bytes');
  await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}/rescan`, { data: {} });
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript(({ ws, id }) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_vault_last', String(id));
  }, { ws: workspaceId, id: vaultId });
});

async function editDeploy(page: Page) {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await tree.getByText('runbooks', { exact: true }).click();
  await tree.getByText('deploy', { exact: true }).click();
  await page.locator('.mode-btn[title^="Edit"]').click();
  const editor = page.locator('.cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+ArrowDown' : 'Control+End');
  await page.keyboard.type('\n');
  return editor;
}

/** Fire a paste (or drop) carrying `name` with `size` bytes (PNG when null). */
async function sendFile(page: Page, kind: 'paste' | 'drop', name: string, size: number | null) {
  await page.evaluate(
    ({ kind, name, size, b64 }) => {
      const bytes =
        size === null ? Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)) : new Uint8Array(size).fill(97);
      const file = new File([bytes], name, { type: size === null ? 'image/png' : 'application/octet-stream' });
      const dt = new DataTransfer();
      dt.items.add(file);
      const target = document.querySelector('.editor-wrap .cm-content')!;
      const ev =
        kind === 'paste'
          ? new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true })
          : new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true });
      target.dispatchEvent(ev);
    },
    { kind, name, size, b64: PNG_B64 },
  );
}

test('a pasted screenshot is saved under attachments/ and embedded at the cursor', async ({ page }) => {
  const editor = await editDeploy(page);
  await sendFile(page, 'paste', 'image.png', null);
  await expect(editor).toContainText('![[attachments/Pasted image ');
  const files = readdirSync(join(vaultDir, 'attachments')).filter((f) => f.startsWith('Pasted image '));
  expect(files).toHaveLength(1);
  expect(files[0]).toMatch(/^Pasted image \d{14}\.png$/);
  // The saved embed reaches disk through autosave.
  await expect
    .poll(() => readFileSync(join(vaultDir, 'runbooks/deploy.md'), 'utf8'), { timeout: 10_000 })
    .toContain(`![[attachments/${files[0]}]]`);
});

test('a dropped file never overwrites an existing attachment', async ({ page }) => {
  const editor = await editDeploy(page);
  await sendFile(page, 'drop', 'diagram.png', null);
  await expect(editor).toContainText('![[attachments/diagram 1.png]]');
  expect(readFileSync(join(vaultDir, 'attachments', 'diagram.png'), 'utf8')).toBe('user bytes');
  expect(existsSync(join(vaultDir, 'attachments', 'diagram 1.png'))).toBe(true);
});

test('an oversized file is refused before upload with a clear message', async ({ page }) => {
  const editor = await editDeploy(page);
  let uploads = 0;
  await page.route('**/vault/vaults/*/attachment?*', (route) => {
    uploads++;
    return route.continue();
  });
  await sendFile(page, 'drop', 'huge.zip', 21 * 1024 * 1024);
  const err = page.getByTestId('load-error');
  await expect(err).toContainText('Couldn’t attach huge.zip');
  await expect(err).toContainText('attachments can be at most 20 MB');
  await expect(err.getByRole('button', { name: 'Retry' })).toHaveCount(0);
  expect(uploads).toBe(0);
  await err.getByRole('button', { name: 'Dismiss' }).click();
  await expect(err).toHaveCount(0);
  await expect(editor).not.toContainText('huge.zip');
});

test('a failed save stays inline with Retry, and Retry inserts the embed', async ({ page }) => {
  const editor = await editDeploy(page);
  await page.route('**/vault/vaults/*/attachment?*', (route) =>
    route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ code: 'internal', message: 'disk full' }) }),
  );
  await sendFile(page, 'paste', 'notes.png', null);
  const err = page.getByTestId('load-error');
  await expect(err).toContainText('Couldn’t attach notes.png');
  await expect(editor).not.toContainText('notes.png');
  await page.unroute('**/vault/vaults/*/attachment?*');
  await err.getByRole('button', { name: 'Retry' }).click();
  await expect(err).toHaveCount(0);
  await expect(editor).toContainText('![[attachments/notes.png]]');
});

test('a text paste is left to the editor', async ({ page }) => {
  const editor = await editDeploy(page);
  await page.evaluate(() => {
    const dt = new DataTransfer();
    dt.setData('text/plain', 'plain pasted words');
    document
      .querySelector('.editor-wrap .cm-content')!
      .dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
  });
  await expect(editor).toContainText('plain pasted words');
  await expect(page.locator('.att')).toHaveCount(0);
});

test('⋯ → Rebuild search index confirms, rebuilds, and search still answers', async ({ page }) => {
  await openPage(page, 'vault');
  await page.getByRole('button', { name: 'More vault actions', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Rebuild search index' }).click();
  await page.getByRole('button', { name: 'Rebuild', exact: true }).click();
  await expect(page.getByText('Rebuilding the search index', { exact: true })).toBeVisible();
  await page.locator('.left-modes button[title="Search"]').click();
  await page.locator('.search input').fill('charging customer');
  await page.locator('.search input').press('Enter');
  await expect(page.locator('.hit .t', { hasText: 'Orders API' })).toBeVisible({ timeout: 15_000 });
});
