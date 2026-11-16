import { test, expect, type Page } from '@playwright/test';
import { appendFileSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { daemonCall, seedPolyglotFiles } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// IDE tool windows on a phone and a touch tablet (round 5, IDE-TREE-DOCK):
//  I4-18 the Tools pane's tab strip keeps every tool and its ⋯ on screen, no dead band under it;
//  I4-20 Git's change list never scrolls sideways and offers no side-by-side diff;
//  I4-23 the tree's root row is the project name (the path is in its tooltip);
//  I4-26 the explorer filter keeps a usable width on a touch tablet.

interface Seed { workspaceId: string; workspaceRoot: string }
let seed: Seed | null = null;

async function getSeed(): Promise<Seed> {
  if (seed) return seed;
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-tools-mobile-')));
  seedPolyglotFiles(root);
  appendFileSync(join(root, 'notes', 'roadmap.md'), '\nChanged on a phone.\n');
  writeFileSync(join(root, 'notes', 'a-very-long-untracked-file-name-for-a-narrow-phone-row.md'), '# New\n');
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE tools mobile ${Date.now()}`, root_path: root });
    await daemonCall(c.ctx, c.base, c.token, 'POST', '/repos', { path: root, workspace_id: ws.id }).catch(() => undefined);
    seed = { workspaceId: ws.id, workspaceRoot: root };
    return seed;
  } finally {
    await c.ctx.dispose();
  }
}

async function boot(page: Page, scheme: 'dark' | 'light'): Promise<void> {
  const s = await getSeed();
  await page.addInitScript(
    ({ wsId, root, uiScheme }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_scheme', uiScheme);
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'p', name: 'Tools mobile', folders: [{ path: root, name: 'tools-mobile' }], updatedAt: Date.now() }]));
    },
    { wsId: s.workspaceId, root: s.workspaceRoot, uiScheme: scheme },
  );
  await page.goto('/#/ide');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
}

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-IDE-TREE-DOCK'), { recursive: true });
  await page.screenshot({ path: join(dir, 'fix-IDE-TREE-DOCK', name) });
}

/** Stacked (phone): pick the pane; a tablet workbench shows tree and dock side by side. */
async function showPane(page: Page, pane: 'files' | 'tools'): Promise<void> {
  const tab = page.getByTestId(`ide-stack-tab-${pane}`);
  await expect(tab.or(page.getByTestId('ide-project-tree')).first()).toBeVisible({ timeout: 20_000 });
  if (!(await tab.isVisible())) return;
  // A tab restored at boot switches the phone to the editor once it loads: pick again until it sticks.
  const shown = pane === 'files' ? page.getByTestId('ide-project-tree') : page.getByTestId('ide-bottom-dock');
  await expect(async () => {
    await tab.click();
    await expect(tab).toHaveAttribute('aria-pressed', 'true');
    await expect(shown).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 20_000 });
}

test.describe('IDE tool windows on phones and tablets', () => {
  test.beforeEach(async ({}, info) => {
    test.skip(!['iphone-portrait', 'ipad-landscape'].includes(info.project.name), 'one phone and one touch tablet');
  });

  test('tree root, filter and the tools strip fit; Git lists stay inside the screen', async ({ page }, info) => {
    test.setTimeout(120_000);
    const p = info.project.name;
    for (const scheme of ['dark', 'light'] as const) {
      await boot(page, scheme);
      if (scheme === 'light') await page.reload();
      await showPane(page, 'files');
      const tree = page.getByTestId('ide-project-tree');
      await expect(tree).toBeVisible({ timeout: 20_000 });
      const root = page.getByTestId('ide-tree-root-node');
      await expect(root).toBeVisible();
      await expect(root).not.toContainText('/private');
      await expect(root).not.toContainText('/var/');
      expect((await page.getByTestId('ide-tree-filter').boundingBox())?.width ?? 0, `${p}: filter width`).toBeGreaterThanOrEqual(96);
      await expectFullyInViewport(page, page.getByTestId('ide-tree-more-btn'), `${p}: explorer ⋯`);
      await shot(page, `${p}-tree-${scheme}.png`);

      await showPane(page, 'tools');
      const dock = page.getByTestId('ide-bottom-dock');
      await expect(dock).toBeVisible();
      for (const id of ['ide-tab-terminal', 'ide-tab-problems', 'ide-dock-tab-git', 'ide-tab-run', 'ide-dock-view-menu']) {
        await expectFullyInViewport(page, page.getByTestId(id), `${p}: ${id}`);
        // On top, not merely visible.
        const onTop = await page.getByTestId(id).evaluate((el) => {
          const r = el.getBoundingClientRect();
          const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
          return !!hit && (hit === el || el.contains(hit));
        });
        expect(onTop, `${p}: ${id} is on top`).toBe(true);
      }
      // No dead band: the selected tab's underline is the header's bottom edge.
      const gap = await dock.evaluate((el) => {
        const head = el.querySelector('.dock-header')!.getBoundingClientRect();
        const tab = el.querySelector('[role="tab"][aria-selected="true"]')!.getBoundingClientRect();
        return Math.round(head.bottom - tab.bottom);
      });
      expect(Math.abs(gap), `${p}: band under the tabs`).toBeLessThanOrEqual(3);
      await page.getByTestId('ide-dock-tab-git').click();
      const gitView = page.getByTestId('ide-git-changes-view');
      await expect(gitView).toContainText('roadmap.md', { timeout: 15_000 });
      await gitView.locator('.gc-file-btn', { hasText: 'roadmap.md' }).first().click();
      await expect(gitView.locator('.gc-diff-scroll')).toContainText('Changed on a phone', { timeout: 15_000 });
      if (p === 'iphone-portrait') {
        await expect(gitView.getByRole('button', { name: /Side.by.side/i })).toHaveCount(0);
        // No sideways scrollbar under the rows; a long name ellipsises and its Stage button stays on screen.
        expect(await gitView.locator('.gc-lists').evaluate((el) => getComputedStyle(el).overflowX), `${p}: change list`).toBe('hidden');
        await expectFullyInViewport(page, gitView.getByRole('button', { name: /^Stage .*a-very-long-untracked/ }), `${p}: Stage on a long row`);
      }
      await expectNoHorizontalOverflow(page);
      await shot(page, `${p}-tools-git-${scheme}.png`);
    }
  });
});
