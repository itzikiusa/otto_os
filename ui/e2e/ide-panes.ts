import { expect, type Page } from '@playwright/test';

// The IDE on a phone (design.md §2, shell/ideLayout.svelte.ts): the explorer
// is the page and a file is pushed over it with "‹ Files"; ⋯ › "Terminal and
// tools" fills the content with the bottom dock (its chevron goes back); the
// agent is a Detail pane (a full-width sheet). On a workbench (tablet,
// desktop) every pane is on screen: nothing to do.
export type IdePane = 'files' | 'editor' | 'tools' | 'agent';

async function stacked(page: Page): Promise<boolean> {
  return page.getByTestId('ide-page').evaluate((el) => el.classList.contains('stack'));
}

export async function idePane(page: Page, pane: IdePane): Promise<void> {
  if (!(await stacked(page))) {
    if (pane === 'agent' && !(await page.getByTestId('ide-agent').isVisible())) await page.getByTestId('ide-toggle-agent').click();
    return;
  }
  const header = page.getByTestId('page-header').first();
  const back = header.getByTestId('phone-back');
  const agent = page.getByTestId('ide-agent');
  const closeTools = page.getByTestId('ide-dock-close-tools');
  if (pane !== 'agent' && (await agent.isVisible())) await page.getByTestId('ide-agent-close').click();
  if (pane === 'files') {
    if (await back.isVisible()) await back.click();
    await expect(page.getByTestId('ide-project-tree')).toBeVisible();
    return;
  }
  // The content page: re-open the marked file (a tap pushes it, unchanged).
  if (!(await back.isVisible())) {
    await page.getByTestId('ide-project-tree').locator('button.tree-item[aria-selected="true"]').first().click();
    await expect(back).toBeVisible();
  }
  if (pane === 'editor') {
    if (await closeTools.isVisible()) await closeTools.click();
  } else if (pane === 'tools') {
    if (!(await closeTools.isVisible())) {
      await header.getByTestId('ide-more-btn').click();
      await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Terminal and tools' }).click();
    }
  } else if (!(await agent.isVisible())) {
    await header.getByTestId('ide-toggle-agent').click();
  }
  await expectIdePane(page, pane);
}

export async function expectIdePane(page: Page, pane: IdePane): Promise<void> {
  if (pane === 'files') await expect(page.getByTestId('ide-project-tree')).toBeVisible();
  if (pane === 'editor') await expect(page.locator('.ide-editor-area .cm-editor').first()).toBeVisible();
  if (pane === 'tools') await expect(page.getByTestId('ide-bottom-dock')).toBeVisible();
  if (pane === 'agent') await expect(page.getByTestId('ide-agent-panel')).toBeVisible();
}
