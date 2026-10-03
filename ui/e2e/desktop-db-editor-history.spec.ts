import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — a query tab's undo history outlives the editor, and the
// clipboard ring pastes copies made in Otto.
//
//   • Query → Structure → Query used to rebuild CodeMirror and wipe ⌘Z for
//     every tab (the parked states lived in the component instance);
//   • a reload renumbered tab ids, so nothing could survive it — tabs now
//     carry a stable uid and the history is persisted to IndexedDB;
//   • ⌥⌘V opens the ring of copies made in Otto and inserts the pick.
// Uses the Docker-free mocked connection (db-mock.ts).
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let workspaceId = '';
let connId = '';
const CONN = `mock-hist-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, workspaceId, CONN);
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openEditor(page: Page): Promise<void> {
  await mockDbRoutes(page, connId);
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: CONN });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.qe-edit .cm-content')).toBeVisible({ timeout: 20_000 });
}

/** The editor's text, read through the CM view. */
async function text(page: Page): Promise<string> {
  return page.evaluate(() => {
    const el = document.querySelector('.qe-edit .cm-content') as HTMLElement & {
      cmTile?: { root?: { view?: { state: { doc: { toString(): string } } } } };
    };
    const view = el.cmTile?.root?.view;
    if (!view) throw new Error('no CodeMirror view');
    return view.state.doc.toString();
  });
}

/** Type two undo events: "SELECT 1" then " + 2". */
async function typeTwoEdits(page: Page): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.press('Delete');
  await page.keyboard.insertText('SELECT 1');
  await page.waitForTimeout(600); // its own undo event
  await page.keyboard.type(' + 2');
  await expect.poll(() => text(page)).toBe('SELECT 1 + 2');
}

async function undo(page: Page): Promise<void> {
  await page.locator('.qe-edit .cm-content').click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.press('ControlOrMeta+Z');
}

test('undo survives a Query → Structure → Query round trip', async ({ page }) => {
  await openEditor(page);
  await typeTwoEdits(page);
  await page.locator('.view-switch button', { hasText: 'Structure' }).click();
  await expect(page.locator('.qe-edit .cm-content')).toHaveCount(0);
  await page.locator('.view-switch button', { hasText: 'Query' }).click();
  await expect(page.locator('.qe-edit .cm-content')).toBeVisible();
  await expect.poll(() => text(page)).toBe('SELECT 1 + 2');
  await undo(page);
  await expect.poll(() => text(page)).toBe('SELECT 1');
});

test('undo survives a reload', async ({ page }) => {
  await openEditor(page);
  await typeTwoEdits(page);
  // History is written 2 s after the last edit (and on page hide).
  await page.waitForTimeout(2_600);
  await page.reload();
  await expect(page.locator('.qe-edit .cm-content')).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => text(page)).toBe('SELECT 1 + 2');
  await undo(page);
  await expect.poll(() => text(page)).toBe('SELECT 1');
});

test('⌥⌘V pastes a copy made in the editor from the clipboard ring', async ({ page }) => {
  await openEditor(page);
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.press('Delete');
  await page.keyboard.insertText('SELECT ring_marker_42');
  await page.keyboard.press('ControlOrMeta+A');
  // Fire the DOM copy the editor records (headless clipboards vary by engine).
  await page.evaluate(() => {
    const el = document.querySelector('.qe-edit .cm-content') as HTMLElement;
    el.dispatchEvent(new ClipboardEvent('copy', { clipboardData: new DataTransfer(), bubbles: true, cancelable: true }));
  });
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.insertText('\n');
  await page.keyboard.press('Alt+ControlOrMeta+KeyV');
  const item = page.locator('.ctx-menu [role="menuitem"]', { hasText: 'ring_marker_42' });
  await expect(item.first()).toBeVisible({ timeout: 5_000 });
  await item.first().click();
  await expect.poll(() => text(page)).toBe('SELECT ring_marker_42\nSELECT ring_marker_42');
});
