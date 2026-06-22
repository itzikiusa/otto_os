import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — Cmd/Ctrl+A selects the WHOLE query (guard).
//
// Guards that an editor-level Cmd/Ctrl+A selects the entire document, not just
// the rendered viewport (CodeMirror 6 only keeps on-screen lines in the DOM).
// The editor binds Mod-a → selectAll at highest precedence so nothing can shadow
// it. NOTE: in headless Chromium the keystroke routes through CodeMirror's keymap
// (where select-all already works), so this guards against a future regression
// that shadows Mod-a — it does NOT reproduce the macOS Tauri-webview path where a
// native "Select All" menu action can act on the virtualized contenteditable.
//
// Method (no query run): type a LONG (400-line) document so most of it is outside
// the rendered viewport, select-all, then type a short sentinel that REPLACES the
// selection. If select-all covered the whole doc, the persisted statement is
// exactly the sentinel; if it only covered the viewport, the remainder survives.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId: string | null = null;

const PHONE_MAX = 640;

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    connId = await seedDockerConnection(ctx, base, workspaceId, 'mysql');
  } catch {
    connId = null;
  }
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

function isPhone(page: Page): boolean {
  const w = page.viewportSize()?.width ?? 0;
  return w > 0 && w <= PHONE_MAX;
}

async function openMysql(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mysql' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
}

async function ensureEditorOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const editor = page.locator('.qe-edit');
  if (!(await editor.isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Editor' }).click();
  }
  await expect(editor).toBeVisible();
}

test('Cmd/Ctrl+A selects the whole query, not just the visible viewport', async ({ page }) => {
  test.skip(connId == null, 'mysql connection unavailable');
  test.setTimeout(90_000);
  await openMysql(page);
  await ensureEditorOpen(page);

  const content = page.locator('.qe-edit .cm-content');
  const mod = process.platform === 'darwin' ? 'Meta' : 'Control';

  // A long document — far taller than the editor viewport + CM's render margin —
  // so a viewport-only select-all would leave most of it behind.
  const longDoc = Array.from({ length: 400 }, (_, i) => `-- line ${i + 1}: SELECT ${i + 1};`).join('\n');

  await content.click();
  await expect(content).toBeFocused({ timeout: 5_000 });
  await page.keyboard.insertText(longDoc);
  // The cursor lands at the end, so CM renders the tail — its presence proves the
  // long doc is loaded before we select-all. (Asserts on the editor's own DOM, so
  // it doesn't depend on workspace/daemon state shared across parallel projects.)
  await expect(content).toContainText('line 400', { timeout: 15_000 });

  // Select-all, then replace the whole selection with a short sentinel.
  await content.click();
  await expect(content).toBeFocused({ timeout: 5_000 });
  await page.keyboard.press(`${mod}+A`);
  await page.keyboard.insertText('SELECT 1');

  // If select-all covered the whole document, the editor now holds ONLY the
  // sentinel — every original "-- line N" is gone. If it only covered the
  // viewport, the unselected remainder (still "-- line …") survives and renders.
  await expect(content).toContainText('SELECT 1', { timeout: 15_000 });
  await expect(content).not.toContainText('-- line', { timeout: 15_000 });
});
