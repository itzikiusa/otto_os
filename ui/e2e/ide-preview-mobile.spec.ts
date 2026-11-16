import { test, expect, type Page } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { bootPreviewIde, previewShot, treeFile } from './ide-preview-fixtures';

// IDE editor preview on a phone: the stacked layout hides the editor sub-bar,
// so the Editor | Preview control lives in the always-visible tab strip (and
// again in its ▾ menu). Split is not offered there. The preview fills the pane,
// scrolls its own wide tables/code, and never scrolls the page sideways.

const PHONES = ['iphone-portrait', 'iphone-landscape', 'iphone-se', 'android-phone'];

async function openReadmeByTap(page: Page): Promise<void> {
  // A fresh project opens its README by itself (and that switches to the
  // Editor pane, racing a tap on Files) — tap it in the tree only when it is
  // not already the open file.
  const active = page.getByTestId('ide-tabs-strip').locator('.ide-tab.active');
  await page.waitForFunction(() => document.querySelector('[data-testid="ide-tabs-strip"] .ide-tab.active') || document.querySelector('[data-testid="ide-project-tree"] button.tree-item'));
  if (!((await active.count()) && (await active.textContent())?.includes('README.md'))) {
    const files = page.getByTestId('ide-stack-tab-files');
    await files.click();
    const file = treeFile(page, 'README.md');
    await expect(file).toBeVisible({ timeout: 15_000 });
    await file.click(); // one tap opens it
  }
  const editorTab = page.getByTestId('ide-stack-tab-editor');
  if ((await editorTab.getAttribute('aria-pressed')) !== 'true') await editorTab.click();
  await expect(editorTab).toHaveAttribute('aria-pressed', 'true');
  await expect(active).toContainText('README.md', { timeout: 15_000 });
}

for (const scheme of ['dark', 'light'] as const) {
  test(`phone: Editor | Preview reachable, preview fills the pane, no sideways scroll (${scheme})`, async ({ page }, info) => {
    test.skip(!PHONES.includes(info.project.name), 'phones');
    test.setTimeout(150_000);
    const p = info.project.name;
    await bootPreviewIde(page, scheme);
    await expect(page.getByTestId('ide-page')).toHaveClass(/\bstack\b/);
    await openReadmeByTap(page);

    const toggle = page.getByTestId('ide-preview-toggle');
    await expectFullyInViewport(page, toggle, `${p}: preview control`);
    await expect(page.getByTestId('ide-preview-mode-split')).toHaveCount(0);
    await expect(page.getByTestId('ide-preview-mode-source')).toHaveAttribute('aria-pressed', 'true');

    // The tab strip's ▾ menu offers the same two modes.
    await page.getByTestId('ide-tabs-overflow-btn').click();
    const menu = page.locator('.ctx-menu');
    await expect(menu.getByRole('menuitem', { name: /^Preview/ }).or(menu.getByRole('menuitemcheckbox', { name: /^Preview/ })).first()).toBeVisible();
    await expect(menu.getByText('Editor and preview')).toHaveCount(0);
    await page.keyboard.press('Escape');

    await page.getByTestId('ide-preview-mode-preview').click();
    const pane = page.getByTestId('ide-preview-pane');
    await expect(pane).toHaveAttribute('data-layout', 'only');
    const md = pane.getByTestId('file-preview-markdown');
    await expect(md.getByRole('heading', { name: 'Otto Preview Fixture', level: 1 })).toBeVisible({ timeout: 15_000 });
    await expect(md.locator('div.diagram-block svg').first()).toBeVisible({ timeout: 30_000 });

    // Fills the editor pane's width (no half-empty screen).
    const vw = page.viewportSize()!.width;
    const box = (await pane.boundingBox())!;
    expect(box.width, `${p}: preview width`).toBeGreaterThanOrEqual(vw * 0.9);
    expect(box.height, `${p}: preview height`).toBeGreaterThan(120);
    await expectNoHorizontalOverflow(page);
    await previewShot(page, `${p}-preview-${scheme}`);

    // Back to the editor from the same control.
    await page.getByTestId('ide-preview-mode-source').click();
    await expect(page.locator('.ide-editor-area .cm-editor').first()).toBeVisible();
    await expect(pane).toHaveCount(0);
  });
}
