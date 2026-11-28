import { test, expect, type Page } from '@playwright/test';
import { idePane } from './ide-panes';
import { expectNoHorizontalOverflow } from './helpers';
import { bootWriteIde, writeShot } from './ide-write-fixtures';

// The IDE's Markdown formatting row on a phone: 44 px targets, the row scrolls inside
// itself (never the page), and a tap formats the line under the cursor.

const PHONES = ['iphone-portrait', 'android-phone'];

async function openReadmeByTap(page: Page): Promise<void> {
  const active = page.getByTestId('ide-tabs-strip').locator('.ide-tab.active');
  await page.waitForFunction(() => document.querySelector('[data-testid="ide-tabs-strip"] .ide-tab.active') || document.querySelector('[data-testid="ide-project-tree"] button.tree-item'));
  if (!((await active.count()) && (await active.textContent())?.includes('README.md'))) {
    await idePane(page, 'files');
    const file = page.getByTestId('ide-project-tree').locator('button.tree-item[title="README.md"], button.tree-item[title$="/README.md"]').first();
    await expect(file).toBeVisible({ timeout: 15_000 });
    await file.tap();
  }
  await idePane(page, 'editor');
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

// A long-press in the code is the system's text selection (Copy / Paste, and a
// selection for Bold): Android's long-press fires `contextmenu`, which used to
// open the IDE's own menu instead. That menu stays one tap away on the ⋯.
test('phone: a long-press in the code leaves the system selection alone', async ({ page }, info) => {
  // iOS Safari fires no contextmenu on a long-press; Android Chrome does.
  test.skip(info.project.name !== 'android-phone', 'Android phone');
  test.setTimeout(150_000);
  await bootWriteIde(page);
  await openReadmeByTap(page);
  const line = page.locator('.ide-editor-area .cm-line').first();
  await expect(line).toBeVisible({ timeout: 15_000 });
  // The editor's menu is live (it loads after the first paint) and one tap away on ⋯.
  await expect(async () => {
    await page.getByTestId('ide-editor-touch-more').tap();
    await expect(page.locator('.ctx-menu')).toContainText('Find Usages', { timeout: 1_000 });
  }).toPass({ timeout: 20_000 });
  await page.keyboard.press('Escape');
  await expect(page.locator('.ctx-menu')).toHaveCount(0);
  const box = (await line.boundingBox())!;
  const prevented = await line.evaluate((el, at) => {
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: at.x, clientY: at.y });
    el.dispatchEvent(ev);
    return ev.defaultPrevented;
  }, { x: box.x + 20, y: box.y + box.height / 2 });
  expect(prevented).toBe(false);
  await expect(page.locator('.ctx-menu')).toHaveCount(0);
});

// A Makefile's ▶ per target shows on a phone too: the compact editor gives the ▶
// slot's room to the breakpoint dot, and a Makefile has no breakpoints.
test('phone: a Makefile shows a tappable ▶ per target', async ({ page }, info) => {
  test.skip(!PHONES.includes(info.project.name), 'phones');
  test.setTimeout(150_000);
  await bootWriteIde(page);
  await idePane(page, 'files');
  const file = page.getByTestId('ide-project-tree').locator('button.tree-item[title="Makefile"], button.tree-item[title$="/Makefile"]').first();
  await expect(file).toBeVisible({ timeout: 15_000 });
  await file.tap();
  const run = page.locator('.ide-editor-area [data-gutter-run][aria-label="Run make build"]');
  await expect(run).toBeVisible({ timeout: 15_000 });
  expect((await run.boundingBox())!.width).toBeGreaterThanOrEqual(15);
  await run.tap();
  await expect(page.locator('.ctx-menu')).toContainText("Run 'make build'");
});
