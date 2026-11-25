import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { expectFullyInViewport, openPage } from './helpers';
import { expectWipFirst, remoteSubject, seedCardRepo, stubForge, type CardRepo } from './git-commit-card-fixture';

// WIP first (G1): a dirty repository opens on History with its WIP row
// selected and the commit card on top — every file included, the message
// drafted — and ONE primary that moves Commit and push → Create pull request
// → Open pull request (the pushed HEAD commit's next step). Open → read → one click.

const NAME = 'e2e-commit-card';
// The forge is stubbed with page.route: keep the app's service worker out
// of the way (on WebKit its fetches bypass the routes).
test.use({ serviceWorkers: 'block' });

test('dirty repo: History on the WIP row → Commit and push → Create pull request', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  // Seeded here, not in beforeAll: every worker of this file runs beforeAll,
  // and a second repo under the same name made the picker open the wrong one.
  const repo: CardRepo = await seedCardRepo(NAME);
  await page.setViewportSize({ width: 1440, height: 900 });
  let open: unknown[] = [];
  await stubForge(page, () => open);
  await openPage(page, 'git');
  await page.locator('.git-tab-new').click();
  const menu = page.locator('.ctx-menu');
  await menu.locator('.ctx-search-input').fill(NAME);
  await menu.getByRole('menuitem', { name: NAME }).first().click();

  await expectWipFirst(page);
  const card = page.getByTestId('commit-card');
  await expectFullyInViewport(page, card, 'commit card');
  // The file count is said by the WIP row and the card's caption only.
  await expect(page.getByTestId('git-state')).not.toContainText('files changed');
  await expect(page.locator('.wip-panel .wp-count')).toHaveCount(0);
  // Repository tabs carry words, never a bare status dot.
  await expect(page.locator('.git-tab-dot')).toHaveCount(0);
  // The first file's diff is open beside the list.
  await expect(page.locator('.wip-panel .wp-diff')).toBeVisible();
  // A one-file diff: its file header names it with its counts, the toolbar does not repeat them (review 2, N5).
  await expect(page.locator('.wip-panel .wp-diff .dfile-head').first()).toBeVisible();
  await expect(page.locator('.wip-panel .wp-diff .diff-stats')).not.toContainText(/file|\+1/);
  if (process.env.OTTO_E2E_SHOTS) for (const s of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: s });
    await expect(page.locator('html')).toHaveAttribute('data-scheme', s);
    await page.screenshot({ path: `${process.env.OTTO_E2E_SHOTS}/git-changes-1440-${s}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
  // One primary; Commit only is the secondary.
  await expect(card.locator('.btn.primary')).toHaveCount(1);
  await expect(card.locator('.wc-commit')).toHaveText(/Commit and push/);
  await expect(card.locator('.wc-commit-only')).toHaveText('Commit only');

  // One click: committed AND pushed to the remote.
  await card.locator('.wc-commit').click();
  await expect.poll(() => remoteSubject(repo), { timeout: 20_000 }).toBe('feat: ship the commit card');

  // History selects the new commit, and offers the next step: the pull request.
  await expect(page.locator('.detail-subject')).toHaveText('feat: ship the commit card', { timeout: 15_000 });
  const next = page.getByTestId('commit-card-primary');
  await expect(next).toHaveAttribute('data-kind', 'create-pr', { timeout: 15_000 });
  await expect(next).toHaveText(/Create pull request/);
  await next.click();
  const sheet = page.getByRole('dialog', { name: 'New pull request' });
  await expect(sheet).toBeVisible();
  await sheet.getByRole('button', { name: 'Cancel' }).click();

  // Someone else opens the PR (an agent, the forge) — no reload: back on
  // History the card reads it again and offers it (it used to keep "Create").
  open = [{ number: 42, title: 'Ship the commit card', state: 'open', source_branch: repo.branch, target_branch: 'main', url: 'https://github.com/acme/x/pull/42' }];
  await page.locator('.rv-tab', { hasText: 'Pull requests' }).click();
  await page.locator('.rv-tab', { hasText: 'History' }).click();
  await expect(next).toHaveAttribute('data-kind', 'pr', { timeout: 15_000 });
  // Merged meanwhile: no open PR any more, the card follows.
  open = [];
  await page.locator('.rv-tab', { hasText: 'Pull requests' }).click();
  await page.locator('.rv-tab', { hasText: 'History' }).click();
  await expect(next).toHaveAttribute('data-kind', 'create-pr', { timeout: 15_000 });
  // With a PR open for the branch the primary opens it (Pull requests view).
  // A reload lands on History's checked-out commit with that step.
  open = [{ number: 42, title: 'Ship the commit card', state: 'open', source_branch: repo.branch, target_branch: 'main', url: 'https://github.com/acme/x/pull/42' }];
  await page.reload();
  await expect(page.locator('.rv-tab[aria-selected="true"]')).toContainText('History', { timeout: 15_000 });
  await expect(next).toHaveAttribute('data-kind', 'pr', { timeout: 15_000 });
  await expect(next).toHaveText(/Open pull request/);
  await next.click();
  await expect(page.locator('.rv-tab[aria-selected="true"]')).toContainText('Pull requests');
});

// A clean default branch has no pull-request step: History must not mount
// the step card there (each mount asks the forge for accounts and open PRs).
test('clean main: History opens on HEAD without asking the forge', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const r = await seedCardRepo('e2e-card-clean-main');
  execFileSync('git', ['-C', r.dir, 'stash', 'push', '-q', '-u']);
  execFileSync('git', ['-C', r.dir, 'checkout', '-q', 'main']);
  await stubForge(page);
  const forge: string[] = [];
  page.on('request', (q) => { if (/\/api\/v1\/(git\/accounts|repos\/[^/]+\/prs\?)/.test(q.url())) forge.push(q.url()); });
  await openPage(page, 'git/' + r.id);
  await expect(page.locator('.detail-subject')).toHaveText('init', { timeout: 20_000 });
  await page.waitForTimeout(1500); // ui-guards: allow — a step card would ask the forge now
  expect(forge).toEqual([]);
  await expect(page.getByTestId('commit-card-next')).toHaveCount(0);
});

// "Commit only" on a branch with no upstream: Publish is the next step, so
// the header's Publish carries the accent (it stayed a plain verb).
test('unpublished branch: after Commit only, Publish is the primary', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  const r = await seedCardRepo('e2e-card-unpublished');
  execFileSync('git', ['-C', r.dir, 'branch', '--unset-upstream']);
  await stubForge(page);
  await openPage(page, 'git/' + r.id);
  await expectWipFirst(page);
  await page.getByTestId('commit-card').locator('.wc-commit-only').click();
  await expect(page.locator('.wip-row')).toHaveCount(0, { timeout: 15_000 });
  await expect(page.locator('.git-header .ph-actions').getByRole('button', { name: /^Publish/ })).toHaveClass(/primary/, { timeout: 15_000 });
});

// 1100 px with the right panel open: the diff's layout switch wraps into
// view, and the branches stay one click away while the refs column steps aside.
test('narrow pane: the diff toolbar fits and the branches peek in', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript(() => { localStorage.setItem('otto_right_open', '1'); localStorage.setItem('otto_right_tab', 'git'); localStorage.setItem('otto_right_width', '300'); });
  await page.setViewportSize({ width: 1100, height: 900 });
  const r = await seedCardRepo('e2e-card-narrow');
  await stubForge(page);
  await openPage(page, 'git/' + r.id);
  await expect(page.locator('.wip-row')).toHaveAttribute('aria-pressed', 'true', { timeout: 20_000 });
  await expect(page.locator('.wp-diff')).toBeVisible({ timeout: 15_000 });
  await expectFullyInViewport(page, page.locator('.wp-diff').getByRole('button', { name: 'Side by side' }), 'diff layout switch');
  await expect(page.locator('.refs-panel')).toBeHidden();
  // The branch button sits clear of the column heading and, once open, of the
  // refs list's first heading (it overlapped "BRANCH / TAG" and "LOCAL").
  const peek = page.getByRole('button', { name: 'Branches & tags' });
  const textBox = (sel: string, text: string) => page.evaluate(([s, t]) => {
    const el = [...document.querySelectorAll(s)].find((e) => e.textContent?.trim().startsWith(t))!;
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    let n: Node | null; while ((n = walker.nextNode()) && !n.textContent?.trim()) { /* first text */ }
    const r = document.createRange(); r.selectNodeContents(n!); const b = r.getBoundingClientRect();
    return { x: b.x, y: b.y, width: b.width, height: b.height };
  }, [sel, text] as const);
  const apart = (a: { x: number; y: number; width: number; height: number }, b: typeof a) =>
    a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y;
  expect(apart((await peek.boundingBox())!, await textBox('.gh-branch', 'BRANCH')), 'peek vs BRANCH / TAG').toBe(true);
  await peek.click();
  await expect(page.locator('.refs-panel')).toBeVisible();
  expect(apart((await peek.boundingBox())!, await textBox('.refs-panel *', 'LOCAL')), 'peek vs LOCAL').toBe(true);
  await expect(page.locator('.refs-panel')).toContainText(r.branch.split('/').pop()!);
});
