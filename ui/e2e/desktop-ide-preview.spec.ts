import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expectNoHorizontalOverflow } from './helpers';
import { bootPreviewIde, previewShot, treeFile, type PreviewSeed } from './ide-preview-fixtures';

// IDE editor preview (desktop): the owner opened README.md in the IDE and got
// only its source. The Editor | Editor and preview | Preview trio at the end of
// the tab strip shows the file rendered with the app's shared FilePreview —
// Markdown (tables, task lists, highlighted fences, Mermaid, relative links and
// images), HTML in a sandbox that never gets Otto's origin, JSON as a tree,
// images as images — live from the unsaved buffer, remembered per file kind.

test.describe.configure({ mode: 'serial' });

async function openFile(page: Page, rel: string): Promise<void> {
  const tree = page.getByTestId('ide-project-tree');
  const parts = rel.split('/');
  for (let i = 0; i < parts.length - 1; i++) {
    const dir = treeFile(page, parts.slice(0, i + 1).join('/'));
    await expect(dir).toBeVisible({ timeout: 15_000 });
    if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.click();
  }
  await expect(tree).toBeVisible();
  const file = treeFile(page, rel);
  await expect(file).toBeVisible({ timeout: 15_000 });
  await file.dblclick();
  const name = parts[parts.length - 1];
  await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab.active')).toContainText(name, { timeout: 15_000 });
}

const toggle = (page: Page) => page.getByTestId('ide-preview-toggle');
const mode = (page: Page, m: 'source' | 'split' | 'preview') => page.getByTestId(`ide-preview-mode-${m}`);
const pane = (page: Page) => page.getByTestId('ide-preview-pane');
const preview = (page: Page) => pane(page).getByTestId('file-preview');

test('README previews rendered: trio, Markdown features, live Split without saving, relative link, memory', async ({ page }) => {
  test.setTimeout(180_000);
  const seed: PreviewSeed = await bootPreviewIde(page);

  // Code files get no control.
  await openFile(page, 'app/main.go');
  await expect(toggle(page)).toHaveCount(0);

  await openFile(page, 'README.md');
  await expect(toggle(page)).toBeVisible();
  for (const [m, label] of [['source', 'Editor'], ['split', 'Editor and preview'], ['preview', 'Preview']] as const) {
    await expect(mode(page, m)).toHaveAttribute('aria-label', label);
    await expect(mode(page, m)).toHaveAttribute('title', label);
  }
  await expect(mode(page, 'source')).toHaveAttribute('aria-pressed', 'true');
  await expect(pane(page)).toHaveCount(0);

  // Preview: rendered in place of the editor.
  await mode(page, 'preview').click();
  await expect(mode(page, 'preview')).toHaveAttribute('aria-pressed', 'true');
  await expect(pane(page)).toHaveAttribute('data-layout', 'only');
  await expect(preview(page)).toHaveAttribute('data-kind', 'markdown');
  await expect(page.locator('.ide-editor-area .cm-editor').first()).toBeHidden();
  const md = preview(page).getByTestId('file-preview-markdown');
  await expect(md.getByRole('heading', { name: 'Otto Preview Fixture', level: 1 })).toBeVisible({ timeout: 15_000 });
  await expect(md.locator('table td', { hasText: 'alpha' })).toBeVisible();
  await expect(md.locator('input[type="checkbox"]')).toHaveCount(2);
  await expect(md.locator('input[type="checkbox"]').first()).toBeChecked();
  await expect(md.locator('pre code.hljs')).toContainText('fmt.Println');
  await expect(md.locator('div.diagram-block svg').first()).toBeVisible({ timeout: 30_000 });
  // The relative image came through the daemon's file route as a blob.
  await expect.poll(() => md.locator('img[alt="one pixel"]').evaluate((i) => (i as HTMLImageElement).src)).toMatch(/^blob:/);

  // Split: editor beside the preview; typing updates the preview, nothing saved.
  await mode(page, 'split').click();
  await expect(pane(page)).toHaveAttribute('data-layout', 'split');
  const editor = page.locator('.ide-editor-area .cm-editor').first();
  await expect(editor).toBeVisible();
  const eBox = (await editor.boundingBox())!;
  const pBox = (await pane(page).boundingBox())!;
  expect(pBox.x, 'preview sits to the right of the editor').toBeGreaterThanOrEqual(eBox.x + eBox.width - 2);
  await expect(page.getByRole('separator', { name: 'Resize the preview' })).toBeVisible();
  await previewShot(page, 'desktop-split-readme-dark');

  await page.locator('.ide-editor-area .cm-content').first().click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('\n\n## Live Section Added\n');
  await expect(md.getByRole('heading', { name: 'Live Section Added' })).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab.active .ide-dirty-dot')).toBeVisible();
  expect(readFileSync(join(seed.workspaceRoot, 'README.md'), 'utf8')).not.toContain('Live Section Added');

  // A relative link opens the other file in the IDE (Markdown is in Split now: remembered per kind).
  await md.getByRole('link', { name: 'guide link' }).click();
  await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab.active')).toContainText('guide.md', { timeout: 15_000 });
  await expect(mode(page, 'split')).toHaveAttribute('aria-pressed', 'true');
  await expect(preview(page).getByRole('heading', { name: 'Guide Page' })).toBeVisible({ timeout: 15_000 });

  // Mode remembered after a reload (localStorage, per kind).
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await openFile(page, 'docs/guide.md');
  await expect(mode(page, 'split')).toHaveAttribute('aria-pressed', 'true');
  await expect(preview(page).getByRole('heading', { name: 'Guide Page' })).toBeVisible({ timeout: 15_000 });
  await expectNoHorizontalOverflow(page);
});

test('HTML runs sandboxed without Otto’s origin; JSON tree; images and SVG as images', async ({ page }) => {
  test.setTimeout(150_000);
  await bootPreviewIde(page);

  await openFile(page, 'index.html');
  await expect(mode(page, 'source')).toHaveAttribute('aria-pressed', 'true');
  await mode(page, 'preview').click();
  const frameEl = pane(page).getByTestId('file-preview-html-frame');
  await expect(frameEl).toBeVisible();
  await expect(frameEl).toHaveAttribute('sandbox', '');
  await expect(pane(page).getByTestId('file-preview-scripts-state')).toHaveText(/Scripts off/);
  const frame = page.frameLocator('[data-testid="file-preview-html-frame"]');
  await expect(frame.getByRole('heading', { name: 'Sandboxed Page' })).toBeVisible();
  await expect(frame.locator('#probe')).toHaveText('not run');

  await pane(page).getByTestId('file-preview-scripts-toggle').click();
  await expect(pane(page).getByTestId('file-preview-html-frame')).toHaveAttribute('sandbox', 'allow-scripts');
  const sandboxAttr = await pane(page).getByTestId('file-preview-html-frame').getAttribute('sandbox');
  expect(sandboxAttr ?? '').not.toContain('allow-same-origin');
  // Asserted from inside the frame: its own script cannot reach the parent or Otto's storage.
  await expect(frame.locator('#probe')).toHaveText('parent:blocked storage:blocked', { timeout: 10_000 });
  await previewShot(page, 'desktop-html-preview-dark');

  // JSON: a tree; broken JSON: an inline message, not a crash.
  await openFile(page, 'data.json');
  await mode(page, 'preview').click();
  const tree = preview(page).getByTestId('file-preview-json');
  await expect(tree).toBeVisible();
  await expect(tree).toContainText('nested');
  await openFile(page, 'broken.json');
  await expect(mode(page, 'preview')).toHaveAttribute('aria-pressed', 'true');
  await expect(preview(page).getByTestId('file-preview-error')).toContainText('Invalid JSON');
  await mode(page, 'source').click();
  await expect(page.locator('.ide-editor-area .cm-editor').first()).toBeVisible();

  // Images preview straight away (no source to edit, so no control).
  await openFile(page, 'pixel.png');
  await expect(toggle(page)).toHaveCount(0);
  await expect(preview(page).getByTestId('file-preview-image')).toBeVisible({ timeout: 15_000 });
  await expect(preview(page).getByTestId('file-preview-image-meta')).toContainText('1 × 1 px');
  await expect(preview(page).getByRole('button', { name: '100%' })).toBeVisible();

  await openFile(page, 'logo.svg');
  await expect(mode(page, 'preview')).toHaveAttribute('aria-pressed', 'true');
  const svgImg = preview(page).locator('img[alt="logo.svg"]');
  await expect(svgImg).toBeVisible();
  expect(await svgImg.getAttribute('src')).toMatch(/^data:image\/svg\+xml/);
});

test('light theme screenshots: Split README and HTML preview', async ({ page }) => {
  test.skip(!process.env.OTTO_E2E_SHOTS, 'screenshots only when OTTO_E2E_SHOTS is set');
  test.setTimeout(120_000);
  await bootPreviewIde(page, 'light');
  await openFile(page, 'README.md');
  await mode(page, 'split').click();
  await expect(preview(page).locator('div.diagram-block svg').first()).toBeVisible({ timeout: 30_000 });
  await previewShot(page, 'desktop-split-readme-light');
  await openFile(page, 'index.html');
  await mode(page, 'preview').click();
  await expect(page.frameLocator('[data-testid="file-preview-html-frame"]').getByRole('heading', { name: 'Sandboxed Page' })).toBeVisible();
  await previewShot(page, 'desktop-html-preview-light');
});
