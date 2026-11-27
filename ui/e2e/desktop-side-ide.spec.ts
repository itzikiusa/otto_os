import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall, seedPolyglotFiles } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

test.use({ serviceWorkers: 'block' });

let seed: { workspaceId: string; root: string } | null = null;
async function getSeed(): Promise<{ workspaceId: string; root: string }> {
  if (seed) return seed;
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-side-ide-')));
  seedPolyglotFiles(root);
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `Side IDE ${Date.now()}`, root_path: root });
    seed = { workspaceId: ws.id, root };
    return seed;
  } finally {
    await c.ctx.dispose();
  }
}

async function bootIde(page: Page, pane = 'files'): Promise<void> {
  const s = await getSeed();
  await page.addInitScript(
    ({ wsId, root, pane }) => {
      if (sessionStorage.getItem('__side_ide_init')) return;
      sessionStorage.setItem('__side_ide_init', '1');
      sessionStorage.setItem('otto.ide.stackPane', pane);
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.agent.provider', 'claude');
      localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-side', name: 'otto-web', folders: [{ path: root, name: 'otto-web' }], updatedAt: Date.now() }]));
    },
    { wsId: s.workspaceId, root: s.root, pane },
  );
  await page.goto('/#/ide');
  await expect(page.getByTestId('ide-page')).toBeVisible({ timeout: 20_000 });
}

async function shot(page: Page, name: string): Promise<void> {
  const out = process.env.OTTO_E2E_SHOTS;
  if (!out) return;
  mkdirSync(out, { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
    await page.screenshot({ path: join(out, `${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

const tree = (page: Page) => page.getByTestId('ide-project-tree');
const row = (page: Page, rel: string) => tree(page).locator(`button.tree-item[title="${rel}"]`).first();

// The IDE's side surfaces on a desktop (design.md §3, §7): the explorer is a
// ListPane whose views are its header (Files | Structure | Breakpoints · + ·
// ⋯); ⋯ starts with "Hide Files ⌘B"; while hidden "Show Files" sits in the
// page header; ⌘B toggles it; its width persists under otto.pane.ide.list.w
// (the old otto.ide.treeW is read once). The agent is the Detail pane
// (⌥⌘B / its ✕ / Esc).

test.use({ viewport: { width: 1440, height: 900 } });

test('explorer pane header, hide / show, ⌘B, width key; agent Detail pane', async ({ page }) => {
  test.setTimeout(120_000);
  await page.addInitScript(() => { if (!localStorage.getItem('otto.pane.ide.list.w')) localStorage.setItem('otto.ide.treeW', '300'); });
  await bootIde(page, 'editor');
  await expect(tree(page)).toBeVisible({ timeout: 20_000 });
  await expect(tree(page).getByTestId('ide-tree-tab-files')).toBeVisible();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto.pane.ide.list.w'))).toBe('300');
  const dir = row(page, 'app');
  await expect(dir).toBeVisible({ timeout: 15_000 });
  if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.click();
  await row(page, 'app/main.go').click();
  await expect(row(page, 'app/main.go')).toHaveAttribute('aria-selected', 'true');
  await shot(page, 'ide-desktop');

  // ⋯ › Hide Files; "Show Files" in the page header brings it back.
  await page.getByTestId('ide-project-tree-more').click();
  await page.getByRole('menuitem', { name: /Hide Files/ }).click();
  await expect(tree(page)).toBeHidden();
  await expect(page.getByTestId('show-list-pane')).toBeVisible();
  await page.getByTestId('show-list-pane').click();
  await expect(tree(page)).toBeVisible();
  // ⌘B (the IDE's standard keymap) toggles the same pane, and it is remembered.
  await page.locator('.ide-editor-area .cm-content').first().click();
  await page.keyboard.press('Meta+b');
  await expect(tree(page)).toBeHidden();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto.pane.ide.list.hidden'))).toBe('1');
  await page.keyboard.press('Meta+b');
  await expect(tree(page)).toBeVisible();

  // The agent: the Detail pane, inline at 1440.
  await page.getByTestId('ide-toggle-agent').click();
  const agent = page.getByTestId('ide-agent');
  await expect(agent).toBeVisible();
  await expect(agent.getByTestId('ide-agent-sessions')).toBeVisible();
  await shot(page, 'ide-desktop-agent');
  await page.getByTestId('ide-agent-close').click();
  await expect(agent).toBeHidden();
  // Structure view.
  await tree(page).getByTestId('ide-tree-tab-structure').click();
  await expect(page.locator('.struct-row').first()).toBeVisible({ timeout: 20_000 });
  await shot(page, 'ide-desktop-structure');
});
