import { test, expect, type Page, type TestInfo } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedGitRepo, seedConflictRepo, seedDirtyRepo, tempDir } from './seed';

// Durable mobile/tablet-layout coverage for the Git page. Asserts the REAL
// workflow on a phone AND in the tablet/landscape range: the commit graph/list
// shows commits (behind the Branches list on a phone), tapping a commit opens
// its diff in the Detail pane (changed files render), the diff area scrolls
// independently, and — the headline fix
// — the diff CODE is fully reachable inside the viewport (not clipped off-screen
// right) at the landscape widths 814/834/932.
//
// THE LANDSCAPE BUG (user, real phone): in the 641–1024px tablet/landscape range
// the PR/commit diff content was CUT OFF / overflowed off-screen right. Root
// cause: the git layout's responsive breakpoint was 820px, so 834 (iPad
// portrait) and 932 (real iPhone-landscape CSS px) fell into the DESKTOP 3-pane
// layout (refs 220 + graph min 280 + diff min 360 ≈ 860px minimum) and the diff
// panel — wider than the available width — clipped its non-wrapping `pre` code.
// The document itself never horizontally scrolled, so a document-overflow check
// alone did NOT catch it. The fix raised the breakpoint to 1024px (stacked
// accordion + wrapping diff for the whole tablet/landscape range); desktop
// (≥1025) is unchanged.
//
// `expectDiffCodeReachable` is the assertion that actually catches the bug: it
// checks every rendered diff code cell's right edge is within the viewport.
//
// Projects this file is meant to run on:
//   iphone-portrait (430)  · iphone-landscape (814)  · ipad-portrait (834)
// It also drives the real-device landscape width (932) explicitly via
// setViewportSize, since Playwright's iPhone-landscape profile is only 814.
//
// NOTE: live Pull Request DATA (list rows, PR detail tabs with a real diff +
// inline comments) needs a real git remote (GitHub/Bitbucket), which the
// isolated test daemon doesn't have. So the PR VIEWS are exercised here for their
// LAYOUT: the PR list toolbar/chips + empty/unreachable state, and the PR-detail
// route renders without overflowing. The PR diff reuses DiffViewer, whose mobile
// fixes are verified via the commit diff below.

let workspaceId = '';
let repoId = '';
// A repo left mid-merge with a real conflict → drives the Conflict Resolver.
let conflictRepoId = '';
// A repo with a dirty working tree → drives the WIP staging/commit flow.
let dirtyRepoId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const r = await seedGitRepo(ctx, base, workspaceId);
  repoId = r.repoId;
  conflictRepoId = (await seedConflictRepo(ctx, base, workspaceId)).repoId;
  dirtyRepoId = (await seedDirtyRepo(ctx, base, workspaceId)).repoId;
  const git = (...args: string[]) =>
    execFileSync('git', ['-C', r.dir, ...args], { stdio: 'ignore' });
  // Several more commits, each touching multiple files with multi-line edits AND
  // some very long lines so (a) the graph/history have many rows, (b) a commit
  // drill-down yields a real multi-file, scrollable diff, and (c) the long lines
  // would visibly overflow off-screen right if the wrap/breakpoint fix regressed.
  const LONG = 'x'.repeat(140);
  for (let c = 1; c <= 5; c++) {
    for (let f = 0; f < 6; f++) {
      const idx = String((c * 6 + f) % 25).padStart(2, '0');
      writeFileSync(
        join(r.dir, `file_${idx}.txt`),
        `commit ${c} change ${f} ${LONG}\nextra A ${c}\nextra B ${f}\nmore ${c}-${f}\n`,
      );
    }
    git('add', '-A');
    git('commit', '-q', '-m', `E2E commit ${c}: multi-file multi-line edits`);
  }
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  // Git's mobile/tablet layout (list push, Detail sheet + wrapping diff) applies at
  // ≤1024px. ipad-landscape (1194px) is the unchanged DESKTOP 3-pane layout,
  // whose render/overflow/a11y is covered by pages.spec — these mobile-flow
  // assertions don't apply there, so skip them on desktop widths.
  const w = page.viewportSize()?.width ?? 0;
  test.skip(w > 1024, 'git mobile/tablet layout applies at ≤1024px; this profile is desktop');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

const isPhone = (info: TestInfo) => !info.project.name.includes('ipad');

/** Assert the document doesn't scroll horizontally (content fits viewport). */
async function expectFitsWidth(page: Page): Promise<void> {
  const o = await page.evaluate(() => {
    const de = document.documentElement;
    return { scrollW: de.scrollWidth, clientW: de.clientWidth };
  });
  // Allow 1px rounding slack.
  expect(o.scrollW, 'page must not overflow horizontally').toBeLessThanOrEqual(o.clientW + 1);
}

/**
 * The bug-catcher. The document never horizontally scrolls (the diff panel uses
 * overflow:hidden / overflow-x:auto), so the *page* fitting tells us nothing
 * about whether the diff code is reachable. Instead, assert that every rendered
 * diff code cell's RIGHT EDGE is within the viewport — i.e. the code is not cut
 * off off-screen right. Covers both the GraphView desktop-ish cells (.dl-code)
 * and the DiffViewer cells (.code).
 */
async function expectDiffCodeReachable(page: Page): Promise<void> {
  // The file block's header paints before its lines (they mount a frame
  // later, later still under load): wait for a code cell, then measure.
  await expect(page.locator('.dl-code, td.code, .vrow .code').locator('visible=true').first(), 'diff code cells should be present').toBeVisible({ timeout: 15_000 });
  const m = await page.evaluate(() => {
    const cells = Array.from(
      document.querySelectorAll('.dl-code, td.code, .vrow .code'),
    ) as HTMLElement[];
    let maxRight = 0;
    let count = 0;
    for (const c of cells) {
      const r = c.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) continue; // not laid out
      count++;
      maxRight = Math.max(maxRight, r.right);
    }
    return { maxRight, count, vw: window.innerWidth };
  });
  expect(m.count, 'diff code cells should be present').toBeGreaterThan(0);
  // Allow a few px slack for borders/scrollbars.
  expect(
    Math.round(m.maxRight),
    `diff code must fit within the viewport (max code right=${Math.round(m.maxRight)} vw=${m.vw}) — content cut off off-screen right`,
  ).toBeLessThanOrEqual(m.vw + 4);
}

/** Open the commit graph and wait for rows. */
async function openGraph(page: Page): Promise<void> {
  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  await showGraph(page);
}

/** History on a phone is its Branches list: tap the checked-out branch to push
 *  the graph (a tablet shows the graph beside the list already). */
async function showGraph(page: Page): Promise<void> {
  const graph = page.locator('.graph-row').first();
  const current = page.locator('.refs-panel .ref-row.current');
  await expect(graph.or(current).first()).toBeVisible({ timeout: 25_000 });
  if (await graph.isHidden()) await current.tap();
  await expect(graph).toBeVisible({ timeout: 25_000 });
}

/** The WIP panel: History lands on it only where its Detail pane fits beside
 *  the graph; on a phone or a narrow tablet tap the WIP row (a sheet opens). */
async function openWip(page: Page): Promise<void> {
  await page.goto(`/#/git/${dirtyRepoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  await showGraph(page);
  const wip = page.locator('.wip-row');
  if ((await wip.getAttribute('aria-pressed')) !== 'true') await wip.tap();
  await expect(page.locator('.wip-panel')).toBeVisible({ timeout: 15_000 });
}

/** A phone (or a phone on its side): the list pushes its content. */
const pushes = (page: Page): boolean => {
  const v = page.viewportSize()!;
  return v.width <= 640 || v.height <= 500;
};

// ────────────────────────────────────────────────────────────────────────────
// GRAPH
// ────────────────────────────────────────────────────────────────────────────

test('graph: lists commits and fits the viewport', async ({ page }) => {
  await openGraph(page);
  const rows = page.locator('.graph-row');
  expect(await rows.count(), 'graph should list multiple commits').toBeGreaterThan(2);
  await expectFitsWidth(page);
});

test('graph: commit drill shows a multi-file diff that scrolls and fits', async ({ page }) => {
  await openGraph(page);
  await page.locator('.graph-row').nth(1).click();
  await expect(page.locator('.detail-panel')).toBeVisible({ timeout: 15_000 });

  // The file list shows every changed file and opens the first one's diff.
  const fileRows = page.locator('.detail-diff .cf-row[data-path]');
  await expect(fileRows.first()).toBeVisible({ timeout: 15_000 });
  expect(await fileRows.count(), 'the list should show multiple changed files').toBeGreaterThan(1);
  await expect(page.locator('.detail-diff .df-block')).toHaveCount(1);

  // "View all files" stacks every file's diff.
  await page.locator('.detail-diff').getByLabel('View all files').check();
  const fileBlocks = page.locator('.detail-diff .df-block');
  await expect(fileBlocks.first()).toBeVisible({ timeout: 15_000 });
  expect(await fileBlocks.count(), 'diff should list multiple changed files').toBeGreaterThan(1);

  // The diff area is its own independent scroll container.
  const diffArea = page.locator('.detail-diff').first();
  const overflowY = await diffArea.evaluate((el) => getComputedStyle(el).overflowY);
  expect(overflowY, 'diff area must be its own scroll container').toBe('auto');

  // It actually overflows + scrolls (the diff is taller than the pane).
  const scrolled = await diffArea.evaluate((el) => {
    const before = el.scrollTop;
    el.scrollTop = 600;
    return { scrollable: el.scrollHeight > el.clientHeight, moved: el.scrollTop > before };
  });
  expect(scrolled.scrollable, 'diff must overflow its container (so it can scroll)').toBe(true);
  expect(scrolled.moved, 'diff container must actually scroll').toBe(true);

  // THE FIX: the long diff code is reachable inside the viewport, not cut off.
  await expectFitsWidth(page);
  await expectDiffCodeReachable(page);
});

test('graph: Branches is a list pane — a phone pushes the graph, the detail is a sheet', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/graph`);
  const current = page.locator('.refs-panel .ref-row.current');
  await expect(current).toBeVisible({ timeout: 25_000 });
  const graph = page.locator('.graph-row').first();
  if (pushes(page)) {
    // The list is the page (the checked-out branch marked); a tap pushes the
    // graph with "‹ Branches", and Back keeps the picked branch marked.
    await expect(graph).toBeHidden();
    await current.tap();
    await expect(graph).toBeVisible({ timeout: 15_000 });
    await expect(current).toBeHidden();
    await expect(page.getByTestId('git-detail')).toHaveCount(0); // the graph, not a sheet over it
    await page.getByRole('button', { name: 'Back to Branches' }).tap();
    await expect(current).toBeVisible();
    await expect(current).toHaveAttribute('aria-selected', 'true');
    await current.tap();
  }
  await expect(graph).toBeVisible();
  // A commit's detail is a sheet over the graph; its Close returns to it.
  await page.locator('.graph-row').nth(1).tap();
  const sheet = page.getByRole('dialog', { name: 'Commit detail' });
  await expect(sheet).toBeVisible({ timeout: 15_000 });
  await sheet.getByRole('button', { name: 'Close Commit detail' }).tap();
  await expect(sheet).toHaveCount(0);
  await expect(graph).toBeVisible();
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// DEEP LINKS — the legacy #/git/<repo>/changes and #/git/<repo>/history both
// land on History (the graph); there is no Changes view any more.
// ────────────────────────────────────────────────────────────────────────────

test('changes/history deep links land on their views', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/changes`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.rv-tabs .rv-tab', { hasText: 'History' })).toHaveAttribute('aria-selected', 'true', { timeout: 25_000 });
  await expect(page.locator('.rv-tabs .rv-tab', { hasText: 'Changes' })).toHaveCount(0);
  // A clean tree: the commit list (on a phone, behind its Branches list), nothing selected.
  await showGraph(page);
  await expectFitsWidth(page);
  await page.goto(`/#/git/${repoId}/history`);
  await showGraph(page);
  await expect(page.locator('.rv-tabs .rv-tab', { hasText: 'History' })).toHaveAttribute('aria-selected', 'true');
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// PULL REQUESTS (layout only — live data needs a real remote)
// ────────────────────────────────────────────────────────────────────────────

test('pull requests: a repo with no remote says so and offers the fix', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/prs`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  // No remote → an honest empty state with "Add a remote…", not a
  // "provider unreachable" error whose Retry can never succeed.
  await expect(page.getByText('No remote yet', { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole('button', { name: 'Add a remote…' })).toBeVisible();
  await expectFitsWidth(page);
});

test.describe('pull requests: review state', () => {
  // The PWA's service worker would answer the API before page.route sees it.
  test.use({ serviceWorkers: 'block' });

  test('pull requests: each open row says where its review stands, and fits', async ({ page }) => {
    // A Bitbucket remote from the start (the forge is read when the repo is added).
    const dir = tempDir('otto-e2e-bb-');
    for (const args of [['init', '-q'], ['remote', 'add', 'origin', 'https://bitbucket.org/acme/app.git'], ['-c', 'user.email=e2e@otto.local', '-c', 'user.name=E2E', 'commit', '-q', '--allow-empty', '-m', 'init']]) {
      execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
    }
    const { ctx, base } = await apiCtx();
    const added = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: 'e2e-bitbucket' } });
    const r = { repoId: (await added.json()).id as string };
    await ctx.dispose();
    const row = (number: number, review: unknown) => ({
      number, title: `Pull request ${number} with a reasonably long title for a phone`, author: 'Reviewer Name',
      state: 'open', source_branch: `feature/long-branch-name-${number}`, target_branch: 'develop',
      updated_at: '2026-11-13T10:00:00Z', url: '', labels: [], ...(review ? { review } : {}),
    });
    await page.route((url) => url.pathname.endsWith(`/api/v1/repos/${r.repoId}/prs`), (route) =>
      route.fulfill({ json: { items: [
        row(1, { status: 'approved', approvals: 2 }),
        row(2, { status: 'changes_requested', approvals: 1 }),
        row(3, { status: 'waiting', approvals: 0 }),
        row(4, null),
        { ...row(5, { status: 'waiting', approvals: 0 }), asks_me: true },
      ], has_more: false, page: 1, per_page: 50 } }),
    );
    // Nothing reaches the real forge: per-row reads (diff stats, …) answer 404.
    await page.route(`**/api/v1/repos/${r.repoId}/prs/**`, (route) => route.fulfill({ status: 404, json: { error: 'stub' } }));
    await page.goto(`/#/git/${r.repoId}/prs`);
    await expect(page.getByTestId('pr-word')).toHaveCount(5, { timeout: 25_000 });
    const word = (n: number) => page.locator(`.pr-row[data-pr="${n}"]`).getByTestId('pr-word');
    await expect(word(1)).toHaveText('Ready to merge');
    await expect(word(2)).toHaveText('Changes requested');
    await expect(word(3)).toHaveText('Waiting for review');
    await expect(word(4)).toHaveText('Open');
    await expect(word(5)).toHaveText('Review requested');
    // The one that asks you leads, under "Needs you".
    await expect(page.locator('.pr-row').first()).toHaveAttribute('data-pr', '5');
    await expect(page.getByRole('region', { name: 'Needs you' })).toContainText('Pull request 5');
    await word(1).scrollIntoViewIfNeeded();
    await expect(word(1)).toBeInViewport();
    await expectFitsWidth(page);
    // Chrome before 128 gives a <button> `align-items: flex-start` (UA sheet,
    // seen on the Android emulator): a long title must still end in an
    // ellipsis inside its row, not run past it under the select box.
    await page.addStyleTag({ content: 'button { align-items: flex-start; }' });
    const pr1 = page.locator('.pr-row[data-pr="1"] .tr-main');
    const rowBox = (await pr1.boundingBox())!;
    const titleBox = (await pr1.locator('.tr-title').boundingBox())!;
    expect(titleBox.x + titleBox.width).toBeLessThanOrEqual(rowBox.x + rowBox.width);
  });
});

test('pull requests: detail route renders without overflow', async ({ page }) => {
  // Deep-link a PR detail. Without a remote the PR won't load real data, but the
  // route + shell (header/back button or skeleton) must render and fit the width.
  await page.goto(`/#/git/${repoId}/pr/1`);
  // Either the PR-detail shell or a toast/skeleton renders; the page must not
  // horizontally overflow regardless.
  await page.waitForTimeout(1500);
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// DiffViewer (unified) via the History diff — file nav / unified rows
// ────────────────────────────────────────────────────────────────────────────

test('diffviewer: a WIP file diff fits and its code is reachable', async ({ page }) => {
  await openWip(page);
  const panel = page.locator('.wip-panel');
  // The commit card sits above the list: the last file is reachable.
  await panel.locator('.wp-name').last().scrollIntoViewIfNeeded();
  await expect(panel.locator('.wp-name').last()).toBeInViewport();
  await panel.locator('.wp-name').first().scrollIntoViewIfNeeded();
  await panel.locator('.wp-name').first().click();
  const diff = page.locator('.wp-diff');
  await expect(diff).toBeVisible({ timeout: 15_000 });
  // The diff renders after its fetch: measure once a code row is on screen.
  await expect(diff.locator('.dl-code, td.code, .vrow .code').first()).toBeVisible({ timeout: 15_000 });
  await expectFitsWidth(page);
  await expectDiffCodeReachable(page);
  // It stays in the Detail pane under the list (no second sheet); ✕ goes back.
  await expect(page.getByTestId('wip-diff-sheet')).toHaveCount(0);
  await diff.getByRole('button', { name: 'Close diff' }).click();
  await expect(diff).toHaveCount(0);
  await expect(panel.locator('.wp-name').first()).toBeVisible();
});

// ────────────────────────────────────────────────────────────────────────────
// REAL-DEVICE LANDSCAPE WIDTHS (834 + 932) — explicit cutoff regression guards
// ────────────────────────────────────────────────────────────────────────────
// These run on every project but FORCE the specific tablet/landscape CSS widths
// where the bug lived (834 = iPad portrait, 932 = real iPhone-14-Pro-Max
// landscape in CSS px). The commit diff's long code must stay reachable.

for (const vw of [834, 932]) {
  test(`landscape ${vw}: commit diff code is reachable (not cut off)`, async ({ page }) => {
    await page.setViewportSize({ width: vw, height: 430 });
    await openGraph(page);
    await page.locator('.graph-row').nth(1).click();
    await expect(page.locator('.detail-panel')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.detail-diff .df-block').first()).toBeVisible({ timeout: 15_000 });
    await expectFitsWidth(page);
    await expectDiffCodeReachable(page);
  });

}

// ────────────────────────────────────────────────────────────────────────────
// Shared helpers for the section suites below
// ────────────────────────────────────────────────────────────────────────────

/** Assert a single element's right edge is within the viewport (not clipped /
 *  pushed off-screen right). */
async function expectElemFits(page: Page, selector: string): Promise<void> {
  const m = await page.locator(selector).first().evaluate((el) => ({
    right: el.getBoundingClientRect().right,
    vw: window.innerWidth,
  }));
  expect(
    Math.round(m.right),
    `${selector} right edge (${Math.round(m.right)}) must be within the viewport (${m.vw})`,
  ).toBeLessThanOrEqual(m.vw + 4);
}

// ────────────────────────────────────────────────────────────────────────────
// SHELL — repo tabs, sub-route nav, toolbar (the git chrome)
// ────────────────────────────────────────────────────────────────────────────

test('shell: repo tab strip + new-tab + sub-route nav + toolbar fit', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  if (page.viewportSize()!.width <= 640) {
    // A phone keeps one header row: the repo is the title, a switcher.
    await expect(page.getByTestId('git-phone-repo')).toBeVisible({ timeout: 15_000 });
    await expectElemFits(page, '[data-testid="git-phone-repo"]');
    await expect(page.locator('.git-tabs')).toHaveCount(0);
  } else {
    // Repo tab strip renders; the "+" (open repo) button stays reachable on-screen.
    await expect(page.locator('.git-tabs')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.git-tab-new')).toBeVisible();
    await expectElemFits(page, '.git-tab-new');
  }
  // The ONE view switch (History · Pull requests · Review; Focus is
  // a page-level view) renders with all views and never overflows the
  // document (it scrolls within its own strip).
  await expect(page.locator('.rv-tabs')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.rv-tabs .rv-tab')).toHaveText(['History', 'Pull requests', 'Review']);
  // The git verbs (Fetch/Pull/Push/Stash/Pop): a row of their own on a phone,
  // the page header's actions on a tablet.
  await expect(page.getByTestId('git-verbs-row').or(page.locator('.git-header .ph-actions .tbtn')).first()).toBeVisible();
  await expectFitsWidth(page);
});

test('shell: tapping a sub-route tab navigates without overflow', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.rv-tabs')).toBeVisible({ timeout: 30_000 });
  // Tap the Review view tab → the local-review panel mounts. The switcher may
  // scroll within its own strip, so scroll the tab into view before tapping.
  const tab = page.locator('.rv-tabs .rv-tab', { hasText: 'Review' });
  await tab.scrollIntoViewIfNeeded();
  await tab.click();
  await expect(page.locator('.lrp')).toBeVisible({ timeout: 20_000 });
  await expectFitsWidth(page);
});

test('shell: many open repo tabs scroll instead of overflowing the page', async ({ page }) => {
  // Open all three seeded repos as tabs, then assert the tablist owns the overflow
  // (its own scroll) while the document itself never scrolls horizontally.
  // Wait for EACH hop's tab to register before the next hop: `.gitpage` is
  // already visible after the first goto, so waiting on it was a no-op and a
  // fast second hash hop raced the tab registration (S12-304, git-mobile:331).
  const ids = [repoId, dirtyRepoId, conflictRepoId];
  const phone = page.viewportSize()!.width <= 640;
  const openCount = (): Promise<number> =>
    page.evaluate(() => (JSON.parse(localStorage.getItem('otto_git_open_tabs') ?? '{}').openRepoIds ?? []).length);
  for (const [i, id] of ids.entries()) {
    await page.goto(`/#/git/${id}/graph`);
    await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
    if (phone) await expect.poll(openCount, { timeout: 15_000 }).toBe(i + 1);
    else await expect(page.locator('.git-tab')).toHaveCount(i + 1, { timeout: 15_000 });
  }
  if (phone) {
    // The title's switcher lists every open repo (one header row, no strip).
    await page.getByTestId('git-phone-repo').click();
    await expect(page.locator('.ctx-menu').getByRole('menuitem').filter({ hasText: /e2e|repo/ }).first()).toBeVisible();
    await page.keyboard.press('Escape');
  } else {
    await expect(page.locator('.git-tab')).toHaveCount(3, { timeout: 15_000 });
    // New-tab affordance still reachable even with several tabs open.
    await expectElemFits(page, '.git-tab-new');
  }
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// WIP — staging / commit flow on a dirty working tree (History's WIP row)
// ────────────────────────────────────────────────────────────────────────────

test('WIP: dirty tree shows file rows + a touch-friendly commit composer', async ({ page }) => {
  await openWip(page);
  const panel = page.locator('.wip-panel');
  // The seeded dirty tree (1 modified + 1 untracked) yields real change rows.
  expect(await panel.locator('.wp-name').count()).toBeGreaterThanOrEqual(2);
  await expectFitsWidth(page);
  // The commit fields use ≥16px on mobile so iOS Safari doesn't auto-zoom.
  const fs = await panel.locator('.subject-input').evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  expect(fs, 'mobile commit summary must be ≥16px (no iOS zoom)').toBeGreaterThanOrEqual(16);
  const box = await panel.locator('.btn.primary').first().boundingBox();
  expect(box?.height ?? 0, 'commit button should be a comfortable tap target').toBeGreaterThanOrEqual(32);
});

// ────────────────────────────────────────────────────────────────────────────
// REVIEW — local code-review panel layout (a real agent run needs CLIs the
// isolated daemon doesn't spawn, so the toolbar + config/empty state is exercised
// for LAYOUT, like the PR views).
// ────────────────────────────────────────────────────────────────────────────

test('review: local-review panel renders and fits', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/review`);
  await expect(page.locator('.rv-tab-scroll')).toBeVisible({ timeout: 25_000 });
  await expect(page.locator('.lrp')).toBeVisible({ timeout: 15_000 });
  // The toolbar (Compare-to select + "Review changes") renders and fits the width.
  await expect(page.locator('.lrp .lrp-toolbar')).toBeVisible({ timeout: 10_000 });
  await expectElemFits(page, '.lrp .lrp-toolbar');
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// CONFLICT RESOLVER — a repo left mid-merge (seedConflictRepo). It is a list /
// detail page (the "Conflicts" list pane): on a phone the list is the page and
// a file is pushed ("‹ Conflicts"); a narrow conflict card stacks A over B
// (container query), and the long conflicting lines wrap so the code stays
// reachable.
// ────────────────────────────────────────────────────────────────────────────

/** The conflict hunk's own code cells (picked lines, base, editor, context)
 *  must all be reachable within the viewport — i.e. long lines wrap, not clip. */
async function expectConflictCodeReachable(page: Page): Promise<void> {
  const m = await page.evaluate(() => {
    const cells = Array.from(
      document.querySelectorAll('.resolver .pick-code, .resolver .base-code, .resolver .edit-editor, .resolver .context'),
    ) as HTMLElement[];
    let maxRight = 0;
    let count = 0;
    for (const c of cells) {
      const r = c.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) continue;
      count++;
      maxRight = Math.max(maxRight, r.right);
    }
    return { maxRight, count, vw: window.innerWidth };
  });
  expect(m.count, 'conflict code cells should be present').toBeGreaterThan(0);
  expect(
    Math.round(m.maxRight),
    `conflict code must fit within the viewport (max right=${Math.round(m.maxRight)} vw=${m.vw})`,
  ).toBeLessThanOrEqual(m.vw + 4);
}

async function openResolver(page: Page): Promise<void> {
  await page.goto(`/#/git/${conflictRepoId}/changes`);
  await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  // The in-progress merge is detected asynchronously → the merge banner
  // appears. Tap its "Resolve conflicts" to open the resolver.
  const open = page.locator('.merge-banner .btn', { hasText: 'Resolve conflicts' });
  await expect(open).toBeVisible({ timeout: 20_000 });
  await open.click();
  await expect(page.locator('.resolver')).toBeVisible({ timeout: 15_000 });
}

/** The selected file's pane: beside the list on a tablet, pushed (one tap) on a phone. */
async function openConflictFile(page: Page): Promise<void> {
  const row = page.getByTestId('conflict-file').first();
  await expect(row).toBeVisible({ timeout: 10_000 });
  if (await page.locator('.resolver .file-detail').isHidden()) await row.tap();
  await expect(page.locator('.resolver .file-detail .pane').first()).toBeVisible({ timeout: 15_000 });
}

/** A conflict card narrower than 600 px stacks its sides (one grid column). */
async function expectHunkLayout(page: Page): Promise<void> {
  const sides = page.locator('.resolver .hunk .sides').first();
  await expect(sides).toBeVisible({ timeout: 15_000 });
  const { w, cols } = await sides.evaluate((el) => ({
    w: (el.closest('.hunk') as HTMLElement).clientWidth,
    cols: getComputedStyle(el).gridTemplateColumns.split(' ').length,
  }));
  expect(cols, `a ${w}px card shows ${w <= 600 ? 'one column' : 'two columns'}`).toBe(w <= 600 ? 1 : 2);
}

test('conflict: resolver renders, the card stacks when narrow, and fits', async ({ page }) => {
  await openResolver(page);
  await openConflictFile(page);
  await expectHunkLayout(page);
  await expectFitsWidth(page);
  await expectConflictCodeReachable(page);
});

test('conflict: on a phone the files list is the page and a file is pushed', async ({ page }, info) => {
  test.skip(!isPhone(info), 'a tablet shows the list beside the file');
  await openResolver(page);
  const row = page.getByTestId('conflict-file').first();
  await expect(row).toBeVisible({ timeout: 10_000 });
  await expect(row).toContainText('Conflict');
  await expect(page.locator('.resolver .file-detail')).toBeHidden();
  await row.tap();
  await expect(page.locator('.resolver .file-detail .pane')).toBeVisible({ timeout: 15_000 });
  await expect(row).toBeHidden();
  await page.getByRole('button', { name: 'Back to Conflicts' }).tap();
  await expect(row).toBeVisible();
  await expect(row.locator('[aria-current="true"]')).toBeVisible();
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// SMALLEST PHONE (320px) — every core git view must still fit, with no
// off-screen cutoff, at the narrowest mainstream width.
// ────────────────────────────────────────────────────────────────────────────

const SMALL = { width: 320, height: 568 };

test('320px: every core view fits with no horizontal overflow', async ({ page }) => {
  await page.setViewportSize(SMALL);

  // Graph + commit list.
  await openGraph(page);
  await expectFitsWidth(page);

  // WIP panel (dirty tree → real rows).
  await openWip(page);
  await expect(page.locator('.wip-panel .wp-name').first()).toBeVisible({ timeout: 15_000 });
  await expectFitsWidth(page);

  // Pull requests (no remote → the add-remote empty state).
  await page.goto(`/#/git/${repoId}/prs`);
  await expect(page.getByText('No remote yet', { exact: true })).toBeVisible({ timeout: 25_000 });
  await expectFitsWidth(page);

  // Local review panel.
  await page.goto(`/#/git/${repoId}/review`);
  await expect(page.locator('.lrp')).toBeVisible({ timeout: 25_000 });
  await expectFitsWidth(page);
});

test('320px: commit diff code is reachable + conflict resolver fits', async ({ page }) => {
  await page.setViewportSize(SMALL);

  // Commit diff: long lines wrap, code reachable.
  await openGraph(page);
  await page.locator('.graph-row').nth(1).click();
  await expect(page.locator('.detail-panel')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.detail-diff .df-block').first()).toBeVisible({ timeout: 15_000 });
  await expectFitsWidth(page);
  await expectDiffCodeReachable(page);

  // Conflict resolver still stacks + fits at 320.
  await openResolver(page);
  await openConflictFile(page);
  await expectHunkLayout(page);
  await expectFitsWidth(page);
  await expectConflictCodeReachable(page);
});
