import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// PR workspace (desktop-browser, MOCK mode — ui/src/lib/api/mock-prs.ts):
// list | open-PR tabs, the T3-style header, Summary / Timeline / Code, the
// Code tab's hide-tests / viewed / tree / commit picker, and opening several
// PRs together from the list. Same UI for GitHub, Bitbucket and GitLab —
// everything goes through the provider-neutral /repos/{id}/prs/* routes.
// ─────────────────────────────────────────────────────────────────────────────

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  });
  await page.setViewportSize({ width: 1600, height: 950 });
});

test('a PR deep link opens in the workspace with header, stats and timeline', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prw')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.prd-title')).toContainText('VIP program');
  // The route is handed back to the tab store; the PR stays open as a tab.
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/git');
  await expect(page.locator('.prw-tab')).toHaveCount(1);
  await expect(page.locator('.prd-stats')).toContainText('23 files');
  await expect(page.locator('.prd-actions').getByRole('button', { name: 'Merge', exact: true })).toBeVisible();
  // Grouped list: my PRs first.
  await expect(page.locator('.prlist')).toContainText('Authored by you');

  await page.getByRole('tab', { name: 'Timeline', exact: true }).click();
  await expect(page.locator('.tlw')).toContainText('opened this pull request');
  await expect(page.locator('.tlw')).toContainText('pushed 1 commit');
  await expect(page.locator('.prd-code')).toBeHidden();
  await expectNoHorizontalOverflow(page);
});

test('Code tab: hide tests, viewed marks persist, tree, one commit', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
  const heads = page.locator('.dfile-head');
  await expect(heads).toHaveCount(23);
  // >5 files: every file starts collapsed, like a reviewable file list.
  await expect(heads.first()).toHaveAttribute('aria-expanded', 'false');

  // Tests across languages (.spec.ts, _test.go, tests/test_*.py, src/test/…Test.java).
  await page.getByRole('button', { name: 'Hide test files' }).click();
  await expect(heads).toHaveCount(17);
  await expect(page.locator('.diff-root')).not.toContainText('levels_test.go');

  // Viewed: ticking folds the file and survives a reload.
  const viewed = page.locator('.prd-code').getByRole('checkbox', { name: /viewed/i }).first();
  await viewed.check();
  await expect(page.locator('.diff-root')).toContainText('1 / 17 viewed');
  await page.reload();
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
  await expect(page.locator('.diff-root')).toContainText('1 / 17 viewed');

  // The diff toolbar stays pinned while the PR pane scrolls a long diff.
  await page.getByRole('button', { name: /expand all/i }).first().click();
  const tb = page.locator('.diff-toolbar.pr-toolbar');
  const paneTop = (await page.locator('.prd-body').boundingBox())!.y;
  await page.locator('.prd-body').evaluate((el) => { el.scrollTop += 2000; });
  await expect.poll(async () => Math.round((await tb.boundingBox())!.y - paneTop)).toBe(0);
  await expect(tb.getByRole('textbox', { name: /search/i })).toBeInViewport();

  await page.getByRole('button', { name: 'Show file tree' }).click();
  await expect(page.locator('.diff-nav .nav-file').first()).toBeVisible();

  // One commit: the picker swaps the diff to that commit's files.
  await page.locator('.prd-commit-pick').first().click();
  await page.locator('.ctx-menu').getByText('active pins page').click();
  await expect(page.locator('.prd-commit-pick').first()).toContainText('active pins page');
  await expect(heads.first()).toBeVisible();
  expect(await heads.count()).toBeLessThan(17);
});

test('Files panel under the list: a tree that jumps into Code and shares Viewed marks', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const panel = page.getByRole('region', { name: 'Files in pull request #51' });
  await expect(panel).toContainText('0/23 viewed');
  // A real tree: folders fold their files away.
  const dir = panel.locator('.nav-dir-row').first();
  await expect(dir).toHaveAttribute('aria-expanded', 'true');
  const before = await panel.locator('.nav-file').count();
  await dir.click();
  await expect(dir).toHaveAttribute('aria-expanded', 'false');
  expect(await panel.locator('.nav-file').count()).toBeLessThan(before);
  await dir.click();

  // A file click brings the Code tab forward and jumps to that file.
  const first = panel.locator('.nav-file').first();
  const path = (await first.getAttribute('title')) ?? '';
  await first.locator('.nav-file-open').click();
  await expect(page.getByRole('tab', { name: 'Code', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator(`.prd-code .dfile-head[title="${path}"], .prd-code .dfile-head:has-text("${path.split('/').pop()}")`).first()).toBeInViewport();

  // Ticking in the panel is the diff's Viewed mark (and the other way round).
  await first.getByRole('checkbox').check();
  await expect(panel).toContainText('1/23 viewed');
  await expect(page.locator('.diff-root')).toContainText('1 / 23 viewed');
  // Hide viewed drops it from the tree once the reader moves on.
  await panel.getByRole('button', { name: 'Next unviewed file', exact: true }).click();
  await panel.getByRole('button', { name: 'Hide viewed files' }).click();
  await expect(panel.locator(`.nav-file[title="${path}"]`)).toHaveCount(0);
});

test('Approve / Request changes live in the header and update at once', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const head = page.locator('.prd-actions');
  const side = page.getByRole('complementary', { name: 'Pull request details' });
  const codeTab = page.getByRole('tab', { name: 'Code', exact: true });

  // Request changes from the Code tab: the composer opens over the diff, and
  // you stay on Code (it used to jump to Summary).
  await codeTab.click();
  await head.getByRole('button', { name: 'Request changes' }).click();
  const composer = page.getByRole('dialog', { name: 'Request changes on #51' });
  await composer.getByRole('textbox', { name: 'What needs to change' }).fill('Cap the retry loop.');
  await composer.getByRole('button', { name: 'Request changes' }).click();
  await page.getByRole('dialog').filter({ hasText: 'Request changes on PR #51?' }).getByRole('button', { name: 'Request changes' }).click();
  await expect(head.getByText('Changes requested')).toBeVisible();
  await expect(codeTab).toHaveAttribute('aria-selected', 'true');

  // The forge names you "Dev Otto" — no git account handle matches that (the
  // Bitbucket shape: username an email, reviewer a display name). After the
  // quiet reload only the daemon's `is_me` says it's you, and the header must
  // keep the status instead of flipping back to a button.
  await page.getByRole('tab', { name: 'Summary', exact: true }).click();
  await expect(side.getByText('Dev Otto')).toBeVisible();
  await expect(side.getByText('Changes requested')).toBeVisible();
  await expect(head.getByText('Changes requested')).toBeVisible();

  // Approve flips your opinion — on the same reviewer row, never a duplicate.
  await head.getByRole('button', { name: 'Approve', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Approve PR' }).click();
  await expect(head.getByText('Approved', { exact: true })).toBeVisible();
  await expect(head.getByText('Changes requested')).toHaveCount(0);
  await expect(side.getByText('Changes requested')).toHaveCount(0);
  await expect(side.getByText('Dev Otto')).toHaveCount(1);
});

test('is:mine trusts the forge’s own "authored by you", not a name match', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  // #49's author is "Dev Otto": no account handle matches, `is_mine` does.
  await page.locator('.pr-search').fill('is:mine');
  await expect(page.locator('.pr-row')).toHaveCount(2);
  await expect(page.locator('.pr-row').filter({ hasText: 'round bonus balance' })).toHaveCount(1);
});

test('Merge after a push you have not read asks first; Reload the diff clears it', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  const prd = page.locator('.prd');
  // The diff you read was loaded at the PR's head.
  await expect(prd).toHaveAttribute('data-diff-head', /^c1fac680/, { timeout: 15_000 });
  // Someone pushes while you're still on the page.
  await page.evaluate(() => localStorage.setItem('otto_mock_pr_pushed', '51'));

  await page.locator('.prd-actions').getByRole('button', { name: 'Merge', exact: true }).click();
  const modal = page.getByRole('dialog', { name: 'Merge pull request' });
  const warn = modal.getByTestId('pr-merge-unseen');
  await expect(warn).toContainText('1 new commit since you opened this diff');
  const mergeBtn = modal.getByRole('button', { name: 'Merge pull request', exact: true });
  // The mock's CI is failing too: "Merge anyway" alone does not release Merge.
  await modal.getByText('Merge anyway').click();
  await expect(mergeBtn).toBeDisabled();
  await warn.getByText('Merge including changes I have not reviewed').click();
  await expect(mergeBtn).toBeEnabled();
  await warn.getByText('Merge including changes I have not reviewed').click();
  await expect(mergeBtn).toBeDisabled();

  // Reload the diff: the dialog closes and the diff is re-read at the new head…
  await warn.getByRole('button', { name: 'Reload the diff' }).click();
  await expect(modal).toBeHidden();
  await expect(prd).toHaveAttribute('data-diff-head', /^c7e57a11/);
  // …so Merge no longer warns: "Merge anyway" (for the failing CI) is enough
  // once the dialog has read the live head.
  await page.locator('.prd-actions').getByRole('button', { name: 'Merge', exact: true }).click();
  await modal.getByText('Merge anyway').click();
  await expect(mergeBtn).toBeEnabled();
  await expect(modal.getByTestId('pr-merge-unseen')).toHaveCount(0);
});

test('PR header never overlaps: repo · #n · state stay whole as the window narrows', async ({ page }) => {
  for (const width of [1600, 1440, 1280, 1100]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/#/git/rep_otto/pr/51');
    await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
    const boxes = await page.evaluate(() => {
      const r = (sel: string) => document.querySelector(sel)!.getBoundingClientRect();
      return { num: r('.prd-num'), state: r('.prd-state'), actions: r('.prd-actions'), slug: r('.prd-slug') };
    });
    const overlap = (a: DOMRect, b: DOMRect) => a.left < b.right - 1 && b.left < a.right - 1 && a.top < b.bottom - 1 && b.top < a.bottom - 1;
    expect(overlap(boxes.num, boxes.state), `#n over the state chip at ${width}px`).toBe(false);
    expect(overlap(boxes.state, boxes.actions), `state chip under the actions at ${width}px`).toBe(false);
    expect(boxes.slug.width, `repo name visible at ${width}px`).toBeGreaterThan(40);
    for (const name of ['Request changes', 'Approve', 'Merge']) {
      await expect(page.locator('.prd-actions').getByRole('button', { name, exact: true })).toBeInViewport({ ratio: 1 });
    }
  }
});

test('select several PRs in the list and open them together as tabs', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.prw-tab')).toHaveCount(1);
  await page.getByRole('checkbox', { name: 'Select #42' }).check();
  await page.getByRole('checkbox', { name: 'Select #50' }).check();
  await page.getByRole('button', { name: /Open 2 together/ }).click();
  await expect(page.locator('.prw-tab')).toHaveCount(3);
  await expect(page.locator('.prw-tab.active')).toContainText('#42');
  // Close the front tab: the neighbour comes forward.
  await page.getByRole('button', { name: 'Close #42' }).click();
  await expect(page.locator('.prw-tab')).toHaveCount(2);
  // Search qualifiers narrow the list.
  await page.locator('.pr-search').fill('label:bug');
  await expect(page.locator('.pr-row')).toHaveCount(1);
});

test('⋯ menu offers merge strategies, agent sessions and copy actions', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'More pull request actions' }).click();
  const menu = page.locator('.ctx-menu');
  for (const label of ['Ask a question', 'Explain this PR', 'Squash and merge', 'Rebase and merge', 'Copy link', 'Open on GitHub']) {
    await expect(menu.getByText(label, { exact: true })).toBeVisible();
  }
  await menu.getByText('Squash and merge', { exact: true }).click();
  const modal = page.getByRole('dialog', { name: 'Merge pull request' });
  await expect(modal).toBeVisible();
  await expect(modal.locator('select')).toHaveValue('squash');
});

test('your review status withdraws from its own menu', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const head = page.locator('.prd-actions');
  const side = page.getByRole('complementary', { name: 'Pull request details' });
  await head.getByRole('button', { name: 'Approve', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Approve PR' }).click();
  // The status is a button now; its menu withdraws (asks first).
  await head.getByRole('button', { name: /^Approved/ }).click();
  await page.locator('.ctx-menu').getByText('Withdraw approval…').click();
  const dlg = page.getByRole('dialog', { name: 'Withdraw your approval of PR #51?' });
  await dlg.getByRole('button', { name: 'Withdraw approval' }).click();
  await expect(head.getByRole('button', { name: 'Approve', exact: true })).toBeVisible();
  await expect(page.locator('.toast-title').filter({ hasText: 'Approval withdrawn' })).toBeVisible();
  // The quiet reload agrees: no approval of yours in the reviewer list.
  await expect(side.locator('.sc-row').filter({ hasText: 'Dev Otto' }).getByText('Approved')).toHaveCount(0);
  await expect(head.getByText('Approved', { exact: true })).toHaveCount(0);
});

test('your own comment: edit → save, then delete after a confirm', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const threads = page.locator('.prd-comments .prd-thread');
  // Someone else's comment has no menu.
  await expect(threads.filter({ hasText: 'thresholds' }).getByRole('button', { name: 'Comment actions' })).toHaveCount(0);

  // By author, not by body: in edit mode the body is a textarea's VALUE, not text.
  const mine = threads.filter({ hasText: /Dev Otto/ });
  await mine.getByRole('button', { name: 'Comment actions' }).click();
  await page.locator('.ctx-menu').getByText('Edit', { exact: true }).click();
  const box = mine.getByRole('textbox', { name: 'Edit comment' });
  await expect(box).toHaveValue('Rebased on develop — ready for another look.');
  await box.fill('Rebased on develop — please re-review.');
  await mine.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(box).toHaveCount(0);
  await expect(mine).toContainText('please re-review');

  await mine.getByRole('button', { name: 'Comment actions' }).click();
  await page.locator('.ctx-menu').getByText('Delete…').click();
  const dlg = page.getByRole('dialog', { name: 'Delete this comment?' });
  await expect(dlg).toContainText('removed for everyone');
  await expect(dlg).toContainText('PR #51');
  await dlg.getByRole('button', { name: 'Delete comment' }).click();
  await expect(page.locator('.toast-title').filter({ hasText: 'Comment deleted' })).toBeVisible();
  await expect(threads.filter({ hasText: 'Rebased on develop' })).toHaveCount(0);
});

test('reactions toggle, review summaries wear their verdict, replies name who they answer', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const threads = page.locator('.prd-comments .prd-thread');
  const thread = threads.filter({ hasText: 'thresholds' });

  // Tap a pill: yours is added; tap again: taken back.
  await thread.getByRole('button', { name: 'Thumbs up: 2 reactions' }).click();
  const mine = thread.getByRole('button', { name: 'Thumbs up: 3 reactions, including yours' });
  await expect(mine).toHaveAttribute('aria-pressed', 'true');
  await mine.click();
  await expect(thread.getByRole('button', { name: 'Thumbs up: 2 reactions' })).toHaveAttribute('aria-pressed', 'false');
  // Add one from the picker.
  await thread.getByRole('button', { name: 'Add a reaction' }).first().click();
  await page.locator('.ctx-menu').getByText('Hooray').click();
  await expect(thread.getByRole('button', { name: 'Hooray: 1 reaction, including yours' })).toBeVisible();

  // A Bitbucket-style reply to a reply says whom it answers.
  await expect(thread).toContainText('↪ replying to dev-otto');

  // A GitHub review summary: verdict chip, Reply, no edit/delete/reactions.
  const review = threads.filter({ hasText: 'handles its errors' });
  await expect(review.locator('.cmt-review')).toHaveText('Approved');
  await expect(review.getByRole('button', { name: 'Reply', exact: true })).toBeVisible();
  await expect(review.getByRole('button', { name: 'Comment actions' })).toHaveCount(0);
  await expect(review.getByRole('button', { name: 'Add a reaction' })).toHaveCount(0);
});

test('merge checks and open tasks show in the side column and as merge reasons', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const checks = page.getByRole('region', { name: 'Merge checks' });
  await expect(checks).toContainText('1 of 3 checks passed');
  await expect(checks).toContainText('At least 2 approvals');
  await expect(checks).toContainText('1 of 2 approvals');
  await expect(checks).toContainText('1 task open');
  // The compact header line stands in for the side column only where that
  // column has dropped below the conversation (a pane under 860px).
  const line = page.getByTestId('pr-merge-checks-line');
  const paneWidth = (): Promise<number> => page.locator('.prd-head').evaluate((el) => el.clientWidth - 32);
  await page.setViewportSize({ width: 2200, height: 950 });
  await expect.poll(paneWidth).toBeGreaterThan(860);
  await expect(line).toBeHidden();
  await page.setViewportSize({ width: 1300, height: 950 });
  await expect.poll(paneWidth).toBeLessThan(860);
  await expect(line).toHaveText('1 of 3 checks passed · 1 task open');
  await page.setViewportSize({ width: 1600, height: 950 });

  await page.locator('.prd-actions').getByRole('button', { name: 'Merge', exact: true }).click();
  const reasons = page.getByRole('dialog', { name: 'Merge pull request' }).locator('.reasons');
  await expect(reasons).toContainText('At least 2 approvals');
  await expect(reasons).toContainText('1 task open');
  // The failing build check is the CI row's job, not repeated.
  await expect(reasons).not.toContainText('All builds passed');
});
