import { test, expect, type Page } from '@playwright/test';
import { bootWriteIde, openTreeFile, writeShot } from './ide-write-fixtures';

// Writing Markdown in the IDE (owner, 2026-11-24: "for .md files maybe have some helper to
// create them more easily, hints or whatever"): a formatting row above the editor, list /
// checklist continuation on Enter, a "/" block menu, Tab through table cells, Format table,
// ⌘B over the JetBrains Go to Definition, the rendered Split, and headings in Structure.

const editor = (page: Page) => page.locator('.ide-editor-area .cm-editor').first();
const lines = (page: Page) => editor(page).locator('.cm-line').allTextContents();
const bar = (page: Page) => page.getByTestId('ide-md-toolbar');

async function selectWord(page: Page, lineText: string, word: string): Promise<void> {
  const l = editor(page).locator('.cm-line', { hasText: lineText }).first();
  const box = await l.evaluate((el, w) => {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const i = n.textContent!.indexOf(w);
      if (i >= 0) {
        const r = document.createRange();
        r.setStart(n, i + 1);
        r.setEnd(n, i + 2);
        const b = r.getBoundingClientRect();
        return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
      }
    }
    return null;
  }, word);
  expect(box).not.toBeNull();
  await page.mouse.dblclick(box!.x, box!.y);
}

for (const scheme of ['dark', 'light'] as const) {
  test(`README: formatting row, Enter continues a checklist, "/" table, Tab cells, Format table, Split, Structure (${scheme})`, async ({ page }) => {
    test.setTimeout(150_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await bootWriteIde(page, scheme);
    await openTreeFile(page, 'cmd/main.go');
    await expect(bar(page)).toHaveCount(0); // code files have no Markdown row
    await openTreeFile(page, 'README.md');
    await expect(bar(page)).toBeVisible();
    for (const [id, title] of [['bold', 'Bold (⌘B)'], ['italic', 'Italic'], ['code', 'Code'], ['link', 'Link (paste a URL over text)'], ['check', 'Checklist'], ['table', 'Table (Tab moves between cells)']] as const) {
      await expect(page.getByTestId(`ide-md-${id}`)).toHaveAttribute('title', title);
      await expect(page.getByTestId(`ide-md-${id}`)).toHaveAttribute('aria-label', title);
    }

    // Bold from the row wraps the selection; ⌘I (a shortcut) too.
    await selectWord(page, 'Intro paragraph', 'Intro');
    await page.getByTestId('ide-md-bold').click();
    await expect.poll(() => lines(page)).toContain('**Intro** paragraph.');
    await selectWord(page, 'paragraph', 'paragraph');
    await page.getByTestId('ide-md-italic').click();
    await expect.poll(() => lines(page)).toContain('**Intro** *paragraph*.');
    // Typing ` over a selection wraps it.
    await selectWord(page, 'Setup', 'Setup');
    await page.keyboard.type('`');
    await expect.poll(() => lines(page)).toContain('## `Setup`');
    await page.keyboard.press('Meta+z');
    // ⌘B in Markdown is Bold (the JetBrains keymap's ⌘B is Go to Definition elsewhere).
    await selectWord(page, 'Run make', 'make');
    await page.keyboard.press('Meta+b');
    await expect.poll(() => lines(page)).toContain('Run **make**.');

    // Enter continues a checklist; Enter on an empty item ends it.
    await editor(page).locator('.cm-content').click();
    await page.keyboard.press('Meta+End');
    await page.keyboard.type('\n- [ ] first');
    await page.keyboard.press('Enter');
    await page.keyboard.type('second');
    await expect.poll(() => lines(page)).toContain('- [ ] second');
    await page.keyboard.press('Enter');
    await page.keyboard.press('Enter');
    const after = await lines(page);
    expect(after[after.length - 1]).toBe('');

    // "/" on the empty line: the block menu; Table inserts a 3×3 table.
    await page.keyboard.type('/');
    const menu = page.locator('.cm-tooltip-autocomplete');
    await expect(menu).toBeVisible({ timeout: 10_000 });
    await expect(menu).toContainText('Heading 1');
    await expect(menu).toContainText('Callout');
    await page.keyboard.type('tab');
    await expect(menu.locator('[aria-selected="true"]')).toContainText('Table');
    await page.keyboard.press('Enter');
    await expect.poll(() => lines(page)).toContain('| Column 1 | Column 2 | Column 3 |');
    // Tab moves to the next cell (the header text is selected, so typing replaces it).
    await page.keyboard.type('Name');
    await page.keyboard.press('Tab');
    await page.keyboard.type('Value');
    await expect.poll(() => lines(page)).toContain('| Name | Value | Column 3 |');
    // ⋯ → Format table aligns the columns.
    await page.getByTestId('ide-md-more').click();
    await page.locator('.ctx-menu').getByText('Format table').click();
    await expect.poll(() => lines(page)).toContain('| ---- | ----- | -------- |');
    await expect.poll(() => lines(page)).toContain('|      |       |          |');

    // Split renders the live buffer next to the editor.
    await page.getByTestId('ide-preview-mode-split').click();
    const md = page.getByTestId('ide-preview-pane').getByTestId('file-preview-markdown');
    await expect(md.getByRole('heading', { name: 'Games Management', level: 1 })).toBeVisible({ timeout: 15_000 });
    await expect(md.locator('strong', { hasText: 'Intro' })).toBeVisible();
    await expect(md.locator('em', { hasText: 'paragraph' })).toBeVisible();
    await expect(md.locator('th', { hasText: 'Value' })).toBeVisible();
    await expect(md.locator('input[type="checkbox"]')).toHaveCount(2);
    await expect(bar(page)).toBeVisible(); // the row stays in Split
    await writeShot(page, `readme-split-${scheme}`);

    // Headings in Structure, a click jumps.
    await page.getByTestId('ide-tree-tab-structure').click();
    const row = page.locator('.struct-row', { hasText: 'Setup' });
    await expect(row).toBeVisible({ timeout: 15_000 });
    await row.click();
    await expect(page.getByTestId('ide-status-cursor')).toHaveText(/^Ln 5,/);
    // Preview only: the row goes (nothing to format).
    await page.getByTestId('ide-preview-mode-preview').click();
    await expect(bar(page)).toHaveCount(0);
    await page.getByTestId('ide-preview-mode-source').click();
  });
}
