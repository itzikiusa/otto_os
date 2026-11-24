import { test, expect, type Page } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';
import { bootWriteIde, writeShot } from './ide-write-fixtures';

// The IDE's Markdown formatting row on a phone: 44 px targets, the row scrolls inside
// itself (never the page), and a tap formats the line under the cursor.

const PHONES = ['iphone-portrait', 'android-phone'];

async function openReadmeByTap(page: Page): Promise<void> {
  const active = page.getByTestId('ide-tabs-strip').locator('.ide-tab.active');
  await page.waitForFunction(() => document.querySelector('[data-testid="ide-tabs-strip"] .ide-tab.active') || document.querySelector('[data-testid="ide-project-tree"] button.tree-item'));
  if (!((await active.count()) && (await active.textContent())?.includes('README.md'))) {
    await page.getByTestId('ide-stack-tab-files').tap();
    const file = page.getByTestId('ide-project-tree').locator('button.tree-item[title="README.md"], button.tree-item[title$="/README.md"]').first();
    await expect(file).toBeVisible({ timeout: 15_000 });
    await file.tap();
  }
  const editorTab = page.getByTestId('ide-stack-tab-editor');
  if ((await editorTab.getAttribute('aria-pressed')) !== 'true') await editorTab.tap();
  await expect(active).toContainText('README.md', { timeout: 15_000 });
}

test('phone: the Markdown row has 44 px targets, scrolls inside itself, and a tap formats', async ({ page }, info) => {
  test.skip(!PHONES.includes(info.project.name), 'phones');
  test.setTimeout(150_000);
  await bootWriteIde(page);
  await expect(page.getByTestId('ide-page')).toHaveClass(/\bstack\b/);
  await openReadmeByTap(page);
  const bar = page.getByTestId('ide-md-toolbar');
  await expect(bar).toBeVisible({ timeout: 15_000 });
  for (const b of await bar.locator('button').all()) {
    const box = (await b.boundingBox())!;
    expect(box.height, await b.getAttribute('aria-label') ?? '').toBeGreaterThanOrEqual(44);
    expect(box.width, await b.getAttribute('aria-label') ?? '').toBeGreaterThanOrEqual(44);
  }
  const vw = page.viewportSize()!.width;
  const barBox = (await bar.boundingBox())!;
  expect(barBox.x).toBeGreaterThanOrEqual(0);
  expect(barBox.x + barBox.width).toBeLessThanOrEqual(vw + 1);
  // The last button is reachable by scrolling the row itself.
  const more = page.getByTestId('ide-md-more');
  await more.scrollIntoViewIfNeeded();
  await expect(more).toBeInViewport();
  await expectNoHorizontalOverflow(page);

  // Tap into the last line, then tap Checklist: the line becomes a checklist item.
  const last = page.locator('.ide-editor-area .cm-line').last();
  await last.tap();
  await page.getByTestId('ide-md-check').scrollIntoViewIfNeeded();
  await page.getByTestId('ide-md-check').tap();
  await expect.poll(async () => (await page.locator('.ide-editor-area .cm-line').allTextContents()).some((t) => t.startsWith('- [ ] '))).toBe(true);
  // The cursor sits after the marker, so typing fills the item.
  await page.keyboard.type('ship it');
  await expect.poll(async () => (await page.locator('.ide-editor-area .cm-line').allTextContents()).includes('- [ ] ship it')).toBe(true);
  await expectNoHorizontalOverflow(page);
  await writeShot(page, `phone-md-row-${info.project.name}`);
});
