import { test, expect } from '@playwright/test';
import { apiCtx } from './seed';
import { expectFullyInViewport } from './helpers';
import { remoteSubject, seedCardRepo, stubForge, type CardRepo } from './git-commit-card-fixture';

// The Agents right panel's Git tab is the Changes view with the commit card
// on top (P5): a session working in a dirty checkout opens the panel and
// commits and pushes from the card (a local bare remote — nothing leaves the
// machine); "Open in Git" goes to the full page; nothing git is read while
// the panel is closed.

test.use({ serviceWorkers: 'block' });
const SHOTS = process.env.OTTO_E2E_SHOTS;
let repo: CardRepo;
let wsId = '';
let sid = '';
let helperId = '';

test.beforeAll(async () => {
  repo = await seedCardRepo('e2e-agents-git-panel');
  const { ctx, base } = await apiCtx();
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Git Panel WS', root_path: repo.dir } });
  expect(w.ok(), await w.text()).toBeTruthy();
  wsId = (await w.json()).id as string;
  const s = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, { data: { kind: 'agent', provider: 'shell', title: 'Rivera', cwd: repo.dir, meta: { origin: 'e2e' } } });
  expect(s.ok(), await s.text()).toBeTruthy();
  sid = (await s.json()).id as string;
  // A session working under Rivera: the details card lists it as a subagent.
  const h = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, { data: { kind: 'agent', provider: 'shell', title: 'Rivera helper', cwd: repo.dir, meta: { origin: 'e2e', parent_session_id: sid } } });
  expect(h.ok(), await h.text()).toBeTruthy();
  helperId = (await h.json()).id as string;
  await ctx.dispose();
});

// The details card heading the panel is facts + doors on Agents: no Commit row
// over the commit card, Changes opens the panel's Git tab, a subagent opens
// as a pane here — and none of it touches the Threads page's saved inspector.
test('the details card on Agents: Changes opens the Git tab, a subagent opens here, no second Commit', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await stubForge(page);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
  await page.goto(`/#/agents/${sid}`);
  await expect(page.locator(`[data-pane-key][data-session="${sid}"]`)).toBeVisible({ timeout: 30_000 });
  await page.locator('.rstrip').getByRole('button', { name: 'Files', exact: true }).click();
  const card = page.getByTestId('agents-details-card');
  await expect(card.getByTestId('details-changes')).toBeVisible({ timeout: 20_000 });
  await expect(card.getByTestId('details-branch')).toBeVisible();
  await expect(card.getByTestId('details-changes-files').or(card.locator('.dc-add'))).toBeVisible({ timeout: 15_000 });
  // The commit card owns Commit → Push → PR: the details card has no second one.
  await expect(card.getByTestId('details-git-action')).toHaveCount(0);
  await expect(card.getByTestId('details-git-menu')).toHaveCount(0);
  await card.getByTestId('details-changes').click();
  await expect(page.getByTestId('commit-card')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.rstrip').getByRole('button', { name: 'Git', exact: true })).toHaveAttribute('aria-pressed', 'true');

  await card.getByTestId('details-subagent').filter({ hasText: 'Rivera helper' }).getByRole('button').first().click();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#/agents/${helperId}`);
  await expect(page.locator(`[data-pane-key][data-session="${helperId}"]`)).toBeVisible({ timeout: 20_000 });
  expect(await page.evaluate(() => localStorage.getItem('otto_threads_v1')), 'the Threads inspector state is untouched').toBeNull();
});

test('a dirty session folder: the Git tab is Changes + the commit card; Commit and push from it', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await stubForge(page);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
  const gitReads: string[] = [];
  page.on('request', (r) => { if (/\/api\/v1\/repos\/[^/]+\/(status|log|diff)/.test(r.url())) gitReads.push(r.url()); });
  await page.goto(`/#/agents/${sid}`);
  await expect(page.locator(`[data-pane-key][data-session="${sid}"]`)).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle');
  expect(gitReads, 'no git reads while the panel is closed').toEqual([]);

  await page.locator('.rstrip').getByRole('button', { name: 'Git', exact: true }).click();
  const card = page.getByTestId('commit-card');
  await expect(card).toBeVisible({ timeout: 20_000 });
  await expectFullyInViewport(page, card, 'commit card');
  await expect(card.getByTestId('commit-card-included')).toHaveText('2 files included');
  await expect(card.locator('.subject-input')).toHaveValue('feat: ship the commit card', { timeout: 15_000 });
  await expect(card.locator('.btn.primary')).toHaveCount(1);
  // Every action sits inside the narrow card (⋯ overflowed it, clipped to a
  // stray dot), and no row holds a lone icon (two files in one folder: no
  // "By folder" toggle, so no header row at all).
  const [cb, mb] = [(await card.boundingBox())!, (await card.getByRole('button', { name: 'Commit options' }).boundingBox())!];
  expect(mb.x).toBeGreaterThanOrEqual(cb.x);
  expect(mb.x + mb.width).toBeLessThanOrEqual(cb.x + cb.width);
  await expect(page.locator('.wip-panel .wp-head')).toHaveCount(0);
  // The whole Git page is gone from the panel: no graph / PR tab set.
  await expect(page.locator('.rv-tab')).toHaveCount(0);
  if (SHOTS) for (const s of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: s });
    await expect(page.locator('html')).toHaveAttribute('data-scheme', s);
    await page.screenshot({ path: `${SHOTS}/agents-git-panel-1440-${s}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });

  // A file's diff on click.
  await page.locator('.wip-panel').getByText('app.txt', { exact: true }).click();
  await expect(page.locator('.wip-panel .wp-diff')).toBeVisible({ timeout: 15_000 });
  await card.locator('.wc-commit').click();
  await expect.poll(() => remoteSubject(repo), { timeout: 20_000 }).toBe('feat: ship the commit card');
  await expect(page.getByTestId('commit-card-primary')).toHaveAttribute('data-kind', 'create-pr', { timeout: 15_000 });

  // The folder shows once (the details card); its Open in › Git goes to the full page.
  await expect(page.getByTestId('git-panel-open')).toHaveCount(0);
  await expect(page.locator('.rpanel-folder')).toHaveCount(0);
  await page.getByTestId('agents-details-card').getByTestId('details-open-in').click();
  await page.getByRole('menuitem', { name: 'Git', exact: true }).click();
  await expect.poll(() => page.evaluate(() => location.hash)).toMatch(/^#\/git\//);
});
