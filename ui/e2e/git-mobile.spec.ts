import { test, expect, type Page, type TestInfo } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedGitRepo, seedConflictRepo, seedDirtyRepo } from './seed';

// Durable mobile/tablet-layout coverage for the Git page. Asserts the REAL
// workflow on a phone AND in the tablet/landscape range: the commit graph/list
// shows commits, tapping a commit drills into its diff (changed files render),
// the diff area scrolls independently, sections collapse, and — the headline fix
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
// A repo with a dirty working tree → drives the Changes staging/commit flow.
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
  // Git's mobile/tablet layout (stacked accordion + wrapping diff) applies at
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
  await expect(page.locator('.graph-row').first()).toBeVisible({ timeout: 25_000 });
}

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
  await expect(
    page.locator('.detail-panel.mob-visible, .detail-panel.detail-visible'),
  ).toBeVisible({ timeout: 15_000 });

  // The diff renders multiple changed files.
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

test('graph: sections are a collapsible accordion at mobile/tablet widths', async ({ page }) => {
  await openGraph(page);
  // The 3-pane desktop layout is replaced by the stacked accordion at ≤1024,
  // which all of iphone-portrait/landscape + ipad-portrait now use.
  const heads = page.locator('.graphview.mobile .mob-sec-head');
  await expect(heads.first()).toBeVisible({ timeout: 10_000 });

  // Expand Branches & Tags → refs panel visible; collapse → hidden.
  const branchesHead = page.locator('.mob-sec-head', { hasText: 'Branches' });
  await branchesHead.click();
  await expect(page.locator('.refs-panel')).toBeVisible();
  await branchesHead.click();
  await expect(page.locator('.refs-panel')).toBeHidden();

  // Tapping a commit auto-collapses the commit list and opens the diff.
  await page.locator('.graph-row').nth(1).click();
  await expect(page.locator('.detail-panel.mob-visible')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.graph-panel.mob-collapsed')).toHaveCount(1);

  // Closing the diff (tap its header) re-opens the commit list.
  await page.locator('.mob-diff-head').click();
  await expect(page.locator('.graph-row').first()).toBeVisible();
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// CHANGES
// ────────────────────────────────────────────────────────────────────────────

test('changes: renders and fits the viewport', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/changes`);
  await expect(page.locator('.changes')).toBeVisible({ timeout: 25_000 });
  // The changed-files accordion header is present at mobile/tablet widths.
  await expect(page.locator('.changes.mobile .mob-sec-head').first()).toBeVisible({
    timeout: 10_000,
  });
  await expectFitsWidth(page);
});

test('changes: file-list section collapses', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/changes`);
  await expect(page.locator('.changes')).toBeVisible({ timeout: 25_000 });
  const head = page.locator('.changes.mobile .mob-sec-head', { hasText: 'Changed files' });
  await expect(head).toBeVisible({ timeout: 10_000 });
  // Collapse → the file list hides.
  await head.click();
  await expect(page.locator('.changes-side.mob-files-collapsed')).toHaveCount(1);
  // Expand again.
  await head.click();
  await expect(page.locator('.changes-side.mob-files-collapsed')).toHaveCount(0);
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// HISTORY
// ────────────────────────────────────────────────────────────────────────────

test('history: lists commits and fits the viewport', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/history`);
  await expect(page.locator('.history')).toBeVisible({ timeout: 25_000 });
  await expect(page.locator('.history .commit').first()).toBeVisible({ timeout: 15_000 });
  expect(await page.locator('.history .commit').count()).toBeGreaterThan(2);
  await expectFitsWidth(page);
});

test('history: commit → diff workflow fits + diff code reachable', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/history`);
  await expect(page.locator('.history .commit').first()).toBeVisible({ timeout: 20_000 });

  await page.locator('.history .commit').nth(1).click();
  const dfile = page.locator('.history .dfile');
  await expect(dfile.first()).toBeVisible({ timeout: 15_000 });
  expect(await dfile.count(), 'history diff should show changed files').toBeGreaterThan(0);
  await expectFitsWidth(page);
  // HistoryView uses DiffViewer (.code cells) — verify the long lines fit.
  await expectDiffCodeReachable(page);
});

test('history: mobile view is a vertical scroll container', async ({ page }, info) => {
  if (!isPhone(info)) return; // checked on the short phone viewport
  await page.goto(`/#/git/${repoId}/history`);
  await expect(page.locator('.history .commit').first()).toBeVisible({ timeout: 20_000 });
  await page.locator('.history .commit').nth(1).click();
  await expect(page.locator('.history .dfile').first()).toBeVisible({ timeout: 15_000 });
  // The mobile history view owns its vertical scroll (column flow, overflow-y
  // auto) so a tall diff is always reachable. Whether it actually overflows
  // depends on diff size vs viewport height, so assert the scroll-container
  // PROPERTY (the robust, height-independent invariant) + that it fits width.
  const overflowY = await page.locator('.history.mobile').first().evaluate(
    (el) => getComputedStyle(el).overflowY,
  );
  expect(overflowY, 'mobile history must be a vertical scroll container').toBe('auto');
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// PULL REQUESTS (layout only — live data needs a real remote)
// ────────────────────────────────────────────────────────────────────────────

test('pull requests: list toolbar/chips render and fit', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/prs`);
  await expect(page.locator('.prlist')).toBeVisible({ timeout: 25_000 });
  // Filter chips render (toolbar layout). Live PR data needs a real remote.
  const chips = page.locator('.prlist .filter-chip');
  await expect(chips.first()).toBeVisible({ timeout: 10_000 });
  expect(await chips.count()).toBe(4); // open / merged / declined / all
  // Switching a filter chip works without overflowing.
  await chips.nth(1).click();
  await expect(chips.nth(1)).toHaveClass(/active/);
  await expectFitsWidth(page);
});

test('pull requests: empty/unreachable state fits the viewport', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/prs`);
  await expect(page.locator('.prlist')).toBeVisible({ timeout: 25_000 });
  // With no remote, the list resolves to either an empty state or a provider-
  // unreachable empty state — either way it must render and fit.
  await expect(page.locator('.prlist .empty, .prlist .pr-rows')).toBeVisible({ timeout: 15_000 });
  await expectFitsWidth(page);
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

test('diffviewer: unified rows + toolbar fit and wrap (no off-screen cutoff)', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/history`);
  await expect(page.locator('.history .commit').first()).toBeVisible({ timeout: 20_000 });
  await page.locator('.history .commit').nth(1).click();
  // DiffViewer toolbar (Unified / Side by side) renders.
  await expect(page.locator('.history .diff-toolbar').first()).toBeVisible({ timeout: 15_000 });
  // Unified table rows are present.
  await expect(page.locator('.history .dtable').first()).toBeVisible();
  // The code cells (including the 140-char long lines) fit within the viewport.
  await expectFitsWidth(page);
  await expectDiffCodeReachable(page);

  // Code cells wrap (white-space pre-wrap) at mobile/tablet widths so long lines
  // don't push past the screen.
  const ws = await page.locator('.history .dtable .code').first().evaluate(
    (el) => getComputedStyle(el).whiteSpace,
  );
  expect(ws, 'diff code should wrap at mobile/tablet widths').toBe('pre-wrap');
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
    await expect(
      page.locator('.detail-panel.mob-visible, .detail-panel.detail-visible'),
    ).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.detail-diff .df-block').first()).toBeVisible({ timeout: 15_000 });
    await expectFitsWidth(page);
    await expectDiffCodeReachable(page);
  });

  test(`landscape ${vw}: history diff code is reachable (not cut off)`, async ({ page }) => {
    await page.setViewportSize({ width: vw, height: 430 });
    await page.goto(`/#/git/${repoId}/history`);
    await expect(page.locator('.history .commit').first()).toBeVisible({ timeout: 20_000 });
    await page.locator('.history .commit').nth(1).click();
    await expect(page.locator('.history .dfile').first()).toBeVisible({ timeout: 15_000 });
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
  // Repo tab strip renders; the "+" (open repo) button stays reachable on-screen.
  await expect(page.locator('.git-tabs')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.git-tab-new')).toBeVisible();
  await expectElemFits(page, '.git-tab-new');
  // The per-repo sub-route nav (graph/changes/history/prs/review) renders with all
  // tabs and never overflows the document (it scrolls within its own strip).
  await expect(page.locator('.rv-tabs')).toBeVisible({ timeout: 15_000 });
  expect(await page.locator('.rv-tabs .rv-tab').count()).toBeGreaterThanOrEqual(4);
  // Toolbar (Fetch/Pull/Push/…) is present in the header.
  await expect(page.locator('.rv-head .toolbar')).toBeVisible();
  await expectFitsWidth(page);
});

test('shell: tapping a sub-route tab navigates without overflow', async ({ page }) => {
  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.rv-tabs')).toBeVisible({ timeout: 30_000 });
  // Tap the History sub-route tab → the History view mounts. Tabs may live in a
  // horizontally-scrollable strip, so scroll it into view before tapping.
  const histTab = page.locator('.rv-tabs .rv-tab', { hasText: 'History' });
  await histTab.scrollIntoViewIfNeeded();
  await histTab.click();
  await expect(page.locator('.history')).toBeVisible({ timeout: 20_000 });
  await expectFitsWidth(page);
});

test('shell: many open repo tabs scroll instead of overflowing the page', async ({ page }) => {
  // Open all three seeded repos as tabs, then assert the tablist owns the overflow
  // (its own scroll) while the document itself never scrolls horizontally.
  for (const id of [repoId, dirtyRepoId, conflictRepoId]) {
    await page.goto(`/#/git/${id}/graph`);
    await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
  }
  await expect(page.locator('.git-tab')).toHaveCount(3, { timeout: 15_000 });
  // New-tab affordance still reachable even with several tabs open.
  await expectElemFits(page, '.git-tab-new');
  await expectFitsWidth(page);
});

// ────────────────────────────────────────────────────────────────────────────
// CHANGES — staging / commit flow on a dirty working tree
// ────────────────────────────────────────────────────────────────────────────

test('changes: dirty tree shows file rows + a touch-friendly commit composer', async ({ page }) => {
  await page.goto(`/#/git/${dirtyRepoId}/changes`);
  await expect(page.locator('.changes.mobile')).toBeVisible({ timeout: 25_000 });
  // The seeded dirty tree (1 modified + 1 untracked) yields real change rows.
  await expect(page.locator('.changes .cs-name').first()).toBeVisible({ timeout: 15_000 });
  expect(await page.locator('.changes .cs-name').count()).toBeGreaterThanOrEqual(2);
  await expectFitsWidth(page);

  // The commit textarea uses ≥16px on mobile so iOS Safari doesn't auto-zoom on
  // focus (the regression the Changes polish pass guards).
  const fs = await page.locator('.changes.mobile .composer textarea').evaluate(
    (el) => parseFloat(getComputedStyle(el).fontSize),
  );
  expect(fs, 'mobile commit textarea must be ≥16px (no iOS zoom)').toBeGreaterThanOrEqual(16);

  // The commit button is a comfortable touch target.
  const box = await page.locator('.changes.mobile .composer .btn.primary').first().boundingBox();
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
// CONFLICT RESOLVER — a repo left mid-merge (seedConflictRepo). On mobile the
// 3-pane resolver collapses to a single column, the conflicted-files strip is a
// collapsible accordion, the hunk renders STACKED (not side-by-side), and the
// long conflicting lines wrap so the code stays reachable.
// ────────────────────────────────────────────────────────────────────────────

/** The conflict hunk's own code cells (stacked ours/theirs, base, editor) must
 *  all be reachable within the viewport — i.e. long lines wrap, not clip. */
async function expectConflictCodeReachable(page: Page): Promise<void> {
  const m = await page.evaluate(() => {
    const cells = Array.from(
      document.querySelectorAll(
        '.resolver .stack-code, .resolver .base-code, .resolver .split-table .code, .resolver .edit-editor, .resolver .context',
      ),
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
  // The in-progress merge is detected asynchronously → the "Resolve conflicts"
  // tab appears. Tap it to open the resolver.
  const tab = page.locator('.rv-tab.conflict-tab');
  await expect(tab).toBeVisible({ timeout: 20_000 });
  await tab.click();
  await expect(page.locator('.resolver')).toBeVisible({ timeout: 15_000 });
}

test('conflict: resolver renders, stacks to one column, and fits', async ({ page }) => {
  await openResolver(page);
  await expect(page.locator('.resolver.mobile')).toBeVisible();
  // The body stacks vertically on mobile (no side-by-side panes).
  const dir = await page.locator('.resolver .resolver-body').evaluate(
    (el) => getComputedStyle(el).flexDirection,
  );
  expect(dir, 'resolver must stack to a single column on mobile').toBe('column');
  // The conflicted file is auto-selected; its pane + a stacked hunk render.
  await expect(page.locator('.resolver .file-row').first()).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.resolver .file-detail .pane').first()).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.resolver .stack-sides').first()).toBeVisible({ timeout: 15_000 });
  // Side-by-side table must NOT be used on mobile (it would overflow).
  expect(await page.locator('.resolver .split-table').count()).toBe(0);
  await expectFitsWidth(page);
  await expectConflictCodeReachable(page);
});

test('conflict: conflicted-files strip is a collapsible accordion', async ({ page }) => {
  await openResolver(page);
  const head = page.locator('.resolver.mobile .mob-sec-head').first();
  await expect(head).toBeVisible({ timeout: 10_000 });
  // Files strip starts open; tapping the header collapses it.
  await expect(page.locator('.resolver .files-panel')).toBeVisible();
  await head.click();
  await expect(page.locator('.resolver .files-panel.mob-collapsed')).toHaveCount(1);
  await expectFitsWidth(page);
});
