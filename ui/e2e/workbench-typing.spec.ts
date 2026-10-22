import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Workbench — typing into a new (extension-less, `auto`) file char by char.
// The language is re-detected from the buffer; a detection change used to swap
// the editor's whole EditorState (the path carried the language), which reset
// the caret to 0 mid-word and dropped the undo history (ui-performance-4-01).
// A sentinel typed after each pause must land at the END of the text: if the
// caret had jumped, it would land at the start.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  workspaceId = await seedWorkspace(c.ctx, c.base);
  await c.ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'Desktop flow');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

/** New file with no extension, so the language is auto-detected. */
async function newUntitled(page: Page, name: string): Promise<void> {
  await page.getByTestId('wb-new').first().click();
  const prompt = page.locator('.sheet[role="dialog"] input.cf-input');
  if (await prompt.waitFor({ state: 'visible', timeout: 2_000 }).then(() => true, () => false)) {
    await prompt.fill(name);
    await prompt.press('Enter');
  }
  const editor = page.getByTestId('wb-editor').locator('.cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
}

const editorText = (page: Page) => page.getByTestId('wb-editor').locator('.cm-content');

for (const [typed, want] of [
  // closeBrackets auto-closes `{` and `"`, and types over the closers.
  ['{"a": 1}', '{"a": 1}'],
  ['# Heading', '# Heading'],
] as const) {
  test(`typing ${typed} char by char keeps the text and the caret`, async ({ page }) => {
    await page.goto('/#/workbench');
    await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
    await newUntitled(page, `typing ${Math.random().toString(36).slice(2, 8)}`);

    await page.keyboard.type(typed, { delay: 60 });
    await expect(editorText(page)).toHaveText(want);

    // Let the idle re-detection land (JSON / Markdown), then type on.
    await page.waitForTimeout(700); // ui-guards: allow — the 400 ms idle re-detection must land before the next keystroke
    await page.keyboard.type('X', { delay: 60 });
    await expect(editorText(page)).toHaveText(`${want}X`);
  });
}
