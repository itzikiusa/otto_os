import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { appendFileSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { bootPolyglotIde, createPolyglotSeed, daemonCall, openPolyglotFile, type PolyglotWorkspaceSeed } from './ide-polyglot-fixtures';

// IDE tool windows (round 5, IDE-TREE-DOCK):
//  I1-11 the tree shows M / ? for changed files WITHOUT the Git tab ever opening (the
//        overlay keys and the tree rows used two spellings of macOS temp paths), and the
//        mark clears after a commit;
//  I1-14 a fresh dock offers Terminal, Problems, Git, Run; Debug appears when a debug run starts;
//  I1-23 the explorer header is filter + New file + ⋯, and switching tabs reveals the file;
//  I1-15 at 1280 the agent opening hides the explorer so the editor keeps ≥ 700 px;
//  I1-24 the Git diff header carries one layout control.

let seed: PolyglotWorkspaceSeed;

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-IDE-TREE-DOCK'), { recursive: true });
  await page.screenshot({ path: join(dir, 'fix-IDE-TREE-DOCK', name) });
}

const treeRow = (page: Page, rel: string) =>
  page.getByTestId('ide-project-tree').locator(`button.tree-item[title="${rel}"], button.tree-item[title$="/${rel}"]`).first();

async function dockTabIds(page: Page): Promise<string[]> {
  return page.getByTestId('ide-dock-tabs').getByRole('tab').evaluateAll((els) => els.map((e) => e.getAttribute('data-testid') ?? ''));
}

async function liveDebugIds(): Promise<string[]> {
  const c = await apiCtx();
  try {
    const r = await daemonCall<{ sessions: { id: string; state: string }[] }>(c.ctx, c.base, c.token, 'GET', '/ide/debug/sessions');
    return r.sessions.filter((s) => s.state !== 'terminated').map((s) => s.id);
  } finally {
    await c.ctx.dispose();
  }
}

test.describe('Desktop IDE — tool windows', () => {
  test.describe.configure({ mode: 'serial' });
  test.use({ viewport: { width: 1280, height: 800 } });

  test.beforeAll(async () => {
    test.setTimeout(60_000);
    seed = await createPolyglotSeed();
    // One tracked change and one untracked file before the IDE ever opens.
    appendFileSync(join(seed.workspaceRoot, 'notes', 'roadmap.md'), '\nChanged outside the IDE.\n');
    writeFileSync(join(seed.workspaceRoot, 'notes', 'untracked-new.md'), '# New\n');
  });

  test.afterAll(() => {
    if (seed?.workspaceRoot) rmSync(seed.workspaceRoot, { recursive: true, force: true });
  });

  test.beforeEach(async ({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop tool windows');
  });

  test('tree git marks without the Git tab, a calm dock and explorer header, reveal on tab switch', async ({ page }) => {
    test.setTimeout(120_000);
    for (const scheme of ['dark', 'light'] as const) {
      await bootPolyglotIde(page, seed, scheme);
      if (scheme === 'light') await page.reload(); // same URL: goto alone would not re-run the init script
      await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 20_000 });
      // I1-11: no Git tab click anywhere before these.
      await expect(treeRow(page, 'notes')).toBeVisible({ timeout: 15_000 });
      if ((await treeRow(page, 'notes').getAttribute('aria-expanded')) !== 'true') await treeRow(page, 'notes').click();
      await expect(treeRow(page, 'notes/roadmap.md').getByTestId('ide-tree-git-status')).toHaveText('M', { timeout: 15_000 });
      await expect(treeRow(page, 'notes/untracked-new.md').getByTestId('ide-tree-git-status')).toHaveText('?', { timeout: 15_000 });
      await expect(treeRow(page, 'app/main.go')).toHaveCount(0);
      // I1-23: filter + New file + one ⋯ — at most 3 controls; the root row shows the name only.
      const toolbar = page.locator('[data-testid="ide-project-tree"] .tree-toolbar');
      expect(await toolbar.locator('button').count()).toBeLessThanOrEqual(3);
      await expect(page.getByTestId('ide-tree-root-node')).not.toContainText('/private');
      const filterW = (await page.getByTestId('ide-tree-filter').boundingBox())?.width ?? 0;
      expect(filterW).toBeGreaterThanOrEqual(96);
      // I1-14: the fresh dock offers the edit loop's four.
      expect(await dockTabIds(page)).toEqual(['ide-tab-terminal', 'ide-tab-problems', 'ide-dock-tab-git', 'ide-tab-run']);
      await expect(page.getByTestId('ide-dock-maximize')).toHaveCount(0);
      await shot(page, `01-tree-badges-dock-${scheme}.png`);
      if (scheme === 'dark') {
        // The ⋯ holds the rest; Debug can be put on the strip from it.
        await page.getByTestId('ide-tree-more-btn').click();
        const menu = page.locator('.ctx-menu');
        for (const name of ['Add folder to project…', 'Local history and deleted files…', 'Collapse all folders', 'Refresh']) {
          await expect(menu.getByRole('menuitem', { name })).toBeVisible();
        }
        await expect(menu.getByRole('menuitemcheckbox', { name: 'Show dotfiles' })).toBeVisible();
        await shot(page, '02-explorer-menu-dark.png');
        await page.keyboard.press('Escape');
      }
    }

    // I1-23: opening a file elsewhere (quick tab switch) reveals it in the tree.
    await openPolyglotFile(page, 'app', 'main.go');
    await openPolyglotFile(page, 'notes', 'roadmap.md');
    await page.getByTestId('ide-tree-more-btn').click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Collapse all folders' }).click();
    await expect(treeRow(page, 'app/main.go')).toHaveCount(0);
    await page.locator('.ide-tab-btn', { hasText: 'main.go' }).first().click();
    await expect(treeRow(page, 'app/main.go')).toBeVisible({ timeout: 10_000 });
    await expect(treeRow(page, 'app/main.go')).toHaveAttribute('aria-selected', 'true');
    await expect(page.locator('.ide-editor-area .cm-content').first()).toContainText('package main');

    // I1-11: the marks clear after a commit.
    await page.getByTestId('ide-dock-tab-git').click();
    await page.getByTestId('ide-git-stage-all').click();
    await expect(page.getByTestId('ide-git-unstage-all')).toBeVisible({ timeout: 10_000 });
    await page.getByTestId('ide-git-commit-input').fill('e2e: commit tool-window changes');
    await page.getByTestId('ide-git-commit-btn').click();
    await expect(page.getByTestId('ide-git-changes-view')).toContainText('Working tree clean', { timeout: 15_000 });
    await expect(treeRow(page, 'notes/roadmap.md').getByTestId('ide-tree-git-status')).toHaveCount(0, { timeout: 15_000 });
    await expect(treeRow(page, 'notes/untracked-new.md').getByTestId('ide-tree-git-status')).toHaveCount(0);
    expect(execFileSync('git', ['status', '--porcelain'], { cwd: seed.workspaceRoot, encoding: 'utf8' }).trim()).toBe('');
  });

  test('Git diff has one layout control; at 1280 the agent leaves the editor ≥ 700 px', async ({ page }) => {
    test.setTimeout(90_000);
    appendFileSync(join(seed.workspaceRoot, 'notes', 'roadmap.md'), '\nA second change.\n');
    await bootPolyglotIde(page, seed, 'dark');
    await page.getByTestId('ide-dock-tab-git').click();
    const gitView = page.getByTestId('ide-git-changes-view');
    await expect(gitView).toContainText('roadmap.md', { timeout: 15_000 });
    await gitView.locator('.gc-file-btn', { hasText: 'roadmap.md' }).first().click();
    await expect(gitView.locator('.gc-diff-scroll')).toContainText('A second change', { timeout: 15_000 });
    await expect(gitView.getByRole('button', { name: /Side-by-side/ })).toHaveCount(0);
    await expect(gitView.getByRole('group', { name: 'Diff layout' })).toHaveCount(1);
    await expect(page.getByTestId('ide-git-open-module')).toHaveCount(0);
    const hunk = gitView.locator('.hunk-header').first();
    await expect(hunk).toBeVisible();
    expect((await hunk.boundingBox())?.height ?? 99).toBeLessThanOrEqual(30);
    await page.getByTestId('ide-git-more-btn').click();
    await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'Open in Git module' })).toBeVisible();
    await page.keyboard.press('Escape');
    await shot(page, '03-git-panel-dark.png');

    await openPolyglotFile(page, 'app', 'main.go');
    await expect(page.getByTestId('ide-project-tree')).toBeVisible();
    await page.getByTestId('ide-toggle-agent').click();
    await expect(page.getByTestId('ide-agent-panel')).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('ide-project-tree')).toHaveCount(0);
    const editorW = (await page.locator('.ide-editor-area').first().boundingBox())?.width ?? 0;
    expect(editorW).toBeGreaterThanOrEqual(700);
    await shot(page, '04-agent-1280-dark.png');
    // Closing the agent brings the explorer back.
    await page.getByTestId('ide-toggle-agent').click();
    await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 10_000 });
  });

  test('agent empty state, settings memory and an empty local history', async ({ page }) => {
    test.setTimeout(90_000);
    await bootPolyglotIde(page, seed, 'dark');
    await openPolyglotFile(page, 'go.mod');
    // I1-28: a file with no snapshots says so in a toast; no empty modal stays open.
    await treeRow(page, 'go.mod').click({ button: 'right' });
    await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Local History' }).click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Show History…' }).click();
    await expect(page.getByText('No local history for go.mod yet')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId('ide-comparison-modal')).toHaveCount(0);

    // I1-18: with nothing remembered ⌘, opens Editor › General & Font, then where it was left.
    await page.keyboard.press('Meta+,');
    const modal = page.getByTestId('ide-settings-modal');
    await expect(modal).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId('ide-settings-nav-editor.font')).toHaveClass(/is-active/);
    const search = page.locator('.st-search-box').first();
    const searchBox = await search.boundingBox();
    const leftBox = await page.locator('.st-left').first().boundingBox();
    expect((searchBox?.y ?? 0) - (leftBox?.y ?? 0)).toBeGreaterThanOrEqual(4);
    await shot(page, '06-settings-dark.png');
    await page.getByTestId('ide-settings-nav-vcs.git').click();
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
    await page.keyboard.press('Meta+,');
    await expect(page.getByTestId('ide-settings-nav-vcs.git')).toHaveClass(/is-active/, { timeout: 10_000 });
    await page.keyboard.press('Escape');

    // I1-16: the agent's empty state names the file and offers prompts that fill the draft.
    await page.getByTestId('ide-toggle-agent').click();
    const panel = page.getByTestId('ide-agent-panel');
    await expect(panel).toContainText('Ask about go.mod', { timeout: 15_000 });
    await expect(panel.getByRole('button', { name: 'Attach active file' })).toHaveCount(0);
    await shot(page, '07-agent-empty-dark.png');
    await page.getByTestId('ide-agent-suggestions').getByRole('button', { name: 'Explain this file' }).click();
    await expect(page.getByTestId('ide-agent-composer')).toHaveValue(/Explain what this file does/);
  });

  test('a real Python debug run reveals the Debug tab', async ({ page }) => {
    test.setTimeout(120_000);
    await bootPolyglotIde(page, seed, 'dark');
    await openPolyglotFile(page, 'pyapp', 'pipeline.py');
    expect(await dockTabIds(page)).not.toContain('ide-tab-debug');
    const before = new Set(await liveDebugIds());
    try {
      await page.getByTestId('ide-debug-btn').click();
      const debugTab = page.getByTestId('ide-tab-debug');
      await expect(debugTab).toBeVisible({ timeout: 30_000 });
      await expect(debugTab).toHaveAttribute('aria-selected', 'true');
      await expect(page.getByTestId('ide-debugger-panel')).toBeVisible({ timeout: 15_000 });
      await shot(page, '05-debug-tab-dark.png');
    } finally {
      const c = await apiCtx();
      try {
        for (const id of (await liveDebugIds()).filter((x) => !before.has(x))) {
          await c.ctx.post(`${c.base}/api/v1/ide/debug/sessions/${encodeURIComponent(id)}/command`, { data: { action: 'stop' } }).catch(() => undefined);
        }
      } finally {
        await c.ctx.dispose();
      }
    }
  });
});
